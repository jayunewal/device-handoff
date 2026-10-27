// SPDX-License-Identifier: Apache-2.0

//! Window streaming on the connections of a session (`docs/protocol.md` section 9, ADR 0009).
//!
//! A source sends the picture of one window to a viewer as a run of encoded frames, one
//! unidirectional stream for each, and each side keeps a control stream open to the other. The
//! rules about frames (which one the viewer decodes next, which one the source gives up on) are
//! in `baton-stream`, which has no input or output. This module carries them out on the streams
//! of a connection. What only a platform can do, capture and encode a picture, decode it and show
//! it, is behind traits that the platform implements: `SourceEvents` and [`Decoder`].
//!
//! `Streaming` holds the sessions of a node, from either side, and is installed as the `Sink`
//! of the streams of kind frame, kind control and kind sound (`crate::uni`). A stream is taken
//! only if its start names a session that is running, that was made with the device that opened
//! the stream, and that lives on the connection the stream came on: sessions live and die with
//! their connection, so a stream of the same device on another connection is not part of the
//! session. Any other stream is stopped with code 499, the code for something stopped on purpose,
//! because a frame can arrive after its session has ended and that is no fault.
//!
//! When the screen of a source is shared whole, its sound can go with it, in packets on streams of
//! their own and with two more messages on the control streams. `sound` holds what the platform is
//! given and provides for it, and what the sessions keep.
//!
//! The two sides write and read their control streams in the same way, so that is here: a stream
//! that carries one kind of message, in which a message that cannot be read, or that stops
//! arriving, is a violation, and whose end is the end of the session.

use std::{
    collections::HashMap,
    ops::ControlFlow,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

pub use baton_proto::stream::{Wanted, WhyNot};
use baton_proto::{
    DeviceId, close_code,
    id::SessionId,
    message::Message,
    stream::{Accepted, Codec},
};
use baton_stream::{KIND_CONTROL, control::Config, header::encode_prefix};
use iroh::endpoint::{Connection, RecvStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    error::{Error, Result},
    gate::code,
    session::Hold,
    trust::PairedDevice,
    uni::{Arrival, Kind, Sink},
    util::lock,
    wire::{ReadFailure, read_next, write_message},
};

mod answer;
#[cfg(test)]
mod answer_tests;
mod approve;
mod offer;
#[cfg(test)]
mod offer_tests;
mod sound;
#[cfg(test)]
mod sound_tests;
mod source;
mod start;
#[cfg(test)]
mod start_tests;
pub(crate) mod unlock;
#[cfg(test)]
mod unlock_tests;
mod view;

/// How many frame streams a viewer takes at once from a source on a connection: six in flight and
/// up to six more that were reset and are still counted (ADR 0009).
pub(crate) const FRAME_STREAMS: u32 = 12;

/// How many control streams a device takes from the other end of a session on a connection.
pub(crate) const CONTROL_STREAMS: u32 = 1;

/// How many packet streams of sound a viewer takes at once from a source on a connection: eight in
/// flight and up to eight more that were reset and are still counted (ADR 0012).
pub(crate) const SOUND_STREAMS: u32 = 16;

const _: () = assert!(FRAME_STREAMS as usize == baton_stream::MAX_FRAME_STREAMS);
const _: () = assert!(SOUND_STREAMS as usize == baton_stream::MAX_SOUND_STREAMS);

use answer::Offers;
pub(crate) use answer::Viewer;
pub use answer::{Viewing, WindowHandler, WindowOffer, WindowVerdict};
pub use approve::{KeyOutcome, UnlockApprover};

use offer::Sending;
pub(crate) use offer::{Offered, Reservation, SETTLE, Sharer, Withdraw, ask};
pub use offer::{ShareOutcome, WindowShare};

pub(crate) use sound::SoundPlan;
pub use sound::{Play, SoundPacket, Speaker, playable};
use source::Source;
pub use source::{NotSent, Picture, SourceEvents, SourceHandle, SourceStats};
pub(crate) use source::{SourceParams, SourceSetup};
use start::Pending;
pub(crate) use start::Starter;
pub use start::{StartHandler, StartOutcome, StartRequest, StartSlot, StartVerdict};
pub use unlock::{
    CancelReason, Finish as UnlockFinish, KeepError, Question, Settled, UnlockAttempt,
    UnlockOutcome, Unlocker,
};
pub(crate) use view::ViewParams;
pub use view::{DecodeError, Decoder, DecoderState, ViewHandle, ViewStats};
use view::{Granted, View};

/// What a viewer said it can take, in `Accepted`. The source keeps within it, and a source that
/// sends a picture beyond it has broken the session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Takes {
    /// The widest picture in pixels.
    pub max_width: u32,
    /// The highest picture in pixels.
    pub max_height: u32,
    /// The most frames a second.
    pub max_fps: u16,
    /// The codecs it can decode. A codec that this version does not know is never one of them.
    pub codecs: Vec<Codec>,
}

impl Takes {
    /// Whether the viewer said that it can take the picture that `config` describes.
    #[must_use]
    pub fn allows(&self, config: &Config) -> bool {
        config.width <= self.max_width
            && config.height <= self.max_height
            && config.fps <= self.max_fps
            && config.codec != Codec::Other
            && self.codecs.contains(&config.codec)
    }

    fn of(accepted: &Accepted) -> Self {
        Self {
            max_width: accepted.max_width,
            max_height: accepted.max_height,
            max_fps: accepted.max_fps,
            codecs: accepted.codecs.clone(),
        }
    }
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum End {
    /// This device ended it: the handle was dropped or ended, the device was unpaired, or the
    /// node stopped.
    Local,
    /// The other device ended it: it sent a `Stop` for it, or finished or reset its control
    /// stream.
    Peer,
    /// The connection ended.
    Connection,
    /// The other device broke a rule of the protocol.
    Violation,
    /// The source did not open its control stream in time. Only a viewer sees this.
    NoControl,
    /// The other device began a message of its control stream and did not finish it in time. The
    /// time is this device's choice and the other device broke no rule, so this device finishes
    /// its own stream.
    Stalled,
    /// The viewer sent no report for the time that ends a session. Only a source sees this.
    Silent,
    /// Something here went wrong that should not be able to.
    Internal,
}

/// A session of this node, from one side or the other.
#[derive(Clone)]
enum Session {
    View(Arc<View>),
    Source(Arc<Source>),
}

impl Session {
    /// Whether `peer` on `connection` is the other end of this session.
    fn belongs_to(&self, peer: &PairedDevice, connection: &Connection) -> bool {
        match self {
            Self::View(view) => view.belongs_to(peer, connection),
            Self::Source(source) => source.belongs_to(peer, connection),
        }
    }

    /// The device at the other end.
    fn peer(&self) -> DeviceId {
        match self {
            Self::View(view) => view.peer(),
            Self::Source(source) => source.peer(),
        }
    }

    /// Whether this device is the source of the session.
    fn is_source(&self) -> bool {
        matches!(self, Self::Source(_))
    }

    /// Ends the session, if it has not ended.
    fn end(&self, how: End) {
        match self {
            Self::View(view) => view.end(how),
            Self::Source(source) => source.end(how),
        }
    }
}

/// What the node keeps about window streaming, under one lock so that a check of several parts is
/// not undone by another task between them.
#[derive(Default)]
struct Table {
    /// The sessions, by the id that the source chose for each.
    sessions: HashMap<SessionId, Session>,
    /// The offers that this device has been made as a viewer.
    offers: Offers,
    /// The place of the one session that this device has as a source, from the offer to the
    /// session. The session itself is in `sessions` from then on.
    sending: Option<Sending>,
    /// The `Stop`s that this device sent and that are not answered yet, by the device they went
    /// to. Each token is cancelled when its `Stop` has been answered or has come to nothing.
    stops: Vec<(DeviceId, CancellationToken)>,
    /// The one start that this device made as a viewer and whose offer has not come. It holds the
    /// viewer's place, as an offer that waits for the person does.
    starting: Option<Pending>,
}

impl Table {
    /// Whether a window is shown.
    fn has_view(&self) -> bool {
        self.sessions
            .values()
            .any(|session| matches!(session, Session::View(_)))
    }

    /// Whether a window is being offered or shared.
    fn has_source(&self) -> bool {
        self.sending.is_some()
            || self
                .sessions
                .values()
                .any(|session| matches!(session, Session::Source(_)))
    }
}

/// The window streaming sessions of a node, from either side.
pub(crate) struct Streaming {
    table: Mutex<Table>,
    /// What a device needs to show the windows of others, if it does.
    viewer: Option<Viewer>,
    /// What a device needs to show its own windows to others, if it does.
    sharer: Option<Sharer>,
}

impl Streaming {
    /// A set of sessions that takes no part on its own: it holds the sessions that are started
    /// on it, which is what the tests of the two sides of a session do.
    #[cfg(test)]
    pub(crate) fn new() -> Arc<Self> {
        Self::taking_part(None, None)
    }

    /// A set of sessions of a device that takes part as a viewer, as a source, both or neither.
    pub(crate) fn taking_part(viewer: Option<Viewer>, sharer: Option<Sharer>) -> Arc<Self> {
        Arc::new(Self {
            table: Mutex::default(),
            viewer,
            sharer,
        })
    }

    /// The same as [`view_in`](Self::view_in), with the lock of the table taken here, for the tests
    /// of the viewer.
    #[cfg(test)]
    pub(crate) fn view(
        self: &Arc<Self>,
        peer: &PairedDevice,
        connection: Connection,
        session: SessionId,
        viewing: Viewing,
        control: bool,
        private: bool,
    ) -> Result<ViewHandle> {
        let granted = Granted {
            control,
            private,
            sound: false,
            unlock: false,
        };
        self.view_granted(peer, connection, session, viewing, granted)
    }

    /// The same, for a source that said what it sends.
    #[cfg(test)]
    fn view_granted(
        self: &Arc<Self>,
        peer: &PairedDevice,
        connection: Connection,
        session: SessionId,
        viewing: Viewing,
        granted: Granted,
    ) -> Result<ViewHandle> {
        let mut table = lock(&self.table);
        self.view_in(&mut table, peer, connection, session, viewing, granted)
    }

    /// Starts the viewer's side of a session that the person accepted: `peer` is the source, and
    /// `connection` is the connection that it offered the window on. The caller holds the lock of
    /// the table, so that it can look at something else in the same hold of it.
    ///
    /// The decoder is given the pictures in the order in which they are to be decoded, and the
    /// messages of the source. `viewing.takes` is what the viewer said it can take: a source that
    /// describes a picture beyond it breaks a rule. `granted` is what the offer granted the viewer
    /// and said of the source: control of the window, without which the session sends nothing that
    /// the person does, a private screen, without which it sends no [`ViewHandle::lift`], and
    /// sound, without which the person cannot ask to hear it. The session ends, and the decoder is
    /// told, when the source ends it, when it breaks a rule, when the connection ends, when the
    /// returned handle is dropped and when the source does not open its control stream in time.
    ///
    /// # Errors
    ///
    /// The id is in use. A source picks it at random, so this is a source that repeats one.
    fn view_in(
        self: &Arc<Self>,
        table: &mut Table,
        peer: &PairedDevice,
        connection: Connection,
        session: SessionId,
        viewing: Viewing,
        granted: Granted,
    ) -> Result<ViewHandle> {
        if table.sessions.contains_key(&session) {
            return Err(Error::protocol("the session id is in use"));
        }
        let (view, handle) = View::start(
            Arc::downgrade(self),
            session,
            peer.id,
            connection,
            viewing,
            granted,
            self.hears(),
        );
        table.sessions.insert(session, Session::View(view));
        Ok(handle)
    }

    /// Starts the source's side of a session that the viewer accepted: `peer` is the viewer, and
    /// `connection` is the connection that the window was offered on.
    ///
    /// The source opens its control stream at once and says what the picture will be like. Then
    /// the platform asks, picture by picture, whether to encode (`SourceHandle::begin`), and
    /// hands over what it encoded (`SourceHandle::sent`). The session ends, and the platform is
    /// told, when the viewer ends it, when it breaks a rule, when the connection ends, when the
    /// returned handle is dropped and when the viewer sends no report for too long. The `hold`,
    /// if there is one, keeps the connection in the pool for as long as the session runs.
    ///
    /// Must be called from within the runtime of the node: the platform calls the handle from
    /// threads of its own, and the frames are written by tasks of that runtime.
    ///
    /// # Errors
    ///
    /// The id is in use, or the picture format is beyond a limit of the protocol or beyond what
    /// the viewer takes.
    pub(crate) fn source(
        self: &Arc<Self>,
        peer: &PairedDevice,
        connection: Connection,
        hold: Option<Hold>,
        session: SessionId,
        setup: SourceSetup,
        events: impl SourceEvents,
    ) -> Result<SourceHandle> {
        let mut table = lock(&self.table);
        if table.sessions.contains_key(&session) {
            return Err(Error::protocol("the session id is in use"));
        }
        let (source, handle) = Source::start(
            Arc::downgrade(self),
            source::Whom {
                session,
                peer: peer.id,
                connection,
                hold,
            },
            setup,
            Box::new(events),
        )?;
        table.sessions.insert(session, Session::Source(source));
        Ok(handle)
    }

    /// Forgets a session that has ended, so that streams that arrive for it later are stopped.
    fn forget(&self, session: &SessionId) {
        lock(&self.table).sessions.remove(session);
    }

    /// Ends every session with `peer` and withdraws its offer that waits, as when it is unpaired.
    pub(crate) fn unpair(&self, peer: &DeviceId) {
        self.end_where(|other| other == peer);
    }

    /// Does the same for every device that `paired` does not say is paired, as when the list of
    /// paired devices changed and not every change was seen.
    pub(crate) fn unpair_unless(&self, paired: impl Fn(&DeviceId) -> bool) {
        self.end_where(|other| !paired(other));
    }

    /// Ends every session, as when the device stops.
    pub(crate) fn shutdown(&self) {
        self.end_where(|_| true);
    }

    fn end_where(&self, doomed: impl Fn(&DeviceId) -> bool) {
        let ending: Vec<Session> = {
            let mut table = lock(&self.table);
            table.offers.withdraw_where(&doomed);
            // The call that made the start hears of it from the sender that is dropped here.
            table.starting.take_if(|pending| doomed(&pending.peer));
            if let Some(sending) = &table.sending
                && doomed(&sending.peer)
            {
                sending.cancel.cancel();
            }
            table
                .sessions
                .values()
                .filter(|session| doomed(&session.peer()))
                .cloned()
                .collect()
        };
        // A session takes the lock itself when it ends, to forget itself.
        for session in ending {
            session.end(End::Local);
        }
    }

    fn find(&self, arrival: &Arrival) -> Option<Session> {
        let session = SessionId::from_bytes(arrival.id);
        let found = lock(&self.table).sessions.get(&session).cloned()?;
        found
            .belongs_to(&arrival.peer, &arrival.connection)
            .then_some(found)
    }

    /// The viewer's side of a session, if there is one.
    #[cfg(test)]
    fn viewing(&self, session: &SessionId) -> Option<Arc<View>> {
        match lock(&self.table).sessions.get(session)? {
            Session::View(view) => Some(Arc::clone(view)),
            Session::Source(_) => None,
        }
    }

    /// The source's side of a session, if there is one.
    #[cfg(test)]
    fn sourcing(&self, session: &SessionId) -> Option<Arc<Source>> {
        match lock(&self.table).sessions.get(session)? {
            Session::Source(source) => Some(Arc::clone(source)),
            Session::View(_) => None,
        }
    }

    /// Whether the node has no session.
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        lock(&self.table).sessions.is_empty()
    }
}

impl Sink for Streaming {
    fn take(&self, arrival: Arrival) {
        let Some(session) = self.find(&arrival) else {
            let mut recv = arrival.recv;
            stop(&mut recv, close_code::CANCELLED);
            return;
        };
        match (session, arrival.kind) {
            (Session::View(view), Kind::Video) => {
                tokio::spawn(view.read_frame(arrival));
            }
            (Session::View(view), Kind::Sound) => {
                tokio::spawn(view.read_packet(arrival));
            }
            (Session::View(view), Kind::Control) => {
                tokio::spawn(view.read_source_control(arrival));
            }
            (Session::Source(source), Kind::Control) => {
                tokio::spawn(source.read_viewer_control(arrival));
            }
            // A frame or a packet goes from the source to the viewer and not back, and lanes are
            // not for this sink: the dispatcher does not send them here.
            (Session::Source(_), Kind::Video | Kind::Sound) | (_, Kind::Lane) => {
                let mut recv = arrival.recv;
                stop(&mut recv, close_code::PROTOCOL_VIOLATION);
            }
        }
    }
}

/// Stops a stream, with the code that says why.
fn stop(recv: &mut RecvStream, error: u32) {
    // A stream that has ended already needs no stopping.
    let _ = recv.stop(code(error));
}

/// How the reading of a control stream ended.
pub(super) enum ControlEnd {
    /// The session ended first, and the stream was stopped.
    Over,
    /// The other device finished the stream between two messages, or reset it, or the connection
    /// went: it has ended the session.
    Peer,
    /// The other device broke a rule, and the stream was stopped with the code that says so.
    Violation,
    /// The other device began a message and did not finish it in time, and the stream was stopped
    /// with 408. The time is this device's own choice, so no rule was broken.
    Stalled,
}

/// Reads the messages of a control stream one after the other and gives each to `each`, until
/// the stream ends, a rule is broken, or `over` is cancelled.
///
/// The wait for a message to begin has no limit, because a control stream can be quiet. Once the
/// length of a message has arrived, the rest of it has `stall`. `each` returns `Break` for a
/// message that breaks a rule of the session.
pub(super) async fn read_control<M: Message>(
    mut recv: RecvStream,
    stall: Duration,
    over: &CancellationToken,
    mut each: impl FnMut(M) -> ControlFlow<()>,
) -> ControlEnd {
    loop {
        let next = tokio::select! {
            () = over.cancelled() => {
                stop(&mut recv, close_code::CANCELLED);
                return ControlEnd::Over;
            }
            next = read_next::<M>(&mut recv, stall) => next,
        };
        match next {
            Ok(Some(message)) => {
                if each(message).is_break() {
                    stop(&mut recv, close_code::PROTOCOL_VIOLATION);
                    return ControlEnd::Violation;
                }
            }
            Ok(None) | Err(ReadFailure::Reset(_) | ReadFailure::Transport(_)) => {
                return ControlEnd::Peer;
            }
            Err(ReadFailure::BadHeader(_) | ReadFailure::BadPayload(_)) => {
                stop(&mut recv, close_code::PROTOCOL_VIOLATION);
                return ControlEnd::Violation;
            }
            Err(ReadFailure::TimedOut) => {
                stop(&mut recv, close_code::TIMEOUT);
                return ControlEnd::Stalled;
            }
        }
    }
}

/// Why the writing of a control stream failed.
pub(super) enum WriteEnd {
    /// The stream could not be opened or started: the connection is gone.
    Connection,
    /// The other device stopped the stream.
    Peer,
}

/// How long a session that has ended already has to open its control stream. Opening a stream
/// takes no time unless the other device has not allowed another.
const OPEN_GRACE: Duration = Duration::from_secs(1);

/// Opens this device's control stream, writes the 17 bytes that start every stream of a session,
/// and then each message that waits in `queue`, one after the other, until `over` is cancelled.
///
/// The stream is then finished, or reset with the code that says why if `how` says that the
/// session ended because the other device broke a rule, or because of a fault here. The other
/// side reads the end of the stream as the end of the session, and a reset as a statement that
/// the session broke a rule. A session that has ended before the stream was opened opens it all
/// the same, and ends it at once, because the other side that has seen no stream waits for one.
pub(super) async fn write_control<M: Message>(
    connection: &Connection,
    session: SessionId,
    mut queue: mpsc::Receiver<M>,
    over: &CancellationToken,
    how: &OnceLock<End>,
) -> std::result::Result<(), WriteEnd> {
    let opened = tokio::select! {
        opened = connection.open_uni() => opened,
        () = async {
            over.cancelled().await;
            tokio::time::sleep(OPEN_GRACE).await;
        } => return Ok(()),
    };
    let Ok(mut send) = opened else {
        return Err(WriteEnd::Connection);
    };
    let start = encode_prefix(KIND_CONTROL, session);
    if send.write_all(&start).await.is_err() {
        return Err(WriteEnd::Connection);
    }
    loop {
        tokio::select! {
            () = over.cancelled() => break,
            message = queue.recv() => {
                let Some(message) = message else { break };
                if write_message(&mut send, &message).await.is_err() {
                    return Err(WriteEnd::Peer);
                }
            }
        }
    }
    match how.get() {
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
    Ok(())
}
