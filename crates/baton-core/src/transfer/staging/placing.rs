// SPDX-License-Identifier: Apache-2.0

//! Putting a received transfer in its place: [`Staging::place`], and the answer it gives.

use std::{
    fmt,
    io::{self, ErrorKind},
    path::PathBuf,
    thread,
    time::Duration,
};

use baton_proto::transfer::FailReason;

use super::{StageError, Staging, State, Top, is_out_of_room};
use crate::{transfer::names::Reserved, util::lock};

/// How many times, in all, a move is tried again while the system says that something has the file
/// open for a moment. A move that cannot work, in a destination that cannot be written, must not
/// make every item wait for it.
const RETRIES: u32 = 100;

/// How long to wait between two tries. A virus scanner is done with a file in a moment.
const RETRY_PAUSE: Duration = Duration::from_millis(50);

/// What became of one top-level item of a transfer.
#[derive(Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The item is in the destination, at this path. The name is the one that was planned, or that
    /// with a number if the planned name was taken.
    InPlace(PathBuf),
    /// None of the files in the item was checked, so there was nothing to put in place. The
    /// files are failed files like any other, and their reasons are the receiver's to give.
    LeftOut,
    /// The item could not be put in place, and none of it is.
    Failed(FailReason),
}

impl fmt::Debug for Outcome {
    // The path has the name of a file in it, and the folder that a person keeps them in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InPlace(_) => f.write_str("InPlace"),
            Self::LeftOut => f.write_str("LeftOut"),
            Self::Failed(reason) => f.debug_tuple("Failed").field(reason).finish(),
        }
    }
}

/// What [`Staging::place`] did.
#[derive(Clone, PartialEq, Eq)]
pub struct Placement {
    items: Vec<(u32, Outcome)>,
    failed: Vec<(u32, FailReason)>,
}

impl fmt::Debug for Placement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Placement")
            .field("items", &self.items.len())
            .field("failed", &self.failed.len())
            .finish()
    }
}

impl Placement {
    /// What became of each top-level item, with its position in the list, in the order of the list.
    pub fn items(&self) -> impl Iterator<Item = (u32, &Outcome)> {
        self.items.iter().map(|(entry, outcome)| (*entry, outcome))
    }

    /// The files that were checked and are not in place, because the item that has them could not
    /// be put in place, with why, in the order of the list. The files that were not checked are
    /// not here: the receiver already knows why each of them failed.
    pub fn failed(&self) -> &[(u32, FailReason)] {
        &self.failed
    }
}

/// How long a move is willing to wait for something that has a file open for a moment, which is
/// used up by every try that is made again, whichever item it was for.
struct Patience {
    retries: u32,
    pause: Duration,
}

impl Patience {
    /// Runs `attempt` until it works, or fails with an error that `transient` does not call
    /// worth another try, or the patience is used up.
    fn run<T>(
        &mut self,
        mut attempt: impl FnMut() -> io::Result<T>,
        transient: impl Fn(&io::Error) -> bool,
    ) -> io::Result<T> {
        loop {
            match attempt() {
                Err(error) if self.retries > 0 && transient(&error) => {
                    self.retries -= 1;
                    thread::sleep(self.pause);
                }
                other => return other,
            }
        }
    }
}

/// Windows says that a file is in use by another process with these two.
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Whether a move that failed with `error` may work in a moment. On Windows a program that has a
/// file inside the item open, a virus scanner for one, makes it fail with a sharing violation or
/// with access denied. Nothing else does that.
fn is_transient(error: &io::Error) -> bool {
    cfg!(windows)
        && (error.kind() == ErrorKind::PermissionDenied
            || matches!(
                error.raw_os_error(),
                Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
            ))
}

/// The reason `Done` gives for a file that is not in place because its item could not be moved:
/// a full disk is `NoSpace`, and anything else is that the receiver could not write it.
fn fail_reason(error: &io::Error) -> FailReason {
    if is_out_of_room(error) {
        FailReason::NoSpace
    } else {
        FailReason::WriteFailed
    }
}

/// A position in the list as the number that the wire uses for it.
fn position(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

impl Staging {
    /// Puts the top-level items of the transfer in the destination, once the transfer has ended
    /// and its files have been checked (protocol.md section 8, "Staging" and "Nothing is
    /// replaced"), and says what became of each. A person never finds a folder that is still
    /// being received, because a folder moves with everything in it in one step. For each item:
    ///
    /// - **Only checked files go.** A file that was not checked ([`verify`](Self::verify)) is
    ///   removed from the staging folder first, so that half of a file never turns up inside a
    ///   folder that is put in place. A folder of which no file was checked is left out whole
    ///   ([`Outcome::LeftOut`]). If a file cannot be removed, the folder that has it inside is not
    ///   put in place, and the files in it that were checked are named as failed.
    /// - **Nothing is replaced.** The move is [`baton_fs::rename_noreplace`]. When the name is
    ///   taken, whoever took it, the item takes the next number (`notes 2.txt`), and not one that
    ///   another item of the same transfer is planned to be called. A volume that cannot refuse to
    ///   replace fails the item, and it is never moved over another.
    /// - **A failure is the item's own.** The others are put in place, and the files of the item
    ///   that failed are in [`Placement::failed`], with why, for the `Done` of the transfer.
    ///
    /// Asking again gives the same answer and moves nothing, since a sender that gave up waiting
    /// for `Done` asks for it again. Nothing else of the transfer is written after this: a call to
    /// [`write_span`](Self::write_span) or [`verify`](Self::verify) fails with
    /// [`StageError::Placed`]. The folder of the transfer in the staging folder is not removed;
    /// [`discard`](Self::discard) does that when the transfer is forgotten, and never touches what
    /// was put in place.
    ///
    /// This is blocking disk work, and on Windows it can wait a moment for a program that has a
    /// file open. Call it on a thread meant for blocking.
    pub fn place(&self) -> Result<Placement, StageError> {
        let mut state = lock(&self.state);
        if let Some(placement) = &state.placed {
            return Ok(placement.clone());
        }
        if state.discarded {
            return Err(StageError::Discarded);
        }
        let reserved = Reserved::new(
            self.tops.iter().filter_map(|top| self.plan.name(top.entry)),
            self.rules,
        );
        let mut patience = Patience {
            retries: RETRIES,
            pause: RETRY_PAUSE,
        };
        let mut items = Vec::with_capacity(self.tops.len());
        let mut failed = Vec::new();
        for top in &self.tops {
            let outcome = self.place_top(&mut state, top, &reserved, &mut patience, &mut failed);
            items.push((position(top.entry), outcome));
        }
        let placement = Placement { items, failed };
        state.placed = Some(placement.clone());
        Ok(placement)
    }

    /// Puts one item in place, and says what became of it. The files of the item that were checked
    /// and are not in place are added to `failed`.
    fn place_top(
        &self,
        state: &mut State,
        top: &Top,
        reserved: &Reserved,
        patience: &mut Patience,
        failed: &mut Vec<(u32, FailReason)>,
    ) -> Outcome {
        let mut checked = Vec::new();
        let mut unchecked = Vec::new();
        for index in top.entry..top.end {
            if self.info.get(index).is_none_or(|info| info.folder) {
                continue;
            }
            let file = position(index);
            if state.slots.get(&file).is_some_and(|slot| slot.verified) {
                checked.push(file);
            } else {
                unchecked.push(file);
            }
        }
        // A file that was not checked must not be inside a folder that a person is given, and
        // what was started of it is of no use to anyone. One that cannot be removed holds the
        // folder back.
        let mut held_back = false;
        for file in &unchecked {
            if self.remove(state, *file).is_err() {
                held_back = true;
            }
        }
        if checked.is_empty() && !unchecked.is_empty() {
            return Outcome::LeftOut;
        }
        let moved = if held_back {
            Err(FailReason::WriteFailed)
        } else {
            self.move_into_place(top, reserved, patience)
                .map_err(|error| fail_reason(&error))
        };
        match moved {
            Ok(path) => Outcome::InPlace(path),
            Err(reason) => {
                failed.extend(checked.into_iter().map(|file| (file, reason)));
                Outcome::Failed(reason)
            }
        }
    }

    /// Moves the item to its name in the destination, or to the first number of it that is free,
    /// and says where it went.
    fn move_into_place(
        &self,
        top: &Top,
        reserved: &Reserved,
        patience: &mut Patience,
    ) -> io::Result<PathBuf> {
        let name = self
            .plan
            .name(top.entry)
            .ok_or_else(|| io::Error::from(ErrorKind::InvalidInput))?;
        // An item at the top of the list is in the folder of the transfer under its own name.
        let from = self.dir.join(name);
        let candidates = std::iter::once(name.to_owned()).chain(reserved.numbered(name));
        for candidate in candidates.take(self.max_name_tries) {
            let to = self.root.join(&candidate);
            match patience.run(|| baton_fs::rename_noreplace(&from, &to), is_transient) {
                Ok(()) => return Ok(to),
                // Taken, by something the person has or by another transfer. The next one.
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        // All the names that were tried are taken. Nothing was replaced.
        Err(io::Error::from(ErrorKind::AlreadyExists))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use baton_proto::{
        id::{DeviceId, TransferId},
        transfer::list::{ListEntry, Listing},
    };

    use super::*;
    use crate::transfer::{
        destination::Destination,
        testing::{bytes, file, folder, listing},
    };

    const SPAN: usize = 1 << 20;

    fn id(n: u8) -> TransferId {
        TransferId::from_bytes([n; TransferId::LEN])
    }

    /// A destination in a folder of its own. The folder goes when the first is dropped.
    fn destination() -> (tempfile::TempDir, Destination) {
        let scratch = tempfile::tempdir().unwrap();
        let destination =
            Destination::open(scratch.path(), &DeviceId::from_bytes([1; DeviceId::LEN])).unwrap();
        (scratch, destination)
    }

    /// The transfer `n` of `entries`, staged in `destination`, with the list it was staged from.
    fn staged(destination: &Destination, n: u8, entries: &[ListEntry]) -> (Staging, Listing) {
        let listing = listing(entries);
        let staging = destination.stage(id(n), &listing).unwrap();
        (staging, listing)
    }

    /// The number of the entry with this path.
    fn at(listing: &Listing, path: &str) -> u32 {
        let index = listing
            .entries()
            .iter()
            .position(|entry| entry.path == path)
            .unwrap();
        u32::try_from(index).unwrap()
    }

    /// What the receive state machine does for a file that came whole: every span, then the check.
    fn receive(staging: &Staging, file: u32, content: &[u8]) {
        for (span, chunk) in content.chunks(SPAN).enumerate() {
            let span = u32::try_from(span).unwrap();
            staging.write_span(file, span, chunk).unwrap();
        }
        staging.verify(file, &blake3::hash(content)).unwrap();
    }

    fn read(path: impl AsRef<std::path::Path>) -> Vec<u8> {
        fs::read(path).unwrap()
    }

    #[test]
    fn a_checked_file_is_put_in_place_under_its_name() {
        let (scratch, destination) = destination();
        let content = bytes(1, 100);
        let (staging, listing) = staged(&destination, 1, &[file("notes.txt", 100)]);
        receive(&staging, at(&listing, "notes.txt"), &content);

        let placement = staging.place().unwrap();
        let items: Vec<_> = placement.items().collect();
        assert_eq!(
            items,
            [(0, &Outcome::InPlace(scratch.path().join("notes.txt")))]
        );
        assert_eq!(placement.failed(), []);
        assert_eq!(read(scratch.path().join("notes.txt")), content);
        // The file left the staging folder, and it keeps the time that the list gave it.
        assert!(!staging.dir.join("notes.txt").exists());
        let modified = fs::metadata(scratch.path().join("notes.txt"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            modified,
            std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000)
        );
    }

    #[test]
    fn a_folder_is_put_in_place_with_everything_in_it() {
        let (scratch, destination) = destination();
        let (a, b, loose) = (bytes(2, 10), bytes(3, 3), bytes(4, 4));
        let (staging, listing) = staged(
            &destination,
            1,
            &[
                folder("Holiday"),
                folder("Holiday/day 1"),
                folder("Holiday/empty"),
                file("Holiday/day 1/a.jpg", 10),
                file("Holiday/b.txt", 3),
                file("loose.txt", 4),
            ],
        );
        receive(&staging, at(&listing, "Holiday/day 1/a.jpg"), &a);
        receive(&staging, at(&listing, "Holiday/b.txt"), &b);
        receive(&staging, at(&listing, "loose.txt"), &loose);

        let placement = staging.place().unwrap();
        let items: Vec<_> = placement.items().collect();
        assert_eq!(
            items,
            [
                (0, &Outcome::InPlace(scratch.path().join("Holiday"))),
                (
                    at(&listing, "loose.txt"),
                    &Outcome::InPlace(scratch.path().join("loose.txt"))
                ),
            ]
        );
        let holiday = scratch.path().join("Holiday");
        assert_eq!(read(holiday.join("day 1").join("a.jpg")), a);
        assert_eq!(read(holiday.join("b.txt")), b);
        assert!(holiday.join("empty").is_dir());
        assert_eq!(read(scratch.path().join("loose.txt")), loose);
        assert!(!staging.dir.join("Holiday").exists());
    }

    #[test]
    fn a_folder_with_no_files_in_it_is_put_in_place_too() {
        let (scratch, destination) = destination();
        let (staging, _) = staged(&destination, 1, &[folder("empty"), folder("empty/inner")]);
        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [Outcome::InPlace(scratch.path().join("empty"))]
        );
        assert!(scratch.path().join("empty").join("inner").is_dir());
    }

    #[test]
    fn a_name_that_is_taken_gets_a_number_and_what_is_there_is_not_touched() {
        let (scratch, destination) = destination();
        let root = scratch.path();
        fs::write(root.join("notes.txt"), b"mine").unwrap();
        fs::write(root.join("notes 2.txt"), b"mine too").unwrap();
        fs::create_dir(root.join("data")).unwrap();
        fs::write(root.join("data").join("x"), b"mine as well").unwrap();
        let (new, other) = (bytes(5, 50), bytes(6, 6));
        let (staging, listing) = staged(
            &destination,
            1,
            &[folder("data"), file("data/x", 6), file("notes.txt", 50)],
        );
        receive(&staging, at(&listing, "notes.txt"), &new);
        receive(&staging, at(&listing, "data/x"), &other);

        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [
                Outcome::InPlace(root.join("data 2")),
                Outcome::InPlace(root.join("notes 3.txt")),
            ]
        );
        assert_eq!(read(root.join("data 2").join("x")), other);
        assert_eq!(read(root.join("notes 3.txt")), new);
        // What the person had is as it was.
        assert_eq!(read(root.join("notes.txt")), b"mine");
        assert_eq!(read(root.join("notes 2.txt")), b"mine too");
        assert_eq!(read(root.join("data").join("x")), b"mine as well");
    }

    #[test]
    fn a_number_steps_over_the_names_of_the_transfer() {
        let (scratch, destination) = destination();
        let root = scratch.path();
        fs::write(root.join("a"), b"mine").unwrap();
        let (staging, listing) = staged(&destination, 1, &[file("a", 1), file("a 2", 1)]);
        receive(&staging, at(&listing, "a"), b"1");
        receive(&staging, at(&listing, "a 2"), b"2");

        staging.place().unwrap();
        // `a` meets what the person has and cannot take `a 2`, which is the name of the next item
        // of the transfer. That item keeps its name.
        assert_eq!(read(root.join("a 3")), b"1");
        assert_eq!(read(root.join("a 2")), b"2");
        assert_eq!(read(root.join("a")), b"mine");
    }

    #[test]
    fn two_transfers_with_one_name_do_not_replace_each_other() {
        let (scratch, destination) = destination();
        let root = scratch.path();
        let (first, first_list) = staged(&destination, 1, &[file("notes.txt", 5)]);
        let (second, second_list) = staged(&destination, 2, &[file("notes.txt", 6)]);
        receive(&first, at(&first_list, "notes.txt"), b"first");
        receive(&second, at(&second_list, "notes.txt"), b"second");

        // Both are done at the same time, and both found the name free when they started.
        first.place().unwrap();
        second.place().unwrap();
        assert_eq!(read(root.join("notes.txt")), b"first");
        assert_eq!(read(root.join("notes 2.txt")), b"second");
    }

    #[test]
    fn only_checked_files_are_put_in_place() {
        let (scratch, destination) = destination();
        let root = scratch.path();
        let ok = bytes(7, 20);
        let long = bytes(8, SPAN + 10);
        let (staging, listing) = staged(
            &destination,
            1,
            &[
                folder("d"),
                file("d/bad", 20),
                file("d/half", (SPAN + 10) as u64),
                file("d/never", 5),
                file("d/ok", 20),
            ],
        );
        receive(&staging, at(&listing, "d/ok"), &ok);
        // A file that came whole and failed its check is removed by the check.
        let bad = at(&listing, "d/bad");
        staging.write_span(bad, 0, &bytes(9, 20)).unwrap();
        assert!(matches!(
            staging.verify(bad, &blake3::hash(b"something else")),
            Err(StageError::Mismatch)
        ));
        // One that is half there, and one that never started.
        let half = at(&listing, "d/half");
        staging.write_span(half, 0, &long[..SPAN]).unwrap();
        assert!(staging.dir.join("d").join("half").exists());

        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [Outcome::InPlace(root.join("d"))]
        );
        // Only the file that was checked is inside, and the files that were not are named by the
        // receiver, who knows why, and not here.
        let names: Vec<_> = fs::read_dir(root.join("d"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["ok"]);
        assert_eq!(placement.failed(), []);
        assert!(!staging.dir.join("d").join("half").exists());
    }

    #[test]
    fn nothing_is_put_in_place_of_an_item_none_of_whose_files_was_checked() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(
            &destination,
            1,
            &[
                folder("d"),
                file("d/x", SPAN as u64 + 1),
                file("gone.txt", 3),
            ],
        );
        // A file that is half there is no more use than one that never came.
        staging
            .write_span(at(&listing, "d/x"), 0, &bytes(10, SPAN))
            .unwrap();

        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [Outcome::LeftOut, Outcome::LeftOut]
        );
        assert_eq!(placement.failed(), []);
        assert!(!scratch.path().join("d").exists());
        assert!(!scratch.path().join("gone.txt").exists());
        assert!(!staging.dir.join("d").join("x").exists());
    }

    #[test]
    fn a_folder_with_a_file_that_cannot_be_removed_is_not_put_in_place() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(
            &destination,
            1,
            &[
                folder("d"),
                file("d/ok", 3),
                file("d/stuck", 3),
                file("fine.txt", 4),
            ],
        );
        let (ok, stuck) = (at(&listing, "d/ok"), at(&listing, "d/stuck"));
        receive(&staging, ok, b"abc");
        receive(&staging, at(&listing, "fine.txt"), b"fine");
        staging.write_span(stuck, 0, b"xyz").unwrap();
        // The system will not let this one go.
        staging.make_unremovable(stuck);

        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [
                Outcome::Failed(FailReason::WriteFailed),
                Outcome::InPlace(scratch.path().join("fine.txt")),
            ]
        );
        // The file of the folder that did check is named, and the folder is not given to anyone.
        assert_eq!(placement.failed(), [(ok, FailReason::WriteFailed)]);
        assert_eq!(staging.stuck(), [stuck]);
        assert!(!scratch.path().join("d").exists());
        assert_eq!(read(scratch.path().join("fine.txt")), b"fine");
    }

    #[test]
    fn placing_tries_again_to_remove_a_file_that_could_not_be_removed_before() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(
            &destination,
            1,
            &[folder("d"), file("d/ok", 3), file("d/x", 3)],
        );
        let (ok, x) = (at(&listing, "d/ok"), at(&listing, "d/x"));
        receive(&staging, ok, b"abc");
        staging.write_span(x, 0, b"xyz").unwrap();
        staging.make_unremovable(x);
        // The receiver dropped the file when its check failed, and the system would not let it go.
        assert!(staging.drop_file(x).is_err());
        assert_eq!(staging.stuck(), [x]);
        // By the time the transfer ends it can be removed, and then nothing holds the folder back.
        staging.make_removable(x);

        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [Outcome::InPlace(scratch.path().join("d"))]
        );
        assert_eq!(placement.failed(), []);
        assert_eq!(staging.stuck(), Vec::<u32>::new());
        assert_eq!(fs::read_dir(scratch.path().join("d")).unwrap().count(), 1);
        assert_eq!(read(scratch.path().join("d").join("ok")), b"abc");
    }

    #[test]
    fn an_item_that_cannot_be_moved_fails_alone() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(
            &destination,
            1,
            &[
                folder("d"),
                file("d/x", 1),
                file("e.txt", 1),
                file("f.txt", 1),
            ],
        );
        for path in ["d/x", "e.txt", "f.txt"] {
            receive(&staging, at(&listing, path), b"1");
        }
        // As if somebody had taken the folder out of the staging folder.
        fs::remove_dir_all(staging.dir.join("d")).unwrap();

        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [
                Outcome::Failed(FailReason::WriteFailed),
                Outcome::InPlace(scratch.path().join("e.txt")),
                Outcome::InPlace(scratch.path().join("f.txt")),
            ]
        );
        assert_eq!(
            placement.failed(),
            [(at(&listing, "d/x"), FailReason::WriteFailed)]
        );
    }

    #[test]
    fn an_item_that_finds_every_name_it_tries_taken_fails_and_replaces_nothing() {
        let (scratch, destination) = destination();
        let root = scratch.path();
        for name in ["n", "n 2", "n 3"] {
            fs::write(root.join(name), name).unwrap();
        }
        let (mut staging, listing) = staged(&destination, 1, &[file("n", 1)]);
        staging.max_name_tries = 3;
        receive(&staging, at(&listing, "n"), b"x");

        let placement = staging.place().unwrap();
        assert_eq!(
            placement
                .items()
                .map(|(_, outcome)| outcome.clone())
                .collect::<Vec<_>>(),
            [Outcome::Failed(FailReason::WriteFailed)]
        );
        assert_eq!(placement.failed(), [(0, FailReason::WriteFailed)]);
        for name in ["n", "n 2", "n 3"] {
            assert_eq!(read(root.join(name)), name.as_bytes());
        }
        assert!(!root.join("n 4").exists());
    }

    #[test]
    fn asking_again_gives_the_same_answer_and_moves_nothing() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(&destination, 1, &[file("notes.txt", 3)]);
        receive(&staging, at(&listing, "notes.txt"), b"abc");

        let first = staging.place().unwrap();
        let second = staging.place().unwrap();
        assert_eq!(first, second);
        assert_eq!(read(scratch.path().join("notes.txt")), b"abc");
        assert!(!scratch.path().join("notes 2.txt").exists());
    }

    #[test]
    fn a_transfer_that_was_put_in_place_takes_no_more_spans() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(&destination, 1, &[file("a", 1), file("b", 1)]);
        let (a, b) = (at(&listing, "a"), at(&listing, "b"));
        receive(&staging, a, b"1");
        staging.place().unwrap();

        // A lane that is late makes no file, and a file is not checked again.
        assert!(matches!(
            staging.write_span(b, 0, b"2"),
            Err(StageError::Placed)
        ));
        assert!(matches!(
            staging.verify(a, &blake3::hash(b"1")),
            Err(StageError::Placed)
        ));
        assert!(!staging.dir.join("b").exists());
        assert!(!scratch.path().join("b").exists());
        // Dropping a file that is in place removes nothing of it.
        staging.drop_file(a).unwrap();
        assert_eq!(read(scratch.path().join("a")), b"1");
    }

    #[test]
    fn discarding_a_transfer_that_is_in_place_keeps_what_was_put_in_place() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(&destination, 1, &[file("a", 1), folder("d")]);
        receive(&staging, at(&listing, "a"), b"1");
        staging.place().unwrap();
        staging.discard().unwrap();
        assert!(!staging.dir.exists());
        assert_eq!(read(scratch.path().join("a")), b"1");
        assert!(scratch.path().join("d").is_dir());
    }

    #[test]
    fn a_transfer_that_was_discarded_cannot_be_put_in_place() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(&destination, 1, &[file("a", 1)]);
        receive(&staging, at(&listing, "a"), b"1");
        staging.discard().unwrap();
        assert!(matches!(staging.place(), Err(StageError::Discarded)));
        assert!(!scratch.path().join("a").exists());
    }

    #[test]
    fn the_items_of_a_list_are_the_entries_at_the_top() {
        let listing = listing(&[
            file("a", 1),
            folder("b"),
            folder("b/c"),
            file("b/c/d", 1),
            file("b/e", 1),
            folder("f"),
            file("g", 1),
        ]);
        let found: Vec<_> = crate::transfer::staging::tops(&listing)
            .iter()
            .map(|top| (top.entry, top.end))
            .collect();
        assert_eq!(found, [(0, 1), (1, 5), (5, 6), (6, 7)]);
    }

    #[test]
    fn a_failed_move_gives_the_reason_for_what_went_wrong() {
        for (kind, reason) in [
            (ErrorKind::StorageFull, FailReason::NoSpace),
            (ErrorKind::QuotaExceeded, FailReason::NoSpace),
            (ErrorKind::Unsupported, FailReason::WriteFailed),
            (ErrorKind::PermissionDenied, FailReason::WriteFailed),
            (ErrorKind::NotFound, FailReason::WriteFailed),
            (ErrorKind::AlreadyExists, FailReason::WriteFailed),
        ] {
            assert_eq!(fail_reason(&io::Error::from(kind)), reason, "{kind:?}");
        }
    }

    #[test]
    fn a_move_is_tried_again_while_something_holds_the_file_for_a_moment() {
        let mut patience = Patience {
            retries: 5,
            pause: Duration::from_millis(1),
        };
        let mut tries = 0;
        let worked = patience.run(
            || {
                tries += 1;
                if tries < 3 {
                    Err(io::Error::from(ErrorKind::PermissionDenied))
                } else {
                    Ok(tries)
                }
            },
            |_| true,
        );
        assert_eq!(worked.unwrap(), 3);
        assert_eq!(patience.retries, 3);

        // An error that is not worth another try comes back at once.
        let mut tries = 0;
        let error = patience
            .run(
                || -> io::Result<()> {
                    tries += 1;
                    Err(io::Error::from(ErrorKind::NotFound))
                },
                |_| false,
            )
            .unwrap_err();
        assert_eq!((error.kind(), tries), (ErrorKind::NotFound, 1));
        assert_eq!(patience.retries, 3);

        // The patience is for all the tries of a transfer: once it is used up, a move that fails
        // that way fails at once, and the next item does not wait for it again.
        let mut tries = 0;
        let error = patience
            .run(
                || -> io::Result<()> {
                    tries += 1;
                    Err(io::Error::from(ErrorKind::PermissionDenied))
                },
                |_| true,
            )
            .unwrap_err();
        assert_eq!((error.kind(), tries), (ErrorKind::PermissionDenied, 4));
        assert_eq!(patience.retries, 0);
        let mut tries = 0;
        let _ = patience.run(
            || -> io::Result<()> {
                tries += 1;
                Err(io::Error::from(ErrorKind::PermissionDenied))
            },
            |_| true,
        );
        assert_eq!(tries, 1);
    }

    #[test]
    fn only_windows_waits_for_a_file_that_is_in_use() {
        let denied = io::Error::from(ErrorKind::PermissionDenied);
        let sharing = io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION);
        let not_found = io::Error::from(ErrorKind::NotFound);
        assert_eq!(is_transient(&denied), cfg!(windows));
        assert_eq!(is_transient(&sharing), cfg!(windows));
        assert!(!is_transient(&not_found));
        assert!(!is_transient(&io::Error::from(ErrorKind::AlreadyExists)));
    }

    #[test]
    fn the_debug_forms_have_no_names_or_folders_in_them() {
        let (scratch, destination) = destination();
        let (staging, listing) = staged(&destination, 1, &[file("secret name.txt", 1)]);
        receive(&staging, at(&listing, "secret name.txt"), b"1");
        let placement = staging.place().unwrap();
        for shown in [
            format!("{placement:?}"),
            format!("{:?}", placement.items().collect::<Vec<_>>()),
        ] {
            assert!(!shown.contains("secret"), "{shown}");
            assert!(!shown.contains(scratch.path().to_str().unwrap()), "{shown}");
        }
    }
}
