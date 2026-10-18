// SPDX-License-Identifier: Apache-2.0

//! Receiving one transfer: the rules of `docs/protocol.md` section 8 for a receiver, applied to
//! what arrives on its lanes.
//!
//! A transfer is received in **attempts**. An attempt is what one connection carries: a repeat of
//! the offer on a new connection starts the next one. What is counted for one attempt (the lane
//! numbers that were used, the `End` and `Skip` frames that were seen) starts again with it, and
//! the staging folder with what it holds goes on from one attempt to the next.
//!
//! The receiving side is built in pieces, each with tests of its own and each used by the next:
//!
//! - [`Violation`] is a rule that a sender broke, and the code that the lanes are stopped with.
//! - [`Lanes`] checks the numbers of the lanes that a transfer opens on one connection.
//! - [`Ledger`] is the book of an attempt: what the sender said of every file, and how each file
//!   came out. [`done`] makes the outcome of the transfer from it.
//! - [`LaneReader`] reads the frames of one lane off a stream, with the checks that need only the
//!   list: where a piece of the list may come, and how many bytes a span has.
//! - [`Intake`] joins the [`Ledger`] to the staging folder: what each frame does to the files, in
//!   the order that keeps the two agreeing however the lanes interleave, and how the transfer ends.
//! - [`Transfer`] is the whole receiver of one transfer: its attempts, the lanes of each read side
//!   by side, the timer that ends an attempt that has stopped moving, and the answers to `Finish`
//!   and `Cancel`. The session gives it streams and asks it for outcomes.
//!
//! None of it touches the network, and nothing here is connected to the session yet.

use baton_proto::{close_code, transfer::list::ListError};

mod attempt;
mod intake;
mod lane;
mod transfer;

pub use attempt::{Lanes, Ledger, done};
pub use intake::{Concluded, Halt, Intake};
pub use lane::{Counted, Fault, Frame, LaneReader, SpanBuf};
pub use transfer::{Attempt, Dropped, LaneEnd, Over, Transfer};

/// A rule of a transfer that the sender broke.
///
/// The receiver stops every lane of the transfer with [`code`](Self::code), drops the transfer, and
/// keeps the connection. What a sender may not do is listed in section 8 of the protocol, under
/// the rules for the receiver. The text of a violation is safe to log: it names no path and holds
/// nothing the sender chose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Violation {
    /// A lane with a number that is not below the number of lanes that were granted.
    #[error("a lane with a number beyond the lanes that were granted")]
    LaneBeyond,
    /// A lane with a number that another lane of the same transfer on the same connection used
    /// already.
    #[error("a lane number was used twice by one transfer on one connection")]
    LaneRepeated,
    /// A frame with a tag that this version does not know, so the length of what follows it is not
    /// known either.
    #[error("a frame with the unknown tag {0}")]
    BadTag(u8),
    /// A piece of the list on a lane other than 0, or after the list was complete.
    #[error("a piece of the list where none may come")]
    ListOutOfPlace,
    /// A span, an `End` or a `Skip` before the list was complete.
    #[error("data before the list was complete")]
    BeforeTheList,
    /// A span of a folder, of a file that is not in the list, or one that the file does not have.
    #[error("a span that the list does not have")]
    NoSuchSpan,
    /// An `End` or a `Skip` for a folder, or for a position that is not in the list.
    #[error("an end or a skip for something that is not a file of the list")]
    NotAFile,
    /// A second `End` or `Skip` for a file within one attempt.
    #[error("a file was ended or skipped twice in one attempt")]
    Twice,
    /// A piece of the list that is not a frame of section 2 holding a list chunk.
    #[error("a piece of the list that is not valid")]
    BadChunk,
    /// A lane that the sender finished in the middle of a frame. A lane that is reset is not this:
    /// it is the stream's error.
    #[error("a lane was finished in the middle of a frame")]
    Cut,
    /// Lane 0 finished before the list was complete.
    #[error("lane 0 was finished before the list was complete")]
    ShortList,
    /// A list that is not the one that was offered.
    #[error("the list is not the one that was offered: {0}")]
    List(#[from] ListError),
}

impl Violation {
    /// The code that the lanes of the transfer are stopped with: 413 for a list that takes more
    /// than the limit in the form that is hashed, and 400 for every other violation.
    pub const fn code(&self) -> u32 {
        match self {
            Self::List(ListError::TooLong) => close_code::TOO_LARGE,
            _ => close_code::PROTOCOL_VIOLATION,
        }
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::limits::LimitError;

    use super::*;

    #[test]
    fn a_list_that_is_too_long_is_stopped_with_413_and_every_other_violation_with_400() {
        assert_eq!(Violation::List(ListError::TooLong).code(), 413);
        for violation in [
            Violation::LaneBeyond,
            Violation::LaneRepeated,
            Violation::BadTag(9),
            Violation::ListOutOfPlace,
            Violation::BeforeTheList,
            Violation::NoSuchSpan,
            Violation::NotAFile,
            Violation::Twice,
            Violation::BadChunk,
            Violation::Cut,
            Violation::ShortList,
            Violation::List(ListError::HashMismatch),
            Violation::List(ListError::OutOfOrder { index: 3 }),
            Violation::List(ListError::Entry(LimitError::Empty { field: "x" })),
        ] {
            assert_eq!(violation.code(), 400, "{violation}");
        }
    }

    #[test]
    fn the_text_of_a_violation_names_nothing_the_sender_chose() {
        // The list errors carry counts and fixed words, and the others carry a tag at most.
        let text = Violation::List(ListError::Mismatch { what: "files" }).to_string();
        assert_eq!(
            text,
            "the list is not the one that was offered: the list has a different number of files \
             than the offer said"
        );
        assert_eq!(
            Violation::BadTag(9).to_string(),
            "a frame with the unknown tag 9"
        );
    }
}
