// SPDX-License-Identifier: Apache-2.0

//! How a transfer ends, as whoever sent it is told.

use baton_proto::{
    message::DeclineReason,
    transfer::{Done, lane::SkipReason},
};

/// How a transfer ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The receiver has the files, except for the ones it names.
    Done(Sent),
    /// The receiver does not take the transfer.
    Declined(DeclineReason),
    /// The receiver ended the transfer with this code, which is the code it stopped a lane with:
    /// it gave the transfer up (499), it has no room for it (507), or it broke a rule (400, 403,
    /// 413). Nothing more is sent.
    Stopped(u32),
    /// This device gave the transfer up. `told` says whether the receiver knows, or cannot have the
    /// transfer. A receiver that was not told keeps what it holds for an hour.
    Cancelled { told: bool },
    /// The transfer could not be carried through.
    Failed(Error),
}

/// A transfer that the receiver answered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sent {
    /// What the receiver says of the files (section 8, Finish).
    pub(crate) done: Done,
    /// How many files `Finish` said were sent in full.
    pub(crate) files: u32,
    /// How many bytes those files add up to.
    pub(crate) bytes: u64,
    /// The files that this device did not send, and why, in the order they were skipped. The
    /// receiver names them in `done` as well.
    pub(crate) skipped: Vec<(u32, SkipReason)>,
}

/// Why attempts at a transfer kept failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Why {
    /// The connection could not be made, or went.
    Connection,
    /// The receiver gave no answer to the offer.
    Silent,
    /// The receiver stopped a lane with 408: it saw no byte for too long.
    TimedOut,
    /// A lane took no byte, or a read of a file did not come back, for too long.
    Stalled,
    /// The receiver forgot the transfer before it was finished.
    Forgotten,
    /// The receiver said it failed.
    Receiver,
}

/// Why a transfer failed, as opposed to how it was ended.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Error {
    /// Every attempt failed, and the last of them failed for this reason.
    #[error("the transfer could not be carried through ({0:?})")]
    GaveUp(Why),
    /// Everything was sent, and the receiver did not say how it went.
    #[error("the other device did not say whether it has the files")]
    NoOutcome,
    /// The other device ended the connection in a way that trying again does not change.
    #[error("the other device ended the connection with {0}")]
    Refused(u32),
    /// The other device answered in a way that makes no sense.
    #[error("the other device answered wrongly: {0}")]
    Protocol(&'static str),
    /// This device failed to make something the transfer needs.
    #[error("the transfer could not be made: {0}")]
    Internal(&'static str),
    /// The program is stopping.
    #[error("the program is stopping")]
    Stopped,
}
