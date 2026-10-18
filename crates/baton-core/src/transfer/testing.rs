// SPDX-License-Identifier: Apache-2.0

//! What the tests of the transfer engine share: lists that were checked against an offer, and
//! data to put in files.

use baton_proto::{
    id::{DeviceId, TransferId},
    limits::MAX_PREVIEW_ITEMS,
    transfer::{
        Offer, PreviewItem, Summary,
        list::{ListChecker, ListChunk, ListEntry, Listing, hash_list},
        path,
    },
};

use super::destination::Destination;

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

/// A summary, a preview and the hash worked out directly from a list that is in order.
fn offer_for(entries: &[ListEntry]) -> Offer {
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
