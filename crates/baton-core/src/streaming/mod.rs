// SPDX-License-Identifier: Apache-2.0

//! Window streaming on the connections of a session (`docs/protocol.md` section 9, ADR 0009).
//!
//! A source sends the picture of one window to a viewer as a run of encoded frames, one
//! unidirectional stream for each, and each side keeps a control stream open to the other. The
//! rules about frames (which one the viewer decodes next, which one the source gives up on) are
//! in `baton-stream`, which has no input or output. This module carries them out on the streams
//! of a connection. What only a platform can do, decode a picture and show it, is behind a trait
//! that the platform implements.
//!
//! [`Streaming`] holds the sessions of a node and is installed as the [`Sink`] of the streams of
//! kind frame and kind control (`crate::uni`). A stream is taken only if its start names a
//! session that is running, that was made with the device that opened the stream, and that lives
//! on the connection the stream came on: sessions live and die with their connection, so a stream
//! of the same device on another connection is not part of the session. Any other stream is
//! stopped with code 499, the code for something stopped on purpose, because a frame can arrive
//! after its session has ended and that is no fault.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use baton_proto::{close_code, id::SessionId};
use iroh::endpoint::{Connection, RecvStream};

use crate::{
    error::{Error, Result},
    gate::code,
    trust::PairedDevice,
    uni::{Arrival, Kind, Sink},
    util::lock,
};

mod view;

use view::View;
#[allow(
    unused_imports,
    reason = "used by the code that makes a session, which comes next"
)]
pub(crate) use view::{DecodeError, Decoder, DecoderState, End, ViewHandle, ViewParams, ViewStats};

/// The window streaming sessions of a node, by the id that the source chose for each.
#[derive(Default)]
pub(crate) struct Streaming {
    views: Mutex<HashMap<SessionId, Arc<View>>>,
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
        let mut views = lock(&self.views);
        if views.contains_key(&session) {
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
        views.insert(session, view);
        Ok(handle)
    }

    /// Forgets a session that has ended, so that streams that arrive for it later are stopped.
    fn forget(&self, session: &SessionId) {
        lock(&self.views).remove(session);
    }

    fn find(&self, arrival: &Arrival) -> Option<Arc<View>> {
        let session = SessionId::from_bytes(arrival.id);
        let view = lock(&self.views).get(&session).cloned()?;
        view.belongs_to(&arrival.peer, &arrival.connection)
            .then_some(view)
    }
}

impl Sink for Streaming {
    fn take(&self, arrival: Arrival) {
        let Some(view) = self.find(&arrival) else {
            let mut recv = arrival.recv;
            stop(&mut recv, close_code::CANCELLED);
            return;
        };
        match arrival.kind {
            Kind::Video => {
                tokio::spawn(view.read_frame(arrival));
            }
            Kind::Control => {
                tokio::spawn(view.read_source_control(arrival));
            }
            // Lanes are not installed for this sink, so the dispatcher does not send them here.
            Kind::Lane => {
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
