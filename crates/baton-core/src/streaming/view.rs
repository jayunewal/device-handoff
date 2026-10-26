// SPDX-License-Identifier: Apache-2.0

//! The viewer's side of a window streaming session.
//!
//! The viewer takes a frame of video from each stream that the dispatcher hands it, reads the
//! picture into memory of its own, and gives it to `baton_stream::viewer::Viewer`, which says what
//! to do next: decode this frame, show that one, forget the others, ask for a key frame. The
//! decoder and the screen are the platform's, behind [`Decoder`].
//!
//! The viewer also has the control streams of the session. It reads the source's, which carry the
//! picture format and the source's notices, and writes its own, which carries a report a few times
//! a second, what it wants, and the requests for a key frame. It opens its own after it has taken
//! the source's: the source made its side of the session before it opened the first stream, so a
//! stream that the viewer opens then is never one for a session that is not there yet.
//!
//! When the person at the source granted control, the same stream carries what the person does on
//! the viewer. It is given to [`ViewHandle::input`], which tags a position with the frame that is
//! on the screen and hands the event to the queue of `baton_stream::input_queue`. That keeps the
//! limits of the protocol (a batch, the rate, what a position may be) and decides what is sent. A
//! task of the session takes a batch out of the queue as soon as the stream to the source has room
//! for it, so that what waits behind a stream that is full is joined and not piled up. A path that
//! is only slow does not fill the stream: its batches wait in the transport and arrive late, and
//! the source drops what has gone out of date.
//!
//! A session ends when the source ends it, when it breaks a rule, when the connection ends, when
//! the person ends it, when the source does not open its control stream in time, and when a message
//! of the source stalls ([`End`]). It ends once: the decoder is told, the pictures that are held
//! are dropped, and later streams of the session are stopped with code 499. The decoder is told
//! nothing after that, whatever was on its way.

use std::{
    collections::{HashMap, VecDeque},
    ops::ControlFlow,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{DeviceId, close_code, id::SessionId};
use baton_stream::{
    KIND_VIDEO,
    control::{KeyFrame, Report, SourceMessage, Thermal, ViewerMessage, Want},
    header::{FrameHeader, PREFIX_LEN, encode_prefix},
    input_queue::{Input, Queue, Refused},
    viewer::{Action, Params as Rules, Refusal, Stats, StreamId, Viewer, Violation},
};
use iroh::endpoint::{Connection, ReadExactError, RecvStream};
use tokio::{
    sync::{Notify, mpsc},
    time::MissedTickBehavior,
};
use tokio_util::sync::CancellationToken;

use super::{
    ControlEnd, End, Streaming, Takes, Viewing, WriteEnd, read_control, stop, write_control,
};
use crate::{trust::PairedDevice, uni::Arrival, util::lock};

/// How much of a picture to ask the transport for at a time.
const READ_CHUNK: usize = 64 * 1024;

/// How many messages to the source wait to be written. A message that finds the queue full is
/// dropped: a report is replaced by the next one, a request for a key frame is repeated, and a
/// source that reads nothing for this long is not going to read them.
const OUTBOX: usize = 64;

/// The time limits and sizes of the viewer's side of a session. The defaults are where ADR 0009
/// starts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewParams {
    /// The rules for choosing the frame to decode, and what the viewer holds at most.
    pub(crate) rules: Rules,
    /// How long a stream may deliver nothing before it is stopped with 408.
    pub(crate) stall: Duration,
    /// How long between two reports.
    pub(crate) report_every: Duration,
    /// How often the viewer looks at the clock, to ask again for a key frame that did not come.
    pub(crate) tick_every: Duration,
    /// How long the source has to open its control stream once the session has been made.
    pub(crate) control_within: Duration,
}

impl Default for ViewParams {
    fn default() -> Self {
        Self {
            rules: Rules::default(),
            stall: Duration::from_secs(10),
            report_every: Duration::from_millis(500),
            tick_every: Duration::from_millis(100),
            control_within: Duration::from_secs(10),
        }
    }
}

/// The decoder refused a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the decoder refused the frame")]
pub struct DecodeError;

/// How the platform's decoder is doing, for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecoderState {
    /// How long the decoder takes for a frame, in microseconds, as it smooths it.
    pub decode_us: u32,
    /// How hot the device is.
    pub thermal: Thermal,
}

/// The decoder and the screen of a session, which the platform provides.
///
/// Every method is called with the session's lock held, so none of them waits: a decoder that
/// takes time hands the picture to a thread of its own, and says if it fails afterwards through
/// [`ViewHandle::decode_failed`].
pub trait Decoder: Send + 'static {
    /// Takes the picture of a frame, in the order in which the frames are to be decoded. The
    /// picture is a frame of the format that the session agreed on, which is H.264 in Annex B
    /// form. Returns an error if the decoder cannot take it at all.
    fn decode(&mut self, frame: &FrameHeader, picture: Vec<u8>) -> Result<(), DecodeError>;

    /// Shows the picture of frame `number`, the newest that was decoded.
    fn show(&mut self, number: u64);

    /// A message from the source: the picture that follows, a pause, the end of one, a notice.
    fn source(&mut self, message: SourceMessage);

    /// How the decoder is doing.
    fn state(&self) -> DecoderState;

    /// The session is over. The decoder lets go of everything it holds, and is called no more.
    fn ended(&mut self);
}

impl Decoder for Box<dyn Decoder> {
    fn decode(&mut self, frame: &FrameHeader, picture: Vec<u8>) -> Result<(), DecodeError> {
        (**self).decode(frame, picture)
    }

    fn show(&mut self, number: u64) {
        (**self).show(number);
    }

    fn source(&mut self, message: SourceMessage) {
        (**self).source(message);
    }

    fn state(&self) -> DecoderState {
        (**self).state()
    }

    fn ended(&mut self) {
        (**self).ended();
    }
}

/// What a session has counted, for the person and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ViewStats {
    /// What the rules of the viewer have counted.
    pub frames: Stats,
    /// The newest frame that was shown.
    pub shown: Option<u64>,
    /// The bytes of pictures that the viewer holds.
    pub held_bytes: usize,
    /// The frame streams that are being read.
    pub streams: usize,
}

/// A frame that waits to be decoded.
struct Picture {
    header: FrameHeader,
    bytes: Vec<u8>,
}

struct State {
    viewer: Viewer,
    /// The pictures of the frames that the viewer holds, by number: the same frames as `viewer`
    /// holds, and no others.
    pictures: HashMap<u64, Picture>,
    decoder: Box<dyn Decoder>,
    /// Whether the decoder was told that the session is over. It is set under the lock that every
    /// call to the decoder is made with, and the decoder is called no more once it is set, whatever
    /// was on its way: a frame that was whole as the session ended, a message of the source, a tick.
    closed: bool,
}

/// Why a frame stream is not read any further.
enum Stop {
    /// Stop the stream with this code. It is no fault of the source.
    Code(u32),
    /// The stream is gone already: the source reset it, or the connection went.
    Gone,
    /// The source broke a rule. The stream is stopped with 400, and the session ends.
    Violation,
}

/// The viewer's side of one session.
pub(super) struct View {
    registry: Weak<Streaming>,
    session: SessionId,
    peer: DeviceId,
    connection: Connection,
    takes: Takes,
    params: ViewParams,
    began: Instant,
    state: Mutex<State>,
    /// What the viewer says to the source, in the order in which it says it.
    outbox: mpsc::Sender<ViewerMessage>,
    /// What the person did on the viewer and has not been sent yet. Never held across a wait.
    input: Mutex<Queue>,
    /// Tells the task that sends the input that something was queued.
    input_ready: Notify,
    /// Whether the source's control stream has been taken. A second one is a violation.
    source_control: AtomicBool,
    /// Cancelled when the source's control stream has been taken.
    control_taken: CancellationToken,
    /// Cancelled when the session ends.
    ended: CancellationToken,
    end: OnceLock<End>,
}

/// What the owner of a session holds. Dropping it ends the session.
pub struct ViewHandle {
    view: Arc<View>,
}

impl std::fmt::Debug for ViewHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewHandle").finish_non_exhaustive()
    }
}

impl ViewHandle {
    /// The decoder failed on a frame after it had taken it. Nothing can be decoded until a key
    /// frame comes, and the viewer asks for one.
    pub fn decode_failed(&self) {
        let view = &self.view;
        let mut state = lock(&view.state);
        let actions = state.viewer.decode_failed(view.now());
        let consistent = view.apply(&mut state, actions);
        drop(state);
        if !consistent {
            view.end(End::Internal);
        }
    }

    /// Tells the source what the viewer can take now, for example after the phone was turned.
    pub fn want(&self, want: Want) {
        self.view.say(ViewerMessage::Want(want));
    }

    /// Whether the person at the source granted control of the window with the offer. If not,
    /// [`input`](Self::input) takes nothing: a viewer cannot ask for control, and cannot turn it
    /// on later.
    #[must_use]
    pub fn control(&self) -> bool {
        lock(&self.view.input).granted()
    }

    /// Gives what the person did on the picture. A position is tagged with the frame that is on
    /// the screen now. The event is sent soon, in a batch with others, unless it is refused:
    /// nothing is sent while no picture has been shown, for a position outside the picture when no
    /// button is down, for the release of something that is not down, and for more than the
    /// protocol allows. None of that ends the session.
    ///
    /// # Errors
    ///
    /// The reason that the event was not taken. Typed text and key codes are not in it.
    pub fn input(&self, input: Input) -> Result<(), Refused> {
        let view = &self.view;
        let shown = lock(&view.state).viewer.shown();
        lock(&view.input).push(shown, input)?;
        view.input_ready.notify_one();
        Ok(())
    }

    /// Releases every button and key that is down, as when the person leaves the screen with a
    /// finger on it. The source releases what it has pressed when the session ends, and when it
    /// hears nothing from the viewer for ten seconds, but a viewer whose screen is gone while the
    /// session goes on reports as ever, so it says so itself.
    pub fn release_input(&self) {
        lock(&self.view.input).release_all();
        self.view.input_ready.notify_one();
    }

    /// What has been counted so far.
    #[must_use]
    pub fn stats(&self) -> ViewStats {
        let state = lock(&self.view.state);
        ViewStats {
            frames: state.viewer.stats(),
            shown: state.viewer.shown(),
            held_bytes: state.viewer.held_bytes(),
            streams: state.viewer.streams(),
        }
    }

    /// Ends the session.
    pub fn end(&self) {
        self.view.end(End::Local);
    }

    /// Waits until the session is over, and says why.
    pub async fn ended(&self) -> End {
        self.view.ended.cancelled().await;
        self.view.end.get().copied().unwrap_or(End::Internal)
    }
}

impl Drop for ViewHandle {
    fn drop(&mut self) {
        self.view.end(End::Local);
    }
}

/// Moves what the person did from `input` to `outbox`, in batches, until `ended`. `now` is the
/// time on the clock of the session.
///
/// A slot in `outbox` is reserved before the batch is taken, so that what the person does while
/// the stream is full is still in the queue of the input, where moves are joined, and is not a
/// batch that is too old by the time it is written.
async fn pump_input(
    input: &Mutex<Queue>,
    ready: &Notify,
    outbox: &mpsc::Sender<ViewerMessage>,
    ended: &CancellationToken,
    now: impl Fn() -> Duration,
) {
    loop {
        let next = lock(input).next_at(now());
        let Some(at) = next else {
            tokio::select! {
                () = ended.cancelled() => return,
                () = ready.notified() => {}
            }
            continue;
        };
        // The rate leaves no room until some of the last second has gone.
        let wait = at.saturating_sub(now());
        if !wait.is_zero() {
            tokio::select! {
                () = ended.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
            }
            continue;
        }
        let slot = tokio::select! {
            () = ended.cancelled() => return,
            slot = outbox.reserve() => match slot {
                Ok(slot) => slot,
                Err(_) => return,
            },
        };
        let batch = lock(input).take(now());
        if let Some(batch) = batch {
            slot.send(ViewerMessage::Input(batch));
        }
    }
}

impl View {
    /// Makes the session and starts what runs for as long as it does.
    pub(super) fn start(
        registry: Weak<Streaming>,
        session: SessionId,
        peer: DeviceId,
        connection: Connection,
        viewing: Viewing,
        control: bool,
    ) -> (Arc<Self>, ViewHandle) {
        let Viewing {
            decoder,
            takes,
            params,
        } = viewing;
        let (outbox, queue) = mpsc::channel(OUTBOX);
        let view = Arc::new(Self {
            registry,
            session,
            peer,
            connection,
            takes,
            params,
            began: Instant::now(),
            state: Mutex::new(State {
                viewer: Viewer::new(params.rules),
                pictures: HashMap::new(),
                decoder,
                closed: false,
            }),
            outbox,
            input: Mutex::new(Queue::new(control)),
            input_ready: Notify::new(),
            source_control: AtomicBool::new(false),
            control_taken: CancellationToken::new(),
            ended: CancellationToken::new(),
            end: OnceLock::new(),
        });
        tokio::spawn(Arc::clone(&view).supervise());
        tokio::spawn(Arc::clone(&view).write_control(queue));
        tokio::spawn(Arc::clone(&view).keep_time());
        if control {
            tokio::spawn(Arc::clone(&view).send_input());
        }
        let handle = ViewHandle {
            view: Arc::clone(&view),
        };
        (view, handle)
    }

    /// The device at the other end.
    pub(super) fn peer(&self) -> DeviceId {
        self.peer
    }

    /// Whether `peer` on `connection` is the other end of this session.
    pub(super) fn belongs_to(&self, peer: &PairedDevice, connection: &Connection) -> bool {
        self.peer == peer.id && self.connection.stable_id() == connection.stable_id()
    }

    fn now(&self) -> Duration {
        self.began.elapsed()
    }

    /// Ends the session, once. The caller does not hold the session's lock.
    pub(super) fn end(&self, how: End) {
        if self.end.set(how).is_err() {
            return;
        }
        self.ended.cancel();
        if let Some(registry) = self.registry.upgrade() {
            registry.forget(&self.session);
        }
        let mut state = lock(&self.state);
        state.closed = true;
        state.pictures.clear();
        state.decoder.ended();
        drop(state);
        tracing::debug!(peer = %self.peer.short(), reason = ?how, "a window streaming session ended");
    }

    /// Queues a message for the source.
    fn say(&self, message: ViewerMessage) {
        // A full queue drops the message, see OUTBOX.
        let _ = self.outbox.try_send(message);
    }

    /// Ends the session when the connection ends, and when the source has not opened its control
    /// stream in time.
    async fn supervise(self: Arc<Self>) {
        tokio::select! {
            () = self.ended.cancelled() => return,
            _ = self.connection.closed() => {
                self.end(End::Connection);
                return;
            }
            () = self.control_taken.cancelled() => {}
            () = tokio::time::sleep(self.params.control_within) => {
                self.end(End::NoControl);
                return;
            }
        }
        tokio::select! {
            () = self.ended.cancelled() => {}
            _ = self.connection.closed() => self.end(End::Connection),
        }
    }

    /// Reports a few times a second and asks again for a key frame that did not come.
    async fn keep_time(self: Arc<Self>) {
        let mut tick = tokio::time::interval(self.params.tick_every);
        let mut report = tokio::time::interval(self.params.report_every);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        report.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = self.ended.cancelled() => return,
                _ = tick.tick() => self.tick(),
                _ = report.tick() => self.report(),
            }
        }
    }

    fn tick(&self) {
        let mut state = lock(&self.state);
        let actions = state.viewer.tick(self.now());
        let consistent = self.apply(&mut state, actions);
        drop(state);
        if !consistent {
            self.end(End::Internal);
        }
    }

    /// Sends a report, once the source can be told: before its control stream is taken, the
    /// source has not made its side of the session.
    fn report(&self) {
        if !self.control_taken.is_cancelled() {
            return;
        }
        let state = lock(&self.state);
        if state.closed {
            return;
        }
        let decoder = state.decoder.state();
        let report = Report {
            shown: state.viewer.shown(),
            decode_us: decoder.decode_us,
            dropped: state.viewer.stats().discarded,
            thermal: decoder.thermal,
        };
        drop(state);
        self.say(ViewerMessage::Report(report));
    }

    /// Carries out what the rules of the viewer say, in order, unless the session is over. Returns
    /// false if they named a picture that the viewer does not hold, which they cannot do, and the
    /// session is to end.
    fn apply(&self, state: &mut State, actions: Vec<Action>) -> bool {
        if state.closed {
            return true;
        }
        let mut queue: VecDeque<Action> = actions.into();
        let mut consistent = true;
        while let Some(action) = queue.pop_front() {
            match action {
                Action::Decode(number) => {
                    let Some(picture) = state.pictures.remove(&number) else {
                        consistent = false;
                        continue;
                    };
                    if state.decoder.decode(&picture.header, picture.bytes).is_ok() {
                        continue;
                    }
                    // What was asked for after this frame and has not been carried out depends
                    // on it, and is dropped. The rest of what was asked for stands.
                    let later = std::mem::take(&mut queue);
                    for action in later {
                        match action {
                            Action::Decode(number) => {
                                state.pictures.remove(&number);
                            }
                            Action::Show(_) => {}
                            keep @ (Action::Discard(_) | Action::RequestKeyFrame { .. }) => {
                                queue.push_back(keep);
                            }
                        }
                    }
                    queue.extend(state.viewer.decode_failed(self.now()));
                }
                Action::Show(number) => state.decoder.show(number),
                Action::Discard(number) => {
                    state.pictures.remove(&number);
                }
                Action::RequestKeyFrame { tried } => {
                    self.say(ViewerMessage::KeyFrame(KeyFrame { tried }));
                }
            }
        }
        consistent
    }

    /// Writes the viewer's control stream, once the source's has been taken: the 17 bytes that
    /// start every stream of a session, and then the messages that wait in the queue.
    async fn write_control(self: Arc<Self>, queue: mpsc::Receiver<ViewerMessage>) {
        tokio::select! {
            () = self.ended.cancelled() => return,
            () = self.control_taken.cancelled() => {}
        }
        match write_control(
            &self.connection,
            self.session,
            queue,
            &self.ended,
            &self.end,
        )
        .await
        {
            Ok(()) => {}
            Err(WriteEnd::Connection) => self.end(End::Connection),
            Err(WriteEnd::Peer) => self.end(End::Peer),
        }
    }

    /// Sends what the person did, in batches, once the source can be told.
    async fn send_input(self: Arc<Self>) {
        tokio::select! {
            () = self.ended.cancelled() => return,
            () = self.control_taken.cancelled() => {}
        }
        pump_input(
            &self.input,
            &self.input_ready,
            &self.outbox,
            &self.ended,
            || self.now(),
        )
        .await;
    }

    /// Reads the source's control stream until the session ends.
    pub(super) async fn read_source_control(self: Arc<Self>, arrival: Arrival) {
        let Arrival { mut recv, slot, .. } = arrival;
        // The place of the stream in the count is let go before the session is ended, so that a
        // session that follows can open its control stream (protocol.md section 9).
        if self.source_control.swap(true, Ordering::SeqCst) {
            // A session has one control stream from each side.
            stop(&mut recv, close_code::PROTOCOL_VIOLATION);
            drop(slot);
            self.end(End::Violation);
            return;
        }
        self.control_taken.cancel();
        let ended = read_control(recv, self.params.stall, &self.ended, |message| {
            // A source sends sound messages only to a viewer that advertised `stream.hear`, and
            // this viewer does not yet (protocol.md section 9).
            if matches!(message, SourceMessage::Sound(_) | SourceMessage::Silent(_)) {
                return ControlFlow::Break(());
            }
            // The source keeps within what the viewer said it can take.
            if let SourceMessage::Config(config) = &message
                && !self.takes.allows(config)
            {
                return ControlFlow::Break(());
            }
            self.deliver(message);
            ControlFlow::Continue(())
        })
        .await;
        drop(slot);
        match ended {
            ControlEnd::Over => {}
            ControlEnd::Peer => self.end(End::Peer),
            ControlEnd::Violation => self.end(End::Violation),
            ControlEnd::Stalled => self.end(End::Stalled),
        }
    }

    /// Gives a message of the source to the decoder, unless the session is over.
    fn deliver(&self, message: SourceMessage) {
        let mut state = lock(&self.state);
        if !state.closed {
            state.decoder.source(message);
        }
    }

    /// Reads the frame of one stream and gives it to the rules of the viewer.
    pub(super) async fn read_frame(self: Arc<Self>, arrival: Arrival) {
        let Arrival { mut recv, slot, .. } = arrival;
        let _slot = slot;
        let started = lock(&self.state).viewer.stream_started();
        let stream = match started {
            Ok(stream) => stream,
            // As many are open as the viewer takes, or it holds as much as it takes. The source
            // sees the stop and treats the frame as lost.
            Err(Refusal::OverBudget | Refusal::TooLarge | Refusal::Unknown) => {
                stop(&mut recv, close_code::CANCELLED);
                return;
            }
        };
        let received = tokio::select! {
            () = self.ended.cancelled() => Err(Stop::Code(close_code::CANCELLED)),
            received = self.receive_frame(&mut recv, stream) => received,
        };
        match received {
            Ok((header, bytes)) => self.frame_arrived(stream, header, bytes),
            Err(why) => {
                lock(&self.state).viewer.stream_aborted(stream);
                match why {
                    Stop::Code(error) => stop(&mut recv, error),
                    Stop::Gone => {}
                    Stop::Violation => {
                        stop(&mut recv, close_code::PROTOCOL_VIOLATION);
                        self.end(End::Violation);
                    }
                }
            }
        }
    }

    /// Reads the rest of the header and then the picture, which the rules of the viewer count as
    /// it comes in, so that a source cannot make the viewer hold more than they allow.
    async fn receive_frame(
        &self,
        recv: &mut RecvStream,
        stream: StreamId,
    ) -> Result<(FrameHeader, Vec<u8>), Stop> {
        let mut head = encode_prefix(KIND_VIDEO, self.session).to_vec();
        head.resize(FrameHeader::LEN, 0);
        let (_, rest) = head.split_at_mut(PREFIX_LEN);
        match tokio::time::timeout(self.params.stall, recv.read_exact(rest)).await {
            Ok(Ok(())) => {}
            // A header that is cut short is a violation (protocol.md section 9).
            Ok(Err(ReadExactError::FinishedEarly(_))) => return Err(Stop::Violation),
            Ok(Err(ReadExactError::ReadError(_))) => return Err(Stop::Gone),
            Err(_) => return Err(Stop::Code(close_code::TIMEOUT)),
        }
        let Ok((header, _)) = FrameHeader::parse(&head) else {
            return Err(Stop::Violation);
        };

        let mut chunks = Vec::new();
        let mut total = 0_usize;
        loop {
            let chunk =
                match tokio::time::timeout(self.params.stall, recv.read_chunk(READ_CHUNK)).await {
                    Ok(Ok(Some(chunk))) => chunk,
                    Ok(Ok(None)) => break,
                    Ok(Err(_)) => return Err(Stop::Gone),
                    Err(_) => return Err(Stop::Code(close_code::TIMEOUT)),
                };
            // Counted before it is kept.
            if let Err(refusal) = lock(&self.state).viewer.stream_read(stream, chunk.len()) {
                return Err(Stop::Code(match refusal {
                    Refusal::TooLarge => close_code::TOO_LARGE,
                    Refusal::OverBudget | Refusal::Unknown => close_code::CANCELLED,
                }));
            }
            total += chunk.len();
            chunks.push(chunk);
        }
        // One copy, into memory of the exact size, which the decoder takes whole.
        let mut picture = Vec::with_capacity(total);
        for chunk in &chunks {
            picture.extend_from_slice(chunk);
        }
        Ok((header, picture))
    }

    /// The frame of a stream is whole.
    fn frame_arrived(&self, stream: StreamId, header: FrameHeader, bytes: Vec<u8>) {
        let mut state = lock(&self.state);
        // A frame that was whole as the session ended is not kept, and not decoded.
        if state.closed {
            return;
        }
        match state.viewer.arrived(self.now(), stream, header) {
            Ok(actions) => {
                state
                    .pictures
                    .insert(header.number, Picture { header, bytes });
                let consistent = self.apply(&mut state, actions);
                drop(state);
                if !consistent {
                    self.end(End::Internal);
                }
            }
            Err(Violation::RepeatedNumber { .. }) => {
                drop(state);
                self.end(End::Violation);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use baton_proto::stream::Codec;
    use baton_stream::{
        KIND_CONTROL,
        control::{
            Config, InputBatch, Pause, PauseReason, Silent, SilentReason, Sound, SoundCodec,
        },
        header::Flags,
        input::{Button, Event, Modifiers, NO_KEY, PointerAction},
    };
    use iroh::endpoint::SendStream;

    use super::*;
    use crate::{
        gate::code,
        testing::{Device, PATIENCE, connect, next, nothing_arrives, stopped_with},
        uni::{Kind, Sinks},
        wire::{ReadFailure, read_next, write_message},
    };

    /// What the decoder of a test was told.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Told {
        Decode(u64),
        Show(u64),
        Source(SourceMessage),
        State,
        Ended,
    }

    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<Told>>>);

    impl Log {
        fn all(&self) -> Vec<Told> {
            lock(&self.0).clone()
        }

        /// The numbers of the frames that were decoded, in order.
        fn decoded(&self) -> Vec<u64> {
            self.all()
                .into_iter()
                .filter_map(|told| match told {
                    Told::Decode(number) => Some(number),
                    _ => None,
                })
                .collect()
        }

        /// The numbers of the frames that were shown, in order.
        fn shown(&self) -> Vec<u64> {
            self.all()
                .into_iter()
                .filter_map(|told| match told {
                    Told::Show(number) => Some(number),
                    _ => None,
                })
                .collect()
        }

        fn source(&self) -> Vec<SourceMessage> {
            self.all()
                .into_iter()
                .filter_map(|told| match told {
                    Told::Source(message) => Some(message),
                    _ => None,
                })
                .collect()
        }

        fn has_ended(&self) -> bool {
            self.all().contains(&Told::Ended)
        }
    }

    /// A decoder that writes down what it is told, and refuses the frames in `refuse`.
    struct Recording {
        log: Log,
        refuse: Arc<Mutex<HashSet<u64>>>,
    }

    impl Decoder for Recording {
        fn decode(&mut self, frame: &FrameHeader, _picture: Vec<u8>) -> Result<(), DecodeError> {
            if lock(&self.refuse).contains(&frame.number) {
                return Err(DecodeError);
            }
            lock(&self.log.0).push(Told::Decode(frame.number));
            Ok(())
        }

        fn show(&mut self, number: u64) {
            lock(&self.log.0).push(Told::Show(number));
        }

        fn source(&mut self, message: SourceMessage) {
            lock(&self.log.0).push(Told::Source(message));
        }

        fn state(&self) -> DecoderState {
            lock(&self.log.0).push(Told::State);
            DecoderState {
                decode_us: 1234,
                thermal: Thermal::Fair,
            }
        }

        fn ended(&mut self) {
            lock(&self.log.0).push(Told::Ended);
        }
    }

    fn session_id() -> SessionId {
        SessionId::from_bytes([7; SessionId::LEN])
    }

    /// What the viewer of most tests says it can take: everything the protocol allows.
    fn takes() -> Takes {
        Takes {
            max_width: 3840,
            max_height: 3840,
            max_fps: 120,
            codecs: vec![Codec::H264],
        }
    }

    /// Parameters with the times shortened, so that a test waits for them.
    fn quick() -> ViewParams {
        ViewParams {
            report_every: Duration::from_millis(40),
            tick_every: Duration::from_millis(20),
            ..ViewParams::default()
        }
    }

    /// A source that does what a test tells it, and a viewer in front of the real thing.
    struct Rig {
        /// The source's device, with the connection it dialed.
        source: Device,
        dialed: Connection,
        /// What the viewer opens towards the source: its control stream.
        control: mpsc::UnboundedReceiver<Arrival>,
        /// The viewer's device, with the connection it accepted.
        viewer: Device,
        accepted: Connection,
        streaming: Arc<Streaming>,
        handle: ViewHandle,
        log: Log,
        refuse: Arc<Mutex<HashSet<u64>>>,
    }

    impl Rig {
        async fn new() -> Self {
            Self::with(quick(), 2).await
        }

        /// A rig whose viewer takes `control_streams` control streams at once.
        async fn with(params: ViewParams, control_streams: u32) -> Self {
            Self::taking(takes(), params, control_streams).await
        }

        /// A rig whose viewer said that it can take `takes`.
        async fn taking(takes: Takes, params: ViewParams, control_streams: u32) -> Self {
            Self::offered(takes, params, control_streams, false).await
        }

        /// A rig where the person at the source granted control with the offer.
        async fn controlling() -> Self {
            Self::offered(takes(), quick(), 2, true).await
        }

        /// A rig whose offer granted control, or did not.
        async fn offered(
            takes: Takes,
            params: ViewParams,
            control_streams: u32,
            granted: bool,
        ) -> Self {
            let streaming = Streaming::new();
            let sinks = Sinks::default()
                .install(Kind::Video, 12, streaming.clone())
                .install(Kind::Control, control_streams, streaming.clone());
            let viewer = Device::new(sinks, PATIENCE).await;
            let (source, mut receivers) = Device::taking(&[(Kind::Control, 1)]).await;
            let control = receivers.remove(0);
            let (dialed, accepted) = connect(&source, &viewer).await;
            let log = Log::default();
            let refuse = Arc::new(Mutex::new(HashSet::new()));
            let paired = viewer.trust.get(&source.id).unwrap();
            let handle = streaming
                .view(
                    &paired,
                    accepted.clone(),
                    session_id(),
                    Viewing::raw(
                        Recording {
                            log: log.clone(),
                            refuse: Arc::clone(&refuse),
                        },
                        takes,
                        params,
                    ),
                    granted,
                )
                .unwrap();
            Self {
                source,
                dialed,
                control,
                viewer,
                accepted,
                streaming,
                handle,
                log,
                refuse,
            }
        }

        /// Opens the source's control stream, as a source does once the viewer has accepted.
        async fn open_source_control(&self) -> SendStream {
            let mut send = self.dialed.open_uni().await.unwrap();
            send.write_all(&encode_prefix(KIND_CONTROL, session_id()))
                .await
                .unwrap();
            send
        }

        /// Opens a frame stream, writes `header` and `picture` on it and finishes it.
        async fn frame(&self, header: &FrameHeader, picture: &[u8]) -> SendStream {
            let mut send = self.partial_frame(header, picture).await;
            send.finish().unwrap();
            send
        }

        /// Like [`Rig::frame`], but the stream stays open.
        async fn partial_frame(&self, header: &FrameHeader, picture: &[u8]) -> SendStream {
            let mut send = self.dialed.open_uni().await.unwrap();
            let mut bytes = header.encode().unwrap();
            bytes.extend_from_slice(picture);
            send.write_all(&bytes).await.unwrap();
            send
        }

        /// Whether the viewer has taken the source's control stream.
        fn control_taken(&self) -> bool {
            self.streaming
                .viewing(&session_id())
                .is_some_and(|view| view.control_taken.is_cancelled())
        }

        /// Waits until the session is over, and says why.
        async fn ended(&self) -> End {
            tokio::time::timeout(PATIENCE, self.handle.ended())
                .await
                .expect("the session did not end")
        }

        /// Waits until `condition` holds.
        async fn until(&self, what: &str, condition: impl Fn(&Self) -> bool) {
            wait_until(what, || condition(self)).await;
        }

        /// The viewer's control stream, as the source sees it. The source's must be open.
        async fn viewer_control(&mut self) -> RecvStream {
            let arrival = next(&mut self.control).await;
            assert!(matches!(arrival.kind, Kind::Control));
            assert_eq!(arrival.id, *session_id().as_bytes());
            arrival.recv
        }
    }

    /// Waits until `condition` holds.
    async fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn key(number: u64) -> FrameHeader {
        FrameHeader {
            session: session_id(),
            number,
            depends_on: number,
            chain: 0,
            epoch: 0,
            capture_time: number * 16_000,
            flags: Flags::KEY | Flags::REFERENCED,
        }
    }

    /// A frame that depends on the one before it, and that later frames refer to.
    fn delta(number: u64) -> FrameHeader {
        FrameHeader {
            depends_on: number - 1,
            flags: Flags::REFERENCED,
            ..key(number)
        }
    }

    /// A frame that depends on frame `needs`.
    fn after(number: u64, needs: u64) -> FrameHeader {
        FrameHeader {
            depends_on: needs,
            ..delta(number)
        }
    }

    /// How the viewer's control stream ends, as the source sees it: `Ok` if it was finished, and
    /// the code if it was reset.
    async fn ends(recv: &mut RecvStream) -> Result<(), u64> {
        let mut buffer = [0_u8; 256];
        loop {
            let read = tokio::time::timeout(PATIENCE, recv.read(&mut buffer))
                .await
                .expect("the stream did not end");
            match read {
                Ok(Some(_)) => {}
                Ok(None) => return Ok(()),
                Err(iroh::endpoint::ReadError::Reset(code)) => return Err(code.into_inner()),
                Err(other) => panic!("the stream failed in another way: {other:?}"),
            }
        }
    }

    async fn read_message_from(recv: &mut RecvStream) -> Option<ViewerMessage> {
        let result = read_next::<ViewerMessage>(recv, PATIENCE).await;
        match result {
            Ok(message) => message,
            Err(ReadFailure::Reset(_) | ReadFailure::Transport(_)) => None,
            Err(other) => panic!("the viewer wrote a message that cannot be read: {other:?}"),
        }
    }

    #[tokio::test]
    async fn frames_are_decoded_in_order_and_the_newest_is_shown() {
        let rig = Rig::new().await;
        for header in [key(0), delta(1), delta(2), delta(3)] {
            rig.frame(&header, &[1; 3000]).await;
        }
        rig.until("the last frame to be shown", |rig| {
            rig.log.shown().last() == Some(&3)
        })
        .await;
        assert_eq!(rig.log.decoded(), [0, 1, 2, 3]);
        // Frames that are decoded together are shown once, and what is shown only goes forward.
        let shown = rig.log.shown();
        assert!(shown.windows(2).all(|pair| pair[0] < pair[1]), "{shown:?}");
        let stats = rig.handle.stats();
        assert_eq!(stats.shown, Some(3));
        assert_eq!(stats.streams, 0);
        assert_eq!(stats.held_bytes, 0);
    }

    #[tokio::test]
    async fn the_picture_reaches_the_decoder_whole() {
        struct Keep(Arc<Mutex<Vec<Vec<u8>>>>);
        impl Decoder for Keep {
            fn decode(&mut self, _: &FrameHeader, picture: Vec<u8>) -> Result<(), DecodeError> {
                lock(&self.0).push(picture);
                Ok(())
            }
            fn show(&mut self, _: u64) {}
            fn source(&mut self, _: SourceMessage) {}
            fn state(&self) -> DecoderState {
                DecoderState {
                    decode_us: 0,
                    thermal: Thermal::Nominal,
                }
            }
            fn ended(&mut self) {}
        }

        let streaming = Streaming::new();
        let sinks = Sinks::default().install(Kind::Video, 12, streaming.clone());
        let viewer = Device::new(sinks, PATIENCE).await;
        let (source, _) = Device::taking(&[]).await;
        let (dialed, accepted) = connect(&source, &viewer).await;
        let pictures = Arc::new(Mutex::new(Vec::new()));
        let paired = viewer.trust.get(&source.id).unwrap();
        let _handle = streaming
            .view(
                &paired,
                accepted,
                session_id(),
                Viewing::raw(Keep(Arc::clone(&pictures)), takes(), ViewParams::default()),
                false,
            )
            .unwrap();
        // Larger than a chunk and than a packet, and not a round number.
        let picture: Vec<u8> = (0..300_007_u32)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let mut send = dialed.open_uni().await.unwrap();
        send.write_all(&key(0).encode().unwrap()).await.unwrap();
        send.write_all(&picture).await.unwrap();
        send.finish().unwrap();
        let deadline = Instant::now() + PATIENCE;
        while lock(&pictures).is_empty() {
            assert!(Instant::now() < deadline, "the picture never arrived");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(lock(&pictures).as_slice(), [picture]);
    }

    #[tokio::test]
    async fn a_frame_waits_for_the_frame_it_depends_on() {
        let rig = Rig::new().await;
        rig.frame(&key(0), &[0; 100]).await;
        // Frame 2 is read before frame 1, as streams may be.
        rig.frame(&after(2, 1), &[0; 100]).await;
        rig.until("frame 0 to be decoded", |rig| rig.log.decoded() == [0])
            .await;
        assert_eq!(rig.handle.stats().held_bytes, 100);
        rig.frame(&delta(1), &[0; 100]).await;
        rig.until("frame 2 to be decoded", |rig| rig.log.decoded().len() == 3)
            .await;
        assert_eq!(rig.log.decoded(), [0, 1, 2]);
        assert_eq!(rig.log.shown().last(), Some(&2));
        assert_eq!(rig.handle.stats().held_bytes, 0);
    }

    #[tokio::test]
    async fn a_stream_that_the_source_resets_is_forgotten() {
        let rig = Rig::new().await;
        rig.frame(&key(0), &[0; 100]).await;
        let mut send = rig.partial_frame(&delta(1), &[0; 5000]).await;
        rig.until("the frame stream to be read", |rig| {
            rig.handle.stats().streams == 1 && rig.handle.stats().held_bytes > 0
        })
        .await;
        send.reset(code(7)).unwrap();
        rig.until("the stream to be forgotten", |rig| {
            let stats = rig.handle.stats();
            stats.streams == 0 && stats.held_bytes == 0
        })
        .await;
        // The reset frame is not decoded, and the next frame that does not need it is.
        rig.frame(&key(2), &[0; 100]).await;
        rig.until("frame 2 to be decoded", |rig| rig.log.decoded() == [0, 2])
            .await;
    }

    #[tokio::test]
    async fn a_picture_over_the_limit_is_stopped_with_413() {
        let rig = Rig::new().await;
        let mut send = rig.partial_frame(&key(0), &[]).await;
        // Written in pieces, so that the viewer reads while the source writes. The writes fail
        // once the viewer has stopped the stream.
        let piece = vec![0_u8; 1024 * 1024];
        for _ in 0..10 {
            if send.write_all(&piece).await.is_err() {
                break;
            }
        }
        assert_eq!(stopped_with(&send).await, 413);
        assert_eq!(rig.log.decoded(), Vec::<u64>::new());
        rig.until("what was read to be forgotten", |rig| {
            rig.handle.stats().held_bytes == 0
        })
        .await;
        // It was no fault of the session.
        rig.frame(&key(1), &[0; 100]).await;
        rig.until("frame 1 to be decoded", |rig| rig.log.decoded() == [1])
            .await;
    }

    #[tokio::test]
    async fn a_frame_stream_beyond_what_the_viewer_takes_is_stopped_with_499() {
        let mut params = quick();
        params.rules.max_streams = 2;
        let rig = Rig::with(params, 2).await;
        let first = rig.partial_frame(&key(0), &[0; 10]).await;
        let second = rig.partial_frame(&delta(1), &[0; 10]).await;
        rig.until("two streams to be read", |rig| {
            rig.handle.stats().streams == 2
        })
        .await;
        let third = rig.partial_frame(&delta(2), &[0; 10]).await;
        assert_eq!(stopped_with(&third).await, 499);
        // The other two are still read.
        assert_eq!(rig.handle.stats().streams, 2);
        drop((first, second));
    }

    #[tokio::test]
    async fn pictures_beyond_what_the_viewer_holds_are_stopped_with_499() {
        let mut params = quick();
        params.rules.max_held_bytes = 100 * 1024;
        let rig = Rig::with(params, 2).await;
        rig.frame(&key(0), &[0; 100]).await;
        // Both wait for frame 1, which does not come.
        rig.frame(&after(3, 2), &vec![0; 60 * 1024]).await;
        rig.until("the first picture to wait", |rig| {
            rig.handle.stats().held_bytes == 60 * 1024
        })
        .await;
        let mut send = rig.partial_frame(&after(4, 3), &[]).await;
        for _ in 0..4 {
            if send.write_all(&[0; 16 * 1024]).await.is_err() {
                break;
            }
        }
        assert_eq!(stopped_with(&send).await, 499);
        assert!(rig.handle.stats().held_bytes <= 100 * 1024);
    }

    #[tokio::test]
    async fn a_stream_for_a_session_that_is_not_running_is_stopped_with_499() {
        let rig = Rig::new().await;
        let other = FrameHeader {
            session: SessionId::from_bytes([9; SessionId::LEN]),
            ..key(0)
        };
        // The stream is left open: one that is finished and acknowledged before the viewer stops
        // it has not been stopped, and which of the two the source sees first is not fixed.
        let send = rig.partial_frame(&other, &[0; 10]).await;
        assert_eq!(stopped_with(&send).await, 499);
        assert_eq!(rig.log.decoded(), Vec::<u64>::new());
    }

    #[tokio::test]
    async fn a_stream_on_another_connection_of_the_same_device_is_not_taken() {
        // The source opens its control stream, so that the session goes on, and the time that a
        // stream may deliver nothing is short, so that it would not go on waiting if the stream
        // were taken.
        let mut params = quick();
        params.stall = Duration::from_millis(200);
        let rig = Rig::with(params, 2).await;
        let _source_control = rig.open_source_control().await;
        rig.until(
            "the source's control stream to be taken",
            Rig::control_taken,
        )
        .await;
        // The same two devices, on a connection of their own.
        let (second, _) = connect(&rig.source, &rig.viewer).await;
        let mut send = second.open_uni().await.unwrap();
        let mut bytes = key(0).encode().unwrap();
        bytes.extend_from_slice(&[0; 10]);
        send.write_all(&bytes).await.unwrap();
        assert_eq!(stopped_with(&send).await, 499);
        assert_eq!(rig.log.decoded(), Vec::<u64>::new());
    }

    #[tokio::test]
    async fn a_header_that_breaks_a_rule_ends_the_session_with_400() {
        let rig = Rig::new().await;
        let mut bytes = key(0).encode().unwrap();
        // The last byte holds the flags, and bit 7 is not defined.
        *bytes.last_mut().unwrap() = 0x80;
        let mut send = rig.dialed.open_uni().await.unwrap();
        send.write_all(&bytes).await.unwrap();
        send.write_all(&[0; 10]).await.unwrap();
        assert_eq!(stopped_with(&send).await, 400);
        assert_eq!(rig.ended().await, End::Violation);
        assert!(rig.log.has_ended());
    }

    #[tokio::test]
    async fn a_header_that_is_cut_short_ends_the_session() {
        let rig = Rig::new().await;
        let bytes = key(0).encode().unwrap();
        let mut send = rig.dialed.open_uni().await.unwrap();
        send.write_all(&bytes[..30]).await.unwrap();
        send.finish().unwrap();
        assert_eq!(rig.ended().await, End::Violation);
    }

    #[tokio::test]
    async fn a_frame_number_that_comes_twice_ends_the_session() {
        let rig = Rig::new().await;
        rig.frame(&key(0), &[0; 10]).await;
        // Frame 2 waits for frame 1, and a second frame 2 cannot be told from it.
        rig.frame(&after(2, 1), &[0; 10]).await;
        rig.until("frame 2 to wait", |rig| rig.handle.stats().held_bytes == 10)
            .await;
        rig.frame(&after(2, 1), &[0; 10]).await;
        assert_eq!(rig.ended().await, End::Violation);
        assert!(rig.log.has_ended());
    }

    #[tokio::test]
    async fn a_stream_that_delivers_nothing_is_stopped_with_408() {
        let mut params = quick();
        params.stall = Duration::from_millis(200);
        let rig = Rig::with(params, 2).await;
        let mut send = rig.dialed.open_uni().await.unwrap();
        // The start and half the rest of the header, and then nothing.
        send.write_all(&key(0).encode().unwrap()[..30])
            .await
            .unwrap();
        assert_eq!(stopped_with(&send).await, 408);
        assert_eq!(rig.handle.stats().streams, 0);
    }

    #[tokio::test]
    async fn a_picture_that_stops_arriving_is_stopped_with_408() {
        let mut params = quick();
        params.stall = Duration::from_millis(200);
        let rig = Rig::with(params, 2).await;
        // The whole header and a part of the picture, and then nothing.
        let send = rig.partial_frame(&key(0), &[0; 100]).await;
        assert_eq!(stopped_with(&send).await, 408);
        rig.until("what was read to be forgotten", |rig| {
            let stats = rig.handle.stats();
            stats.streams == 0 && stats.held_bytes == 0
        })
        .await;
        assert_eq!(rig.log.decoded(), Vec::<u64>::new());
        // It was no fault of the session.
        rig.frame(&key(1), &[0; 100]).await;
        rig.until("frame 1 to be decoded", |rig| rig.log.decoded() == [1])
            .await;
    }

    #[tokio::test]
    async fn a_decoder_that_refuses_a_frame_makes_the_viewer_ask_for_a_key_frame() {
        let mut rig = Rig::new().await;
        let mut source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        lock(&rig.refuse).insert(1);
        rig.frame(&key(0), &[0; 100]).await;
        rig.frame(&delta(1), &[0; 100]).await;
        // The viewer says how it is doing, and then that it needs a key frame: frame 1 is the
        // newest that it handed to the decoder.
        let mut asked = None;
        while asked.is_none() {
            match read_message_from(&mut viewer_control).await.unwrap() {
                ViewerMessage::KeyFrame(request) => asked = Some(request),
                ViewerMessage::Report(_) => {}
                other => panic!("unexpected message {other:?}"),
            }
        }
        assert_eq!(asked, Some(KeyFrame { tried: 1 }));
        // A key frame numbered above the request answers it, and is decoded.
        rig.frame(&key(2), &[0; 100]).await;
        rig.until("the key frame to be decoded", |rig| {
            rig.log.decoded() == [0, 2]
        })
        .await;
        source_control.finish().unwrap();
    }

    #[tokio::test]
    async fn a_failure_that_the_platform_reports_later_asks_for_a_key_frame_too() {
        let mut rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        rig.frame(&key(0), &[0; 100]).await;
        rig.frame(&delta(1), &[0; 100]).await;
        rig.until("two frames to be decoded", |rig| {
            rig.log.decoded() == [0, 1]
        })
        .await;
        rig.handle.decode_failed();
        let mut asked = None;
        while asked.is_none() {
            if let ViewerMessage::KeyFrame(request) =
                read_message_from(&mut viewer_control).await.unwrap()
            {
                asked = Some(request);
            }
        }
        assert_eq!(asked, Some(KeyFrame { tried: 1 }));
    }

    #[tokio::test]
    async fn frames_that_were_decoded_together_with_a_refused_one_are_not_decoded() {
        let rig = Rig::new().await;
        lock(&rig.refuse).insert(1);
        // Frames 1 and 2 arrive before the key frame that they need, and wait for it.
        rig.frame(&delta(1), &[0; 100]).await;
        rig.frame(&delta(2), &[0; 100]).await;
        rig.until("both frames to wait", |rig| {
            rig.handle.stats().held_bytes == 200
        })
        .await;
        // The key frame frees both at once. The decoder refuses frame 1, and frame 2 refers to it.
        rig.frame(&key(0), &[0; 100]).await;
        rig.until("what waited to be let go of", |rig| {
            rig.handle.stats().held_bytes == 0
        })
        .await;
        assert_eq!(rig.log.decoded(), [0]);
        assert!(!rig.log.shown().contains(&2), "{:?}", rig.log.shown());
    }

    #[tokio::test]
    async fn the_viewer_opens_its_control_stream_after_the_source_has_opened_its_own() {
        let mut rig = Rig::new().await;
        nothing_arrives(&mut rig.control).await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        // And then reports, a few times a second, with what the decoder says about itself.
        rig.frame(&key(0), &[0; 100]).await;
        rig.until("frame 0 to be shown", |rig| rig.log.shown() == [0])
            .await;
        let mut report = None;
        while report.is_none() {
            if let ViewerMessage::Report(r) = read_message_from(&mut viewer_control).await.unwrap()
                && r.shown == Some(0)
            {
                report = Some(r);
            }
        }
        let report = report.unwrap();
        assert_eq!(report.decode_us, 1234);
        assert_eq!(report.thermal, Thermal::Fair);
        assert_eq!(report.dropped, 0);
    }

    #[tokio::test]
    async fn what_the_viewer_wants_is_told_to_the_source() {
        let mut rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        let want = Want {
            width: 1170,
            height: 2532,
            fps: 60,
        };
        rig.handle.want(want);
        loop {
            if let ViewerMessage::Want(told) = read_message_from(&mut viewer_control).await.unwrap()
            {
                assert_eq!(told, want);
                break;
            }
        }
    }

    #[tokio::test]
    async fn messages_of_the_source_reach_the_decoder() {
        let rig = Rig::new().await;
        let mut send = rig.open_source_control().await;
        let config = Config {
            epoch: 0,
            codec: Codec::H264,
            width: 1440,
            height: 900,
            fps: 60,
        };
        let messages = [
            SourceMessage::Config(config),
            SourceMessage::Pause(Pause {
                reason: PauseReason::Locked,
            }),
            SourceMessage::Resume,
        ];
        for message in &messages {
            write_message(&mut send, message).await.unwrap();
        }
        rig.until("the messages to arrive", |rig| rig.log.source().len() == 3)
            .await;
        assert_eq!(rig.log.source(), messages);
    }

    #[tokio::test]
    async fn a_message_of_the_source_that_stalls_ends_the_session_and_blames_no_one() {
        let mut params = quick();
        params.stall = Duration::from_millis(200);
        let mut rig = Rig::with(params, 2).await;
        let mut source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        // The length of a message of 20 bytes, and then nothing.
        source_control.write_all(&[0, 0, 0, 20]).await.unwrap();
        assert_eq!(stopped_with(&source_control).await, 408);
        assert_eq!(rig.ended().await, End::Stalled);
        assert!(rig.log.has_ended());
        // The source broke no rule, so the viewer's stream is finished and not reset.
        assert_eq!(ends(&mut viewer_control).await, Ok(()));
    }

    #[tokio::test]
    async fn a_picture_beyond_what_the_viewer_said_it_takes_is_a_violation() {
        let said = Takes {
            max_width: 1440,
            max_height: 900,
            max_fps: 60,
            codecs: vec![Codec::H264],
        };
        let config = Config {
            epoch: 0,
            codec: Codec::H264,
            width: 1440,
            height: 900,
            fps: 60,
        };
        // Exactly what the viewer said is within it.
        let rig = Rig::taking(said.clone(), quick(), 2).await;
        let mut send = rig.open_source_control().await;
        write_message(&mut send, &SourceMessage::Config(config))
            .await
            .unwrap();
        rig.until("the picture format to arrive", |rig| {
            rig.log.source() == [SourceMessage::Config(config)]
        })
        .await;
        drop(rig);

        // Anything beyond it, or a codec that the viewer did not list, is not.
        for (what, beyond) in [
            (
                "wider",
                Config {
                    width: 1441,
                    ..config
                },
            ),
            (
                "higher",
                Config {
                    height: 901,
                    ..config
                },
            ),
            ("faster", Config { fps: 61, ..config }),
            (
                "a codec the viewer did not list",
                Config {
                    codec: Codec::Other,
                    ..config
                },
            ),
        ] {
            let rig = Rig::taking(said.clone(), quick(), 2).await;
            let mut send = rig.open_source_control().await;
            write_message(&mut send, &SourceMessage::Config(beyond))
                .await
                .unwrap();
            assert_eq!(rig.ended().await, End::Violation, "{what}");
            assert_eq!(stopped_with(&send).await, 400, "{what}");
            assert!(rig.log.source().is_empty(), "{what}");
        }
    }

    #[tokio::test]
    async fn sound_from_a_source_is_a_violation_while_the_viewer_does_not_take_it() {
        for message in [
            SourceMessage::Sound(Sound {
                codec: SoundCodec::Aac,
                rate: 48_000,
                channels: 2,
                samples: 1024,
            }),
            SourceMessage::Silent(Silent {
                reason: SilentReason::Failed,
            }),
        ] {
            let rig = Rig::new().await;
            let mut send = rig.open_source_control().await;
            write_message(&mut send, &message).await.unwrap();
            assert_eq!(rig.ended().await, End::Violation, "{message:?}");
            assert_eq!(stopped_with(&send).await, 400, "{message:?}");
            assert!(rig.log.source().is_empty(), "{message:?}");
        }
    }

    #[tokio::test]
    async fn a_message_that_cannot_be_read_ends_the_session() {
        let rig = Rig::new().await;
        let mut send = rig.open_source_control().await;
        // A length of 5 and five bytes that are not a message.
        send.write_all(&[0, 0, 0, 5, 0xff, 0xff, 0xff, 0xff, 0xff])
            .await
            .unwrap();
        assert_eq!(rig.ended().await, End::Violation);
        assert_eq!(stopped_with(&send).await, 400);
    }

    #[tokio::test]
    async fn a_second_control_stream_from_the_source_ends_the_session() {
        let rig = Rig::with(quick(), 2).await;
        let _first = rig.open_source_control().await;
        rig.until("the first to be taken", Rig::control_taken).await;
        let mut second = rig.open_source_control().await;
        assert_eq!(stopped_with(&second).await, 400);
        assert_eq!(rig.ended().await, End::Violation);
        let _ = second.finish();
    }

    #[tokio::test]
    async fn the_end_of_the_sources_control_stream_ends_the_session() {
        let rig = Rig::new().await;
        let mut send = rig.open_source_control().await;
        send.finish().unwrap();
        assert_eq!(rig.ended().await, End::Peer);
        assert!(rig.log.has_ended());
    }

    #[tokio::test]
    async fn a_session_ends_with_its_connection() {
        let rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        rig.dialed.close(code(close_code::NORMAL), b"done");
        assert_eq!(rig.ended().await, End::Connection);
        assert!(rig.log.has_ended());
    }

    #[tokio::test]
    async fn a_session_ends_when_the_source_does_not_open_its_control_stream() {
        let mut params = quick();
        params.control_within = Duration::from_millis(200);
        let rig = Rig::with(params, 2).await;
        assert_eq!(rig.ended().await, End::NoControl);
        assert!(rig.log.has_ended());
    }

    #[tokio::test]
    async fn dropping_the_handle_ends_the_session_and_finishes_the_control_stream() {
        let mut rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        let streaming = Arc::clone(&rig.streaming);
        let Rig {
            handle,
            log,
            dialed,
            ..
        } = rig;
        drop(handle);
        // The stream ends where a message would have started, and is not reset.
        assert_eq!(ends(&mut viewer_control).await, Ok(()));
        assert!(log.has_ended());
        // The session is forgotten, so that its id is free again.
        wait_until("the session to be forgotten", || streaming.is_empty()).await;
        // Frames that come after are for a session that is not running.
        let mut send = dialed.open_uni().await.unwrap();
        send.write_all(&key(0).encode().unwrap()).await.unwrap();
        assert_eq!(stopped_with(&send).await, 499);
    }

    #[tokio::test]
    async fn a_frame_that_is_whole_after_the_end_is_not_decoded() {
        let rig = Rig::new().await;
        let view = &rig.handle.view;
        let stream = lock(&view.state).viewer.stream_started().unwrap();
        lock(&view.state).viewer.stream_read(stream, 10).unwrap();
        rig.handle.end();
        view.frame_arrived(stream, key(0), vec![0; 10]);
        assert_eq!(rig.log.all(), [Told::Ended]);
        // And the picture is not kept.
        assert!(lock(&view.state).pictures.is_empty());
    }

    #[tokio::test]
    async fn nothing_else_that_was_on_its_way_reaches_the_decoder_after_the_end() {
        let rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        rig.until(
            "the source's control stream to be taken",
            Rig::control_taken,
        )
        .await;
        rig.frame(&key(0), &[0; 10]).await;
        rig.until("frame 0 to be shown", |rig| rig.log.shown() == [0])
            .await;
        let view = &rig.handle.view;
        rig.handle.end();
        let told = rig.log.all();
        assert_eq!(told.last(), Some(&Told::Ended));
        // What a task that had passed its check of the session when it ended can still do.
        view.deliver(SourceMessage::Resume);
        view.report();
        view.tick();
        rig.handle.decode_failed();
        // And whatever the rules of the viewer still ask for.
        let mut state = lock(&view.state);
        assert!(view.apply(&mut state, vec![Action::Show(0)]));
        drop(state);
        assert_eq!(rig.log.all(), told);
    }

    #[tokio::test]
    async fn the_pictures_that_wait_are_dropped_when_the_session_ends() {
        let rig = Rig::new().await;
        // Frame 1 waits for the key frame, which has not come.
        rig.frame(&delta(1), &[0; 100]).await;
        rig.until("frame 1 to wait", |rig| {
            rig.handle.stats().held_bytes == 100
        })
        .await;
        let view = &rig.handle.view;
        assert_eq!(lock(&view.state).pictures.len(), 1);
        rig.handle.end();
        assert!(lock(&view.state).pictures.is_empty());
    }

    #[tokio::test]
    async fn the_person_can_end_the_session() {
        let mut rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        rig.handle.end();
        assert_eq!(rig.ended().await, End::Local);
        assert!(rig.log.has_ended());
        while read_message_from(&mut viewer_control).await.is_some() {}
        // Ending it again changes nothing, and the decoder was told once.
        rig.handle.end();
        let ended = rig
            .log
            .all()
            .iter()
            .filter(|told| **told == Told::Ended)
            .count();
        assert_eq!(ended, 1);
    }

    #[tokio::test]
    async fn a_violation_resets_the_viewers_control_stream_with_400() {
        let mut rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        let mut bytes = key(0).encode().unwrap();
        *bytes.last_mut().unwrap() = 0x80;
        let mut send = rig.dialed.open_uni().await.unwrap();
        send.write_all(&bytes).await.unwrap();
        assert_eq!(ends(&mut viewer_control).await, Err(400));
    }

    #[tokio::test]
    async fn a_session_id_that_is_in_use_is_refused() {
        let rig = Rig::new().await;
        let paired = rig.viewer.trust.get(&rig.source.id).unwrap();
        let again = rig.streaming.view(
            &paired,
            rig.accepted.clone(),
            session_id(),
            Viewing::raw(
                Recording {
                    log: Log::default(),
                    refuse: Arc::default(),
                },
                takes(),
                quick(),
            ),
            false,
        );
        assert!(again.is_err());
        // The session that was there first is untouched.
        rig.frame(&key(0), &[0; 10]).await;
        rig.until("frame 0 to be decoded", |rig| rig.log.decoded() == [0])
            .await;
    }
    // What the person does.

    fn pointer(x: f32, y: f32, action: PointerAction) -> Input {
        Input::Pointer {
            x,
            y,
            modifiers: Modifiers::NONE,
            action,
        }
    }

    /// A press of the left button, or a touch.
    fn tap() -> Input {
        pointer(0.5, 0.25, PointerAction::Down(Button::Left))
    }

    fn pressed(code: u16, down: bool) -> Input {
        Input::Key {
            code,
            down,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: None,
        }
    }

    fn typed(text: &str) -> Input {
        Input::Key {
            code: NO_KEY,
            down: true,
            modifiers: Modifiers::NONE,
            repeat: false,
            text: Some(text.to_owned()),
        }
    }

    /// Reads the viewer's control stream until `count` events of input have arrived, and gives
    /// them as the batches they came in, with the time each batch arrived. Reports are skipped.
    async fn read_input(recv: &mut RecvStream, count: usize) -> Vec<(Instant, InputBatch)> {
        // The reports keep coming, and each of them restarts the wait for a message, so the wait
        // for the input has a limit of its own.
        let deadline = Instant::now() + PATIENCE * 2;
        let mut batches = Vec::new();
        let mut events = 0;
        while events < count {
            assert!(Instant::now() < deadline, "the input did not arrive");
            match read_message_from(recv).await.expect("the stream ended") {
                ViewerMessage::Input(batch) => {
                    events += batch.events.len();
                    batches.push((Instant::now(), batch));
                }
                ViewerMessage::Report(_) => {}
                other => panic!("unexpected message {other:?}"),
            }
        }
        batches
    }

    fn events_of(batches: &[(Instant, InputBatch)]) -> Vec<Event> {
        batches
            .iter()
            .flat_map(|(_, batch)| batch.events.clone())
            .collect()
    }

    /// A rig where control was granted and the first picture is on the screen, with the viewer's
    /// control stream open.
    async fn showing_a_picture() -> (Rig, SendStream, RecvStream) {
        let mut rig = Rig::controlling().await;
        let source_control = rig.open_source_control().await;
        let viewer_control = rig.viewer_control().await;
        rig.frame(&key(0), &[0; 100]).await;
        rig.until("frame 0 to be shown", |rig| rig.log.shown() == [0])
            .await;
        (rig, source_control, viewer_control)
    }

    #[tokio::test]
    async fn what_the_person_does_reaches_the_source_with_the_frame_that_was_shown() {
        let (rig, _source_control, mut viewer_control) = showing_a_picture().await;
        assert!(rig.handle.control());
        rig.handle.input(tap()).unwrap();
        let batches = read_input(&mut viewer_control, 1).await;
        assert_eq!(
            events_of(&batches),
            [Event::Pointer {
                frame: 0,
                x: 0.5,
                y: 0.25,
                modifiers: Modifiers::NONE,
                action: PointerAction::Down(Button::Left),
            }]
        );
        // The next picture is the one that the next position is in.
        rig.frame(&delta(1), &[0; 100]).await;
        rig.until("frame 1 to be shown", |rig| {
            rig.log.shown().last() == Some(&1)
        })
        .await;
        rig.handle
            .input(pointer(0.5, 0.25, PointerAction::Up(Button::Left)))
            .unwrap();
        let batches = read_input(&mut viewer_control, 1).await;
        assert!(matches!(
            events_of(&batches)[..],
            [Event::Pointer {
                frame: 1,
                action: PointerAction::Up(Button::Left),
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn a_session_where_control_was_not_granted_sends_no_input() {
        let mut rig = Rig::new().await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        rig.frame(&key(0), &[0; 100]).await;
        rig.until("frame 0 to be shown", |rig| rig.log.shown() == [0])
            .await;
        assert!(!rig.handle.control());
        assert_eq!(rig.handle.input(tap()), Err(Refused::NotGranted));
        assert_eq!(rig.handle.input(pressed(4, true)), Err(Refused::NotGranted));
        assert_eq!(rig.handle.input(typed("a")), Err(Refused::NotGranted));
        rig.handle.release_input();
        // The viewer goes on reporting, and nothing else comes.
        for _ in 0..5 {
            match read_message_from(&mut viewer_control).await.unwrap() {
                ViewerMessage::Report(_) => {}
                other => panic!("unexpected message {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_position_waits_for_a_picture_and_a_key_does_not() {
        let mut rig = Rig::controlling().await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        assert_eq!(rig.handle.input(tap()), Err(Refused::NoPicture));
        rig.handle.input(pressed(4, true)).unwrap();
        let batches = read_input(&mut viewer_control, 1).await;
        assert_eq!(
            events_of(&batches),
            [Event::Key {
                code: 4,
                down: true,
                modifiers: Modifiers::NONE,
                repeat: false,
                text: None,
            }]
        );
    }

    #[tokio::test]
    async fn input_is_sent_when_the_source_has_opened_its_control_stream_and_not_before() {
        let mut rig = Rig::controlling().await;
        rig.handle.input(typed("hello")).unwrap();
        // The viewer has not opened its stream, because the source has not opened its own.
        nothing_arrives(&mut rig.control).await;
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        let batches = read_input(&mut viewer_control, 1).await;
        assert_eq!(texts(&events_of(&batches)), ["hello"]);
    }

    fn texts(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Key { text, .. } => text.clone(),
                Event::Pointer { .. } => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn what_is_sent_keeps_to_the_rate_that_the_source_takes() {
        let (rig, _source_control, mut viewer_control) = showing_a_picture().await;
        // 700 events, which is two seconds and a bit at 240 a second. The sliding second is the
        // business of the queue and is checked there; what is checked here is that the task that
        // sends takes its batches at the pace the queue gives, and not faster.
        for number in 0..700_u32 {
            rig.handle.input(typed(&(number % 10).to_string())).unwrap();
        }
        let started = Instant::now();
        let batches = read_input(&mut viewer_control, 700).await;
        assert!(started.elapsed() >= Duration::from_millis(1900));
        assert_eq!(events_of(&batches).len(), 700);
        assert!(batches.iter().all(|(_, batch)| batch.events.len() <= 64));
        // And in order.
        let expected: Vec<String> = (0..700_u32)
            .map(|number| (number % 10).to_string())
            .collect();
        assert_eq!(texts(&events_of(&batches)), expected);
    }

    #[tokio::test]
    async fn moves_that_wait_for_room_are_joined_into_the_latest() {
        let (rig, _source_control, mut viewer_control) = showing_a_picture().await;
        // The whole second is used up at once.
        for _ in 0..240 {
            rig.handle.input(typed("a")).unwrap();
        }
        let used = read_input(&mut viewer_control, 240).await;
        let moved = Instant::now();
        for step in 0..100_u8 {
            let at = f32::from(step) / 100.0;
            rig.handle
                .input(pointer(at, at, PointerAction::Move))
                .unwrap();
        }
        // They wait for the second to pass, and then one of them is sent: where the pointer is.
        let batches = read_input(&mut viewer_control, 1).await;
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].1.events.len(), 1);
        assert!(batches[0].0.duration_since(used[0].0) >= Duration::from_millis(900));
        assert!(batches[0].0.duration_since(moved) >= Duration::from_millis(500));
        assert!(matches!(
            batches[0].1.events[..],
            [Event::Pointer {
                frame: 0,
                action: PointerAction::Move,
                ..
            }]
        ));
        let Event::Pointer { x, y, .. } = batches[0].1.events[0] else {
            unreachable!()
        };
        assert!(
            (x - 0.99).abs() < 1e-6 && (y - 0.99).abs() < 1e-6,
            "{x} {y}"
        );
    }

    #[tokio::test]
    async fn release_input_lifts_what_is_down() {
        let (rig, _source_control, mut viewer_control) = showing_a_picture().await;
        rig.handle.input(tap()).unwrap();
        rig.handle.input(pressed(4, true)).unwrap();
        let down = events_of(&read_input(&mut viewer_control, 2).await);
        assert_eq!(down.len(), 2);
        // The person leaves the screen with a finger and a key down.
        rig.handle.release_input();
        let up = events_of(&read_input(&mut viewer_control, 2).await);
        assert!(matches!(
            up[..],
            [
                Event::Pointer {
                    frame: 0,
                    action: PointerAction::Up(Button::Left),
                    ..
                },
                Event::Key {
                    code: 4,
                    down: false,
                    ..
                }
            ]
        ));
        // And nothing is down any more.
        assert_eq!(
            rig.handle.input(pressed(4, false)),
            Err(Refused::NotPressed)
        );
    }

    #[tokio::test]
    async fn moves_that_pile_up_before_the_source_is_ready_are_joined() {
        let mut rig = Rig::controlling().await;
        rig.frame(&key(0), &[0; 100]).await;
        rig.until("frame 0 to be shown", |rig| rig.log.shown() == [0])
            .await;
        // The source has not opened its control stream, so nothing can be sent yet. The task
        // that sends gets its turn between the moves.
        for step in 1..=4_u8 {
            let at = f32::from(step) / 10.0;
            rig.handle
                .input(pointer(at, at, PointerAction::Move))
                .unwrap();
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        let _source_control = rig.open_source_control().await;
        let mut viewer_control = rig.viewer_control().await;
        let batches = read_input(&mut viewer_control, 1).await;
        assert_eq!(
            events_of(&batches),
            [Event::Pointer {
                frame: 0,
                x: 0.4,
                y: 0.4,
                modifiers: Modifiers::NONE,
                action: PointerAction::Move,
            }]
        );
    }

    /// Runs the pump and the person together, and fails the test if they do not both end in time.
    async fn pumped(pump: impl Future<Output = ()>, person: impl Future<Output = ()>) {
        tokio::time::timeout(PATIENCE, async {
            tokio::join!(pump, person);
        })
        .await
        .expect("the pump did not end");
    }

    /// The next message to the source, or a failure if none comes in time.
    async fn received(stream: &mut mpsc::Receiver<ViewerMessage>) -> ViewerMessage {
        tokio::time::timeout(PATIENCE, stream.recv())
            .await
            .expect("nothing came in time")
            .expect("the stream was closed")
    }

    #[tokio::test]
    async fn the_pump_sleeps_until_the_rate_leaves_room_and_does_not_spin() {
        let (outbox, mut stream) = mpsc::channel(8);
        let input = Mutex::new(Queue::new(true));
        let ready = Notify::new();
        let ended = CancellationToken::new();
        // The whole second is used up, and one more event waits for it to pass.
        for _ in 0..240 {
            lock(&input).push(Some(0), typed("a")).unwrap();
        }
        while lock(&input).take(Duration::ZERO).is_some() {}
        lock(&input).push(Some(0), typed("b")).unwrap();
        let asked = std::cell::Cell::new(0_usize);
        let began = Instant::now();
        let pump = pump_input(&input, &ready, &outbox, &ended, || {
            asked.set(asked.get() + 1);
            began.elapsed()
        });
        let person = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            // It asked when there is room, and waited for that time. It did not ask again and
            // again.
            assert!(asked.get() < 10, "{} questions", asked.get());
            assert!(stream.try_recv().is_err());
            ended.cancel();
        };
        pumped(pump, person).await;
    }

    #[tokio::test]
    async fn the_pump_ends_with_the_session_while_it_waits_for_room() {
        let (outbox, _stream) = mpsc::channel(1);
        outbox
            .try_send(ViewerMessage::KeyFrame(KeyFrame { tried: 0 }))
            .unwrap();
        let input = Mutex::new(Queue::new(true));
        let ready = Notify::new();
        let ended = CancellationToken::new();
        let began = Instant::now();
        lock(&input)
            .push(Some(0), pointer(0.1, 0.1, PointerAction::Move))
            .unwrap();
        let pump = pump_input(&input, &ready, &outbox, &ended, || began.elapsed());
        let person = async {
            // The pump has found the move, and waits for room that does not come.
            tokio::time::sleep(Duration::from_millis(50)).await;
            ended.cancel();
        };
        pumped(pump, person).await;
    }

    #[tokio::test]
    async fn the_pump_ends_by_itself_when_the_stream_to_the_source_is_gone() {
        let (outbox, stream) = mpsc::channel(1);
        drop(stream);
        let input = Mutex::new(Queue::new(true));
        let ready = Notify::new();
        let ended = CancellationToken::new();
        let began = Instant::now();
        lock(&input)
            .push(Some(0), pointer(0.1, 0.1, PointerAction::Move))
            .unwrap();
        let pump = pump_input(&input, &ready, &outbox, &ended, || began.elapsed());
        pumped(pump, async {}).await;
        // The session is not over: nothing wrote to it.
        assert!(!ended.is_cancelled());
    }

    #[tokio::test]
    async fn moves_that_wait_for_room_in_the_stream_are_joined_into_the_latest() {
        // The stream to the source is full: there is room for nothing until a message is read.
        let (outbox, mut stream) = mpsc::channel(1);
        outbox
            .try_send(ViewerMessage::KeyFrame(KeyFrame { tried: 0 }))
            .unwrap();
        let input = Mutex::new(Queue::new(true));
        let ready = Notify::new();
        let ended = CancellationToken::new();
        let began = Instant::now();
        let step = |at: f32| {
            lock(&input)
                .push(Some(0), pointer(at, at, PointerAction::Move))
                .unwrap();
            ready.notify_one();
        };
        let pump = pump_input(&input, &ready, &outbox, &ended, || began.elapsed());
        let person = async {
            step(0.1);
            // The pump has found the move, and waits for room.
            tokio::time::sleep(Duration::from_millis(50)).await;
            step(0.2);
            step(0.3);
            tokio::time::sleep(Duration::from_millis(50)).await;
            // Room is made, and what is sent is where the pointer is by then.
            assert!(matches!(
                received(&mut stream).await,
                ViewerMessage::KeyFrame(_)
            ));
            let ViewerMessage::Input(batch) = received(&mut stream).await else {
                panic!("the input did not come");
            };
            assert_eq!(
                batch.events,
                [Event::Pointer {
                    frame: 0,
                    x: 0.3,
                    y: 0.3,
                    modifiers: Modifiers::NONE,
                    action: PointerAction::Move,
                }]
            );
            // And nothing follows.
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(stream.try_recv().is_err());
            ended.cancel();
        };
        pumped(pump, person).await;
    }
}
