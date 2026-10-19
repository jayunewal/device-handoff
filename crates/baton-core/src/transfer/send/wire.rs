// SPDX-License-Identifier: Apache-2.0

//! What the sender needs from the network, and nothing more.
//!
//! The engine does not know about QUIC. It asks a [`Wire`] for a connection to the receiver, and
//! with the [`Link`] that gives it, makes the requests of section 8 and opens lanes. A lane is
//! written through a [`LaneOut`], which is what a unidirectional stream gives: a write that takes
//! as much as the stream has room for, an end, and a reset.
//!
//! The wire over a node's connections comes with the part of the sender that is connected to the
//! node. The tests of the engine use two others: one that talks to a real table of incoming
//! transfers in memory, and one that a test scripts, so that every fault that section 8 tells the
//! sender about (a stop with each code, a stall, a connection that goes) happens when the test says.

use std::time::Duration;

use baton_proto::{
    id::TransferId,
    message::ErrorCode,
    transfer::{Cancel, TransferReply, TransferRequest},
};

use crate::util::BoxFuture;

/// Where connections to one receiver come from.
pub(crate) trait Wire: Send + Sync + 'static {
    /// A connection to the receiver that is new, even if one is up already: a transfer that is
    /// tried again does not go on the connection of the attempt before (section 8, Resuming).
    fn connect(&self) -> BoxFuture<'_, Result<Box<dyn Link>, WireError>>;
}

/// One connection to the receiver. Dropping it lets go of the connection, and a transfer holds it
/// from before the offer until the outcome is known.
pub(crate) trait Link: Send + Sync {
    /// Makes `request` on a stream of its own, and waits `deadline` at most for the reply. A request
    /// that has no reply by then is dropped, as a request is that the connection goes with.
    fn request(
        &self,
        request: TransferRequest,
        deadline: Duration,
    ) -> BoxFuture<'_, Result<Reply, WireError>>;

    /// Opens lane `number` of the transfer `id`, and writes its header. It waits for as long as the
    /// receiver allows no more streams, and the caller bounds that.
    ///
    /// The caller drops the future when the time it allows runs out, and when the attempt is given
    /// up. A stream that was opened by then must not be left to be finished: QUIC finishes a stream
    /// that is dropped, and the receiver would take a lane that has its header and no frames for a
    /// lane that ended, which for lane 0 is a violation. So an implementation resets the stream
    /// until it hands the lane over, as a lane that is handed over is reset when it is dropped.
    fn open_lane(
        &self,
        id: TransferId,
        number: u8,
    ) -> BoxFuture<'_, Result<Box<dyn LaneOut>, WireError>>;
}

/// Tells the receiver on `link` that the transfer `id` is given up, and waits `wait` at most for it
/// to say so.
///
/// `Ok(true)` is for a receiver that holds nothing of the transfer now: it gave it up on the word, or
/// it does not know it, or it takes no part in transfers. `Ok(false)` is for one that said something
/// else. The request goes to any device that is paired, whatever its hello says about taking files: a
/// device that stopped taking files may still hold a transfer that it took before.
pub(crate) async fn cancel(
    link: &dyn Link,
    id: TransferId,
    wait: Duration,
) -> Result<bool, WireError> {
    let request = TransferRequest::Cancel(Cancel { id });
    Ok(match link.request(request, wait).await? {
        Reply::Transfer(TransferReply::Cancelled(cancelled)) => cancelled.id == id,
        // A receiver that takes no part in transfers has nothing of this one.
        Reply::Transfer(TransferReply::Declined(declined)) => declined.id == id,
        _ => false,
    })
}

/// A lane that is open for writing.
pub(crate) trait LaneOut: Send {
    /// Writes some of `bytes`, which are not empty, and says how many it took, at least one. It
    /// waits for as long as the stream has no room, which is what makes a sender that is faster
    /// than the receiver wait.
    fn write<'a>(&'a mut self, bytes: &'a [u8]) -> BoxFuture<'a, Result<usize, LaneError>>;

    /// Ends the lane after its last byte. The bytes that were written still arrive.
    ///
    /// # Errors
    ///
    /// The lane was stopped before it was ended, which is [`LaneError::Stopped`] with the code, or
    /// the connection went. QUIC does not report a stop when a stream that was stopped is finished,
    /// so an implementation looks for the stop, without waiting, before it finishes the stream.
    fn finish(&mut self) -> Result<(), LaneError>;

    /// Gives the lane up with `code`: what was written may or may not arrive, and the receiver is
    /// told that the sender abandoned the lane.
    fn reset(&mut self, code: u32);
}

/// What came back for a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// A reply of section 8.
    Transfer(TransferReply),
    /// `Error` with this code: the other device could not process the request.
    Error(ErrorCode),
}

/// Why a request could not be made, or a lane not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WireError {
    /// The connection could not be made, or went. A new one may do better.
    #[error("the connection to the other device failed")]
    Lost,
    /// No reply came in time.
    #[error("the other device did not answer in time")]
    TimedOut,
    /// The other device ended the connection, or refused to make one, in a way that trying again
    /// does not change: it does not know this device (403), or this device broke a rule (400).
    #[error("the other device ended the connection with {0}")]
    Refused(u32),
}

/// Why a write to a lane, or the end of it, failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LaneError {
    /// The receiver stopped the lane with this code (`STOP_SENDING`).
    #[error("the receiver stopped the lane with {0}")]
    Stopped(u32),
    /// The stream or the connection failed.
    #[error("the lane failed")]
    Lost,
}
