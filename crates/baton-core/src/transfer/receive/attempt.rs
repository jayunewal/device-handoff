// SPDX-License-Identifier: Apache-2.0

//! What one attempt knows: which lane numbers have been used, and what became of every file.
//!
//! Both are plain bookkeeping with no I/O, so every rule here is tested on its own. The lanes of
//! an attempt share one [`Ledger`] behind a lock, and take turns in it for as long as a call
//! takes, which is never long.

use baton_proto::{
    id::TransferId,
    limits::{MAX_FAILURES_LISTED, MAX_LANES},
    transfer::{Done, FailReason, Failure, list::Listing},
};
use blake3::Hash;

use super::Violation;

// The lane numbers that were used are kept as the bits of a byte.
const _: () = assert!(MAX_LANES <= 8);

/// The lane numbers that one transfer uses on one connection: how many were granted, and which have
/// been used.
///
/// Lane numbers are counted for each transfer on each connection, because the header of a lane
/// names its transfer and no attempt, and a connection can carry two transfers at once. A number
/// that comes twice for a transfer on a connection is a lane of a second attempt that was opened on
/// the connection of the first, which the sender must not do (section 8, Resuming), or a sender
/// that has lost count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lanes {
    granted: u8,
    /// Bit `n` is set once lane `n` has been taken.
    taken: u8,
}

impl Lanes {
    /// The lanes numbered below `granted`, which is at most [`MAX_LANES`] whatever is given.
    pub fn new(granted: u8) -> Self {
        Self {
            granted: granted.min(MAX_LANES),
            taken: 0,
        }
    }

    /// Takes lane number `lane`, which is a violation if it is not below the number of lanes that
    /// were granted, or if it was taken before.
    pub fn take(&mut self, lane: u8) -> Result<(), Violation> {
        if lane >= self.granted {
            return Err(Violation::LaneBeyond);
        }
        // `lane` is below `granted`, which is at most 8.
        let bit = 1_u8 << lane;
        if self.taken & bit != 0 {
            return Err(Violation::LaneRepeated);
        }
        self.taken |= bit;
        Ok(())
    }
}

/// What an attempt knows of one file.
#[derive(Debug)]
struct Book {
    size: u64,
    /// The sender has said `End` or `Skip` for the file in this attempt.
    told: bool,
    /// The sender has said that it will not send the file.
    skipped: bool,
    /// The hash that `End` gave, until [`Ledger::start_check`] hands it out.
    end: Option<Hash>,
    /// The check of the file has been handed out.
    checking: bool,
    /// How the file came out. The first outcome stays, because it is the first thing that went
    /// wrong, and a late result must not change what the transfer has said already.
    outcome: Option<Result<(), FailReason>>,
}

impl Book {
    const fn new(size: u64) -> Self {
        Self {
            size,
            told: false,
            skipped: false,
            end: None,
            checking: false,
            outcome: None,
        }
    }
}

/// The book of one attempt: what the sender said of every file of the list, and how each file
/// came out.
///
/// A file is named by its position in the list, as in the frames. A folder has a place in the
/// book, so that the numbers agree, and no frames: `End` or `Skip` for one is a violation. A file
/// is **settled** once it has an outcome: checked, failed, or skipped by the sender. The transfer
/// can be answered when every file is.
///
/// A file is checked when two things have happened: the sender has said `End`, and every span is
/// written. They can happen in either order and on different lanes, so each lane asks
/// [`start_check`](Self::start_check) after it did its part, and the file is handed out for its
/// check to exactly one of them. A lane must record `End` ([`end`](Self::end)) before it asks the
/// disk whether the file is complete, and write a span before it asks for the check, so that the
/// last of the two to happen sees the other.
#[derive(Debug)]
pub struct Ledger {
    /// One for each entry of the list, `None` for a folder.
    books: Vec<Option<Book>>,
    /// How many files have no outcome yet.
    unsettled: usize,
    /// How many files the list holds, and how many bytes they add up to.
    files: u32,
    bytes: u64,
    /// How many of them the sender said it would not send, and how many bytes those hold.
    skipped_files: u32,
    skipped_bytes: u64,
}

impl Ledger {
    /// A book for the files of `listing`, with nothing known of any of them.
    pub fn new(listing: &Listing) -> Self {
        let mut files = 0_u32;
        let mut bytes = 0_u64;
        let books = listing
            .entries()
            .iter()
            .map(|entry| {
                if entry.folder {
                    return None;
                }
                files = files.saturating_add(1);
                bytes = bytes.saturating_add(entry.size);
                Some(Book::new(entry.size))
            })
            .collect();
        Self {
            books,
            unsettled: usize::try_from(files).unwrap_or(usize::MAX),
            files,
            bytes,
            skipped_files: 0,
            skipped_bytes: 0,
        }
    }

    fn book(&self, file: u32) -> Option<&Book> {
        self.books.get(usize::try_from(file).ok()?)?.as_ref()
    }

    fn book_mut(&mut self, file: u32) -> Option<&mut Book> {
        self.books.get_mut(usize::try_from(file).ok()?)?.as_mut()
    }

    /// Records that the sender sent all of file `file` and that its hash is `hash`.
    ///
    /// A violation if `file` is a folder or is not in the list, or if the sender has said `End` or
    /// `Skip` for it in this attempt already. The hash is kept until
    /// [`start_check`](Self::start_check) hands it out.
    pub fn end(&mut self, file: u32, hash: Hash) -> Result<(), Violation> {
        let book = self.book_mut(file).ok_or(Violation::NotAFile)?;
        if book.told {
            return Err(Violation::Twice);
        }
        book.told = true;
        book.end = Some(hash);
        Ok(())
    }

    /// Records that the sender will not send file `file`, which settles it as
    /// [`Skipped`](FailReason::Skipped) unless it came out some other way before. Returns whether
    /// it did settle: the caller removes what is staged of the file either way.
    ///
    /// A violation under the same conditions as for [`end`](Self::end).
    pub fn skip(&mut self, file: u32) -> Result<bool, Violation> {
        let book = self.book_mut(file).ok_or(Violation::NotAFile)?;
        if book.told {
            return Err(Violation::Twice);
        }
        book.told = true;
        book.skipped = true;
        book.end = None;
        let size = book.size;
        self.skipped_files = self.skipped_files.saturating_add(1);
        self.skipped_bytes = self.skipped_bytes.saturating_add(size);
        Ok(self.settle(file, Err(FailReason::Skipped)))
    }

    /// Whether the spans of file `file` are still wanted: it is a file and has no outcome yet. A
    /// span of any other file is read to its end and dropped, as the protocol asks for a span of a
    /// file that was skipped, one that failed, or one that was checked already.
    pub fn wants(&self, file: u32) -> bool {
        self.book(file).is_some_and(|book| book.outcome.is_none())
    }

    /// Hands out the hash of file `file` for its check, once: if the sender has said `End`, the
    /// file has no outcome yet, and no one has been given the check before. The caller then checks
    /// the file against it, and reports the result with [`settle`](Self::settle).
    pub fn start_check(&mut self, file: u32) -> Option<Hash> {
        let book = self.book_mut(file)?;
        if book.outcome.is_some() || book.checking {
            return None;
        }
        let hash = book.end.take()?;
        book.checking = true;
        Some(hash)
    }

    /// Gives file `file` its outcome, unless it has one: the first one stays. Returns whether this
    /// one was taken. A folder, or a position that is not in the list, takes none.
    pub fn settle(&mut self, file: u32, outcome: Result<(), FailReason>) -> bool {
        let Some(book) = self.book_mut(file) else {
            return false;
        };
        book.checking = false;
        if book.outcome.is_some() {
            return false;
        }
        book.outcome = Some(outcome);
        self.unsettled = self.unsettled.saturating_sub(1);
        true
    }

    /// Whether every file has an outcome. A transfer that holds no file, only folders, is settled
    /// from the start.
    pub fn is_settled(&self) -> bool {
        self.unsettled == 0
    }

    /// What the sender should have said in `Finish`, going by its own `Skip` frames: how many files
    /// it sent in full (those of the list, less the ones it skipped) and how many bytes they add
    /// up to. `Finish` can come before the last frames do, so this is compared with it when the
    /// transfer is settled.
    pub fn sent(&self) -> (u32, u64) {
        (
            self.files.saturating_sub(self.skipped_files),
            self.bytes.saturating_sub(self.skipped_bytes),
        )
    }

    /// The files that did not come out well, in the order of the list, each with its reason. A
    /// file with no outcome yet gets `unsettled`: the reason the transfer gives to what it stops
    /// waiting for.
    pub fn failures(&self, unsettled: FailReason) -> Vec<(u32, FailReason)> {
        self.books
            .iter()
            .enumerate()
            .filter_map(|(index, book)| {
                let file = u32::try_from(index).ok()?;
                match book.as_ref()?.outcome {
                    Some(Ok(())) => None,
                    Some(Err(reason)) => Some((file, reason)),
                    None => Some((file, unsettled)),
                }
            })
            .collect()
    }

    /// The files that were not checked in this attempt, in the order of the list. What is staged
    /// of them is not put in place: a file that was checked in an attempt before has not been
    /// checked against what the sender holds now.
    pub fn unchecked(&self) -> Vec<u32> {
        self.books
            .iter()
            .enumerate()
            .filter_map(|(index, book)| {
                let file = u32::try_from(index).ok()?;
                match book.as_ref()?.outcome {
                    Some(Ok(())) => None,
                    _ => Some(file),
                }
            })
            .collect()
    }
}

/// The outcome of a transfer, as `Done` carries it, from the files that did not come out well.
///
/// `failures` can be in any order and can name a file more than once, as when the lanes and the
/// placing of the items each report on the files that they know of: the order they come in is the
/// order of precedence, so the first reason given for a file is the one that is named. `Done`
/// names at most 256 files in the order of the list and counts the rest.
pub fn done(id: TransferId, mut failures: Vec<(u32, FailReason)>) -> Done {
    // The sort keeps equal files in the order they came in, and the next step keeps the first.
    failures.sort_by_key(|&(file, _)| file);
    failures.dedup_by_key(|&mut (file, _)| file);
    let more = failures.len().saturating_sub(MAX_FAILURES_LISTED);
    failures.truncate(MAX_FAILURES_LISTED);
    Done {
        id,
        failed: failures
            .into_iter()
            .map(|(file, reason)| Failure { file, reason })
            .collect(),
        more_failed: u32::try_from(more).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::{frame, message::Response, transfer::TransferReply};

    use super::*;
    use crate::transfer::testing::{file, folder, listing};

    const FOLDER: u32 = 0;
    const X: u32 = 1;
    const Y: u32 = 2;
    const Z: u32 = 3;

    /// `a` (a folder), `a/x` (10 bytes), `a/y` (20 bytes) and `z` (5 bytes), in that order.
    fn sample() -> Listing {
        listing(&[folder("a"), file("a/x", 10), file("a/y", 20), file("z", 5)])
    }

    fn hash(seed: u8) -> Hash {
        Hash::from_bytes([seed; blake3::OUT_LEN])
    }

    fn id() -> TransferId {
        TransferId::from_bytes([7; TransferId::LEN])
    }

    #[test]
    fn the_sample_is_in_the_order_these_tests_assume() {
        let listing = sample();
        let paths: Vec<&str> = listing.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["a", "a/x", "a/y", "z"]);
    }

    // The numbers of the lanes.

    #[test]
    fn each_lane_below_the_number_granted_is_taken_once() {
        let mut lanes = Lanes::new(3);

        lanes.take(0).unwrap();
        lanes.take(2).unwrap();
        lanes.take(1).unwrap();

        assert_eq!(lanes.take(1), Err(Violation::LaneRepeated));
        assert_eq!(lanes.take(0), Err(Violation::LaneRepeated));
        assert_eq!(lanes.take(3), Err(Violation::LaneBeyond));
        assert_eq!(lanes.take(255), Err(Violation::LaneBeyond));
    }

    #[test]
    fn a_lane_beyond_the_number_granted_does_not_use_up_a_number() {
        let mut lanes = Lanes::new(1);

        assert_eq!(lanes.take(1), Err(Violation::LaneBeyond));
        assert_eq!(lanes.take(1), Err(Violation::LaneBeyond));
        lanes.take(0).unwrap();
    }

    #[test]
    fn no_more_lanes_than_the_most_there_are_are_granted() {
        let mut lanes = Lanes::new(200);

        for lane in 0..MAX_LANES {
            lanes.take(lane).unwrap();
        }
        assert_eq!(lanes.take(MAX_LANES), Err(Violation::LaneBeyond));
        assert_eq!(lanes.take(199), Err(Violation::LaneBeyond));
    }

    #[test]
    fn no_lanes_granted_means_no_lane_can_be_taken() {
        assert_eq!(Lanes::new(0).take(0), Err(Violation::LaneBeyond));
    }

    // The book.

    #[test]
    fn every_file_of_a_new_book_has_no_outcome_and_a_folder_is_not_a_file() {
        let ledger = Ledger::new(&sample());

        assert!(!ledger.is_settled());
        assert!(!ledger.wants(FOLDER));
        assert!(ledger.wants(X));
        assert!(ledger.wants(Y));
        assert!(ledger.wants(Z));
        assert!(!ledger.wants(4), "beyond the list");
        assert!(!ledger.wants(u32::MAX));
        assert_eq!(ledger.sent(), (3, 35));
    }

    #[test]
    fn a_transfer_of_folders_only_is_settled_from_the_start() {
        let ledger = Ledger::new(&listing(&[folder("a"), folder("a/b")]));

        assert!(ledger.is_settled());
        assert_eq!(ledger.sent(), (0, 0));
        assert_eq!(ledger.failures(FailReason::Incomplete).len(), 0);
        assert_eq!(ledger.unchecked().len(), 0);
    }

    #[test]
    fn an_end_or_a_skip_for_a_folder_or_a_position_not_in_the_list_is_a_violation() {
        let mut ledger = Ledger::new(&sample());

        for position in [FOLDER, 4, 1000, u32::MAX] {
            assert_eq!(
                ledger.end(position, hash(1)),
                Err(Violation::NotAFile),
                "end {position}"
            );
            assert_eq!(
                ledger.skip(position),
                Err(Violation::NotAFile),
                "skip {position}"
            );
        }
        assert!(!ledger.is_settled());
        assert_eq!(ledger.sent(), (3, 35), "nothing was counted");
    }

    #[test]
    fn a_second_end_or_skip_for_a_file_is_a_violation_whatever_the_first_one_was() {
        let mut ledger = Ledger::new(&sample());
        ledger.end(X, hash(1)).unwrap();
        ledger.skip(Y).unwrap();

        assert_eq!(ledger.end(X, hash(1)), Err(Violation::Twice));
        assert_eq!(ledger.skip(X), Err(Violation::Twice));
        assert_eq!(ledger.skip(Y), Err(Violation::Twice));
        assert_eq!(ledger.end(Y, hash(2)), Err(Violation::Twice));
        // The other file is not touched by any of it.
        ledger.end(Z, hash(3)).unwrap();
    }

    #[test]
    fn a_second_end_is_a_violation_after_the_check_took_the_first() {
        let mut ledger = Ledger::new(&sample());
        ledger.end(X, hash(1)).unwrap();
        assert_eq!(ledger.start_check(X), Some(hash(1)));
        ledger.settle(X, Ok(()));

        assert_eq!(ledger.end(X, hash(1)), Err(Violation::Twice));
    }

    #[test]
    fn a_skip_settles_the_file_and_takes_it_out_of_what_finish_counts() {
        let mut ledger = Ledger::new(&sample());

        assert_eq!(ledger.skip(Y), Ok(true));

        assert!(!ledger.wants(Y));
        assert_eq!(ledger.sent(), (2, 15));
        assert_eq!(
            ledger.failures(FailReason::Incomplete),
            [
                (X, FailReason::Incomplete),
                (Y, FailReason::Skipped),
                (Z, FailReason::Incomplete)
            ]
        );
        assert_eq!(ledger.start_check(Y), None);
    }

    #[test]
    fn a_skip_after_a_failure_leaves_the_failure_as_it_was() {
        let mut ledger = Ledger::new(&sample());
        assert!(ledger.settle(X, Err(FailReason::WriteFailed)));

        assert_eq!(ledger.skip(X), Ok(false));

        assert_eq!(
            ledger.failures(FailReason::Incomplete)[0],
            (X, FailReason::WriteFailed)
        );
        assert_eq!(ledger.sent(), (2, 25), "the sender still did not send it");
    }

    #[test]
    fn the_check_of_a_file_is_handed_out_once_and_only_after_its_end() {
        let mut ledger = Ledger::new(&sample());
        assert_eq!(ledger.start_check(X), None, "no end yet");

        ledger.end(X, hash(5)).unwrap();

        assert_eq!(ledger.start_check(X), Some(hash(5)));
        assert_eq!(ledger.start_check(X), None, "handed out already");
        assert_eq!(ledger.start_check(Y), None, "another file");
        assert_eq!(ledger.start_check(FOLDER), None);
        assert_eq!(ledger.start_check(99), None);
    }

    #[test]
    fn a_check_is_never_handed_out_for_a_file_that_has_an_outcome() {
        let mut ledger = Ledger::new(&sample());
        ledger.settle(X, Err(FailReason::NoSpace));

        ledger.end(X, hash(5)).unwrap();

        assert_eq!(ledger.start_check(X), None);
        assert!(!ledger.wants(X));
    }

    #[test]
    fn the_first_outcome_of_a_file_stays() {
        let mut ledger = Ledger::new(&sample());

        assert!(ledger.settle(X, Err(FailReason::NoSpace)));
        assert!(!ledger.settle(X, Ok(())));
        assert!(!ledger.settle(X, Err(FailReason::Mismatch)));

        assert_eq!(
            ledger.failures(FailReason::Incomplete)[0],
            (X, FailReason::NoSpace)
        );
    }

    #[test]
    fn a_folder_or_a_position_not_in_the_list_takes_no_outcome() {
        let mut ledger = Ledger::new(&sample());

        assert!(!ledger.settle(FOLDER, Ok(())));
        assert!(!ledger.settle(99, Ok(())));
        assert!(!ledger.is_settled());
    }

    #[test]
    fn the_attempt_is_settled_when_every_file_has_an_outcome_of_any_kind() {
        let mut ledger = Ledger::new(&sample());
        ledger.end(X, hash(1)).unwrap();
        assert_eq!(ledger.start_check(X), Some(hash(1)));
        ledger.settle(X, Ok(()));
        assert!(!ledger.is_settled());
        ledger.skip(Y).unwrap();
        assert!(!ledger.is_settled());

        ledger.settle(Z, Err(FailReason::Mismatch));

        assert!(ledger.is_settled());
        // Settling again changes nothing, and does not count a file twice.
        ledger.settle(Z, Ok(()));
        assert!(ledger.is_settled());
    }

    #[test]
    fn failures_are_in_the_order_of_the_list_and_the_rest_get_the_given_reason() {
        let mut ledger = Ledger::new(&sample());
        ledger.settle(Z, Err(FailReason::Mismatch));
        ledger.settle(X, Ok(()));

        assert_eq!(
            ledger.failures(FailReason::NoSpace),
            [(Y, FailReason::NoSpace), (Z, FailReason::Mismatch)]
        );
    }

    #[test]
    fn the_files_that_were_not_checked_are_the_failed_and_the_waiting_ones() {
        let mut ledger = Ledger::new(&sample());
        ledger.settle(X, Ok(()));
        ledger.settle(Z, Err(FailReason::Mismatch));

        assert_eq!(ledger.unchecked(), [Y, Z]);
    }

    #[test]
    fn what_finish_should_say_counts_the_skipped_files_out() {
        let mut ledger = Ledger::new(&sample());
        ledger.skip(X).unwrap();
        ledger.skip(Z).unwrap();

        assert_eq!(ledger.sent(), (1, 20));
    }

    // The outcome of the transfer.

    fn valid_on_the_wire(done: Done) -> Done {
        frame::encode(&Response::Transfer(TransferReply::Done(done.clone()))).unwrap();
        done
    }

    #[test]
    fn done_names_failures_in_the_order_of_the_list() {
        let done = valid_on_the_wire(done(
            id(),
            vec![
                (9, FailReason::Mismatch),
                (2, FailReason::Incomplete),
                (5, FailReason::NoSpace),
            ],
        ));

        assert_eq!(done.id, id());
        assert_eq!(
            done.failed,
            [
                Failure {
                    file: 2,
                    reason: FailReason::Incomplete
                },
                Failure {
                    file: 5,
                    reason: FailReason::NoSpace
                },
                Failure {
                    file: 9,
                    reason: FailReason::Mismatch
                },
            ]
        );
        assert_eq!(done.more_failed, 0);
    }

    #[test]
    fn done_with_no_failures_is_empty() {
        let done = valid_on_the_wire(done(id(), Vec::new()));

        assert_eq!(done.failed.len(), 0);
        assert_eq!(done.more_failed, 0);
    }

    #[test]
    fn the_first_reason_given_for_a_file_is_the_one_that_is_named() {
        let done = valid_on_the_wire(done(
            id(),
            vec![
                (4, FailReason::Mismatch),
                (1, FailReason::WriteFailed),
                (4, FailReason::Incomplete),
                (4, FailReason::Other),
            ],
        ));

        assert_eq!(
            done.failed,
            [
                Failure {
                    file: 1,
                    reason: FailReason::WriteFailed
                },
                Failure {
                    file: 4,
                    reason: FailReason::Mismatch
                },
            ]
        );
    }

    #[test]
    fn done_names_256_files_and_counts_the_rest() {
        // In the opposite order, so that it is the sort that puts the first 256 in front.
        let failures = (0..300)
            .rev()
            .map(|file| (file, FailReason::Incomplete))
            .collect();

        let done = valid_on_the_wire(done(id(), failures));

        assert_eq!(done.failed.len(), MAX_FAILURES_LISTED);
        assert_eq!(done.more_failed, 44);
        assert_eq!(done.failed.first().map(|f| f.file), Some(0));
        assert_eq!(done.failed.last().map(|f| f.file), Some(255));
    }

    #[test]
    fn done_with_exactly_256_failures_counts_none_more() {
        let failures = (0..256).map(|file| (file, FailReason::Skipped)).collect();

        let done = valid_on_the_wire(done(id(), failures));

        assert_eq!(done.failed.len(), 256);
        assert_eq!(done.more_failed, 0);

        let failures = (0..257).map(|file| (file, FailReason::Skipped)).collect();

        let done = valid_on_the_wire(super::done(id(), failures));

        assert_eq!(done.failed.len(), 256);
        assert_eq!(done.more_failed, 1);
    }

    #[test]
    fn a_file_named_twice_is_counted_once_when_the_names_run_out() {
        // 256 files, one of them named twice: the repeat is not one more failure.
        let mut failures: Vec<(u32, FailReason)> = (0..256)
            .map(|file| (file, FailReason::Incomplete))
            .collect();
        failures.push((17, FailReason::Mismatch));

        let done = valid_on_the_wire(done(id(), failures));

        assert_eq!(done.more_failed, 0);
        assert_eq!(done.failed[17].reason, FailReason::Incomplete);
    }
}
