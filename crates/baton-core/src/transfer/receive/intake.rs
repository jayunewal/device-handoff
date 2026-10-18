// SPDX-License-Identifier: Apache-2.0

//! The book of an attempt joined to the disk: what the frames of a lane do to the files of a
//! transfer, and how the transfer ends.
//!
//! Two things know about a file. The [`Ledger`] knows what the sender said of it and how it came
//! out, and [`Staging`] holds its bytes. They are separate so that each is tested alone, and they
//! have to agree: a file that the staging has checked is a file that the ledger has settled, and
//! the other way round. The lanes of an attempt run side by side and in any order, so the
//! agreement is kept here, in the order of the calls and in one gate that every call and the end
//! of the transfer share:
//!
//! - **`End` is recorded before the disk is asked** whether the file is complete, and **a span is
//!   written before the check is asked for**. A file is checked when both have happened, in either
//!   order and on different lanes, and [`Ledger::start_check`] gives the check to exactly one of
//!   the lanes that got there. So a file never waits for a check that no lane is going to make.
//! - **A call and the end of the transfer do not overlap.** Putting the items in place ends the
//!   transfer, and it has to find every file that was checked settled in the ledger, or `Done` would
//!   name a file that is in place as failed. Every call that takes a frame holds the gate open from
//!   its start to its end, a check included, from the moment it takes the hash to the moment it
//!   writes the outcome. [`Intake::conclude`] closes the gate, which waits for the calls that are
//!   under way and turns away the ones that come after.
//! - **A file that fails fails alone.** The error of one file's disk work gives that file its
//!   outcome and removes what is staged of it, and the lane reads on. Only a full disk stops the
//!   lane, because every file after this one would fail the same way.
//! - **What was checked in an earlier attempt is checked again.** The staging keeps its files from
//!   one attempt to the next, and each attempt has a book of its own. A file that this attempt did
//!   not check, because the sender did not send it again or skipped it, is removed before the items
//!   are put in place: the sender may hold another file under that name by now.
//! - **An attempt that is over does nothing.** The books of two attempts share one staging folder,
//!   and neither knows what the other has been told of a file. A late `skip` of the old attempt
//!   would drop a file that the new one has checked, and a late span of the old attempt would
//!   complete a file for a book that has no `End`, so that the new book, which holds one, never
//!   asks for the check. So the book of an attempt is [retired](Intake::retire) before the next
//!   attempt's book takes its first call. That closes the gate as `conclude` does: it waits for the
//!   calls under way, and every call after that is [`Gone`](Halt::Gone). A lane that was stopped
//!   can be in the middle of a write, or hold a frame it has not applied, so stopping its stream is
//!   not enough: the retiring is what waits.
//!
//! Every method is blocking disk work, apart from the ones that only look at the ledger, and is
//! called on a thread meant for blocking. `retire` and `conclude` wait for the other calls.

use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard},
};

use baton_proto::transfer::{Done, FailReason, list::Listing};
use blake3::Hash;

use super::{Ledger, Violation, done};
use crate::{
    transfer::staging::{StageError, Staging, Written},
    util::lock,
};

/// Why a lane has to stop.
#[derive(Debug)]
pub enum Halt {
    /// The sender broke a rule of the transfer.
    Violation(Violation),
    /// The disk is full. The file that was being written has failed with `NoSpace`.
    NoSpace,
    /// The transfer has been put in place, or discarded, or the attempt of this book is over, and
    /// the book takes nothing more.
    Gone,
}

/// How a transfer ended: what `Done` says, and what the sender's own frames add up to.
#[derive(Debug)]
pub struct Concluded {
    /// The answer to `Finish`.
    pub done: Done,
    /// How many files the sender sent in full, going by its `Skip` frames, and how many bytes
    /// those add up to. `Finish` has the sender's own count, and the two are compared.
    pub sent: (u32, u64),
}

/// Whether a book still takes calls. See the module documentation.
///
/// A call enters the gate and stays in it until the guard is dropped, and any number of calls can
/// be in it at once. Closing the gate waits for them, and nothing enters after that.
struct Gate(RwLock<bool>);

impl Gate {
    const fn new() -> Self {
        Self(RwLock::new(true))
    }

    /// Enters the gate: `Gone` if it is closed. It is held open for as long as the guard lives.
    fn enter(&self) -> Result<RwLockReadGuard<'_, bool>, Halt> {
        let open = self.0.read().unwrap_or_else(PoisonError::into_inner);
        if *open { Ok(open) } else { Err(Halt::Gone) }
    }

    /// Closes the gate, once the calls in it have returned. Says whether it was open.
    fn close(&self) -> bool {
        let mut open = self.0.write().unwrap_or_else(PoisonError::into_inner);
        std::mem::replace(&mut *open, false)
    }
}

/// What one attempt knows of the files of a transfer, and the staging folder they are written to.
pub struct Intake {
    listing: Arc<Listing>,
    staging: Arc<Staging>,
    ledger: Mutex<Ledger>,
    gate: Gate,
}

impl fmt::Debug for Intake {
    // The list has the names of the person's files, and a log gets pasted into a bug report.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Intake")
            .field("staging", &self.staging)
            .finish_non_exhaustive()
    }
}

impl Intake {
    /// A book for the files of `listing`, with nothing known of any of them yet. `staging` is the
    /// folder that the transfer is received into, which an earlier attempt may have written in.
    pub fn new(listing: Arc<Listing>, staging: Arc<Staging>) -> Self {
        let ledger = Ledger::new(&listing);
        Self {
            listing,
            staging,
            ledger: Mutex::new(ledger),
            gate: Gate::new(),
        }
    }

    /// The list of the transfer, which the frames of the lanes are read by.
    pub fn listing(&self) -> &Listing {
        &self.listing
    }

    /// Whether the spans of file `file` are still wanted. A span of any other file is read to its
    /// end and dropped (see [`Ledger::wants`]).
    pub fn wants(&self, file: u32) -> bool {
        lock(&self.ledger).wants(file)
    }

    /// Whether every file has an outcome. A transfer of folders only has, from the start.
    pub fn is_settled(&self) -> bool {
        lock(&self.ledger).is_settled()
    }

    /// Writes span `span` of file `file`, and checks the file if that was the span it lacked and
    /// the sender has said `End`. Says whether every file is settled now.
    ///
    /// A span that the staging holds already, or that another lane is writing, is dropped: the
    /// first bytes stay (protocol.md section 8, rules for the receiver). A file that fails fails
    /// alone, and the call is `Ok`. A span that is in hand when another lane skips the file can
    /// still make the staged file; `conclude` removes it with the others that were not checked.
    ///
    /// `Gone` if the book was retired or concluded.
    pub fn span(&self, file: u32, span: u32, data: &[u8]) -> Result<bool, Halt> {
        let _open = self.gate.enter()?;
        if self.wants(file) {
            match self.staging.write_span(file, span, data) {
                Ok(Written::New { complete: true }) => self.check(file)?,
                Ok(Written::New { complete: false } | Written::Duplicate) => {}
                Err(error) => self.fail(file, &error)?,
            }
        }
        Ok(self.is_settled())
    }

    /// Records that the sender sent all of file `file`, with this hash, and checks the file if
    /// every span is written. Says whether every file is settled now.
    ///
    /// A violation if `file` is not a file of the list, or if the sender has said `End` or `Skip`
    /// for it in this attempt already. `Gone` if the book was retired or concluded.
    pub fn end(&self, file: u32, hash: Hash) -> Result<bool, Halt> {
        let _open = self.gate.enter()?;
        lock(&self.ledger)
            .end(file, hash)
            .map_err(Halt::Violation)?;
        match self.staging.is_complete(file) {
            Ok(true) => self.check(file)?,
            Ok(false) => {}
            Err(error) => self.fail(file, &error)?,
        }
        Ok(self.is_settled())
    }

    /// Records that the sender will not send file `file`, and removes what is staged of it. Says
    /// whether every file is settled now.
    ///
    /// A violation under the same conditions as for [`end`](Self::end), and `Gone` as there.
    pub fn skip(&self, file: u32) -> Result<bool, Halt> {
        let _open = self.gate.enter()?;
        lock(&self.ledger).skip(file).map_err(Halt::Violation)?;
        // Whatever the file came out as before, its bytes are of no use now. One that cannot be
        // removed is on record in the staging, and keeps what it is inside from being put in place.
        let _ = self.staging.drop_file(file);
        Ok(self.is_settled())
    }

    /// Ends the attempt of this book without putting anything in place: the calls under way
    /// return first, and every call after that is `Gone`. Call it before the book of the next
    /// attempt takes its first call, and after the stream of every lane of this attempt is
    /// stopped. It can be called again, and after `conclude`.
    pub fn retire(&self) {
        self.gate.close();
    }

    /// Stands in for a call that is under way and does not return, for the tests of what waits for
    /// a book to be retired. The call returns when the guard is dropped.
    #[cfg(test)]
    pub(crate) fn call_under_way(&self) -> impl Drop + '_ {
        self.gate.enter().expect("the gate is open")
    }

    /// Ends the transfer: puts what was checked in place, and says how every file came out. A file
    /// that has no outcome yet gets `unsettled`.
    ///
    /// The calls under way return first, and nothing is taken after this: `Gone` is the answer
    /// to every call, including a second `conclude`, which would not know what the first has
    /// put in place. The book gives its outcome once, and the caller keeps it. `Gone` too if the
    /// transfer was discarded or the book was retired.
    pub fn conclude(&self, unsettled: FailReason) -> Result<Concluded, Halt> {
        if !self.gate.close() {
            return Err(Halt::Gone);
        }
        let (mut failures, unchecked, sent) = {
            let ledger = lock(&self.ledger);
            (
                ledger.failures(unsettled),
                ledger.unchecked(),
                ledger.sent(),
            )
        };
        for file in unchecked {
            // A file that cannot be removed is on record, and the staging does not put in place
            // what it is inside. The staging removes the files that were not checked as well, but
            // one that an earlier attempt checked is a file that it takes for checked.
            let _ = self.staging.drop_file(file);
        }
        let placement = self.staging.place().map_err(|_| Halt::Gone)?;
        // The reasons of the files come first, and then the ones that could not be put in place.
        failures.extend_from_slice(placement.failed());
        Ok(Concluded {
            done: done(self.staging.id(), failures),
            sent,
        })
    }

    /// Checks file `file` against the hash that `End` gave, if the sender has said `End`, the file
    /// has no outcome, and no one else has been given the check. The caller is in the gate.
    fn check(&self, file: u32) -> Result<(), Halt> {
        let Some(hash) = lock(&self.ledger).start_check(file) else {
            return Ok(());
        };
        match self.staging.verify(file, &hash) {
            Ok(()) => {
                lock(&self.ledger).settle(file, Ok(()));
                Ok(())
            }
            Err(error) => self.fail(file, &error),
        }
    }

    /// Gives file `file` the outcome that `error` calls for, and removes what is staged of it.
    /// `Ok` if the lane can go on, because the error is the file's own. The caller is in the gate.
    fn fail(&self, file: u32, error: &StageError) -> Result<(), Halt> {
        let reason = match error {
            StageError::Discarded | StageError::Placed => return Err(Halt::Gone),
            StageError::NoSpace => FailReason::NoSpace,
            StageError::Mismatch => FailReason::Mismatch,
            StageError::Incomplete => FailReason::Incomplete,
            StageError::Io(error) => {
                tracing::warn!(kind = ?error.kind(), "a received file could not be written");
                FailReason::WriteFailed
            }
            StageError::Plan(_) | StageError::NoSuchFile { .. } | StageError::Hash(_) => {
                tracing::error!("the staging refused a file that the list has");
                FailReason::Other
            }
        };
        // A file that has an outcome keeps it: the first thing that went wrong is the one named,
        // and a file that was checked is not removed by a late error.
        if lock(&self.ledger).settle(file, Err(reason)) {
            let _ = self.staging.drop_file(file);
        }
        if reason == FailReason::NoSpace {
            Err(Halt::NoSpace)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            Barrier,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    use baton_proto::{id::TransferId, transfer::list::ListEntry};

    use super::*;
    use crate::transfer::testing::{bytes, destination, file, folder, listing};

    const MIB: usize = 1 << 20;

    /// The staging folder of a transfer of some entries, which attempts take turns at.
    struct Rig {
        scratch: tempfile::TempDir,
        listing: Arc<Listing>,
        staging: Arc<Staging>,
    }

    impl Rig {
        /// `entries` must be in the order of the format.
        fn new(entries: &[ListEntry]) -> Self {
            let (scratch, destination) = destination();
            let listing = Arc::new(listing(entries));
            let id = TransferId::from_bytes([9; TransferId::LEN]);
            let staging = Arc::new(destination.stage(id, &listing).unwrap());
            Self {
                scratch,
                listing,
                staging,
            }
        }

        /// The book of a new attempt.
        fn attempt(&self) -> Intake {
            Intake::new(Arc::clone(&self.listing), Arc::clone(&self.staging))
        }

        /// The folder that the items are put in.
        fn root(&self) -> &Path {
            self.scratch.path()
        }

        /// The folder of the transfer in the staging folder of the destination.
        fn staged(&self) -> PathBuf {
            let hidden = fs::read_dir(self.root())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".baton-incoming-")
                })
                .unwrap();
            hidden.join(self.staging.id().to_string())
        }
    }

    fn hash(content: &[u8]) -> Hash {
        blake3::hash(content)
    }

    /// The files that did not come out well, as `Done` names them.
    fn failed(concluded: &Concluded) -> Vec<(u32, FailReason)> {
        concluded
            .done
            .failed
            .iter()
            .map(|failure| (failure.file, failure.reason))
            .collect()
    }

    /// Every file of the transfer is in place.
    fn all_in_place(concluded: &Concluded) {
        let failed = failed(concluded);
        assert_eq!(failed.len(), 0, "{failed:?}");
        assert_eq!(concluded.done.more_failed, 0);
    }

    fn violation<T: fmt::Debug>(result: Result<T, Halt>) -> Violation {
        match result {
            Err(Halt::Violation(violation)) => violation,
            other => panic!("expected a violation, got {other:?}"),
        }
    }

    #[test]
    fn a_book_can_be_shared_between_the_threads_that_read_the_lanes() {
        fn shared<T: Send + Sync>() {}
        shared::<Intake>();
    }

    // The order of the frames of a file.

    #[test]
    fn a_file_is_checked_when_its_last_span_comes_after_its_end() {
        let content = bytes(1, MIB + 5);
        let rig = Rig::new(&[file("big", content.len() as u64)]);
        let intake = rig.attempt();

        assert!(!intake.end(0, hash(&content)).unwrap());
        assert!(!intake.span(0, 0, &content[..MIB]).unwrap());
        assert!(intake.span(0, 1, &content[MIB..]).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("big")).unwrap(), content);
    }

    #[test]
    fn a_file_is_checked_when_its_end_comes_after_its_last_span() {
        let content = bytes(2, MIB + 5);
        let rig = Rig::new(&[file("big", content.len() as u64)]);
        let intake = rig.attempt();

        assert!(!intake.span(0, 1, &content[MIB..]).unwrap());
        assert!(!intake.span(0, 0, &content[..MIB]).unwrap(), "no end yet");
        assert!(intake.end(0, hash(&content)).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("big")).unwrap(), content);
    }

    #[test]
    fn an_empty_file_has_only_an_end() {
        let rig = Rig::new(&[file("empty", 0)]);
        let intake = rig.attempt();

        assert!(!intake.is_settled());
        assert!(intake.end(0, hash(b"")).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("empty")).unwrap(), b"");
    }

    #[test]
    fn a_transfer_of_folders_only_is_settled_from_the_start() {
        let rig = Rig::new(&[folder("d"), folder("d/e")]);
        let intake = rig.attempt();

        assert!(intake.is_settled());
        let concluded = intake.conclude(FailReason::Incomplete).unwrap();

        all_in_place(&concluded);
        assert_eq!(concluded.sent, (0, 0));
        assert!(rig.root().join("d").join("e").is_dir());
    }

    #[test]
    fn a_span_that_comes_twice_keeps_the_first_bytes() {
        let first = bytes(3, MIB + 5);
        let mut second = first.clone();
        second[7] ^= 0xff;
        let rig = Rig::new(&[file("f", first.len() as u64)]);
        let intake = rig.attempt();

        assert!(!intake.span(0, 0, &first[..MIB]).unwrap());
        assert!(!intake.span(0, 0, &second[..MIB]).unwrap());
        assert!(!intake.span(0, 1, &first[MIB..]).unwrap());
        assert!(intake.end(0, hash(&first)).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("f")).unwrap(), first);
    }

    // How a file can come out.

    #[test]
    fn a_file_without_the_hash_it_was_given_fails_alone_and_is_not_put_in_place() {
        let rig = Rig::new(&[file("a", 10), file("b", 10)]);
        let intake = rig.attempt();

        intake.span(0, 0, &bytes(4, 10)).unwrap();
        // The hash is that of other bytes.
        assert!(!intake.end(0, hash(&bytes(5, 10))).unwrap());
        intake.span(1, 0, &bytes(6, 10)).unwrap();
        assert!(intake.end(1, hash(&bytes(6, 10))).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        assert_eq!(failed(&concluded), [(0, FailReason::Mismatch)]);
        assert_eq!(concluded.done.more_failed, 0);
        assert!(!rig.root().join("a").exists());
        assert_eq!(fs::read(rig.root().join("b")).unwrap(), bytes(6, 10));
    }

    #[test]
    fn a_skipped_file_is_removed_and_its_late_spans_are_dropped() {
        let content = bytes(7, MIB + 5);
        let rig = Rig::new(&[file("x", content.len() as u64), file("y", 4)]);
        let intake = rig.attempt();

        intake.span(0, 0, &content[..MIB]).unwrap();
        assert!(intake.wants(0));
        assert!(!intake.skip(0).unwrap());
        assert!(!intake.wants(0));
        assert!(!rig.staged().join("x").exists());
        // A span that comes after the skip is dropped, and makes no file.
        assert!(!intake.span(0, 1, &content[MIB..]).unwrap());
        assert!(!rig.staged().join("x").exists());
        intake.span(1, 0, b"yyyy").unwrap();
        assert!(intake.end(1, hash(b"yyyy")).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        assert_eq!(failed(&concluded), [(0, FailReason::Skipped)]);
        assert_eq!(concluded.sent, (1, 4));
        assert!(!rig.root().join("x").exists());
        assert_eq!(fs::read(rig.root().join("y")).unwrap(), b"yyyy");
    }

    #[test]
    fn a_file_with_no_end_when_the_transfer_ends_gets_the_reason_it_is_given() {
        let content = bytes(8, MIB + 5);
        let rig = Rig::new(&[file("big", content.len() as u64), file("none", 3)]);
        let intake = rig.attempt();
        intake.span(0, 0, &content[..MIB]).unwrap();

        let concluded = intake.conclude(FailReason::NoSpace).unwrap();

        assert_eq!(
            failed(&concluded),
            [(0, FailReason::NoSpace), (1, FailReason::NoSpace)]
        );
        assert!(!rig.root().join("big").exists());
        assert!(!rig.root().join("none").exists());
    }

    #[test]
    fn a_file_whose_disk_work_fails_fails_alone() {
        let rig = Rig::new(&[file("blocked", 3), file("fine", 3)]);
        // A folder where the file is to be made.
        fs::create_dir(rig.staged().join("blocked")).unwrap();
        let intake = rig.attempt();

        assert!(!intake.span(0, 0, b"abc").unwrap());
        assert!(!intake.wants(0));
        intake.span(1, 0, b"def").unwrap();
        // The sender does not know, and says End for both.
        intake.end(0, hash(b"abc")).unwrap();
        assert!(intake.end(1, hash(b"def")).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        assert_eq!(failed(&concluded), [(0, FailReason::WriteFailed)]);
        assert!(!rig.root().join("blocked").exists());
        assert_eq!(fs::read(rig.root().join("fine")).unwrap(), b"def");
    }

    #[test]
    fn a_full_disk_fails_the_file_and_stops_the_lane() {
        let rig = Rig::new(&[file("a", 3), file("b", 3)]);
        let intake = rig.attempt();

        let halt = intake.fail(0, &StageError::NoSpace);

        assert!(matches!(halt, Err(Halt::NoSpace)), "{halt:?}");
        assert!(!intake.wants(0));
        // The file that is not settled is a victim of the same disk.
        let concluded = intake.conclude(FailReason::NoSpace).unwrap();
        assert_eq!(
            failed(&concluded),
            [(0, FailReason::NoSpace), (1, FailReason::NoSpace)]
        );
    }

    #[test]
    fn the_first_thing_that_went_wrong_with_a_file_is_the_one_named() {
        let rig = Rig::new(&[file("a", 3)]);
        let intake = rig.attempt();
        intake.fail(0, &StageError::Mismatch).unwrap();

        intake.fail(0, &StageError::NoSpace).unwrap_err();

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        assert_eq!(failed(&concluded), [(0, FailReason::Mismatch)]);
    }

    #[test]
    fn a_file_that_was_checked_is_not_removed_by_a_late_error() {
        let rig = Rig::new(&[file("a", 3)]);
        let intake = rig.attempt();
        intake.span(0, 0, b"abc").unwrap();
        assert!(intake.end(0, hash(b"abc")).unwrap());

        intake.fail(0, &StageError::NoSpace).unwrap_err();

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("a")).unwrap(), b"abc");
    }

    // The rules of the sender.

    #[test]
    fn an_end_or_a_skip_for_what_is_not_a_file_or_for_a_file_twice_is_a_violation() {
        let rig = Rig::new(&[folder("d"), file("d/f", 1)]);
        let intake = rig.attempt();

        for position in [0, 5] {
            assert_eq!(
                violation(intake.end(position, hash(b""))),
                Violation::NotAFile
            );
            assert_eq!(violation(intake.skip(position)), Violation::NotAFile);
        }
        // Ended, and not complete yet.
        assert!(!intake.end(1, hash(b"x")).unwrap());
        assert_eq!(violation(intake.end(1, hash(b"x"))), Violation::Twice);
        assert_eq!(violation(intake.skip(1)), Violation::Twice);
        assert!(!intake.is_settled());
    }

    // More than one attempt.

    #[test]
    fn a_file_checked_in_an_earlier_attempt_goes_in_place_only_if_this_attempt_checks_it_too() {
        let rig = Rig::new(&[file("a", 3), file("b", 3)]);
        let first = rig.attempt();
        for (file, content) in [(0, b"aaa"), (1, b"bbb")] {
            first.span(file, 0, content).unwrap();
            first.end(file, hash(content)).unwrap();
        }
        assert!(first.is_settled());

        // The connection dropped before `Finish`. The sender sends `a` again, and says nothing of
        // `b` before the attempt ends.
        let second = rig.attempt();
        assert!(!second.is_settled());
        assert!(!second.span(0, 0, b"aaa").unwrap(), "held already");
        assert!(!second.end(0, hash(b"aaa")).unwrap(), "b is not settled");
        let concluded = second.conclude(FailReason::Incomplete).unwrap();

        assert_eq!(failed(&concluded), [(1, FailReason::Incomplete)]);
        assert_eq!(fs::read(rig.root().join("a")).unwrap(), b"aaa");
        assert!(!rig.root().join("b").exists());
    }

    #[test]
    fn a_file_that_changed_between_two_attempts_fails_its_check() {
        let rig = Rig::new(&[file("a", 3)]);
        let first = rig.attempt();
        first.span(0, 0, b"old").unwrap();

        // The file was edited, and the sender sends it again in full with the hash of the new
        // bytes. The span that the staging holds stays, so the check does not pass.
        let second = rig.attempt();
        assert!(!second.span(0, 0, b"new").unwrap());
        assert!(second.end(0, hash(b"new")).unwrap());
        let concluded = second.conclude(FailReason::Incomplete).unwrap();

        assert_eq!(failed(&concluded), [(0, FailReason::Mismatch)]);
        assert!(!rig.root().join("a").exists());
    }

    // The end of the transfer.

    #[test]
    fn nothing_more_is_taken_when_the_transfer_has_ended() {
        let content = bytes(9, MIB + 1);
        let rig = Rig::new(&[file("big", content.len() as u64), file("empty", 0)]);
        let intake = rig.attempt();
        intake.span(0, 0, &content[..MIB]).unwrap();

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        assert_eq!(
            failed(&concluded),
            [(0, FailReason::Incomplete), (1, FailReason::Incomplete)]
        );

        // A span that comes late, a file that would be checked now, and a skip.
        assert!(matches!(
            intake.span(0, 1, &content[MIB..]),
            Err(Halt::Gone)
        ));
        assert!(matches!(intake.end(1, hash(b"")), Err(Halt::Gone)));
        assert!(matches!(intake.skip(1), Err(Halt::Gone)));
        assert!(!rig.root().join("big").exists());
        assert!(!rig.root().join("empty").exists());
    }

    #[test]
    fn a_book_gives_its_outcome_once() {
        let rig = Rig::new(&[file("a", 3), file("b", 3)]);
        let intake = rig.attempt();
        intake.span(0, 0, b"aaa").unwrap();
        assert!(!intake.end(0, hash(b"aaa")).unwrap());

        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        assert_eq!(failed(&concluded), [(1, FailReason::Incomplete)]);
        assert_eq!(concluded.sent, (2, 6));

        // A skip that comes late is not taken, so a second conclusion has nothing to say that the
        // first did not, and it says nothing.
        assert!(matches!(intake.skip(1), Err(Halt::Gone)));
        assert!(matches!(
            intake.conclude(FailReason::Incomplete),
            Err(Halt::Gone)
        ));
        assert!(!intake.is_settled());
    }

    #[test]
    fn a_transfer_that_was_discarded_is_gone_to_the_lanes() {
        let rig = Rig::new(&[file("a", 3)]);
        let intake = rig.attempt();
        rig.staging.discard().unwrap();

        assert!(matches!(intake.span(0, 0, b"abc"), Err(Halt::Gone)));
        assert!(matches!(
            intake.conclude(FailReason::Incomplete),
            Err(Halt::Gone)
        ));
        // The span made no file in a folder that is gone.
        assert!(!rig.staged().join("a").exists());
    }

    #[test]
    fn the_book_does_not_print_the_names_of_the_files() {
        let rig = Rig::new(&[file("secret name.txt", 3)]);
        let intake = rig.attempt();

        let text = format!("{intake:?}");

        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("Intake"), "{text}");
    }

    // Lanes that run side by side.

    #[test]
    fn a_file_is_checked_whichever_lane_comes_last() {
        // One lane writes the span of each file and another says `End`, so that for each file the
        // two come within a moment of each other, in either order.
        const FILES: usize = 300;
        let contents: Vec<Vec<u8>> = (0..FILES)
            .map(|i| bytes(u8::try_from(i % 251).unwrap(), 10 + i))
            .collect();
        let entries: Vec<ListEntry> = contents
            .iter()
            .enumerate()
            .map(|(i, content)| file(&format!("f{i:03}"), content.len() as u64))
            .collect();
        let rig = Rig::new(&entries);
        let intake = rig.attempt();
        let start = Barrier::new(2);

        thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                for (i, content) in contents.iter().enumerate() {
                    intake.span(u32::try_from(i).unwrap(), 0, content).unwrap();
                }
            });
            scope.spawn(|| {
                start.wait();
                for (i, content) in contents.iter().enumerate() {
                    intake
                        .end(u32::try_from(i).unwrap(), hash(content))
                        .unwrap();
                }
            });
        });

        assert!(intake.is_settled(), "a file was left waiting for its check");
        let concluded = intake.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        for (i, content) in contents.iter().enumerate() {
            let placed = fs::read(rig.root().join(format!("f{i:03}"))).unwrap();
            assert_eq!(&placed, content, "file {i}");
        }
    }

    // Attempts that overlap.

    #[test]
    fn a_late_skip_or_end_of_a_retired_book_does_not_undo_what_the_next_attempt_checked() {
        let rig = Rig::new(&[file("y", 3)]);
        let old = rig.attempt();
        let new = rig.attempt();
        old.retire();
        new.span(0, 0, b"yyy").unwrap();
        assert!(new.end(0, hash(b"yyy")).unwrap());

        // The lanes of the old attempt had these in hand when the attempt ended.
        assert!(matches!(old.skip(0), Err(Halt::Gone)));
        assert!(matches!(old.end(0, hash(b"other")), Err(Halt::Gone)));
        assert!(matches!(old.span(0, 0, b"zzz"), Err(Halt::Gone)));

        let concluded = new.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("y")).unwrap(), b"yyy");
    }

    #[test]
    fn a_late_span_of_a_retired_book_does_not_take_the_check_from_the_next_attempt() {
        let content = bytes(10, MIB + 5);
        let rig = Rig::new(&[file("big", content.len() as u64)]);
        let old = rig.attempt();
        let new = rig.attempt();
        old.retire();
        // The new attempt has `End`, and span 1 is missing.
        new.span(0, 0, &content[..MIB]).unwrap();
        assert!(!new.end(0, hash(&content)).unwrap());

        // The old attempt's lane was writing span 1 when the attempt ended.
        assert!(matches!(old.span(0, 1, &content[MIB..]), Err(Halt::Gone)));
        // So span 1 of the new attempt is the one that completes the file, and asks for the check.
        assert!(new.span(0, 1, &content[MIB..]).unwrap());

        let concluded = new.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("big")).unwrap(), content);
    }

    #[test]
    fn a_retired_book_puts_nothing_in_place_and_leaves_what_it_checked_to_the_next_attempt() {
        let rig = Rig::new(&[file("a", 3)]);
        let old = rig.attempt();
        old.span(0, 0, b"aaa").unwrap();
        assert!(old.end(0, hash(b"aaa")).unwrap());

        old.retire();
        old.retire();

        assert!(matches!(
            old.conclude(FailReason::Incomplete),
            Err(Halt::Gone)
        ));
        assert!(!rig.root().join("a").exists());
        // The file the old attempt checked is still held, and the next attempt checks it again.
        let new = rig.attempt();
        assert!(!new.span(0, 0, b"aaa").unwrap(), "held already");
        assert!(new.end(0, hash(b"aaa")).unwrap());
        let concluded = new.conclude(FailReason::Incomplete).unwrap();
        all_in_place(&concluded);
        assert_eq!(fs::read(rig.root().join("a")).unwrap(), b"aaa");
    }

    #[test]
    fn retiring_a_book_waits_for_the_calls_under_way() {
        const FILES: usize = 300;
        let contents: Vec<Vec<u8>> = (0..FILES)
            .map(|i| bytes(u8::try_from(i % 251).unwrap(), 10 + i))
            .collect();
        let entries: Vec<ListEntry> = contents
            .iter()
            .enumerate()
            .map(|(i, content)| file(&format!("f{i:03}"), content.len() as u64))
            .collect();
        let rig = Rig::new(&entries);
        let intake = rig.attempt();
        let start = Barrier::new(2);
        // How many files hold all their spans.
        let complete = || {
            (0..FILES)
                .filter(|&i| rig.staging.is_complete(u32::try_from(i).unwrap()).unwrap())
                .count()
        };

        thread::scope(|scope| {
            let writer = scope.spawn(|| {
                start.wait();
                let mut written = 0;
                for (i, content) in contents.iter().enumerate() {
                    match intake.span(u32::try_from(i).unwrap(), 0, content) {
                        Ok(_) => written += 1,
                        Err(Halt::Gone) => break,
                        Err(other) => panic!("{other:?}"),
                    }
                }
                written
            });
            start.wait();
            // The writer is in the middle of its files, and a call of it is likely under way.
            while complete() < 5 {
                thread::yield_now();
            }

            intake.retire();
            let at_retire = complete();
            writer.join().unwrap();

            assert_eq!(
                complete(),
                at_retire,
                "a call went on after the book was retired"
            );
        });
    }

    // The gate.

    #[test]
    fn a_gate_lets_calls_in_until_it_is_closed_and_it_closes_once() {
        let gate = Gate::new();

        assert!(gate.enter().is_ok());
        assert!(gate.close());
        assert!(!gate.close());
        assert!(matches!(gate.enter(), Err(Halt::Gone)));
    }

    #[test]
    fn closing_a_gate_waits_for_the_calls_in_it() {
        let gate = Gate::new();
        let done = AtomicBool::new(false);

        thread::scope(|scope| {
            let inside = gate.enter().unwrap();
            let waiting = scope.spawn(|| {
                gate.close();
                done.store(true, Ordering::SeqCst);
            });
            // Long enough for a gate that does not wait to have closed.
            thread::sleep(Duration::from_millis(100));
            assert!(
                !done.load(Ordering::SeqCst),
                "the gate closed with a call in it"
            );

            drop(inside);
            waiting.join().unwrap();

            assert!(done.load(Ordering::SeqCst));
        });
        assert!(matches!(gate.enter(), Err(Halt::Gone)));
    }
}
