// SPDX-License-Identifier: Apache-2.0

//! Window streaming on the connections of a session (`docs/protocol.md` section 9, ADR 0009).
//!
//! A source sends the picture of one window to a viewer as a run of encoded frames, one
//! unidirectional stream for each, and each side keeps a control stream open to the other. The
//! rules about frames (which one the viewer decodes next, which one the source gives up on) are
//! in `baton-stream`, which has no input or output. This module carries them out on the streams
//! of a connection. What only a platform can do, capture and encode a picture, decode it and show
//! it, is behind traits that the platform implements: [`SourceEvents`] and [`Decoder`].
//!
//! [`Streaming`] holds the sessions of a node, from either side, and is installed as the [`Sink`]
//! of the streams of kind frame and kind control (`crate::uni`). A stream is taken only if its
//! start names a session that is running, that was made with the device that opened the stream,
//! and that lives on the connection the stream came on: sessions live and die with their
//! connection, so a stream of the same device on another connection is not part of the session.
//! Any other stream is stopped with code 499, the code for something stopped on purpose, because
//! a frame can arrive after its session has ended and that is no fault.
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

use baton_proto::{close_code, id::SessionId, message::Message};
use baton_stream::{KIND_CONTROL, header::encode_prefix};
use iroh::endpoint::{Connection, RecvStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    error::{Error, Result},
    gate::code,
    trust::PairedDevice,
    uni::{Arrival, Kind, Sink},
    util::lock,
    wire::{ReadFailure, read_next, write_message},
};

mod source;
mod view;

use source::Source;
#[allow(
    unused_imports,
    reason = "used by the code that makes a session, which comes next"
)]
pub(crate) use source::{
    NotSent, Picture, SourceEvents, SourceHandle, SourceParams, SourceSetup, SourceStats,
};
use view::View;
#[allow(
    unused_imports,
    reason = "used by the code that makes a session, which comes next"
)]
pub(crate) use view::{DecodeError, Decoder, DecoderState, ViewHandle, ViewParams, ViewStats};

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    /// This device ended it, by dropping or ending the handle.
    Local,
    /// The other device ended it: it finished or reset its control stream.
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
}

/// The window streaming sessions of a node, by the id that the source chose for each.
#[derive(Default)]
pub(crate) struct Streaming {
    sessions: Mutex<HashMap<SessionId, Session>>,
}

impl Streaming {
    /// An empty set of sessions, to be installed as the sink of frames and of control streams.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Starts the viewer's side of a session that the person accepted: `peer` is the source, and
    /// `connection` is the connection that it offered the window on.
    ///
    /// The decoder is given the pictures in the order in which they are to be decoded, and the
    /// messages of the source. The session ends, and the decoder is told, when the source ends
    /// it, when it breaks a rule, when the connection ends, when the returned handle is dropped
    /// and when the source does not open its control stream in time.
    ///
    /// # Errors
    ///
    /// The id is in use. A source picks it at random, so this is a source that repeats one.
    pub(crate) fn view(
        self: &Arc<Self>,
        peer: &PairedDevice,
        connection: Connection,
        session: SessionId,
        decoder: impl Decoder,
        params: ViewParams,
    ) -> Result<ViewHandle> {
        let mut sessions = lock(&self.sessions);
        if sessions.contains_key(&session) {
            return Err(Error::protocol("the session id is in use"));
        }
        let (view, handle) = View::start(
            Arc::downgrade(self),
            session,
            peer.id,
            connection,
            Box::new(decoder),
            params,
        );
        sessions.insert(session, Session::View(view));
        Ok(handle)
    }

    /// Starts the source's side of a session that the viewer accepted: `peer` is the viewer, and
    /// `connection` is the connection that the window was offered on.
    ///
    /// The source opens its control stream at once and says what the picture will be like. Then
    /// the platform asks, picture by picture, whether to encode (`SourceHandle::begin`), and
    /// hands over what it encoded (`SourceHandle::sent`). The session ends, and the platform is
    /// told, when the viewer ends it, when it breaks a rule, when the connection ends, when the
    /// returned handle is dropped and when the viewer sends no report for too long.
    ///
    /// Must be called from within the runtime of the node: the platform calls the handle from
    /// threads of its own, and the frames are written by tasks of that runtime.
    ///
    /// # Errors
    ///
    /// The id is in use, or the picture format is beyond a limit of the protocol.
    pub(crate) fn source(
        self: &Arc<Self>,
        peer: &PairedDevice,
        connection: Connection,
        session: SessionId,
        setup: SourceSetup,
        events: impl SourceEvents,
    ) -> Result<SourceHandle> {
        let mut sessions = lock(&self.sessions);
        if sessions.contains_key(&session) {
            return Err(Error::protocol("the session id is in use"));
        }
        let (source, handle) = Source::start(
            Arc::downgrade(self),
            source::Whom {
                session,
                peer: peer.id,
                connection,
            },
            setup,
            Box::new(events),
        )?;
        sessions.insert(session, Session::Source(source));
        Ok(handle)
    }

    /// Forgets a session that has ended, so that streams that arrive for it later are stopped.
    fn forget(&self, session: &SessionId) {
        lock(&self.sessions).remove(session);
    }

    fn find(&self, arrival: &Arrival) -> Option<Session> {
        let session = SessionId::from_bytes(arrival.id);
        let found = lock(&self.sessions).get(&session).cloned()?;
        found
            .belongs_to(&arrival.peer, &arrival.connection)
            .then_some(found)
    }

    /// The viewer's side of a session, if there is one.
    #[cfg(test)]
    fn viewing(&self, session: &SessionId) -> Option<Arc<View>> {
        match lock(&self.sessions).get(session)? {
            Session::View(view) => Some(Arc::clone(view)),
            Session::Source(_) => None,
        }
    }

    /// The source's side of a session, if there is one.
    #[cfg(test)]
    fn sourcing(&self, session: &SessionId) -> Option<Arc<Source>> {
        match lock(&self.sessions).get(session)? {
            Session::Source(source) => Some(Arc::clone(source)),
            Session::View(_) => None,
        }
    }

    /// Whether the node has no session.
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        lock(&self.sessions).is_empty()
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
            (Session::View(view), Kind::Control) => {
                tokio::spawn(view.read_source_control(arrival));
            }
            (Session::Source(source), Kind::Control) => {
                tokio::spawn(source.read_viewer_control(arrival));
            }
            // A frame goes from the source to the viewer and not back, and lanes are not for this
            // sink: the dispatcher does not send them here.
            (Session::Source(_), Kind::Video) | (_, Kind::Lane) => {
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
            Ok(None) | Err(ReadFailure::Transport(_)) => return ControlEnd::Peer,
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
