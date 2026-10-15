// SPDX-License-Identifier: Apache-2.0

//! The list of files in a transfer, and the check of it against the offer that was accepted.
//!
//! An offer carries a summary and a preview, which is what a person sees before they say yes.
//! The list is what then arrives on lane 0: every file and every folder, in pieces of at most
//! [`MAX_LIST_FRAME_ENTRIES`] entries. The person agreed to the summary and the preview, so the
//! list has to be exactly that, and [`ListChecker`] sees to it while the list comes in, with
//! memory that grows with the list and not with anything the sender claims.
//!
//! The order of the list is part of the format. Entries are sorted by [`path::compare`], which
//! puts a folder directly before what is inside it, and a folder is always listed before the
//! entries inside it. Two things follow without a table of the paths seen so far: a path cannot
//! appear twice, and an entry is inside a folder that was listed (so a file never has files
//! inside it, and nothing is created in a folder that was not asked for).
//!
//! The offer also carries the hash of the whole list ([`ListHash`], made by [`ListHasher`]). It
//! makes "the list must match the offer" exact, names and times included, and it is what tells a
//! repeat of an offer from another offer under the same id. A receiver holds the whole list
//! before it writes anything, so the list has a size limit of its own, [`MAX_LIST_BYTES`].

use std::{cmp::Ordering, fmt};

use data_encoding::HEXLOWER;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_bytes::ByteArray;

use super::{Offer, PreviewItem, Summary, hash, is_default, path};
use crate::{
    limits::{
        LimitError, MAX_FILE_BYTES, MAX_FRAME_LEN, MAX_LIST_BYTES, MAX_LIST_FRAME_ENTRIES,
        MAX_PATH_BYTES,
    },
    message::Message,
};

// Even with generous room for the other fields, a full piece of the list fits in a frame.
const _: () = assert!(MAX_LIST_FRAME_ENTRIES * (MAX_PATH_BYTES + 128) <= MAX_FRAME_LEN);

/// What the hash of a list is made under. Changing it changes every list hash.
const LIST_HASH_CONTEXT: &str = "baton 2026-10-15 transfer list v1";

/// The bytes that an entry takes in the form that is hashed, besides its path: flags (1), size
/// (8), time (8) and the length of the path (2).
pub const ENTRY_FIXED_LEN: usize = 1 + 8 + 8 + 2;

/// One file or folder in the list.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEntry {
    /// Where it goes, relative to the folder the transfer is put in. See [`path`].
    pub path: String,
    /// Whether it is a folder. A folder has no size and no time.
    #[serde(default, skip_serializing_if = "is_default")]
    pub folder: bool,
    /// Size of a file in bytes, at most [`MAX_FILE_BYTES`]. It is 0 for a folder.
    #[serde(default, skip_serializing_if = "is_default")]
    pub size: u64,
    /// When the file was last changed, in milliseconds since 1970, if the sender knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_ms: Option<i64>,
}

// What people call their files is theirs. A message is logged with `{:?}` when something goes
// wrong, and logs get pasted into bug reports, so a path is shown by its size only.
impl fmt::Debug for ListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListEntry")
            .field("path_bytes", &self.path.len())
            .field("folder", &self.folder)
            .field("size", &self.size)
            .field("modified_ms", &self.modified_ms)
            .finish()
    }
}

impl ListEntry {
    /// How many bytes the entry takes in the form that is hashed.
    fn hashed_len(&self) -> usize {
        ENTRY_FIXED_LEN.saturating_add(self.path.len())
    }

    pub(super) fn validate(&self) -> Result<(), LimitError> {
        path::check_path(&self.path).map_err(|e| e.into_limit("file path"))?;
        if self.folder {
            if self.size != 0 || self.modified_ms.is_some() {
                return Err(LimitError::Invalid {
                    field: "folder",
                    reason: "has a size or a time",
                });
            }
        } else if self.size > MAX_FILE_BYTES {
            return Err(LimitError::Invalid {
                field: "file size",
                reason: "is larger than the limit",
            });
        }
        Ok(())
    }
}

/// A piece of the list of files, as it is sent on lane 0 in a frame of section 2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListChunk {
    /// The next entries of the list, in order. There are between 1 and
    /// [`MAX_LIST_FRAME_ENTRIES`] of them.
    pub entries: Vec<ListEntry>,
}

impl Message for ListChunk {
    const MAX_FRAME_LEN: usize = MAX_FRAME_LEN;

    fn validate(&self) -> Result<(), LimitError> {
        if self.entries.is_empty() {
            return Err(LimitError::Empty {
                field: "list entries",
            });
        }
        if self.entries.len() > MAX_LIST_FRAME_ENTRIES {
            return Err(LimitError::TooMany {
                field: "list entries",
                count: self.entries.len(),
                max: MAX_LIST_FRAME_ENTRIES,
            });
        }
        self.entries.iter().try_for_each(ListEntry::validate)
    }
}

/// The BLAKE3 hash of a whole list, as [`ListHasher`] makes it.
///
/// An offer carries it. The list that arrives has to hash to it, and an offer that is repeated
/// is the same offer only if it carries the same one.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListHash([u8; Self::LEN]);

impl ListHash {
    /// Length of the hash in bytes.
    pub const LEN: usize = blake3::OUT_LEN;

    /// Wraps the bytes of a hash.
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The bytes of the hash.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Display for ListHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for ListHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ListHash({self})")
    }
}

impl Serialize for ListHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ByteArray::new(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ListHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

/// Makes the hash of a list, an entry at a time.
///
/// It is BLAKE3 under the context `baton 2026-10-15 transfer list v1` (its key derivation mode),
/// of every entry in the order of the list, each as
///
/// ```text
/// flags u8 | size u64 | modified i64 | path length u16 | path
/// ```
///
/// Numbers are big-endian. Bit 0 of `flags` is set for a folder, bit 1 when the entry has a
/// time, and the other bits are 0. The size of a folder is 0, and the time is 0 when there is
/// none. An entry takes [`ENTRY_FIXED_LEN`] bytes and its path, so the entries need no separator,
/// and the hash does not depend on how the list was cut into frames. Everything an entry says is
/// in it, the time included, so a list that was made again after a file changed is another list.
#[derive(Debug, Clone)]
pub struct ListHasher {
    hasher: blake3::Hasher,
    bytes: usize,
}

impl Default for ListHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl ListHasher {
    /// Starts the hash of a list.
    pub fn new() -> Self {
        Self {
            hasher: blake3::Hasher::new_derive_key(LIST_HASH_CONTEXT),
            bytes: 0,
        }
    }

    /// Adds the next entry of the list.
    pub fn update(&mut self, entry: &ListEntry) {
        let flags = u8::from(entry.folder) | (u8::from(entry.modified_ms.is_some()) << 1);
        let path_len = u16::try_from(entry.path.len()).unwrap_or(u16::MAX);
        self.hasher
            .update(&[flags])
            .update(&entry.size.to_be_bytes())
            .update(&entry.modified_ms.unwrap_or(0).to_be_bytes())
            .update(&path_len.to_be_bytes())
            .update(entry.path.as_bytes());
        self.bytes = self.bytes.saturating_add(entry.hashed_len());
    }

    /// How many bytes the entries added so far take in the form that is hashed. A list may take
    /// at most [`MAX_LIST_BYTES`].
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The hash of the entries added so far.
    pub fn finalize(&self) -> ListHash {
        ListHash(*self.hasher.finalize().as_bytes())
    }
}

/// The hash of a list.
pub fn hash_list<'a>(entries: impl IntoIterator<Item = &'a ListEntry>) -> ListHash {
    let mut hasher = ListHasher::new();
    for entry in entries {
        hasher.update(entry);
    }
    hasher.finalize()
}

/// Why a list does not match the offer that was accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ListError {
    /// The offer that the list is checked against is not valid.
    #[error("the offer is not valid: {0}")]
    Offer(LimitError),
    /// An entry of the list is not valid.
    #[error("an entry of the list is not valid: {0}")]
    Entry(LimitError),
    /// An entry is not after the one before it.
    #[error("entry {index} is not after the entry before it")]
    OutOfOrder {
        /// Position of the entry in the list.
        index: u32,
    },
    /// An entry is not inside a folder that was listed.
    #[error("entry {index} is not inside a folder that was listed before it")]
    MissingParent {
        /// Position of the entry in the list.
        index: u32,
    },
    /// The list has more of something than the offer said.
    #[error("the list has more {what} than the offer said")]
    TooMany {
        /// What there are too many of: `files`, `folders` or `items`.
        what: &'static str,
    },
    /// The files in the list are larger together than the offer said.
    #[error("the files in the list are larger together than the offer said")]
    TooBig,
    /// The list ended with a count that differs from the offer.
    #[error("the list has a different number of {what} than the offer said")]
    Mismatch {
        /// What differs: `files`, `folders`, `items` or `bytes`.
        what: &'static str,
    },
    /// A top-level item is not what the preview showed.
    #[error("top-level item {item} is not what the preview showed")]
    PreviewMismatch {
        /// Position of the item among the top-level items.
        item: usize,
    },
    /// The list takes more than [`MAX_LIST_BYTES`] in the form that is hashed.
    #[error("the list is longer than {MAX_LIST_BYTES} bytes")]
    TooLong,
    /// The list is not the one whose hash the offer carried.
    #[error("the list does not have the hash that the offer carried")]
    HashMismatch,
}

/// Checks the list as it comes in, piece by piece, against the offer that was accepted.
///
/// After an error the checker has seen a list it cannot accept, and has nothing more to give.
#[derive(Debug)]
pub struct ListChecker {
    summary: Summary,
    preview: Vec<PreviewItem>,
    /// The hash of the list that the offer carried.
    list_hash: ListHash,
    /// The hash of the entries that came in, which has to end up as `list_hash`.
    hasher: ListHasher,
    entries: Vec<ListEntry>,
    /// The names of the folders the last entry is inside, outermost first, and the entry
    /// itself if it is a folder. Entries that follow can only be inside these.
    open: Vec<String>,
    files: u32,
    folders: u32,
    items: u32,
    bytes: u64,
    /// The top-level folder that is being added up, if the preview shows it.
    top: Option<Top>,
}

#[derive(Debug)]
struct Top {
    slot: usize,
    files: u64,
    bytes: u64,
}

impl ListChecker {
    /// Starts checking a list against `offer`.
    pub fn new(offer: &Offer) -> Result<Self, ListError> {
        offer.validate().map_err(ListError::Offer)?;
        Ok(Self {
            summary: offer.summary,
            preview: offer.preview.clone(),
            list_hash: offer.list_hash,
            hasher: ListHasher::new(),
            entries: Vec::new(),
            open: Vec::new(),
            files: 0,
            folders: 0,
            items: 0,
            bytes: 0,
            top: None,
        })
    }

    /// Checks the next piece of the list.
    pub fn push(&mut self, chunk: ListChunk) -> Result<(), ListError> {
        chunk.validate().map_err(ListError::Entry)?;
        chunk
            .entries
            .into_iter()
            .try_for_each(|entry| self.push_entry(entry))
    }

    /// Ends the list, which must be the whole of what the offer said, down to its hash.
    pub fn finish(mut self) -> Result<Listing, ListError> {
        self.finish_top()?;
        for (what, got, expected) in [
            (
                "items",
                u64::from(self.items),
                u64::from(self.summary.items),
            ),
            (
                "files",
                u64::from(self.files),
                u64::from(self.summary.files),
            ),
            (
                "folders",
                u64::from(self.folders),
                u64::from(self.summary.folders),
            ),
            ("bytes", self.bytes, self.summary.bytes),
        ] {
            if got != expected {
                return Err(ListError::Mismatch { what });
            }
        }
        // The counts and the preview are what a person agreed to and are told first. The hash is
        // what is left: names, sizes and times, all of them.
        if self.hasher.finalize() != self.list_hash {
            return Err(ListError::HashMismatch);
        }
        Ok(Listing {
            entries: self.entries,
        })
    }

    fn push_entry(&mut self, entry: ListEntry) -> Result<(), ListError> {
        let index = u32::try_from(self.entries.len()).unwrap_or(u32::MAX);

        // Room comes first, so that a list can never grow past what the offer said, or past what
        // a list may take whatever the offer said.
        if entry.folder {
            if self.folders >= self.summary.folders {
                return Err(ListError::TooMany { what: "folders" });
            }
        } else if self.files >= self.summary.files {
            return Err(ListError::TooMany { what: "files" });
        }
        if self.hasher.bytes().saturating_add(entry.hashed_len()) > MAX_LIST_BYTES {
            return Err(ListError::TooLong);
        }

        if let Some(previous) = self.entries.last()
            && path::compare(&previous.path, &entry.path) != Ordering::Less
        {
            return Err(ListError::OutOfOrder { index });
        }

        let (parent, name) = path::split_name(&entry.path);
        let depth = if parent.is_empty() {
            0
        } else {
            parent.split('/').count()
        };
        let inside_open_folders = self.open.len() >= depth
            && self
                .open
                .iter()
                .zip(parent.split('/').filter(|_| depth > 0))
                .all(|(open, part)| open == part);
        if !inside_open_folders {
            return Err(ListError::MissingParent { index });
        }
        self.open.truncate(depth);

        if depth == 0 {
            self.start_item(&entry)?;
        } else if let Some(top) = &mut self.top
            && !entry.folder
        {
            top.files += 1;
            top.bytes = top.bytes.saturating_add(entry.size);
        }

        if entry.folder {
            self.folders += 1;
            self.open.push(name.to_owned());
        } else {
            self.files += 1;
            self.bytes = self
                .bytes
                .checked_add(entry.size)
                .filter(|bytes| *bytes <= self.summary.bytes)
                .ok_or(ListError::TooBig)?;
        }
        self.hasher.update(&entry);
        self.entries.push(entry);
        Ok(())
    }

    /// Begins the next top-level item, which is the one the preview shows at that place if the
    /// preview reaches so far.
    fn start_item(&mut self, entry: &ListEntry) -> Result<(), ListError> {
        self.finish_top()?;
        if self.items >= self.summary.items {
            return Err(ListError::TooMany { what: "items" });
        }
        let slot = usize::try_from(self.items).unwrap_or(usize::MAX);
        self.items += 1;
        let Some(shown) = self.preview.get(slot) else {
            return Ok(());
        };
        if shown.name != entry.path || shown.folder != entry.folder {
            return Err(ListError::PreviewMismatch { item: slot });
        }
        if entry.folder {
            self.top = Some(Top {
                slot,
                files: 0,
                bytes: 0,
            });
        } else if shown.files != 0 || shown.bytes != entry.size {
            return Err(ListError::PreviewMismatch { item: slot });
        }
        Ok(())
    }

    /// Compares the top-level folder that has just ended with what the preview showed of it.
    fn finish_top(&mut self) -> Result<(), ListError> {
        let Some(top) = self.top.take() else {
            return Ok(());
        };
        let matches = self
            .preview
            .get(top.slot)
            .is_some_and(|shown| u64::from(shown.files) == top.files && shown.bytes == top.bytes);
        if matches {
            Ok(())
        } else {
            Err(ListError::PreviewMismatch { item: top.slot })
        }
    }
}

/// A list that matches its offer. The position of an entry in it is the number of the file
/// that lane frames refer to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    entries: Vec<ListEntry>,
}

impl Listing {
    /// All entries, in order.
    pub fn entries(&self) -> &[ListEntry] {
        &self.entries
    }

    /// File number `index`, or `None` if there is no such entry or it is a folder.
    pub fn file(&self, index: u32) -> Option<&ListEntry> {
        self.entries
            .get(usize::try_from(index).ok()?)
            .filter(|entry| !entry.folder)
    }

    /// The length of span `span` of file number `file`, or `None` if there is no such span.
    pub fn span_size(&self, file: u32, span: u32) -> Option<u32> {
        hash::span_size(self.file(file)?.size, span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::TransferId;

    fn file(path: &str, size: u64) -> ListEntry {
        ListEntry {
            path: path.into(),
            folder: false,
            size,
            modified_ms: Some(1_700_000_000_000),
        }
    }

    fn folder(path: &str) -> ListEntry {
        ListEntry {
            path: path.into(),
            folder: true,
            size: 0,
            modified_ms: None,
        }
    }

    /// For an offer of a list that is refused before its hash is looked at.
    fn no_hash() -> ListHash {
        ListHash::from_bytes([0; ListHash::LEN])
    }

    /// A summary, a preview and the hash worked out directly from a list that is in order.
    fn offer_for(entries: &[ListEntry]) -> Offer {
        let top_level: Vec<&ListEntry> = entries.iter().filter(|e| !e.path.contains('/')).collect();
        let preview = top_level
            .iter()
            .take(crate::limits::MAX_PREVIEW_ITEMS)
            .map(|item| {
                let prefix = format!("{}/", item.path);
                let inside = entries
                    .iter()
                    .filter(|e| !e.folder && e.path.starts_with(&prefix));
                PreviewItem {
                    name: item.path.clone(),
                    folder: item.folder,
                    files: if item.folder {
                        u32::try_from(inside.clone().count()).unwrap()
                    } else {
                        0
                    },
                    bytes: if item.folder {
                        inside.map(|e| e.size).sum()
                    } else {
                        item.size
                    },
                }
            })
            .collect();
        Offer {
            id: TransferId::from_bytes([9; 16]),
            summary: Summary {
                items: u32::try_from(top_level.len()).unwrap(),
                files: u32::try_from(entries.iter().filter(|e| !e.folder).count()).unwrap(),
                folders: u32::try_from(entries.iter().filter(|e| e.folder).count()).unwrap(),
                bytes: entries.iter().map(|e| e.size).sum(),
            },
            preview,
            list_hash: hash_list(entries),
        }
    }

    fn holiday() -> Vec<ListEntry> {
        vec![
            folder("Holiday"),
            folder("Holiday/day 1"),
            file("Holiday/day 1/a.jpg", 3_000),
            file("Holiday/day 1/b.jpg", 4_000),
            folder("Holiday/empty"),
            file("Holiday/notes.txt", 20),
            file("report.pdf", 100_000),
            folder("zoo"),
            file("zoo/z.txt", 1),
        ]
    }

    fn check(entries: &[ListEntry], offer: &Offer, piece: usize) -> Result<Listing, ListError> {
        let mut checker = ListChecker::new(offer)?;
        for chunk in entries.chunks(piece) {
            checker.push(ListChunk {
                entries: chunk.to_vec(),
            })?;
        }
        checker.finish()
    }

    #[test]
    fn a_list_that_matches_its_offer_is_accepted_however_it_is_cut_up() {
        let entries = holiday();
        let offer = offer_for(&entries);
        assert_eq!(offer.summary.items, 3);
        assert_eq!(offer.summary.files, 5);
        assert_eq!(offer.summary.folders, 4);
        for piece in 1..=entries.len() {
            let listing = check(&entries, &offer, piece).unwrap();
            assert_eq!(listing.entries(), entries.as_slice());
        }
    }

    #[test]
    fn a_list_out_of_order_or_with_a_repeat_is_refused() {
        let entries = holiday();
        let offer = offer_for(&entries);

        // Two neighbours inside a folder the wrong way round.
        let mut swapped = entries.clone();
        swapped.swap(2, 3);
        assert_eq!(
            check(&swapped, &offer, 100).unwrap_err(),
            ListError::OutOfOrder { index: 3 }
        );

        // Two top-level items the wrong way round. Where the preview shows them it notices
        // first, and where it does not, the order is all there is to go by.
        let mut swapped = entries.clone();
        swapped.swap(6, 7);
        assert_eq!(
            check(&swapped, &offer, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 1 }
        );
        let twenty: Vec<ListEntry> = (0..20).map(|i| file(&format!("f{i:02}"), 1)).collect();
        let offer = offer_for(&twenty);
        let mut swapped = twenty.clone();
        swapped.swap(17, 18);
        assert_eq!(
            check(&swapped, &offer, 100).unwrap_err(),
            ListError::OutOfOrder { index: 18 }
        );

        // The same path twice.
        let entries = holiday();
        let mut repeated = entries.clone();
        repeated.insert(3, file("Holiday/day 1/a.jpg", 3_000));
        let mut bigger = offer_for(&entries);
        bigger.summary.files += 1;
        bigger.summary.bytes += 3_000;
        assert_eq!(
            check(&repeated, &bigger, 100).unwrap_err(),
            ListError::OutOfOrder { index: 3 }
        );

        // Sorted by the whole text, as a plain sort would do: `t/a-b` lands between `t/a` and
        // `t/a/b`, because `-` is before `/`, and the folder is cut off from what is inside it.
        let plain = vec![
            folder("t"),
            folder("t/a"),
            file("t/a-b", 1),
            file("t/a/b", 1),
        ];
        let right = [
            folder("t"),
            folder("t/a"),
            file("t/a/b", 1),
            file("t/a-b", 1),
        ];
        let offer = offer_for(&right);
        assert!(check(&right, &offer, 100).is_ok());
        assert_eq!(
            check(&plain, &offer, 100).unwrap_err(),
            ListError::OutOfOrder { index: 3 }
        );
    }

    #[test]
    fn an_entry_must_be_inside_a_folder_that_was_listed() {
        // One top-level item, shown as `top`, in a transfer of so many folders, files and bytes.
        let offer = |top: PreviewItem, folders, files, bytes| Offer {
            id: TransferId::from_bytes([1; 16]),
            summary: Summary {
                items: 1,
                files,
                folders,
                bytes,
            },
            preview: vec![top],
            list_hash: no_hash(),
        };
        let shown_folder = |name: &str, files, bytes| PreviewItem {
            name: name.into(),
            folder: true,
            files,
            bytes,
        };

        // No entry for the folder at all.
        let entries = vec![file("a/b", 1)];
        assert_eq!(
            check(&entries, &offer(shown_folder("a", 1, 1), 1, 1, 1), 10).unwrap_err(),
            ListError::MissingParent { index: 0 }
        );

        // A file has nothing inside it.
        let entries = vec![file("a", 1), file("a/b", 1)];
        let shown_file = PreviewItem {
            name: "a".into(),
            folder: false,
            files: 0,
            bytes: 1,
        };
        assert_eq!(
            check(&entries, &offer(shown_file, 0, 2, 2), 10).unwrap_err(),
            ListError::MissingParent { index: 1 }
        );

        // A folder next to the one the entry is in is not its parent: `a/y` was never listed,
        // and `a/x` that was is not a stand-in for it.
        let entries = vec![
            folder("a"),
            folder("a/x"),
            file("a/x/z", 1),
            file("a/y/w", 1),
        ];
        assert_eq!(
            check(&entries, &offer(shown_folder("a", 2, 2), 2, 2, 2), 10).unwrap_err(),
            ListError::MissingParent { index: 3 }
        );

        // A folder in between that was never listed: `a/x` is, but `a/x/q` is not.
        let entries = vec![folder("a"), folder("a/x"), file("a/x/q/r", 1)];
        assert_eq!(
            check(&entries, &offer(shown_folder("a", 1, 1), 2, 1, 1), 10).unwrap_err(),
            ListError::MissingParent { index: 2 }
        );
    }

    #[test]
    fn a_list_cannot_have_more_than_the_offer_said() {
        let entries = holiday();
        let without = |last: usize| offer_for(&entries[..last]);

        // The offer stops before `zoo/z.txt`, `zoo` and `report.pdf`.
        assert_eq!(
            check(&entries, &without(8), 100).unwrap_err(),
            ListError::TooMany { what: "files" }
        );
        assert_eq!(
            check(&entries, &without(7), 100).unwrap_err(),
            ListError::TooMany { what: "folders" }
        );

        // One more file than the offer counts, at the top level, after a complete folder.
        let entries = vec![folder("d"), file("d/x", 1), file("d/y", 1), file("e", 0)];
        let offer = Offer {
            id: TransferId::from_bytes([1; 16]),
            summary: Summary {
                items: 1,
                files: 3,
                folders: 1,
                bytes: 2,
            },
            preview: vec![PreviewItem {
                name: "d".into(),
                folder: true,
                files: 2,
                bytes: 2,
            }],
            list_hash: no_hash(),
        };
        assert_eq!(
            check(&entries, &offer, 100).unwrap_err(),
            ListError::TooMany { what: "items" }
        );

        // Files that are larger together than the offer says, caught as soon as they are.
        let entries = vec![folder("d"), file("d/x", 10)];
        let offer = Offer {
            id: TransferId::from_bytes([1; 16]),
            summary: Summary {
                items: 1,
                files: 1,
                folders: 1,
                bytes: 5,
            },
            preview: vec![PreviewItem {
                name: "d".into(),
                folder: true,
                files: 1,
                bytes: 5,
            }],
            list_hash: no_hash(),
        };
        assert_eq!(check(&entries, &offer, 100).unwrap_err(), ListError::TooBig);
    }

    #[test]
    fn a_list_that_ends_early_or_adds_up_differently_is_refused() {
        let entries = holiday();
        let offer = offer_for(&entries);

        // Cut short in the middle of a folder, which the preview notices first.
        let mut checker = ListChecker::new(&offer).unwrap();
        checker
            .push(ListChunk {
                entries: entries[..4].to_vec(),
            })
            .unwrap();
        assert_eq!(
            checker.finish().unwrap_err(),
            ListError::PreviewMismatch { item: 0 }
        );

        // A summary that claims one more of something than the list holds.
        for (change, what) in [
            (
                (|s: &mut Summary| s.files += 1) as fn(&mut Summary),
                "files",
            ),
            (|s: &mut Summary| s.folders += 1, "folders"),
            (|s: &mut Summary| s.bytes += 1, "bytes"),
        ] {
            let mut other = offer.clone();
            change(&mut other.summary);
            assert_eq!(
                check(&entries, &other, 100).unwrap_err(),
                ListError::Mismatch { what }
            );
        }

        // Seventeen items, of which the preview shows sixteen, and a list of sixteen.
        let sixteen: Vec<ListEntry> = (0..16).map(|i| file(&format!("f{i:02}"), 1)).collect();
        let mut seventeen = offer_for(&sixteen);
        seventeen.summary.items = 17;
        seventeen.summary.files = 17;
        seventeen.summary.bytes = 17;
        assert_eq!(
            check(&sixteen, &seventeen, 100).unwrap_err(),
            ListError::Mismatch { what: "items" }
        );
    }

    #[test]
    fn what_arrives_must_be_what_the_preview_showed() {
        let entries = holiday();
        let offer = offer_for(&entries);

        // Another name, in the place of a file or a folder.
        let mut renamed = offer.clone();
        renamed.preview[1].name = "invoice.pdf".into();
        assert_eq!(
            check(&entries, &renamed, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 1 }
        );
        let mut renamed = offer.clone();
        renamed.preview[0].name = "Holidays".into();
        assert_eq!(
            check(&entries, &renamed, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 0 }
        );

        // A file shown as a folder, and the other way round. Each is an offer that is valid
        // by itself, so it is the list that does not match it.
        let mut folder_for_file = offer.clone();
        folder_for_file.preview[1].folder = true;
        folder_for_file.preview[1].files = 1;
        assert_eq!(folder_for_file.validate(), Ok(()));
        assert_eq!(
            check(&entries, &folder_for_file, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 1 }
        );
        let mut file_for_folder = offer.clone();
        file_for_folder.preview[0].folder = false;
        file_for_folder.preview[0].files = 0;
        assert_eq!(file_for_folder.validate(), Ok(()));
        assert_eq!(
            check(&entries, &file_for_folder, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 0 }
        );

        // A size that differs, for a file and for what a folder holds.
        let mut size = offer.clone();
        size.preview[1].bytes -= 1;
        assert_eq!(
            check(&entries, &size, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 1 }
        );
        let mut inside = offer.clone();
        inside.preview[0].bytes -= 1;
        inside.preview[2].bytes += 1;
        assert_eq!(inside.validate(), Ok(()));
        assert_eq!(
            check(&entries, &inside, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 0 }
        );

        // A folder with fewer files in it than the list gives it, and with more.
        let mut fewer = offer.clone();
        fewer.preview[0].files -= 1;
        assert_eq!(fewer.validate(), Ok(()));
        assert_eq!(
            check(&entries, &fewer, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 0 }
        );
        let mut more = offer.clone();
        more.preview[2].files += 1;
        more.summary.files += 1;
        assert_eq!(more.validate(), Ok(()));
        assert_eq!(
            check(&entries, &more, 100).unwrap_err(),
            ListError::PreviewMismatch { item: 2 }
        );
    }

    #[test]
    fn a_preview_that_stops_short_does_not_check_the_rest() {
        // 20 top-level files: the preview shows the first 16.
        let entries: Vec<ListEntry> = (0_u32..20)
            .map(|i| file(&format!("f{i:02}.txt"), u64::from(i) + 1))
            .collect();
        let offer = offer_for(&entries);
        assert_eq!(offer.preview.len(), 16);
        assert_eq!(offer.summary.items, 20);
        assert!(check(&entries, &offer, 7).is_ok());
    }

    #[test]
    fn an_entry_that_is_not_valid_is_refused_with_the_reason() {
        let ok = vec![file("a", 1)];
        let offer = offer_for(&ok);
        for bad in [
            file("", 1),
            file("../a", 1),
            file("/a", 1),
            file("a/../b", 1),
            file("a\0b", 1),
            file(&"n".repeat(300), 1),
            ListEntry {
                size: MAX_FILE_BYTES + 1,
                ..file("a", 0)
            },
            ListEntry {
                size: 5,
                ..folder("a")
            },
            ListEntry {
                modified_ms: Some(1),
                ..folder("a")
            },
        ] {
            let mut checker = ListChecker::new(&offer).unwrap();
            assert!(
                matches!(
                    checker.push(ListChunk { entries: vec![bad] }),
                    Err(ListError::Entry(_))
                ),
                "an entry that is not valid"
            );
        }
        let mut checker = ListChecker::new(&offer).unwrap();
        assert!(matches!(
            checker.push(ListChunk { entries: vec![] }),
            Err(ListError::Entry(LimitError::Empty { .. }))
        ));
    }

    #[test]
    fn a_listing_gives_the_files_and_the_spans_they_have() {
        let entries = vec![folder("d"), file("d/big", hash::SPAN_LEN + 7), file("e", 0)];
        let offer = offer_for(&entries);
        let listing = check(&entries, &offer, 2).unwrap();
        assert!(listing.file(0).is_none(), "a folder is not a file");
        assert_eq!(listing.file(1).unwrap().path, "d/big");
        assert!(listing.file(3).is_none());
        assert!(listing.file(u32::MAX).is_none());
        assert_eq!(listing.span_size(1, 0), Some(1 << 20));
        assert_eq!(listing.span_size(1, 1), Some(7));
        assert_eq!(listing.span_size(1, 2), None);
        assert_eq!(listing.span_size(0, 0), None);
        assert_eq!(listing.span_size(2, 0), None, "an empty file has no spans");
    }

    #[test]
    fn a_name_is_not_printed_in_a_log() {
        let secret = ListEntry {
            path: "Tax returns 2025/passport scan.pdf".into(),
            folder: false,
            size: 5,
            modified_ms: None,
        };
        let shown = format!("{secret:?}");
        assert!(
            !shown.contains("Tax") && !shown.contains("passport"),
            "{shown}"
        );
        assert!(shown.contains("path_bytes: 34"), "{shown}");
        let chunk = format!(
            "{:?}",
            ListChunk {
                entries: vec![secret]
            }
        );
        assert!(!chunk.contains("passport"), "{chunk}");
    }

    /// The list of the test vector in `docs/protocol.md`: a folder, a file in it with a time, a
    /// file without a time or a size, and a file whose name is not ASCII and whose time is
    /// before 1970.
    fn vector_list() -> Vec<ListEntry> {
        vec![
            folder("Holiday"),
            ListEntry {
                path: "Holiday/a.txt".into(),
                folder: false,
                size: 5,
                modified_ms: Some(1_700_000_000_000),
            },
            ListEntry {
                path: "b".into(),
                folder: false,
                size: 0,
                modified_ms: None,
            },
            ListEntry {
                path: "Ärger.txt".into(),
                folder: false,
                size: 1024,
                modified_ms: Some(-1),
            },
        ]
    }

    /// What [`vector_list`] is hashed as, written out by hand, one entry to a line: flags, size,
    /// time, the length of the path in bytes (not characters), the path. The spaces are for
    /// reading.
    const VECTOR_STREAM: &str = "\
        01 0000000000000000 0000000000000000 0007 486f6c69646179 \
        02 0000000000000005 0000018bcfe56800 000d 486f6c696461792f612e747874 \
        00 0000000000000000 0000000000000000 0001 62 \
        02 0000000000000400 ffffffffffffffff 000a c384726765722e747874";

    /// The BLAKE3 hash, in its key derivation mode under `baton 2026-10-15 transfer list v1`, of
    /// [`VECTOR_STREAM`]. It was made by the C reference implementation of BLAKE3, not by this code.
    const VECTOR_HASH: &str = "995bf65ca3365c2cfb21667b2099730a378325363410779c479a102f69eefb99";

    #[test]
    fn a_list_has_the_hash_that_the_protocol_gives() {
        let entries = vector_list();
        assert_eq!(hash_list(&entries).to_string(), VECTOR_HASH);

        // The same hash from the stream of bytes, in one piece: how a list is cut up, or that it
        // is hashed an entry at a time, makes no difference.
        let stream = data_encoding::HEXLOWER
            .decode(VECTOR_STREAM.replace(' ', "").as_bytes())
            .unwrap();
        assert_eq!(stream.len(), 107);
        let mut once = blake3::Hasher::new_derive_key("baton 2026-10-15 transfer list v1");
        once.update(&stream);
        assert_eq!(once.finalize().as_bytes(), hash_list(&entries).as_bytes());

        // And what the hasher counts is those same bytes.
        let mut hasher = ListHasher::new();
        assert_eq!(hasher.bytes(), 0);
        for entry in &entries {
            hasher.update(entry);
        }
        assert_eq!(hasher.bytes(), stream.len());
        assert_eq!(hasher.finalize(), hash_list(&entries));
    }

    #[test]
    fn the_list_of_the_vector_is_a_list_that_is_accepted() {
        let entries = vector_list();
        let offer = offer_for(&entries);
        assert_eq!(offer.list_hash.to_string(), VECTOR_HASH);
        assert_eq!(offer.summary.items, 3);
        assert_eq!(offer.summary.bytes, 1_029);
        for piece in 1..=entries.len() {
            assert_eq!(check(&entries, &offer, piece).unwrap().entries(), entries);
        }
    }

    #[test]
    fn a_list_hash_is_shown_as_hex_and_travels_as_32_bytes() {
        let hash = hash_list(&vector_list());
        assert_eq!(hash.to_string(), VECTOR_HASH);
        assert_eq!(format!("{hash:?}"), format!("ListHash({VECTOR_HASH})"));
        assert_eq!(hash.as_bytes().len(), 32);
        assert_eq!(ListHash::from_bytes(*hash.as_bytes()), hash);

        let mut bytes = Vec::new();
        ciborium::into_writer(&hash, &mut bytes).unwrap();
        assert_eq!(bytes.get(..2), Some([0x58, 0x20].as_slice()));
        assert_eq!(bytes.get(2..), Some(hash.as_bytes().as_slice()));
        assert_eq!(ciborium::from_reader::<ListHash, _>(&*bytes).unwrap(), hash);
    }

    /// A change to a list, for the tests that try one change after another.
    type Change = fn(&mut Vec<ListEntry>);

    #[test]
    fn every_field_of_an_entry_is_in_the_hash() {
        let before = hash_list(&vector_list());
        let changes: [(&str, Change); 9] = [
            ("a folder is a file", |e| e[0].folder = false),
            ("the size", |e| e[1].size += 1),
            ("the time", |e| e[1].modified_ms = Some(1_700_000_000_001)),
            ("a time that was not there", |e| e[2].modified_ms = Some(0)),
            ("a time that is gone", |e| e[1].modified_ms = None),
            ("the name", |e| e[2].path = "c".into()),
            ("the case of a name", |e| e[0].path = "holiday".into()),
            ("an entry less", |e| {
                e.pop();
            }),
            ("the order", |e| e.swap(1, 2)),
        ];
        for (what, change) in changes {
            let mut entries = vector_list();
            change(&mut entries);
            assert_ne!(hash_list(&entries), before, "{what}");
        }
        // Where an entry ends and the next begins is part of what is hashed, so moving a
        // letter from one path to the next one makes another list.
        let split = |a: &str, b: &str| {
            hash_list(&[
                ListEntry {
                    path: a.into(),
                    ..file("x", 1)
                },
                ListEntry {
                    path: b.into(),
                    ..file("x", 1)
                },
            ])
        };
        assert_ne!(split("ab", "c"), split("a", "bc"));
    }

    #[test]
    fn a_list_that_is_another_one_than_was_offered_is_refused_though_the_counts_agree() {
        let entries = holiday();
        let offer = offer_for(&entries);
        // Each of these leaves the counts, the sizes added up and the preview as they were, so
        // only the hash can tell.
        let changes: [(&str, Change); 5] = [
            ("another time", |e| e[3].modified_ms = Some(1)),
            ("a time that is gone", |e| e[3].modified_ms = None),
            ("another name", |e| e[5].path = "Holiday/notes.txu".into()),
            ("another name for a folder", |e| {
                e[4].path = "Holiday/emptz".into();
            }),
            ("two sizes the other way round", |e| {
                let (a, b) = (e[2].size, e[3].size);
                e[2].size = b;
                e[3].size = a;
            }),
        ];
        for (what, change) in changes {
            let mut other = entries.clone();
            change(&mut other);
            assert_eq!(
                check(&other, &offer, 100).unwrap_err(),
                ListError::HashMismatch,
                "{what}"
            );
        }
        // An offer whose hash is not the list's, with all else right.
        let mut other = offer.clone();
        other.list_hash = no_hash();
        assert_eq!(
            check(&entries, &other, 100).unwrap_err(),
            ListError::HashMismatch
        );
    }

    /// A folder `a` with `files` files in it, of size 0, each with a name of 255 digits. In the form
    /// that is hashed a file takes 276 bytes and the folder 20.
    fn long_list(files: usize) -> (Vec<ListEntry>, Offer) {
        let mut entries = vec![folder("a")];
        entries.extend((0..files).map(|i| file(&format!("a/{i:0>255}"), 0)));
        let files = u32::try_from(files).unwrap();
        let offer = Offer {
            id: TransferId::from_bytes([1; 16]),
            summary: Summary {
                items: 1,
                files,
                folders: 1,
                bytes: 0,
            },
            preview: vec![PreviewItem {
                name: "a".into(),
                folder: true,
                files,
                bytes: 0,
            }],
            list_hash: hash_list(&entries),
        };
        (entries, offer)
    }

    #[test]
    fn a_list_is_refused_at_the_entry_that_takes_it_past_the_limit() {
        let file_len = ENTRY_FIXED_LEN + "a/".len() + 255;
        let folder_len = ENTRY_FIXED_LEN + "a".len();
        let fits = (MAX_LIST_BYTES - folder_len) / file_len;
        assert_eq!(fits, 60_786, "the numbers of this test are for 16 MiB");

        // As many as fit are accepted, and that is not far from what the limit allows.
        let (entries, offer) = long_list(fits);
        let mut hasher = ListHasher::new();
        for entry in &entries {
            hasher.update(entry);
        }
        assert!(MAX_LIST_BYTES - hasher.bytes() < file_len);
        assert!(check(&entries, &offer, MAX_LIST_FRAME_ENTRIES).is_ok());

        // One more is not, and the checker says so as soon as it is given that entry.
        let (entries, offer) = long_list(fits + 1);
        assert_eq!(
            check(&entries, &offer, MAX_LIST_FRAME_ENTRIES).unwrap_err(),
            ListError::TooLong
        );
        let mut checker = ListChecker::new(&offer).unwrap();
        let (last, before) = entries.split_last().unwrap();
        for chunk in before.chunks(MAX_LIST_FRAME_ENTRIES) {
            checker
                .push(ListChunk {
                    entries: chunk.to_vec(),
                })
                .unwrap();
        }
        assert_eq!(
            checker.push(ListChunk {
                entries: vec![last.clone()]
            }),
            Err(ListError::TooLong)
        );
    }
}
