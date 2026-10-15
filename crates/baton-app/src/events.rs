// SPDX-License-Identifier: Apache-2.0

//! What happens while the app runs, as it happens.

use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

use crate::model::{OfferOutcome, PairingRequest, Received};

/// Something a screen may want to react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A device handed this one something. It is in the inbox already.
    Received(Received),
    /// The list of paired devices changed: one was added, renamed, forgotten, or heard from.
    /// Read it again with [`App::devices`](crate::App::devices).
    DevicesChanged,
    /// A device wants to pair, using the offer that is showing. Show the request and answer
    /// it with [`App::answer_pairing`](crate::App::answer_pairing).
    PairingRequested(PairingRequest),
    /// A request no longer needs an answer: the offer ended, or the device gave up. Take the
    /// question off the screen.
    PairingRequestEnded {
        /// The [`PairingRequest::id`] that ended.
        id: u64,
    },
    /// The offer that was showing ended.
    OfferEnded(OfferOutcome),
    /// Events were missed because they came faster than they were read. Read the devices and
    /// the inbox again.
    ///
    /// A pairing request, or the end of an offer, among the missed events is not recovered
    /// this way yet: a request nobody answers runs out as a no after two minutes, and an
    /// offer ends when its time is up.
    Resync,
}

/// A stream of [`Event`]s from one point on.
///
/// Get one with [`App::events`](crate::App::events) *before* reading the inbox or the
/// devices, so that whatever arrives while reading is not lost: it is either in what was
/// read or in the events that follow.
#[derive(Debug)]
pub struct Events {
    rx: broadcast::Receiver<Event>,
    stopped: CancellationToken,
}

impl Events {
    pub(crate) const fn new(rx: broadcast::Receiver<Event>, stopped: CancellationToken) -> Self {
        Self { rx, stopped }
    }

    /// Waits for the next event. Returns `None` once the app has stopped, and after that
    /// always.
    pub async fn next(&mut self) -> Option<Event> {
        tokio::select! {
            biased;
            () = self.stopped.cancelled() => None,
            received = self.rx.recv() => match received {
                Ok(event) => Some(event),
                Err(RecvError::Lagged(_)) => Some(Event::Resync),
                Err(RecvError::Closed) => None,
            },
        }
    }
}
