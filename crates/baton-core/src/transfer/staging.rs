// SPDX-License-Identifier: Apache-2.0

//! One transfer that is being received: its staging folder, and the files that are written in it.
//!
//! [`Destination::stage`](super::destination::Destination::stage) makes the folder of a transfer
//! with every folder of its list in it, under the names that [`names::plan`] chose. A file is made
//! when its first span arrives. Each span is hashed on its own and written at its place in the
//! file, so spans can come in any order and from several lanes at once, and memory is bounded by
//! the spans being written and not by the size of any file (ADR 0008). The value of each span is
//! kept, 32 bytes a span, and [`Staging::verify`] merges them when the sender says what the hash
//! of the file is.
//!
//! This is the disk side of receiving. It knows nothing about lanes, attempts or what a sender may
//! send and when: the state machine that applies the rules of protocol.md section 8 decides what
//! to call, and what each answer means for the transfer. What is kept here is what has to do with
//! the bytes on the disk:
//!
//! - **The bytes and the values agree.** The value of a span is kept only after its bytes are
//!   written, and a span that is held already, or is being written by another lane, is never
//!   written again. So the bytes a file has are the first that arrived for each span, and the hash
//!   that is checked is the hash of those. A sender that sends two versions of a span cannot get
//!   the second into a file whose hash says the first.
//! - **Nothing is mixed.** A file is made with `create_new`, so a name that is taken already
//!   fails that file and never gets the bytes of another.
//! - **A file that fails is removed,** and can start again from its first span.
//! - **A received file is never executable,** and takes the time that the sender gave it.
//!
//! All of it is blocking disk work. Code that runs on an async executor calls it on a thread
//! meant for blocking, as the rest of this crate does for its files.

use std::{
    collections::{BTreeSet, HashMap, hash_map::Entry},
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, ErrorKind},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use baton_proto::{
    id::TransferId,
    transfer::{
        hash::{self, FileHasher, HashError, SPAN_LEN},
        list::Listing,
    },
};

use super::names::{self, Plan, PlanError, Rules};
use crate::util::lock;

/// The most files that a transfer keeps open at once. A phone allows a process a few hundred open
/// files, and a transfer has at most eight lanes, each writing one span at a time.
const MAX_OPEN_FILES: usize = 16;

/// Why a staging folder could not be made or written, or a file could not be checked.
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    /// The names of the list could not be planned. A list that was checked against its offer
    /// never has this problem.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// The list has no file with this number: the number is out of range, or it is a folder.
    #[error("the transfer has no file number {file}")]
    NoSuchFile {
        /// The number that was asked for.
        file: u32,
    },
    /// A span that does not exist, or whose bytes are not as many as it has.
    #[error(transparent)]
    Hash(#[from] HashError),
    /// A file was checked before all its spans were written.
    #[error("the file does not have all its spans yet")]
    Incomplete,
    /// The spans of a file do not make the hash that the sender gave. The file is removed.
    #[error("the file does not have the hash that its sender gave")]
    Mismatch,
    /// The disk is full, or the quota of the app is used up.
    #[error("there is no room on the disk")]
    NoSpace,
    /// Anything else the disk said. The text is the system's own and has no name of a file in it.
    #[error("the staging folder could not be written: {0}")]
    Io(io::Error),
}

impl From<io::Error> for StageError {
    fn from(error: io::Error) -> Self {
        if matches!(
            error.kind(),
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded
        ) {
            Self::NoSpace
        } else {
            Self::Io(error)
        }
    }
}

/// What writing a span did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// The span is written and its value is kept. `complete` is true if it was the last span the
    /// file was missing, so that the file can be checked as soon as its `End` is in.
    New {
        /// Whether every span of the file is written now.
        complete: bool,
    },
    /// The file has this span already, or another lane is writing it now. The bytes were not
    /// written, and the first ones are the ones the file has.
    Duplicate,
}

/// What the list says about an entry, and nothing else of it.
#[derive(Clone, Copy)]
struct Info {
    folder: bool,
    size: u64,
    modified_ms: Option<i64>,
}

/// What is known about a file that has been started.
struct Slot {
    /// The values of the spans that are written.
    hasher: FileHasher,
    /// The spans that are being written now, so that no other lane writes them too.
    pending: Vec<u32>,
    /// Whether the file has been made. It is made once, with `create_new`.
    created: bool,
    /// The file, open for writing, for as long as it is one of the few that are kept open.
    handle: Option<Arc<File>>,
    /// Whether the time of the file has been set.
    stamped: bool,
}

impl Slot {
    fn new(size: u64) -> Result<Self, HashError> {
        Ok(Self {
            hasher: FileHasher::new(size)?,
            pending: Vec::new(),
            created: false,
            handle: None,
            stamped: false,
        })
    }
}

#[derive(Default)]
struct State {
    slots: HashMap<u32, Slot>,
    /// The files that hold a handle, the one used longest ago first.
    open: Vec<u32>,
    /// Files that were dropped and could not be removed from the disk.
    stuck: BTreeSet<u32>,
}

/// A transfer that is being received into a staging folder.
///
/// It can be shared between the tasks that read the lanes: every method takes `&self`. See the
/// [module documentation](self).
pub struct Staging {
    id: TransferId,
    dir: PathBuf,
    plan: Plan,
    info: Vec<Info>,
    max_open: usize,
    state: Mutex<State>,
}

impl fmt::Debug for Staging {
    // What people call their files is theirs, and so is the place they keep them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Staging")
            .field("id", &self.id)
            .field("entries", &self.info.len())
            .finish_non_exhaustive()
    }
}

impl Staging {
    /// Makes the folder of transfer `id` in `parent`, and the folders of `listing` in it.
    ///
    /// `parent` is the staging folder of a destination. The folder of the transfer must not exist:
    /// a name taken by a folder of the list means the volume treats two names as one that the
    /// rules did not, and the transfer is not started, with nothing left behind.
    pub(super) fn create(
        parent: &Path,
        id: TransferId,
        listing: &Listing,
        rules: Rules,
    ) -> Result<Self, StageError> {
        let plan = names::plan(listing.entries(), rules)?;
        let info = listing
            .entries()
            .iter()
            .map(|entry| Info {
                folder: entry.folder,
                size: entry.size,
                modified_ms: entry.modified_ms,
            })
            .collect();
        let dir = parent.join(id.to_string());
        make_dir(&dir, true)?;
        let staging = Self {
            id,
            dir,
            plan,
            info,
            max_open: MAX_OPEN_FILES,
            state: Mutex::new(State::default()),
        };
        if let Err(e) = staging.make_folders() {
            // Nothing of a transfer that did not start stays behind.
            let _ = fs::remove_dir_all(&staging.dir);
            return Err(e);
        }
        Ok(staging)
    }

    /// The id of the transfer.
    pub fn id(&self) -> TransferId {
        self.id
    }

    /// Writes span `span` of file number `file`, and keeps its value.
    ///
    /// `data` must be exactly the bytes of the span. The file is made when its first span
    /// arrives. A span that the file has already, or that is being written by another lane, is
    /// not written again ([`Written::Duplicate`]): the first bytes stay. If the disk fails, the
    /// span is not kept and the file is damaged: the caller drops it with
    /// [`drop_file`](Self::drop_file), and it can start again.
    pub fn write_span(&self, file: u32, span: u32, data: &[u8]) -> Result<Written, StageError> {
        let info = self.file_info(file)?;
        let expected =
            hash::span_size(info.size, span).ok_or(HashError::NoSuchSpan { index: span })?;
        if usize::try_from(expected) != Ok(data.len()) {
            return Err(HashError::WrongLength {
                index: span,
                expected,
                got: data.len(),
            }
            .into());
        }
        let handle = match self.claim(file, span)? {
            Claim::Held => return Ok(Written::Duplicate),
            Claim::Writing(handle) => handle,
        };
        let written = hash::hash_span(info.size, span, data)
            .map_err(StageError::from)
            .and_then(|value| {
                write_all_at(&handle, data, u64::from(span) * SPAN_LEN)?;
                Ok(value)
            });
        // This lane lets go of the file before the span counts as written. Whoever checks the file
        // next sets its time, and a file that is closed afterwards by a handle that wrote is
        // given the time of that write on some systems.
        drop(handle);
        match written {
            Ok(value) => {
                let complete = self.release(file, span, Some(value));
                Ok(Written::New { complete })
            }
            Err(e) => {
                self.release(file, span, None);
                Err(e)
            }
        }
    }

    /// Whether every span of file number `file` is written. An empty file has none to write, so it
    /// always is.
    pub fn is_complete(&self, file: u32) -> Result<bool, StageError> {
        let info = self.file_info(file)?;
        if info.size == 0 {
            return Ok(true);
        }
        Ok(lock(&self.state)
            .slots
            .get(&file)
            .is_some_and(|slot| slot.hasher.is_complete()))
    }

    /// Checks file number `file` against `expected`, the hash its sender gave with `End`.
    ///
    /// The spans' values are merged and compared. If they match, the file takes the time the
    /// list gave it and is closed, and stays in staging where [`Staging`] keeps it; it can be
    /// checked again, and a resumed transfer does that for every file. If they do not, the file
    /// is removed and [`StageError::Mismatch`] says so, and the file can start again. An empty
    /// file is made here, since it has no span to make it.
    pub fn verify(&self, file: u32, expected: &blake3::Hash) -> Result<(), StageError> {
        let info = self.file_info(file)?;
        let mut state = lock(&self.state);
        if info.size == 0 && !state.slots.contains_key(&file) {
            state.slots.insert(file, Slot::new(0)?);
            if let Err(e) = self.handle(&mut state, file) {
                state.slots.remove(&file);
                return Err(e);
            }
        }
        let slot = state.slots.get(&file).ok_or(StageError::Incomplete)?;
        if !slot.hasher.is_complete() {
            return Err(StageError::Incomplete);
        }
        if slot.hasher.finish()? != *expected {
            // The file's own error is the mismatch; a file that cannot be removed is on record.
            let _ = self.remove(&mut state, file);
            return Err(StageError::Mismatch);
        }
        let stamped = slot.stamped;
        if !stamped {
            if let Some(time) = info.modified_ms.and_then(system_time) {
                // A time that cannot be set (a volume that does not keep one, a date it cannot
                // hold) does not fail a file whose bytes are right.
                let _ = self
                    .handle(&mut state, file)
                    .and_then(|handle| handle.set_modified(time).map_err(StageError::from));
            }
            if let Some(slot) = state.slots.get_mut(&file) {
                slot.stamped = true;
            }
        }
        Self::close(&mut state, file);
        Ok(())
    }

    /// Removes what is written of file number `file`, so that it can start again from its first
    /// span, as a file does after the sender skipped it or it failed.
    ///
    /// A span that another lane is writing at this moment ends up in a file that is gone. If the
    /// file cannot be removed, it is on record in [`stuck`](Self::stuck), and nothing that has
    /// it inside may be put in place.
    pub fn drop_file(&self, file: u32) -> Result<(), StageError> {
        self.file_info(file)?;
        let mut state = lock(&self.state);
        self.remove(&mut state, file).map_err(StageError::from)
    }

    /// The files that were dropped, or failed their check, and could not be removed. A folder
    /// that has one of them inside must not be put in place.
    pub fn stuck(&self) -> Vec<u32> {
        lock(&self.state).stuck.iter().copied().collect()
    }

    /// Removes the folder of the transfer and everything in it, as for a transfer that was
    /// cancelled, or whose decision is forgotten. The lanes of the transfer are stopped by then,
    /// and a call that still comes in fails, since there is no folder to write in.
    pub fn discard(&self) -> io::Result<()> {
        {
            // Close what is open first: some systems do not remove a folder with an open file in it.
            let mut state = lock(&self.state);
            state.slots.clear();
            state.open.clear();
        }
        fs::remove_dir_all(&self.dir)
    }

    fn file_info(&self, file: u32) -> Result<Info, StageError> {
        usize::try_from(file)
            .ok()
            .and_then(|index| self.info.get(index))
            .copied()
            .filter(|info| !info.folder)
            .ok_or(StageError::NoSuchFile { file })
    }

    fn staged_path(&self, index: usize) -> Result<PathBuf, StageError> {
        self.plan
            .path(index)
            .map(|path| self.dir.join(path))
            .ok_or(StageError::Plan(PlanError::MissingParent { index }))
    }

    fn make_folders(&self) -> Result<(), StageError> {
        // The list puts a folder before what is inside it, so each one's folder is there.
        for (index, info) in self.info.iter().enumerate() {
            if info.folder {
                make_dir(&self.staged_path(index)?, false)?;
            }
        }
        Ok(())
    }

    /// Takes a place for span `span` of file `file`, and a handle to write it with, or says that
    /// the span is held or is being written.
    fn claim(&self, file: u32, span: u32) -> Result<Claim, StageError> {
        let info = self.file_info(file)?;
        let mut state = lock(&self.state);
        let slot = match state.slots.entry(file) {
            Entry::Occupied(slot) => slot.into_mut(),
            Entry::Vacant(slot) => slot.insert(Slot::new(info.size)?),
        };
        if slot.hasher.has(span) || slot.pending.contains(&span) {
            return Ok(Claim::Held);
        }
        let handle = self.handle(&mut state, file)?;
        if let Some(slot) = state.slots.get_mut(&file) {
            slot.pending.push(span);
        }
        Ok(Claim::Writing(handle))
    }

    /// Gives the place of a span back, and keeps its value if its bytes were written. Says whether
    /// the file is complete now.
    fn release(&self, file: u32, span: u32, value: Option<hash::SpanValue>) -> bool {
        let mut state = lock(&self.state);
        // A file that was dropped meanwhile has no slot, and nothing to keep.
        let Some(slot) = state.slots.get_mut(&file) else {
            return false;
        };
        slot.pending.retain(|pending| *pending != span);
        match value {
            Some(value) => slot.hasher.insert(span, value).is_ok() && slot.hasher.is_complete(),
            None => false,
        }
    }

    /// The open file for `file`, made or opened if it is not open. The slot must exist.
    fn handle(&self, state: &mut State, file: u32) -> Result<Arc<File>, StageError> {
        let size = self.file_info(file)?.size;
        let path = self.staged_path(usize::try_from(file).unwrap_or(usize::MAX))?;
        let slot = state
            .slots
            .get_mut(&file)
            .ok_or(StageError::NoSuchFile { file })?;
        if let Some(handle) = &slot.handle {
            let handle = Arc::clone(handle);
            state.open.retain(|open| *open != file);
            state.open.push(file);
            return Ok(handle);
        }
        let opened = if slot.created {
            OpenOptions::new().write(true).open(&path)?
        } else {
            let made = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            // The file has its size from the start, so that spans can be written in any order,
            // and where the system keeps the room for it, the disk is found full now.
            if let Err(e) = made.set_len(size) {
                drop(made);
                let _ = fs::remove_file(&path);
                return Err(e.into());
            }
            slot.created = true;
            made
        };
        let handle = Arc::new(opened);
        slot.handle = Some(Arc::clone(&handle));
        state.open.push(file);
        self.evict(state, file);
        Ok(handle)
    }

    /// Closes the files used longest ago while more than the limit are open. A file with a span
    /// being written is left open, and so is `keep`.
    fn evict(&self, state: &mut State, keep: u32) {
        while state.open.len() > self.max_open {
            let victim = state.open.iter().copied().find(|file| {
                *file != keep
                    && state
                        .slots
                        .get(file)
                        .is_some_and(|slot| slot.pending.is_empty())
            });
            match victim {
                Some(file) => Self::close(state, file),
                None => break,
            }
        }
    }

    fn close(state: &mut State, file: u32) {
        if let Some(slot) = state.slots.get_mut(&file) {
            slot.handle = None;
        }
        state.open.retain(|open| *open != file);
    }

    /// Forgets file `file` and removes it from the disk.
    fn remove(&self, state: &mut State, file: u32) -> io::Result<()> {
        state.open.retain(|open| *open != file);
        let Some(slot) = state.slots.remove(&file) else {
            return Ok(());
        };
        if !slot.created {
            return Ok(());
        }
        drop(slot);
        let Ok(path) = self.staged_path(usize::try_from(file).unwrap_or(usize::MAX)) else {
            return Ok(());
        };
        match fs::remove_file(path) {
            Err(e) if e.kind() != ErrorKind::NotFound => {
                state.stuck.insert(file);
                Err(e)
            }
            _ => Ok(()),
        }
    }
}

enum Claim {
    /// The span is held, or another lane is writing it.
    Held,
    /// The caller writes the span with this file.
    Writing(Arc<File>),
}

/// Makes a folder that must not exist yet. Where the system has modes, a private one has only its
/// owner in it, and any other follows the mask of the user like any folder they make.
#[cfg(unix)]
pub(super) fn make_dir(path: &Path, private: bool) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    if private {
        builder.mode(0o700);
    }
    builder.create(path)
}

#[cfg(not(unix))]
pub(super) fn make_dir(path: &Path, _private: bool) -> io::Result<()> {
    fs::create_dir(path)
}

/// The time that the list gave a file, as the system takes it, if it can be one.
fn system_time(modified_ms: i64) -> Option<SystemTime> {
    let since = Duration::from_millis(modified_ms.unsigned_abs());
    if modified_ms >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(since)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(since)
    }
}

#[cfg(unix)]
fn write_all_at(file: &File, data: &[u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, data, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, mut data: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let written = file.seek_write(data, offset)?;
        if written == 0 {
            return Err(ErrorKind::WriteZero.into());
        }
        data = data.get(written..).unwrap_or_default();
        offset += written as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::thread;

    use baton_proto::id::DeviceId;
    use proptest::prelude::*;

    use super::*;
    use crate::transfer::{
        destination::Destination,
        testing::{bytes, file, folder, listing},
    };

    const SPAN: usize = 1 << 20;
    const _: () = assert!(SPAN as u64 == SPAN_LEN);

    fn n32(n: usize) -> u32 {
        u32::try_from(n).unwrap()
    }

    fn id() -> TransferId {
        TransferId::from_bytes([7; TransferId::LEN])
    }

    /// A destination in a folder of its own. The folder goes when the first is dropped.
    fn destination() -> (tempfile::TempDir, Destination) {
        let scratch = tempfile::tempdir().unwrap();
        let destination =
            Destination::open(scratch.path(), &DeviceId::from_bytes([1; DeviceId::LEN])).unwrap();
        (scratch, destination)
    }

    fn staged(
        destination: &Destination,
        entries: &[baton_proto::transfer::list::ListEntry],
    ) -> Staging {
        destination.stage(id(), &listing(entries)).unwrap()
    }

    #[test]
    fn the_folders_of_the_list_are_made_and_the_files_wait_for_their_first_span() {
        let (_scratch, destination) = destination();
        let staging = staged(
            &destination,
            &[
                folder("Holiday"),
                folder("Holiday/day 1"),
                folder("Holiday/empty"),
                file("Holiday/day 1/a.jpg", 10),
                file("notes.txt", 5),
            ],
        );
        assert!(staging.dir.join("Holiday").join("day 1").is_dir());
        assert!(staging.dir.join("Holiday").join("empty").is_dir());
        assert!(!staging.dir.join("notes.txt").exists());
        assert!(
            !staging
                .dir
                .join("Holiday")
                .join("day 1")
                .join("a.jpg")
                .exists()
        );
    }

    #[test]
    fn a_file_written_in_any_order_has_the_hash_of_its_content() {
        let (_scratch, destination) = destination();
        let size = 3 * SPAN + 123;
        let content = bytes(1, size);
        let staging = staged(&destination, &[file("big", size as u64)]);
        let spans: Vec<&[u8]> = content.chunks(SPAN).collect();

        let mut outcomes = Vec::new();
        for span in [2_usize, 0, 3, 1] {
            outcomes.push(staging.write_span(0, n32(span), spans[span]).unwrap());
        }
        assert_eq!(
            outcomes,
            [
                Written::New { complete: false },
                Written::New { complete: false },
                Written::New { complete: false },
                Written::New { complete: true },
            ]
        );
        assert!(staging.is_complete(0).unwrap());
        staging.verify(0, &blake3::hash(&content)).unwrap();
        assert_eq!(fs::read(staging.dir.join("big")).unwrap(), content);
    }

    #[test]
    fn lanes_may_write_the_spans_of_one_file_at_the_same_time() {
        let (_scratch, destination) = destination();
        let size = 8 * SPAN + 5;
        let content = bytes(2, size);
        let staging = staged(&destination, &[file("big", size as u64)]);
        let spans: Vec<&[u8]> = content.chunks(SPAN).collect();
        let complete = Mutex::new(Vec::new());
        thread::scope(|scope| {
            for lane in 0..4_usize {
                let (staging, spans, complete) = (&staging, &spans, &complete);
                scope.spawn(move || {
                    // Each lane takes every fourth span, the last ones first.
                    for span in (0..spans.len()).rev().filter(|span| span % 4 == lane) {
                        let written = staging.write_span(0, n32(span), spans[span]).unwrap();
                        lock(complete).push(written);
                    }
                });
            }
        });
        let complete = complete.into_inner().unwrap();
        assert_eq!(complete.len(), spans.len());
        assert_eq!(
            complete
                .iter()
                .filter(|written| **written == Written::New { complete: true })
                .count(),
            1,
            "exactly one write is the one that completes the file"
        );
        staging.verify(0, &blake3::hash(&content)).unwrap();
        assert_eq!(fs::read(staging.dir.join("big")).unwrap(), content);
    }

    #[test]
    fn a_span_that_comes_twice_keeps_the_first_bytes() {
        let (_scratch, destination) = destination();
        let size = SPAN + 10;
        let first = bytes(3, size);
        let second = bytes(4, size);
        let staging = staged(&destination, &[file("f", size as u64)]);

        assert_eq!(
            staging.write_span(0, 0, &first[..SPAN]).unwrap(),
            Written::New { complete: false }
        );
        // The other bytes are not written, and not counted either.
        assert_eq!(
            staging.write_span(0, 0, &second[..SPAN]).unwrap(),
            Written::Duplicate
        );
        assert_eq!(
            staging.write_span(0, 1, &first[SPAN..]).unwrap(),
            Written::New { complete: true }
        );
        staging.verify(0, &blake3::hash(&first)).unwrap();
        assert_eq!(fs::read(staging.dir.join("f")).unwrap(), first);
    }

    #[test]
    fn a_span_that_another_lane_is_writing_is_not_written_again() {
        let (_scratch, destination) = destination();
        let size = SPAN + 10;
        let content = bytes(5, size);
        let staging = staged(&destination, &[file("f", size as u64)]);
        staging.write_span(0, 0, &content[..SPAN]).unwrap();

        // As if a lane had begun writing span 1 and not finished.
        lock(&staging.state)
            .slots
            .get_mut(&0)
            .unwrap()
            .pending
            .push(1);
        assert_eq!(
            staging.write_span(0, 1, &content[SPAN..]).unwrap(),
            Written::Duplicate
        );
        assert!(!staging.is_complete(0).unwrap());

        lock(&staging.state)
            .slots
            .get_mut(&0)
            .unwrap()
            .pending
            .clear();
        assert_eq!(
            staging.write_span(0, 1, &content[SPAN..]).unwrap(),
            Written::New { complete: true }
        );
    }

    #[test]
    fn a_file_with_the_wrong_hash_is_removed_and_starts_again() {
        let (_scratch, destination) = destination();
        let size = SPAN + 10;
        let content = bytes(6, size);
        let staging = staged(&destination, &[file("f", size as u64)]);
        let write_all = || {
            staging.write_span(0, 0, &content[..SPAN]).unwrap();
            staging.write_span(0, 1, &content[SPAN..]).unwrap();
        };

        write_all();
        let wrong = blake3::hash(b"something else");
        assert!(matches!(
            staging.verify(0, &wrong),
            Err(StageError::Mismatch)
        ));
        assert!(!staging.dir.join("f").exists());
        assert!(!staging.is_complete(0).unwrap());
        assert_eq!(staging.stuck(), Vec::<u32>::new());

        // The sender sends it all again, and this time the hash is right.
        write_all();
        staging.verify(0, &blake3::hash(&content)).unwrap();
        assert_eq!(fs::read(staging.dir.join("f")).unwrap(), content);
    }

    #[test]
    fn a_file_that_is_checked_again_is_checked_against_the_new_hash() {
        // A resumed transfer sends an `End` for every file again, those that were whole before
        // included, and a file that was edited in between must not pass.
        let (_scratch, destination) = destination();
        let content = bytes(7, 100);
        let staging = staged(&destination, &[file("f", 100)]);
        staging.write_span(0, 0, &content).unwrap();
        staging.verify(0, &blake3::hash(&content)).unwrap();
        staging.verify(0, &blake3::hash(&content)).unwrap();
        assert!(matches!(
            staging.verify(0, &blake3::hash(&bytes(8, 100))),
            Err(StageError::Mismatch)
        ));
        assert!(!staging.dir.join("f").exists());
    }

    #[test]
    fn an_empty_file_is_made_when_it_is_checked() {
        let (_scratch, destination) = destination();
        let staging = staged(&destination, &[file("empty", 0), file("empty too", 0)]);
        // It has no span to write, so it is whole at once, and not on the disk yet.
        assert!(staging.is_complete(0).unwrap());
        assert!(!staging.dir.join("empty").exists());

        staging.verify(0, &blake3::hash(b"")).unwrap();
        assert_eq!(fs::metadata(staging.dir.join("empty")).unwrap().len(), 0);

        // And a sender that gives another hash for nothing is not believed.
        assert!(matches!(
            staging.verify(1, &blake3::hash(b"x")),
            Err(StageError::Mismatch)
        ));
        assert!(!staging.dir.join("empty too").exists());
    }

    #[test]
    fn a_span_has_exactly_the_bytes_the_list_says() {
        let (_scratch, destination) = destination();
        let staging = staged(
            &destination,
            &[folder("d"), file("d/f", 100), file("g", SPAN as u64 + 1)],
        );
        // The numbers are the positions in the list: 0 is the folder, 1 and 2 are the files.
        assert!(matches!(
            staging.write_span(1, 0, &[0; 99]),
            Err(StageError::Hash(HashError::WrongLength {
                expected: 100,
                got: 99,
                ..
            }))
        ));
        assert!(matches!(
            staging.write_span(1, 1, &[0; 100]),
            Err(StageError::Hash(HashError::NoSuchSpan { index: 1 }))
        ));
        assert!(matches!(
            staging.write_span(2, 0, &[0; 10]),
            Err(StageError::Hash(HashError::WrongLength { .. }))
        ));
        assert!(matches!(
            staging.write_span(0, 0, &[]),
            Err(StageError::NoSuchFile { file: 0 })
        ));
        assert!(matches!(
            staging.write_span(3, 0, &[0; 10]),
            Err(StageError::NoSuchFile { file: 3 })
        ));
        // None of that made a file.
        assert!(!staging.dir.join("d").join("f").exists());
        assert!(!staging.dir.join("g").exists());
    }

    #[test]
    fn a_file_that_is_not_whole_is_not_checked() {
        let (_scratch, destination) = destination();
        let staging = staged(&destination, &[file("f", SPAN as u64 + 1)]);
        assert!(matches!(
            staging.verify(0, &blake3::hash(b"x")),
            Err(StageError::Incomplete)
        ));
        staging.write_span(0, 0, &bytes(9, SPAN)).unwrap();
        assert!(matches!(
            staging.verify(0, &blake3::hash(b"x")),
            Err(StageError::Incomplete)
        ));
        // It is still there, and still waiting for the rest.
        assert!(staging.dir.join("f").exists());
    }

    #[test]
    fn a_file_takes_the_time_the_list_gave_it() {
        let (_scratch, destination) = destination();
        let content = bytes(10, 100);
        let staging = staged(&destination, &[file("f", 100), file("empty", 0)]);
        // The list is in the order of the format, so `empty` is number 0 and `f` is number 1.
        let (empty, whole) = (0, 1);
        staging.write_span(whole, 0, &content).unwrap();
        staging.verify(whole, &blake3::hash(&content)).unwrap();
        staging.verify(empty, &blake3::hash(b"")).unwrap();

        let wanted = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        for name in ["f", "empty"] {
            let modified = fs::metadata(staging.dir.join(name))
                .unwrap()
                .modified()
                .unwrap();
            assert_eq!(modified, wanted, "{name}");
        }
    }

    #[test]
    fn times_before_1970_and_beyond_what_a_system_can_hold_do_not_panic() {
        assert_eq!(
            system_time(-1_500),
            Some(SystemTime::UNIX_EPOCH - Duration::from_millis(1_500))
        );
        assert_eq!(
            system_time(1_500),
            Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_500))
        );
        // Whether such a time can be made is up to the system; the call must come back.
        let _ = system_time(i64::MIN);
        let _ = system_time(i64::MAX);
    }

    #[cfg(unix)]
    #[test]
    fn a_received_file_is_never_executable() {
        use std::os::unix::fs::PermissionsExt;
        let (_scratch, destination) = destination();
        let content = bytes(11, 100);
        let staging = staged(&destination, &[file("f", 100)]);
        staging.write_span(0, 0, &content).unwrap();
        staging.verify(0, &blake3::hash(&content)).unwrap();
        let mode = fs::metadata(staging.dir.join("f"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0, "{mode:o}");
    }

    #[test]
    fn only_a_few_files_are_open_at_once() {
        let (_scratch, destination) = destination();
        let size = SPAN + 10;
        let entries: Vec<_> = (0..6)
            .map(|n| file(&format!("f{n}"), size as u64))
            .collect();
        let contents: Vec<Vec<u8>> = (0..6_u8).map(|n| bytes(20 + n, size)).collect();
        let mut staging = staged(&destination, &entries);
        staging.max_open = 2;

        // First span of every file, then the second: the files are closed in between and opened
        // again, and the spans of a file go where they belong.
        for part in 0..2_usize {
            for (n, content) in contents.iter().enumerate() {
                let range = if part == 0 { 0..SPAN } else { SPAN..size };
                staging
                    .write_span(n32(n), n32(part), &content[range])
                    .unwrap();
                assert!(lock(&staging.state).open.len() <= 2);
            }
        }
        for (n, content) in contents.iter().enumerate() {
            staging.verify(n32(n), &blake3::hash(content)).unwrap();
            assert_eq!(
                fs::read(staging.dir.join(format!("f{n}"))).unwrap(),
                *content
            );
        }
        assert!(
            lock(&staging.state).open.is_empty(),
            "a checked file is closed"
        );
    }

    #[test]
    fn a_name_that_is_taken_fails_the_file_and_is_left_alone() {
        let (_scratch, destination) = destination();
        let content = bytes(12, 100);
        let staging = staged(&destination, &[file("f", 100)]);
        // As if two names that the volume takes for one had both got through the plan.
        fs::write(staging.dir.join("f"), b"somebody else's").unwrap();

        for _ in 0..2 {
            assert!(matches!(
                staging.write_span(0, 0, &content),
                Err(StageError::Io(e)) if e.kind() == ErrorKind::AlreadyExists
            ));
        }
        // Dropping the file does not remove what is not ours.
        staging.drop_file(0).unwrap();
        assert_eq!(fs::read(staging.dir.join("f")).unwrap(), b"somebody else's");
    }

    #[test]
    fn a_dropped_file_is_removed_and_starts_again() {
        let (_scratch, destination) = destination();
        let size = SPAN + 10;
        let content = bytes(13, size);
        let staging = staged(&destination, &[file("f", size as u64)]);
        staging.write_span(0, 0, &content[..SPAN]).unwrap();
        assert!(staging.dir.join("f").exists());

        staging.drop_file(0).unwrap();
        assert!(!staging.dir.join("f").exists());
        assert!(!staging.is_complete(0).unwrap());
        // Dropping what is not there is nothing.
        staging.drop_file(0).unwrap();

        staging.write_span(0, 0, &content[..SPAN]).unwrap();
        staging.write_span(0, 1, &content[SPAN..]).unwrap();
        staging.verify(0, &blake3::hash(&content)).unwrap();
        assert_eq!(fs::read(staging.dir.join("f")).unwrap(), content);
    }

    #[test]
    fn a_span_that_ends_after_its_file_was_dropped_is_not_kept() {
        let (_scratch, destination) = destination();
        let size = SPAN + 10;
        let content = bytes(14, size);
        let staging = staged(&destination, &[file("f", size as u64)]);
        staging.write_span(0, 0, &content[..SPAN]).unwrap();
        // The lane is in the middle of span 1 when the file is dropped.
        lock(&staging.state)
            .slots
            .get_mut(&0)
            .unwrap()
            .pending
            .push(1);
        staging.drop_file(0).unwrap();
        assert!(!staging.release(0, 1, Some([0; 32])));
        assert!(!staging.is_complete(0).unwrap());
    }

    #[test]
    fn the_transfer_is_removed_with_what_it_holds() {
        let (_scratch, destination) = destination();
        let content = bytes(15, 100);
        let staging = staged(&destination, &[folder("d"), file("d/f", 100)]);
        staging.write_span(1, 0, &content).unwrap();
        staging.discard().unwrap();
        assert!(!staging.dir.exists());
        // A lane that is late gets an error and makes nothing.
        assert!(staging.write_span(1, 0, &content).is_err());
        assert!(!staging.dir.exists());
    }

    #[test]
    fn a_transfer_id_that_is_in_use_is_not_started_twice_and_the_first_is_not_touched() {
        let (_scratch, destination) = destination();
        let entries = [folder("d"), file("d/f", 100)];
        let first = staged(&destination, &entries);
        first.write_span(1, 0, &bytes(16, 100)).unwrap();
        assert!(matches!(
            destination.stage(id(), &listing(&entries)),
            Err(StageError::Io(e)) if e.kind() == ErrorKind::AlreadyExists
        ));
        assert!(first.dir.join("d").join("f").exists());
    }

    #[test]
    fn a_folder_that_is_there_already_is_not_joined() {
        let (_scratch, destination) = destination();
        let staging = staged(
            &destination,
            &[folder("d"), folder("d/e"), file("d/e/f", 1)],
        );
        staging.write_span(2, 0, b"x").unwrap();
        // As if two folders that the volume takes for one had both got through the plan: the
        // second would be the first, with what is inside both of them in it.
        assert!(matches!(
            staging.make_folders(),
            Err(StageError::Io(e)) if e.kind() == ErrorKind::AlreadyExists
        ));
        assert_eq!(
            fs::read(staging.dir.join("d").join("e").join("f")).unwrap(),
            b"x"
        );
    }

    #[test]
    fn names_are_the_planned_ones() {
        let (_scratch, destination) = destination();
        let staging = staged(&destination, &[file("Notes.txt", 1), file("notes.txt", 1)]);
        staging.write_span(0, 0, b"a").unwrap();
        staging.write_span(1, 0, b"b").unwrap();
        // The list puts `Notes.txt` first, because its bytes sort first. Where the volume takes
        // the two for one name, the second has a number.
        assert_eq!(fs::read(staging.dir.join("Notes.txt")).unwrap(), b"a");
        let second = if destination.rules().ignores_case {
            "notes 2.txt"
        } else {
            "notes.txt"
        };
        assert_eq!(fs::read(staging.dir.join(second)).unwrap(), b"b");
    }

    #[test]
    fn the_errors_and_the_debug_form_have_no_names_or_folders_in_them() {
        let (scratch, destination) = destination();
        let staging = staged(&destination, &[file("secret name.txt", 10)]);
        fs::write(staging.dir.join("secret name.txt"), b"taken").unwrap();
        let error = staging.write_span(0, 0, &[0; 10]).unwrap_err();
        for shown in [
            format!("{error}"),
            format!("{error:?}"),
            format!("{staging:?}"),
        ] {
            assert!(!shown.contains("secret"), "{shown}");
            assert!(!shown.contains(scratch.path().to_str().unwrap()), "{shown}");
        }
    }

    #[test]
    fn a_full_disk_is_no_space_and_other_errors_are_not() {
        assert!(matches!(
            StageError::from(io::Error::from(ErrorKind::StorageFull)),
            StageError::NoSpace
        ));
        assert!(matches!(
            StageError::from(io::Error::from(ErrorKind::QuotaExceeded)),
            StageError::NoSpace
        ));
        assert!(matches!(
            StageError::from(io::Error::from(ErrorKind::PermissionDenied)),
            StageError::Io(_)
        ));
    }

    proptest! {
        // A few cases only: each writes a few megabytes.
        #![proptest_config(ProptestConfig::with_cases(12))]

        #[test]
        fn any_files_in_any_order_come_out_whole(
            sizes in prop::collection::vec(
                prop_oneof![
                    Just(0_usize),
                    Just(1),
                    Just(SPAN - 1),
                    Just(SPAN),
                    Just(SPAN + 1),
                    0..(2 * SPAN + 1000),
                ],
                1..5,
            ),
            seed in any::<u64>(),
        ) {
            let (_scratch, destination) = destination();
            let entries: Vec<_> = sizes
                .iter()
                .enumerate()
                .map(|(n, size)| file(&format!("f{n}"), *size as u64))
                .collect();
            let contents: Vec<Vec<u8>> = sizes
                .iter()
                .enumerate()
                .map(|(n, size)| bytes(u8::try_from(n).unwrap(), *size))
                .collect();
            let staging = staged(&destination, &entries);

            // Every span of every file, in an order that the seed makes up.
            let mut order: Vec<(usize, usize)> = contents
                .iter()
                .enumerate()
                .flat_map(|(n, content)| (0..content.len().div_ceil(SPAN)).map(move |span| (n, span)))
                .collect();
            order.sort_by_key(|(n, span)| {
                (seed ^ ((*n as u64) << 32 | *span as u64)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            });
            for (n, span) in order {
                let content = &contents[n];
                let end = ((span + 1) * SPAN).min(content.len());
                staging
                    .write_span(n32(n), n32(span), &content[span * SPAN..end])
                    .unwrap();
            }
            for (n, content) in contents.iter().enumerate() {
                prop_assert!(staging.is_complete(n32(n)).unwrap());
                staging.verify(n32(n), &blake3::hash(content)).unwrap();
                prop_assert_eq!(&fs::read(staging.dir.join(format!("f{n}"))).unwrap(), content);
            }
        }
    }
}
