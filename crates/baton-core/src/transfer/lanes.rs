// SPDX-License-Identifier: Apache-2.0

//! Takes the lanes of the transfers that other devices send.
//!
//! By the time a lane gets here, the dispatcher (see [`uni`](crate::uni)) has read its first 17
//! bytes, checked that the device is still paired, and counted the stream. What is left is what only
//! this module knows: which transfer the lane belongs to, and which attempt of it (section 8,
//! Lanes). The 18th byte is the number of the lane, and everything after it is frames.
//!
//! The table of incoming transfers says which attempt a lane belongs to ([`Incoming::lane`]): the
//! one begun on the connection the lane came on, if the transfer was accepted and is going. A lane
//! that belongs to nothing is stopped with 499. It can arrive late, and that is no one's fault.
//!
//! A lane lets go in the order that section 9 asks for. It lets go of the stream and then of the
//! stream's place in the count of the dispatcher, and only then of the guard that `Finish` and
//! `Cancel` wait for, so that a sender which has its `Done` can open the lanes of the next transfer
//! at once without finding the places of the last one still taken.

use std::time::Duration;

use baton_proto::{close_code, id::TransferId};

use super::{incoming::Incoming, receive::LaneEnd};
use crate::{
    uni::{Arrival, Sink, stop},
    util::contained,
};

/// What takes the lanes of a node that takes files.
pub(crate) struct LaneSink {
    incoming: Incoming,
    /// How long a lane has to send its number after the start that the dispatcher read.
    number_deadline: Duration,
}

impl LaneSink {
    pub(crate) fn new(incoming: Incoming, number_deadline: Duration) -> Self {
        Self {
            incoming,
            number_deadline,
        }
    }
}

impl Sink for LaneSink {
    fn take(&self, arrival: Arrival) {
        let incoming = self.incoming.clone();
        let deadline = self.number_deadline;
        drop(tokio::spawn(contained(
            "a lane",
            serve_lane(incoming, deadline, arrival),
        )));
    }
}

async fn serve_lane(incoming: Incoming, deadline: Duration, arrival: Arrival) {
    let Arrival {
        id,
        peer,
        link,
        mut recv,
        slot,
        ..
    } = arrival;
    let Some((attempt, guard)) = incoming.lane(&peer.id, TransferId::from_bytes(id), link) else {
        stop(&mut recv, close_code::CANCELLED);
        return;
    };
    let mut number = [0_u8; 1];
    match tokio::time::timeout(deadline, recv.read_exact(&mut number)).await {
        Ok(Ok(())) => match attempt.lane(number[0], &mut recv).await {
            LaneEnd::Finished | LaneEnd::Failed => {}
            LaneEnd::Stopped(code) => stop(&mut recv, code),
        },
        Err(_) => stop(&mut recv, close_code::TIMEOUT),
        // The peer reset the stream or the connection went: there is nothing to stop.
        Ok(Err(_)) => {}
    }
    drop(recv);
    drop(slot);
    drop(guard);
}
