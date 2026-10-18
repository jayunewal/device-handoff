// SPDX-License-Identifier: Apache-2.0

//! The files and folders a person chose to send, walked into the list that the offer carries.
//!
//! The sender looks at everything before it offers anything (ADR 0008). The person at the other
//! end is asked about the summary and the first names of an offer, and the list that follows has
//! to be exactly that, so the walk is finished, with every limit met, before the offer is made. A
//! tree that is too large is an error here and not a failure halfway through a transfer.
//!
//! What the walk does with what it finds:
//!
//! - **What was chosen is followed if it is a link, and what is inside a chosen folder is not.**
//!   The person chose the item, so it is sent, under the name it was chosen by. A link inside a
//!   folder is left out, so that a folder cannot send what it points to (`~/.ssh`, say), and so
//!   are devices, sockets and the other things that are not files or folders. The hidden folders
//!   in which Baton keeps what it is receiving are not sent either, and are not counted as left
//!   out, since a person never sees them.
//! - **Nothing is left out without being counted.** A name that is not text or that is too long,
//!   something that is too deep, something that cannot be read, and a file larger than a transfer
//!   may hold are left out like a link, and each is counted by its reason in [`LeftOut`], so that
//!   a person can be told before anything is sent. A folder that cannot be read is left out with
//!   what is in it, and does not become an empty folder on the other device.
//! - **The list is in the order of the format.** Names are sorted by their bytes, name by name,
//!   with a folder directly before what is inside it ([`path::compare`]), whatever order the
//!   system lists them in. Two chosen items with the same name are told apart the way a receiver
//!   does it, with a number before the extension, and the one that was chosen first keeps its
//!   name.
//! - **The list says what the walk saw.** A file has the size and the modification time that the
//!   walk found. The sender compares both with the file before it reads it and after it has read
//!   it (ADR 0008), so every such reading goes through the same function.

use std::{
    collections::HashSet,
    ffi::OsStr,
    fmt, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use baton_proto::{
    id::TransferId,
    limits::{
        MAX_FILE_BYTES, MAX_LIST_BYTES, MAX_PREVIEW_ITEMS, MAX_TRANSFER_FILES, MAX_TRANSFER_FOLDERS,
    },
    transfer::{
        Offer, PreviewItem, Summary,
        list::{ListEntry, ListHash, ListHasher},
        path,
    },
};

use crate::transfer::{destination::STAGING_PREFIX, names::numbered};

/// Why a walk gave no list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    /// Nothing was left to send: nothing was chosen, or all of it was left out. The figures say
    /// why.
    #[error("there is nothing to send")]
    Empty(LeftOut),
    /// There is more than a transfer may have.
    #[error("there are more than {max} {what}")]
    TooMany {
        /// What there is too much of.
        what: &'static str,
        /// What a transfer may have.
        max: u32,
    },
    /// The names, sizes and times of the list take more room than a list may.
    #[error("the list of names is longer than a transfer may have")]
    ListTooLong,
    /// The files add up to more than a number of bytes can hold.
    #[error("the files add up to more than a transfer can hold")]
    TooBig,
}

/// What a walk left out of the list, by reason. A person is told before anything is sent, so that
/// nothing they expected to arrive is missing without their knowing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LeftOut {
    /// Links inside a chosen folder. They are not followed, so that a folder cannot send what it
    /// points to.
    pub links: u32,
    /// Devices, sockets, pipes and other things that are neither a file nor a folder.
    pub special: u32,
    /// Things with a name that cannot be sent: not text, too long, or too deep in the folders.
    pub names: u32,
    /// Things that could not be read.
    pub unreadable: u32,
    /// Files larger than a transfer may hold.
    pub too_large: u32,
}

impl LeftOut {
    /// How many things were left out, whatever the reason.
    pub fn total(&self) -> u32 {
        [
            self.links,
            self.special,
            self.names,
            self.unreadable,
            self.too_large,
        ]
        .into_iter()
        .fold(0, u32::saturating_add)
    }

    fn add(&mut self, reason: Reason) {
        let count = match reason {
            Reason::Link => &mut self.links,
            Reason::Special => &mut self.special,
            Reason::Name => &mut self.names,
            Reason::Unreadable => &mut self.unreadable,
            Reason::TooLarge => &mut self.too_large,
        };
        *count = count.saturating_add(1);
    }
}

#[derive(Debug, Clone, Copy)]
enum Reason {
    Link,
    Special,
    Name,
    Unreadable,
    TooLarge,
}

/// What a walk may find before it gives up. The tests walk with smaller limits than the protocol's.
#[derive(Debug, Clone, Copy)]
struct Limits {
    files: u32,
    folders: u32,
    /// Everything that a folder lists, whether it is sent or left out. A folder with a million
    /// links has no files to send and is still a million things to read, and the names of a
    /// folder are held in memory while it is walked.
    looked_at: u32,
    list_bytes: usize,
    file_bytes: u64,
}

impl Limits {
    const PROTOCOL: Self = Self {
        files: MAX_TRANSFER_FILES,
        folders: MAX_TRANSFER_FOLDERS,
        looked_at: 4 * (MAX_TRANSFER_FILES + MAX_TRANSFER_FOLDERS),
        list_bytes: MAX_LIST_BYTES,
        file_bytes: MAX_FILE_BYTES,
    };
}

/// The files and folders that a person chose, as the list of an offer.
///
/// It is made once, by [`Selection::walk`], and kept for the whole transfer: a transfer that is
/// resumed on a new connection offers the same list again, with the same hash.
pub struct Selection {
    entries: Vec<ListEntry>,
    /// Where each top-level item starts in `entries`, and where it is.
    items: Vec<(usize, PathBuf)>,
    summary: Summary,
    preview: Vec<PreviewItem>,
    list_hash: ListHash,
    left_out: LeftOut,
}

impl fmt::Debug for Selection {
    // What people call their files is theirs, and a log gets pasted into a bug report.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Selection")
            .field("summary", &self.summary)
            .field("left_out", &self.left_out)
            .finish_non_exhaustive()
    }
}

impl Selection {
    /// Walks everything in `chosen`, which is read from disk: a folder is read in full, so call
    /// this on a thread meant for blocking work.
    ///
    /// The items of the list are sorted by name, whatever order they were chosen in.
    pub fn walk(chosen: &[PathBuf]) -> Result<Self, SelectionError> {
        Self::walk_within(chosen, Limits::PROTOCOL)
    }

    fn walk_within(chosen: &[PathBuf], limits: Limits) -> Result<Self, SelectionError> {
        let mut left_out = LeftOut::default();
        let mut roots = Vec::with_capacity(chosen.len());
        for path in chosen {
            match Root::inspect(path) {
                Ok(root) => roots.push(root),
                Err(reason) => left_out.add(reason),
            }
        }
        let roots = name_roots(roots, &mut left_out);
        let mut walker = Walker::new(limits, left_out);
        let mut items = Vec::with_capacity(roots.len());
        for root in roots {
            let start = walker.entries.len();
            walker.root(&root)?;
            // An item that was left out, or a folder that could not be read, adds nothing.
            if walker.entries.len() > start {
                items.push((start, root.real));
            }
        }
        walker.finish(items)
    }

    /// The offer of this list under the transfer id `id`.
    pub fn offer(&self, id: TransferId) -> Offer {
        Offer {
            id,
            summary: self.summary,
            preview: self.preview.clone(),
            list_hash: self.list_hash,
        }
    }

    /// How much there is.
    pub fn summary(&self) -> Summary {
        self.summary
    }

    /// What was left out and why.
    pub fn left_out(&self) -> LeftOut {
        self.left_out
    }

    /// Every entry of the list, in order. The position of an entry is the number of the file
    /// that lane frames refer to.
    pub fn entries(&self) -> &[ListEntry] {
        &self.entries
    }

    /// File number `index`, or `None` if there is no such entry or it is a folder.
    pub fn file(&self, index: u32) -> Option<&ListEntry> {
        self.entries
            .get(usize::try_from(index).ok()?)
            .filter(|entry| !entry.folder)
    }

    /// Where entry number `index` is on this device, or `None` if there is no such entry.
    ///
    /// It is the folder or file that was chosen with the names inside it added, so a link that
    /// was chosen is already resolved. What is at the place now may not be what the walk saw, and
    /// whoever reads it compares size and time with the entry first.
    pub fn source(&self, index: u32) -> Option<PathBuf> {
        let index = usize::try_from(index).ok()?;
        let entry = self.entries.get(index)?;
        let slot = self
            .items
            .partition_point(|(start, _)| *start <= index)
            .checked_sub(1)?;
        let (_, mut path) = self.items.get(slot)?.clone();
        // The first name of a path is the name of the item, which may be a number away from what
        // it is called here.
        for name in entry.path.split('/').skip(1) {
            path.push(name);
        }
        Some(path)
    }
}

/// When a file was last changed, in milliseconds since 1970, which is what a list says.
///
/// Every comparison of a file with its entry reads the time through this, so that they agree on
/// what the same time is. A time before 1970 is negative, and is cut toward 0 to a millisecond.
/// `None` is for a system that gives no time.
pub(crate) fn modified_ms(metadata: &fs::Metadata) -> Option<i64> {
    millis_since_epoch(metadata.modified().ok()?)
}

fn millis_since_epoch(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_millis()).ok(),
        Err(before) => i64::try_from(before.duration().as_millis())
            .ok()
            .map(|millis| -millis),
    }
}

/// An item that was chosen.
struct Root {
    /// What it is called in the list: its own name, and a number added if another item has it.
    name: String,
    /// The place it is at, with every link followed.
    real: PathBuf,
    folder: bool,
    size: u64,
    modified_ms: Option<i64>,
}

impl Root {
    fn inspect(chosen: &Path) -> Result<Self, Reason> {
        // The name is the one it was chosen by, also when it is a link to something else.
        let name = chosen
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or(Reason::Name)?;
        path::check_name(name).map_err(|_| Reason::Name)?;
        let real = fs::canonicalize(chosen).map_err(|_| Reason::Unreadable)?;
        let metadata = fs::metadata(&real).map_err(|_| Reason::Unreadable)?;
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(Reason::Special);
        }
        let folder = metadata.is_dir();
        Ok(Self {
            name: name.to_owned(),
            real,
            folder,
            size: if folder { 0 } else { metadata.len() },
            modified_ms: if folder { None } else { modified_ms(&metadata) },
        })
    }
}

/// Chooses the names of the chosen items, and puts them in the order of the list.
///
/// The same item chosen twice is one item. Items with the same name from different places are
/// told apart as a receiver tells names apart: the one chosen first keeps its name and each later
/// one gets the first number that no other item has or is going to have, before its extension.
fn name_roots(roots: Vec<Root>, left_out: &mut LeftOut) -> Vec<Root> {
    let mut seen = HashSet::new();
    let roots: Vec<Root> = roots
        .into_iter()
        .filter(|root| seen.insert((root.name.clone(), root.real.clone())))
        .collect();
    let reserved: HashSet<String> = roots.iter().map(|root| root.name.clone()).collect();
    let mut used = HashSet::new();
    let mut named = Vec::with_capacity(roots.len());
    for mut root in roots {
        if used.contains(&root.name) {
            let free = (2..=u32::MAX)
                .map(|number| numbered(&root.name, number))
                .find(|name| !used.contains(name) && !reserved.contains(name));
            let Some(name) = free else {
                left_out.add(Reason::Name);
                continue;
            };
            root.name = name;
        }
        used.insert(root.name.clone());
        named.push(root);
    }
    named.sort_by(|a, b| a.name.cmp(&b.name));
    named
}

/// The path of an entry that is called `name` inside the folder `folder`, if it can be sent.
fn child_path(folder: &str, name: &str) -> Option<String> {
    let path = format!("{folder}/{name}");
    path::check_path(&path).ok()?;
    Some(path)
}

/// A file or a folder inside a folder that is being walked.
struct Child {
    name: String,
    entry: fs::DirEntry,
    folder: bool,
}

struct Walker {
    limits: Limits,
    entries: Vec<ListEntry>,
    hasher: ListHasher,
    files: u32,
    folders: u32,
    bytes: u64,
    looked_at: u32,
    left_out: LeftOut,
}

impl Walker {
    fn new(limits: Limits, left_out: LeftOut) -> Self {
        Self {
            limits,
            entries: Vec::new(),
            hasher: ListHasher::new(),
            files: 0,
            folders: 0,
            bytes: 0,
            looked_at: 0,
            left_out,
        }
    }

    fn finish(self, items: Vec<(usize, PathBuf)>) -> Result<Selection, SelectionError> {
        if self.entries.is_empty() {
            return Err(SelectionError::Empty(self.left_out));
        }
        let summary = Summary {
            items: u32::try_from(items.len()).unwrap_or(u32::MAX),
            files: self.files,
            folders: self.folders,
            bytes: self.bytes,
        };
        Ok(Selection {
            preview: preview(&self.entries, &items),
            list_hash: self.hasher.finalize(),
            entries: self.entries,
            items,
            summary,
            left_out: self.left_out,
        })
    }

    fn root(&mut self, root: &Root) -> Result<(), SelectionError> {
        if root.folder {
            self.folder(&root.real, &root.name)
        } else {
            self.file(root.name.clone(), root.size, root.modified_ms)
        }
    }

    /// Adds the folder `dir`, which is called `path` in the list, and what is in it.
    fn folder(&mut self, dir: &Path, path: &str) -> Result<(), SelectionError> {
        let Some(children) = self.children(dir)? else {
            self.left_out.add(Reason::Unreadable);
            return Ok(());
        };
        if self.folders >= self.limits.folders {
            return Err(SelectionError::TooMany {
                what: "folders",
                max: self.limits.folders,
            });
        }
        self.folders += 1;
        self.push(ListEntry {
            path: path.to_owned(),
            folder: true,
            size: 0,
            modified_ms: None,
        })?;
        for child in children {
            let Some(child_path) = child_path(path, &child.name) else {
                self.left_out.add(Reason::Name);
                continue;
            };
            if child.folder {
                self.folder(&child.entry.path(), &child_path)?;
            } else {
                match child.entry.metadata() {
                    Ok(metadata) if metadata.is_file() => {
                        self.file(child_path, metadata.len(), modified_ms(&metadata))?;
                    }
                    Ok(_) => self.left_out.add(Reason::Special),
                    Err(_) => self.left_out.add(Reason::Unreadable),
                }
            }
        }
        Ok(())
    }

    /// The files and folders in `dir` that are to be sent, in the order of their names, or `None`
    /// if it cannot be read.
    fn children(&mut self, dir: &Path) -> Result<Option<Vec<Child>>, SelectionError> {
        let Ok(listing) = fs::read_dir(dir) else {
            return Ok(None);
        };
        let mut children = Vec::new();
        for item in listing {
            self.looked_at += 1;
            if self.looked_at > self.limits.looked_at {
                return Err(SelectionError::TooMany {
                    what: "things in the folders to look at",
                    max: self.limits.looked_at,
                });
            }
            let Ok(entry) = item else {
                self.left_out.add(Reason::Unreadable);
                continue;
            };
            let Ok(name) = entry.file_name().into_string() else {
                self.left_out.add(Reason::Name);
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                self.left_out.add(Reason::Unreadable);
                continue;
            };
            if kind.is_symlink() {
                self.left_out.add(Reason::Link);
            } else if kind.is_dir() {
                // What Baton is receiving is not in a state to be sent, and a person never sees
                // the folder it is in.
                if !name.starts_with(STAGING_PREFIX) {
                    children.push(Child {
                        name,
                        entry,
                        folder: true,
                    });
                }
            } else if kind.is_file() {
                children.push(Child {
                    name,
                    entry,
                    folder: false,
                });
            } else {
                self.left_out.add(Reason::Special);
            }
        }
        children.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        Ok(Some(children))
    }

    fn file(
        &mut self,
        path: String,
        size: u64,
        modified_ms: Option<i64>,
    ) -> Result<(), SelectionError> {
        if size > self.limits.file_bytes {
            self.left_out.add(Reason::TooLarge);
            return Ok(());
        }
        if self.files >= self.limits.files {
            return Err(SelectionError::TooMany {
                what: "files",
                max: self.limits.files,
            });
        }
        self.bytes = self.bytes.checked_add(size).ok_or(SelectionError::TooBig)?;
        self.files += 1;
        self.push(ListEntry {
            path,
            folder: false,
            size,
            modified_ms,
        })
    }

    fn push(&mut self, entry: ListEntry) -> Result<(), SelectionError> {
        self.hasher.update(&entry);
        if self.hasher.bytes() > self.limits.list_bytes {
            return Err(SelectionError::ListTooLong);
        }
        self.entries.push(entry);
        Ok(())
    }
}

/// The first top-level items as an offer shows them: for a folder, how many files are in it, in
/// every folder inside it, and how many bytes they add up to.
fn preview(entries: &[ListEntry], items: &[(usize, PathBuf)]) -> Vec<PreviewItem> {
    let mut shown = Vec::new();
    for (slot, (start, _)) in items.iter().enumerate().take(MAX_PREVIEW_ITEMS) {
        let end = items.get(slot + 1).map_or(entries.len(), |(next, _)| *next);
        let Some((item, inside)) = entries.get(*start..end).and_then(<[_]>::split_first) else {
            continue;
        };
        let files = inside.iter().filter(|entry| !entry.folder);
        shown.push(PreviewItem {
            name: item.path.clone(),
            folder: item.folder,
            files: u32::try_from(files.clone().count()).unwrap_or(u32::MAX),
            bytes: if item.folder {
                files.fold(0, |sum, entry| u64::saturating_add(sum, entry.size))
            } else {
                item.size
            },
        });
    }
    shown
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use baton_proto::transfer::list::{ListChecker, ListChunk, Listing};

    use super::*;
    use crate::transfer::testing::offer_for;

    /// The time that `make` gives a file: a whole number of seconds, since some volumes keep no
    /// more.
    const MADE: i64 = 1_700_000_000_000;

    fn made() -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(u64::try_from(MADE).unwrap())
    }

    /// A file of `size` bytes at `relative` inside `root`, and the folders above it.
    fn make(root: &Path, relative: &str, size: usize) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'x'; size]).unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(made())
            .unwrap();
        path
    }

    fn make_folder(root: &Path, relative: &str) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn walk(chosen: &[&Path]) -> Selection {
        let chosen: Vec<PathBuf> = chosen.iter().map(|path| path.to_path_buf()).collect();
        Selection::walk(&chosen).unwrap()
    }

    fn paths(selection: &Selection) -> Vec<&str> {
        selection
            .entries()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect()
    }

    fn id() -> TransferId {
        TransferId::from_bytes([9; TransferId::LEN])
    }

    /// The list as a receiver takes it in, cut into pieces of `piece` entries.
    fn received(selection: &Selection, piece: usize) -> Listing {
        let mut checker = ListChecker::new(&selection.offer(id())).unwrap();
        for chunk in selection.entries().chunks(piece) {
            checker
                .push(ListChunk {
                    entries: chunk.to_vec(),
                })
                .unwrap();
        }
        checker.finish().unwrap()
    }

    /// The tree that most tests walk, with names that tell a byte order from a text order and a
    /// folder from the names around it.
    fn holiday(root: &Path) -> Vec<PathBuf> {
        make(root, "Holiday/day 1/a.jpg", 3_000);
        make(root, "Holiday/day 1/b.jpg", 4_000);
        make_folder(root, "Holiday/empty");
        make(root, "Holiday/notes.txt", 20);
        make(root, "a/x", 1);
        make(root, "a-b", 2);
        make(root, "report.pdf", 100_000);
        make(root, "zoo/z.txt", 1);
        // Chosen in no order at all.
        ["zoo", "report.pdf", "Holiday", "a-b", "a"]
            .iter()
            .map(|name| root.join(name))
            .collect()
    }

    #[test]
    fn a_walk_gives_the_list_in_the_order_of_the_format() {
        let scratch = scratch();
        let chosen = holiday(scratch.path());
        let selection = Selection::walk(&chosen).unwrap();
        assert_eq!(
            paths(&selection),
            [
                "Holiday",
                "Holiday/day 1",
                "Holiday/day 1/a.jpg",
                "Holiday/day 1/b.jpg",
                "Holiday/empty",
                "Holiday/notes.txt",
                "a",
                "a/x",
                "a-b",
                "report.pdf",
                "zoo",
                "zoo/z.txt",
            ]
        );
        for pair in selection.entries().windows(2) {
            assert!(
                path::compare(&pair[0].path, &pair[1].path).is_lt(),
                "{pair:?}"
            );
        }
    }

    #[test]
    fn the_list_is_what_a_receiver_accepts_however_it_is_cut_up() {
        let scratch = scratch();
        let selection = Selection::walk(&holiday(scratch.path())).unwrap();
        for piece in [1, 2, 5, 100] {
            assert_eq!(received(&selection, piece).entries(), selection.entries());
        }
    }

    #[test]
    fn the_entries_say_what_was_seen() {
        let scratch = scratch();
        let selection = Selection::walk(&holiday(scratch.path())).unwrap();
        let by_path = |path: &str| {
            selection
                .entries()
                .iter()
                .find(|entry| entry.path == path)
                .unwrap()
        };
        let notes = by_path("Holiday/notes.txt");
        assert!(!notes.folder && notes.size == 20 && notes.modified_ms == Some(MADE));
        let day = by_path("Holiday/day 1");
        assert!(day.folder && day.size == 0 && day.modified_ms.is_none());
        let empty = by_path("Holiday/empty");
        assert!(empty.folder && empty.size == 0 && empty.modified_ms.is_none());
        assert_eq!(by_path("report.pdf").size, 100_000);
    }

    #[test]
    fn the_summary_and_the_preview_are_those_of_the_list() {
        let scratch = scratch();
        let selection = Selection::walk(&holiday(scratch.path())).unwrap();
        assert_eq!(
            selection.summary(),
            Summary {
                items: 5,
                files: 7,
                folders: 5,
                bytes: 3_000 + 4_000 + 20 + 1 + 2 + 100_000 + 1,
            }
        );
        // An offer worked out from the list in another way.
        assert_eq!(selection.offer(id()), offer_for(selection.entries()));
        let preview = selection.offer(id()).preview;
        assert_eq!(preview.len(), 5);
        assert_eq!((preview[0].folder, preview[0].files), (true, 3));
        assert_eq!(preview[0].bytes, 7_020);
        assert_eq!((preview[3].folder, preview[3].files), (false, 0));
        assert_eq!(preview[3].bytes, 100_000);
    }

    #[test]
    fn the_preview_shows_the_first_sixteen_items() {
        let scratch = scratch();
        let chosen: Vec<PathBuf> = (0..20)
            .map(|n| make(scratch.path(), &format!("f{n:02}"), n + 1))
            .collect();
        let selection = Selection::walk(&chosen).unwrap();
        let offer = selection.offer(id());
        assert_eq!(offer.summary.items, 20);
        assert_eq!(offer.preview.len(), 16);
        assert_eq!(offer.preview[15].name, "f15");
        assert_eq!(offer.preview[15].bytes, 16);
        assert_eq!(offer, offer_for(selection.entries()));
        received(&selection, 7);
    }

    #[test]
    fn an_empty_folder_is_a_folder() {
        let scratch = scratch();
        let empty = make_folder(scratch.path(), "nothing");
        let selection = walk(&[&empty]);
        assert_eq!(paths(&selection), ["nothing"]);
        assert_eq!(
            selection.summary(),
            Summary {
                items: 1,
                files: 0,
                folders: 1,
                bytes: 0
            }
        );
        received(&selection, 1);
    }

    #[test]
    fn a_walk_of_an_unchanged_tree_gives_the_same_hash_and_a_changed_file_another() {
        let scratch = scratch();
        let chosen = holiday(scratch.path());
        let before = Selection::walk(&chosen).unwrap().offer(id()).list_hash;
        assert_eq!(
            Selection::walk(&chosen).unwrap().offer(id()).list_hash,
            before
        );

        let notes = scratch.path().join("Holiday/notes.txt");
        fs::File::options()
            .write(true)
            .open(&notes)
            .unwrap()
            .set_modified(made() + Duration::from_secs(60))
            .unwrap();
        assert_ne!(
            Selection::walk(&chosen).unwrap().offer(id()).list_hash,
            before
        );

        make(scratch.path(), "Holiday/notes.txt", 21);
        assert_ne!(
            Selection::walk(&chosen).unwrap().offer(id()).list_hash,
            before
        );
    }

    #[test]
    fn every_entry_is_found_where_it_was_walked() {
        let scratch = scratch();
        let chosen = holiday(scratch.path());
        let selection = Selection::walk(&chosen).unwrap();
        for (index, entry) in selection.entries().iter().enumerate() {
            let index = u32::try_from(index).unwrap();
            let source = selection.source(index).unwrap();
            assert_eq!(
                source.canonicalize().unwrap(),
                scratch.path().join(&entry.path).canonicalize().unwrap(),
                "{entry:?}"
            );
            assert_eq!(selection.file(index).is_some(), !entry.folder);
        }
        let past_the_end = u32::try_from(selection.entries().len()).unwrap();
        assert!(selection.source(past_the_end).is_none());
        assert!(selection.file(past_the_end).is_none());
    }

    #[test]
    fn the_staging_folders_of_baton_are_not_sent_and_not_counted() {
        let scratch = scratch();
        make(scratch.path(), "Downloads/a.txt", 3);
        make(
            scratch.path(),
            &format!("Downloads/{STAGING_PREFIX}0123abcd/half-a-file"),
            9,
        );
        let selection = walk(&[&scratch.path().join("Downloads")]);
        assert_eq!(paths(&selection), ["Downloads", "Downloads/a.txt"]);
        assert_eq!(selection.left_out(), LeftOut::default());

        // A person who chose one of them gets what they chose.
        let chosen = scratch
            .path()
            .join(format!("Downloads/{STAGING_PREFIX}0123abcd"));
        assert_eq!(walk(&[&chosen]).summary().files, 1);
    }

    #[test]
    fn names_that_were_chosen_twice_are_told_apart_and_the_first_keeps_its_name() {
        let scratch = scratch();
        let chosen = [
            make(scratch.path(), "b/x.txt", 1),
            make(scratch.path(), "a/x.txt", 2),
            make(scratch.path(), "a/y/x.txt", 3),
            // Already has the name that the second would get.
            make(scratch.path(), "c/x 2.txt", 4),
        ];
        let selection = Selection::walk(&chosen).unwrap();
        let sizes: Vec<(&str, u64)> = selection
            .entries()
            .iter()
            .map(|entry| (entry.path.as_str(), entry.size))
            .collect();
        assert_eq!(
            sizes,
            [("x 2.txt", 4), ("x 3.txt", 2), ("x 4.txt", 3), ("x.txt", 1)]
        );
        // Each is read from the place it came from, under the name it has in the list.
        for (index, size) in [(0, 4), (1, 2), (2, 3), (3, 1)] {
            let source = selection.source(index).unwrap();
            assert_eq!(fs::metadata(source).unwrap().len(), size);
        }
        received(&selection, 2);
    }

    #[test]
    fn a_folder_that_is_numbered_takes_its_contents_with_it() {
        let scratch = scratch();
        make(scratch.path(), "one/photos/a.jpg", 1);
        make(scratch.path(), "two/photos/b.jpg", 2);
        let selection = walk(&[
            &scratch.path().join("one/photos"),
            &scratch.path().join("two/photos"),
        ]);
        // What is inside a folder follows it, also when another folder's name starts the same.
        assert_eq!(
            paths(&selection),
            ["photos", "photos/a.jpg", "photos 2", "photos 2/b.jpg"]
        );
        assert_eq!(
            selection.source(3).unwrap(),
            scratch
                .path()
                .join("two/photos/b.jpg")
                .canonicalize()
                .unwrap()
        );
        received(&selection, 3);
    }

    #[test]
    fn the_same_item_chosen_twice_is_sent_once() {
        let scratch = scratch();
        let file = make(scratch.path(), "d/x.txt", 5);
        let again = scratch.path().join("d/../d/x.txt");
        let selection = walk(&[&file, &again, &file]);
        assert_eq!(paths(&selection), ["x.txt"]);
        assert_eq!(selection.summary().items, 1);
    }

    #[test]
    fn what_is_chosen_and_cannot_be_sent_is_left_out_and_counted() {
        let scratch = scratch();
        let file = make(scratch.path(), "ok.txt", 1);
        let missing = scratch.path().join("gone.txt");
        let nameless = Path::new("/");
        let selection = walk(&[&missing, &file, nameless]);
        assert_eq!(paths(&selection), ["ok.txt"]);
        assert_eq!(
            selection.left_out(),
            LeftOut {
                unreadable: 1,
                names: 1,
                ..LeftOut::default()
            }
        );
        assert_eq!(selection.left_out().total(), 2);
    }

    #[test]
    fn nothing_to_send_says_why() {
        let scratch = scratch();
        let missing = scratch.path().join("gone.txt");
        assert_eq!(
            Selection::walk(&[missing]).unwrap_err(),
            SelectionError::Empty(LeftOut {
                unreadable: 1,
                ..LeftOut::default()
            })
        );
        assert_eq!(
            Selection::walk(&[]).unwrap_err(),
            SelectionError::Empty(LeftOut::default())
        );
    }

    #[test]
    fn a_folder_that_cannot_be_read_is_left_out_whole() {
        let scratch = scratch();
        let mut walker = Walker::new(Limits::PROTOCOL, LeftOut::default());
        walker
            .folder(&scratch.path().join("not there"), "not there")
            .unwrap();
        assert!(walker.entries.is_empty() && walker.folders == 0);
        assert_eq!(walker.left_out.unreadable, 1);
    }

    #[test]
    fn a_folder_deeper_than_a_path_may_be_is_left_out() {
        let scratch = scratch();
        let deep = vec!["d"; 70].join("/");
        make_folder(scratch.path(), &deep);
        let selection = walk(&[&scratch.path().join("d")]);
        assert_eq!(selection.entries().len(), 64);
        assert_eq!(
            selection.entries().last().unwrap().path.split('/').count(),
            64
        );
        assert_eq!(selection.left_out().names, 1);
        received(&selection, 10);
    }

    #[test]
    fn a_name_is_sent_only_if_a_receiver_takes_its_path() {
        assert_eq!(child_path("a/b", "c.txt").as_deref(), Some("a/b/c.txt"));
        let long = "n".repeat(255);
        assert!(child_path("a", &long).is_some());
        assert!(child_path("a", &format!("{long}n")).is_none());
        // A path of 1024 bytes is the longest, and the name that makes it longer is left out.
        let folder = [
            "f".repeat(200),
            "f".repeat(200),
            "f".repeat(200),
            "f".repeat(200),
        ]
        .join("/");
        assert!(child_path(&folder, &"n".repeat(1024 - folder.len() - 1)).is_some());
        assert!(child_path(&folder, &"n".repeat(1024 - folder.len())).is_none());
        let deep = vec!["d"; 63].join("/");
        assert!(child_path(&deep, "d").is_some());
        assert!(child_path(&format!("{deep}/d"), "d").is_none());
        assert!(child_path("a", "b\0c").is_none());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_name_that_is_not_text_is_left_out() {
        use std::os::unix::ffi::OsStrExt;

        let scratch = scratch();
        make(scratch.path(), "d/fine.txt", 1);
        let odd = scratch
            .path()
            .join("d")
            .join(OsStr::from_bytes(b"caf\xe9.txt"));
        fs::write(&odd, "x").unwrap();
        let selection = walk(&[&scratch.path().join("d")]);
        assert_eq!(paths(&selection), ["d", "d/fine.txt"]);
        assert_eq!(selection.left_out().names, 1);
        // Chosen by name, it cannot be sent either.
        assert_eq!(
            Selection::walk(&[odd]).unwrap_err(),
            SelectionError::Empty(LeftOut {
                names: 1,
                ..LeftOut::default()
            })
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_link_inside_a_folder_is_left_out_and_not_followed() {
        use std::os::unix::fs::symlink;

        let scratch = scratch();
        let outside = make(scratch.path(), "outside/secret.txt", 7);
        make(scratch.path(), "d/real.txt", 3);
        symlink("real.txt", scratch.path().join("d/to-file")).unwrap();
        symlink(
            outside.parent().unwrap(),
            scratch.path().join("d/to-folder"),
        )
        .unwrap();
        symlink("nowhere", scratch.path().join("d/dangling")).unwrap();
        let selection = walk(&[&scratch.path().join("d")]);
        assert_eq!(paths(&selection), ["d", "d/real.txt"]);
        assert_eq!(selection.left_out().links, 3);
        assert_eq!(selection.left_out().total(), 3);
        assert_eq!(selection.summary().bytes, 3);
    }

    #[test]
    #[cfg(unix)]
    fn a_link_that_was_chosen_is_followed_and_keeps_its_name() {
        use std::os::unix::fs::symlink;

        let scratch = scratch();
        make(scratch.path(), "data/final.pdf", 11);
        make(scratch.path(), "stuff/inner.txt", 2);
        symlink("data/final.pdf", scratch.path().join("report")).unwrap();
        symlink("stuff", scratch.path().join("things")).unwrap();
        let selection = walk(&[
            &scratch.path().join("report"),
            &scratch.path().join("things"),
        ]);
        assert_eq!(paths(&selection), ["report", "things", "things/inner.txt"]);
        assert_eq!(selection.entries()[0].size, 11);
        assert_eq!(selection.left_out(), LeftOut::default());
        assert_eq!(
            selection.source(0).unwrap(),
            scratch
                .path()
                .join("data/final.pdf")
                .canonicalize()
                .unwrap()
        );
        assert_eq!(
            selection.source(2).unwrap(),
            scratch
                .path()
                .join("stuff/inner.txt")
                .canonicalize()
                .unwrap()
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_socket_is_not_sent() {
        let scratch = scratch();
        make(scratch.path(), "d/a.txt", 1);
        let socket = std::os::unix::net::UnixListener::bind(scratch.path().join("d/s")).unwrap();
        let selection = walk(&[&scratch.path().join("d")]);
        assert_eq!(paths(&selection), ["d", "d/a.txt"]);
        assert_eq!(selection.left_out().special, 1);
        // Chosen itself, it is not sent either.
        assert_eq!(
            Selection::walk(&[scratch.path().join("d/s")]).unwrap_err(),
            SelectionError::Empty(LeftOut {
                special: 1,
                ..LeftOut::default()
            })
        );
        drop(socket);
    }

    #[test]
    fn too_many_files_or_folders_are_an_error_and_not_a_part_of_the_list() {
        let scratch = scratch();
        make(scratch.path(), "d/a", 1);
        make(scratch.path(), "d/b", 1);
        make(scratch.path(), "d/c/e", 1);
        let chosen = [scratch.path().join("d")];
        let walk_with = |limits| Selection::walk_within(&chosen, limits).unwrap_err();
        assert_eq!(
            walk_with(Limits {
                files: 2,
                ..Limits::PROTOCOL
            }),
            SelectionError::TooMany {
                what: "files",
                max: 2
            }
        );
        assert_eq!(
            walk_with(Limits {
                folders: 1,
                ..Limits::PROTOCOL
            }),
            SelectionError::TooMany {
                what: "folders",
                max: 1
            }
        );
        // The limits are exactly what may be there.
        let enough = Limits {
            files: 3,
            folders: 2,
            ..Limits::PROTOCOL
        };
        assert_eq!(
            Selection::walk_within(&chosen, enough)
                .unwrap()
                .summary()
                .files,
            3
        );
    }

    #[test]
    fn what_a_folder_lists_is_limited_whether_it_is_sent_or_not() {
        let scratch = scratch();
        for name in ["a", "b", "c", "d"] {
            make(scratch.path(), &format!("d/{name}"), 1);
        }
        let chosen = [scratch.path().join("d")];
        let few = Limits {
            looked_at: 3,
            ..Limits::PROTOCOL
        };
        assert_eq!(
            Selection::walk_within(&chosen, few).unwrap_err(),
            SelectionError::TooMany {
                what: "things in the folders to look at",
                max: 3
            }
        );
        let enough = Limits {
            looked_at: 4,
            ..Limits::PROTOCOL
        };
        assert_eq!(
            Selection::walk_within(&chosen, enough)
                .unwrap()
                .summary()
                .files,
            4
        );
    }

    #[test]
    fn a_list_that_would_take_too_much_room_is_an_error() {
        let scratch = scratch();
        make(scratch.path(), "d/a", 1);
        make(scratch.path(), "d/b", 1);
        let chosen = [scratch.path().join("d")];
        // `d`, `d/a` and `d/b` take 19 bytes each besides their paths: 1 + 3 + 3 + 3 bytes of path.
        let hashed = 3 * 19 + 1 + 3 + 3;
        let tight = Limits {
            list_bytes: hashed - 1,
            ..Limits::PROTOCOL
        };
        assert_eq!(
            Selection::walk_within(&chosen, tight).unwrap_err(),
            SelectionError::ListTooLong
        );
        let exact = Limits {
            list_bytes: hashed,
            ..Limits::PROTOCOL
        };
        assert_eq!(
            Selection::walk_within(&chosen, exact)
                .unwrap()
                .summary()
                .files,
            2
        );
    }

    #[test]
    fn a_file_larger_than_a_transfer_may_hold_is_left_out() {
        let scratch = scratch();
        make(scratch.path(), "d/small", 5);
        make(scratch.path(), "d/big", 6);
        let big_alone = make(scratch.path(), "big", 6);
        let limits = Limits {
            file_bytes: 5,
            ..Limits::PROTOCOL
        };
        let selection = Selection::walk_within(&[scratch.path().join("d")], limits).unwrap();
        assert_eq!(paths(&selection), ["d", "d/small"]);
        assert_eq!(selection.left_out().too_large, 1);
        assert_eq!(
            Selection::walk_within(&[big_alone], limits).unwrap_err(),
            SelectionError::Empty(LeftOut {
                too_large: 1,
                ..LeftOut::default()
            })
        );
    }

    #[test]
    fn sizes_that_do_not_fit_in_a_number_are_an_error() {
        let mut walker = Walker::new(
            Limits {
                file_bytes: u64::MAX,
                ..Limits::PROTOCOL
            },
            LeftOut::default(),
        );
        walker.file("a".to_owned(), u64::MAX / 2 + 1, None).unwrap();
        assert_eq!(
            walker
                .file("b".to_owned(), u64::MAX / 2 + 1, None)
                .unwrap_err(),
            SelectionError::TooBig
        );
    }

    #[test]
    fn a_time_is_read_as_milliseconds_since_1970() {
        let at = |millis: u64| UNIX_EPOCH + Duration::from_millis(millis);
        assert_eq!(millis_since_epoch(UNIX_EPOCH), Some(0));
        assert_eq!(
            millis_since_epoch(at(1_700_000_000_123)),
            Some(1_700_000_000_123)
        );
        // A part of a millisecond is cut off, and before 1970 the cut is toward 0.
        assert_eq!(
            millis_since_epoch(at(5) + Duration::from_micros(900)),
            Some(5)
        );
        let before = |duration: Duration| UNIX_EPOCH - duration;
        assert_eq!(
            millis_since_epoch(before(Duration::from_millis(1500))),
            Some(-1500)
        );
        assert_eq!(
            millis_since_epoch(before(Duration::from_micros(2900))),
            Some(-2)
        );
        let scratch = scratch();
        let file = make(scratch.path(), "f", 1);
        assert_eq!(modified_ms(&fs::metadata(file).unwrap()), Some(MADE));
    }

    #[test]
    fn nothing_that_was_chosen_is_in_a_debug_print() {
        let scratch = scratch();
        let selection = walk(&[&make(scratch.path(), "private-name.txt", 4)]);
        let shown = format!("{selection:?} {:?}", selection.offer(id()));
        assert!(!shown.contains("private-name"), "{shown}");
        assert!(shown.contains("bytes: 4"), "{shown}");
    }
}
