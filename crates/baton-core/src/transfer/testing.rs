// SPDX-License-Identifier: Apache-2.0

//! What the tests of the transfer engine share: lists that were checked against an offer, data to
//! put in files, the bytes of the frames of a lane, a sample transfer to receive, and ways to wait.

use std::{fs, future::Future, path::Path, time::Duration};

use baton_proto::{
    frame,
    id::{DeviceId, TransferId},
    limits::MAX_PREVIEW_ITEMS,
    transfer::{
        Finish, Offer, PreviewItem, Summary,
        lane::{FrameHead, SkipReason, TAG_LIST},
        list::{ListChecker, ListChunk, ListEntry, Listing, hash_list},
        path,
    },
};
use blake3::Hash;

use super::destination::Destination;

/// The longest a test waits for something that has to happen.
pub(crate) const WAIT: Duration = Duration::from_secs(20);

/// Waits for `work`, but not for ever.
pub(crate) async fn within<T>(what: &str, work: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, work)
        .await
        .unwrap_or_else(|_| panic!("{what} took too long"))
}

/// Waits until `ready` is true, but not for ever.
pub(crate) async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    within(what, async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
}

/// A destination in a folder of its own. The folder goes when the first is dropped.
pub(crate) fn destination() -> (tempfile::TempDir, Destination) {
    let scratch = tempfile::tempdir().unwrap();
    let destination =
        Destination::open(scratch.path(), &DeviceId::from_bytes([1; DeviceId::LEN])).unwrap();
    (scratch, destination)
}

/// A file of the list, with a time that is a whole number of seconds (some volumes keep no more).
pub(crate) fn file(path: &str, size: u64) -> ListEntry {
    ListEntry {
        path: path.into(),
        folder: false,
        size,
        modified_ms: Some(1_600_000_000_000),
    }
}

/// A folder of the list.
pub(crate) fn folder(path: &str) -> ListEntry {
    ListEntry {
        path: path.into(),
        folder: true,
        size: 0,
        modified_ms: None,
    }
}

/// The list that an offer of `entries` carries, checked against it, with the entries put in the
/// order of the format first.
pub(crate) fn listing(entries: &[ListEntry]) -> Listing {
    let mut entries = entries.to_vec();
    entries.sort_by(|a, b| path::compare(&a.path, &b.path));
    let mut checker = ListChecker::new(&offer_for(&entries)).unwrap();
    for chunk in entries.chunks(100) {
        checker
            .push(ListChunk {
                entries: chunk.to_vec(),
            })
            .unwrap();
    }
    checker.finish().unwrap()
}

/// The offer of `entries`, which must be in the order of the format: a summary, a preview and the
/// hash worked out directly from the list. Its id is `[9; 16]`.
pub(crate) fn offer_for(entries: &[ListEntry]) -> Offer {
    let count = |wanted: fn(&&ListEntry) -> bool| {
        u32::try_from(entries.iter().filter(wanted).count()).unwrap()
    };
    let top_level: Vec<&ListEntry> = entries.iter().filter(|e| !e.path.contains('/')).collect();
    let preview = top_level
        .iter()
        .take(MAX_PREVIEW_ITEMS)
        .map(|item| {
            let prefix = format!("{}/", item.path);
            let inside: Vec<&ListEntry> = entries
                .iter()
                .filter(|e| !e.folder && e.path.starts_with(&prefix))
                .collect();
            PreviewItem {
                name: item.path.clone(),
                folder: item.folder,
                files: if item.folder {
                    u32::try_from(inside.len()).unwrap()
                } else {
                    0
                },
                bytes: if item.folder {
                    inside.iter().map(|e| e.size).sum()
                } else {
                    item.size
                },
            }
        })
        .collect();
    Offer {
        id: TransferId::from_bytes([9; TransferId::LEN]),
        summary: Summary {
            items: u32::try_from(top_level.len()).unwrap(),
            files: count(|e| !e.folder),
            folders: count(|e| e.folder),
            bytes: entries.iter().map(|e| e.size).sum(),
        },
        preview,
        list_hash: hash_list(entries),
    }
}

/// `len` bytes that depend on `seed` and look like nothing, so that two spans never match by
/// accident.
pub(crate) fn bytes(seed: u8, len: usize) -> Vec<u8> {
    let mut out = vec![0; len];
    blake3::Hasher::new()
        .update(&[seed])
        .finalize_xof()
        .fill(&mut out);
    out
}

// The bytes of the frames of a lane.

/// The bytes of the head of a frame, which is its tag and what the tag says follows.
pub(crate) fn head(head: FrameHead) -> Vec<u8> {
    let mut out = Vec::new();
    head.encode_into(&mut out);
    out
}

/// A `Span` frame.
pub(crate) fn span(file: u32, span: u32, data: &[u8]) -> Vec<u8> {
    let mut out = head(FrameHead::Span { file, span });
    out.extend_from_slice(data);
    out
}

/// An `End` frame whose hash is `seed` repeated, which is no file's hash.
pub(crate) fn end(file: u32, seed: u8) -> Vec<u8> {
    end_with(file, Hash::from_bytes([seed; blake3::OUT_LEN]))
}

/// An `End` frame with the hash `hash`.
pub(crate) fn end_with(file: u32, hash: Hash) -> Vec<u8> {
    head(FrameHead::End { file, hash })
}

/// A `Skip` frame.
pub(crate) fn skip(file: u32, reason: SkipReason) -> Vec<u8> {
    head(FrameHead::Skip { file, reason })
}

/// A piece of the list.
pub(crate) fn chunk(entries: Vec<ListEntry>) -> ListChunk {
    ListChunk { entries }
}

/// A `List` frame holding `chunk`.
pub(crate) fn list(chunk: &ListChunk) -> Vec<u8> {
    let mut out = vec![TAG_LIST];
    out.extend_from_slice(&frame::encode(chunk).unwrap());
    out
}

/// A lane made of `parts`, one after another.
pub(crate) fn lane(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

// The sample transfer, which the tests that receive a whole one use.

/// The entries of the sample, in the order of the list: `x`, which has 10 bytes, and the empty file
/// `zero`.
pub(crate) fn sample_entries() -> Vec<ListEntry> {
    vec![file("x", 10), file("zero", 0)]
}

/// What `x` of the sample holds.
pub(crate) fn sample_x() -> Vec<u8> {
    bytes(1, 10)
}

/// The offer of the sample under the id `id`.
pub(crate) fn sample_offer(id: TransferId) -> Offer {
    Offer {
        id,
        ..offer_for(&sample_entries())
    }
}

/// The `Finish` of the sample under the id `id`.
pub(crate) fn sample_finish(id: TransferId) -> Finish {
    Finish {
        id,
        files: 2,
        bytes: 10,
    }
}

/// One lane that carries all of the sample: the list, and then each file.
pub(crate) fn sample_lane() -> Vec<u8> {
    let x = sample_x();
    lane(&[
        &list(&chunk(sample_entries())),
        &span(0, 0, &x),
        &end_with(0, blake3::hash(&x)),
        &end_with(1, blake3::hash(b"")),
    ])
}

/// How many staging folders of transfers that are being received `root`, the folder of a
/// destination, holds. They are in a folder of the destination's own.
pub(crate) fn staging_folders(root: &Path) -> usize {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".baton-incoming-")
        })
        .map(|holder| fs::read_dir(holder.path()).unwrap().count())
        .sum()
}
