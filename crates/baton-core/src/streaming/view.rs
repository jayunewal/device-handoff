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
//! A session ends when the source ends it, when it breaks a rule, when the connection ends, when
//! the person ends it, when the source does not open its control stream in time, and when a message
//! of the source stalls. It ends once: the decoder is told, the pictures that are held are dropped,
//! and later streams of the session are stopped with code 499. The decoder is told nothing after
//! that, whatever was on its way.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{DeviceId, close_code, id::SessionId};
use baton_stream::{
    KIND_CONTROL, KIND_VIDEO,
    control::{KeyFrame, Report, SourceMessage, Thermal, ViewerMessage, Want},
    header::{FrameHeader, PREFIX_LEN, encode_prefix},
    viewer::{Action, Params as Rules, Refusal, Stats, StreamId, Viewer, Violation},
};
use iroh::endpoint::{Connection, ReadExactError, RecvStream};
use tokio::{sync::mpsc, time::MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::{Streaming, stop};
use crate::{
    gate::code,
    trust::PairedDevice,
    uni::Arrival,
    util::lock,
    wire::{ReadFailure, read_next, write_message},
};

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
pub(crate) struct DecodeError;

/// How the platform's decoder is doing, for the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecoderState {
    /// How long the decoder takes for a frame, in microseconds, as it smooths it.
    pub(crate) decode_us: u32,
    /// How hot the device is.
    pub(crate) thermal: Thermal,
}

/// The decoder and the screen of a session, which the platform provides.
///
/// Every method is called with the session's lock held, so none of them waits: a decoder that
/// takes time hands the picture to a thread of its own, and says if it fails afterwards through
/// [`ViewHandle::decode_failed`].
pub(crate) trait Decoder: Send + 'static {
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

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    /// This device ended it, by dropping or ending the handle.
    Local,
    /// The source ended it: it finished or reset its control stream.
    Source,
    /// The connection ended.
    Connection,
    /// The source broke a rule of the protocol.
    Violation,
    /// The source did not open its control stream in time.
    NoControl,
    /// The source began a message of its control stream and did not finish it in time. The time is
    /// this device's choice and the source broke no rule, so the viewer finishes its own stream.
    Stalled,
    /// Something here went wrong that should not be able to.
    Internal,
}

/// What a session has counted, for the person and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewStats {
    /// What the rules of the viewer have counted.
    pub(crate) frames: Stats,
    /// The newest frame that was shown.
    pub(crate) shown: Option<u64>,
    /// The bytes of pictures that the viewer holds.
    pub(crate) held_bytes: usize,
    /// The frame streams that are being read.
    pub(crate) streams: usize,
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
    params: ViewParams,
    began: Instant,
    state: Mutex<State>,
    /// What the viewer says to the source, in the order in which it says it.
    outbox: mpsc::Sender<ViewerMessage>,
    /// Whether the source's control stream has been taken. A second one is a violation.
    source_control: AtomicBool,
    /// Cancelled when the source's control stream has been taken.
    control_taken: CancellationToken,
    /// Cancelled when the session ends.
    ended: CancellationToken,
    end: OnceLock<End>,
}

/// What the owner of a session holds. Dropping it ends the session.
pub(crate) struct ViewHandle {
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
    pub(crate) fn decode_failed(&self) {
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
    pub(crate) fn want(&self, want: Want) {
        self.view.say(ViewerMessage::Want(want));
    }

    /// What has been counted so far.
    pub(crate) fn stats(&self) -> ViewStats {
        let state = lock(&self.view.state);
        ViewStats {
            frames: state.viewer.stats(),
            shown: state.viewer.shown(),
            held_bytes: state.viewer.held_bytes(),
            streams: state.viewer.streams(),
        }
    }

    /// Ends the session.
    pub(crate) fn end(&self) {
        self.view.end(End::Local);
    }

    /// Waits until the session is over, and says why.
    pub(crate) async fn ended(&self) -> End {
        self.view.ended.cancelled().await;
        self.view.end.get().copied().unwrap_or(End::Internal)
    }
}

impl Drop for ViewHandle {
    fn drop(&mut self) {
        self.view.end(End::Local);
    }
}

impl View {
    /// Makes the session and starts what runs for as long as it does.
    pub(super) fn start(
        registry: Weak<Streaming>,
        session: SessionId,
        peer: DeviceId,
        connection: Connection,
        decoder: Box<dyn Decoder>,
        params: ViewParams,
    ) -> (Arc<Self>, ViewHandle) {
        let (outbox, queue) = mpsc::channel(OUTBOX);
        let view = Arc::new(Self {
            registry,
            session,
            peer,
            connection,
            params,
            began: Instant::now(),
            state: Mutex::new(State {
                viewer: Viewer::new(params.rules),
                pictures: HashMap::new(),
                decoder,
                closed: false,
            }),
            outbox,
            source_control: AtomicBool::new(false),
            control_taken: CancellationToken::new(),
            ended: CancellationToken::new(),
            end: OnceLock::new(),
        });
        tokio::spawn(Arc::clone(&view).supervise());
        tokio::spawn(Arc::clone(&view).write_control(queue));
        tokio::spawn(Arc::clone(&view).keep_time());
        let handle = ViewHandle {
            view: Arc::clone(&view),
        };
        (view, handle)
    }

    /// Whether `peer` on `connection` is the other end of this session.
    pub(super) fn belongs_to(&self, peer: &PairedDevice, connection: &Connection) -> bool {
        self.peer == peer.id && self.connection.stable_id() == connection.stable_id()
    }

    fn now(&self) -> Duration {
        self.began.elapsed()
    }

    /// Ends the session, once. The caller does not hold the session's lock.
    fn end(&self, how: End) {
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

    /// Writes the viewer's control stream: the 17 bytes that start every stream of a session, and
    /// then the messages that wait in the queue, one after the other.
    async fn write_control(self: Arc<Self>, mut queue: mpsc::Receiver<ViewerMessage>) {
        tokio::select! {
            () = self.ended.cancelled() => return,
            () = self.control_taken.cancelled() => {}
        }
        let opened = tokio::select! {
            () = self.ended.cancelled() => return,
            opened = self.connection.open_uni() => opened,
        };
        let Ok(mut send) = opened else {
            self.end(End::Connection);
            return;
        };
        let start = encode_prefix(KIND_CONTROL, self.session);
        if send.write_all(&start).await.is_err() {
            self.end(End::Connection);
            return;
        }
        loop {
            tokio::select! {
                () = self.ended.cancelled() => break,
                message = queue.recv() => {
                    let Some(message) = message else { break };
                    if write_message(&mut send, &message).await.is_err() {
                        self.end(End::Source);
                        break;
                    }
                }
            }
        }
        // The source reads the end of the stream as the end of the session, and a reset as the
        // viewer saying that the source broke a rule.
        match self.end.get() {
            Some(End::Violation) => {
                let _ = send.reset(code(close_code::PROTOCOL_VIOLATION));
            }
            Some(End::Internal) => {
                let _ = send.reset(code(close_code::INTERNAL));
            }
            _ => {
                let _ = send.finish();
            }
        }
    }

    /// Reads the source's control stream until the session ends.
    pub(super) async fn read_source_control(self: Arc<Self>, arrival: Arrival) {
        let Arrival { mut recv, slot, .. } = arrival;
        let _slot = slot;
        if self.source_control.swap(true, Ordering::SeqCst) {
            // A session has one control stream from each side.
            stop(&mut recv, close_code::PROTOCOL_VIOLATION);
            self.end(End::Violation);
            return;
        }
        self.control_taken.cancel();
        loop {
            let next = tokio::select! {
                () = self.ended.cancelled() => {
                    stop(&mut recv, close_code::CANCELLED);
                    return;
                }
                next = read_next::<SourceMessage>(&mut recv, self.params.stall) => next,
            };
            match next {
                Ok(Some(message)) => self.deliver(message),
                // The source finished its stream between two messages, or reset it, or the
                // connection went: it has ended the session.
                Ok(None) | Err(ReadFailure::Transport(_)) => {
                    self.end(End::Source);
                    return;
                }
                Err(ReadFailure::BadHeader(_) | ReadFailure::BadPayload(_)) => {
                    stop(&mut recv, close_code::PROTOCOL_VIOLATION);
                    self.end(End::Violation);
                    return;
                }
                // The time is this device's own choice, so the source broke no rule.
                Err(ReadFailure::TimedOut) => {
                    stop(&mut recv, close_code::TIMEOUT);
                    self.end(End::Stalled);
                    return;
                }
            }
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
        control::{Config, Pause, PauseReason},
        header::Flags,
    };
    use iroh::endpoint::SendStream;

    use super::*;
    use crate::{
        testing::{Device, PATIENCE, connect, next, nothing_arrives, stopped_with},
        uni::{Kind, Sinks},
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
                    Recording {
                        log: log.clone(),
                        refuse: Arc::clone(&refuse),
                    },
                    params,
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
            lock(&self.streaming.views)
                .get(&session_id())
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
            Err(ReadFailure::Transport(_)) => None,
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
                Keep(Arc::clone(&pictures)),
                ViewParams::default(),
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
        assert_eq!(rig.ended().await, End::Source);
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
        wait_until("the session to be forgotten", || {
            lock(&streaming.views).is_empty()
        })
        .await;
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
            Recording {
                log: Log::default(),
                refuse: Arc::default(),
            },
            quick(),
        );
        assert!(again.is_err());
        // The session that was there first is untouched.
        rig.frame(&key(0), &[0; 10]).await;
        rig.until("frame 0 to be decoded", |rig| rig.log.decoded() == [0])
            .await;
    }
}
