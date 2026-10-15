// SPDX-License-Identifier: Apache-2.0

//! Feeds arbitrary bytes to the parts of a file transfer that read what a device on the network
//! sends: the header of a lane, the head of a frame, the list of files and the hash of a file
//! that is made from its spans.
//!
//! The first byte of the input picks the part. Besides "never panics", each part is checked for
//! what it promises:
//!
//! - a lane header or a frame head that is read can be written and read again, to the same value;
//! - a list that the checker accepts is in the order of the protocol, every entry is inside a
//!   folder that came before it, and it is what the offer said: the same counts and sizes, and the
//!   same top-level items as the preview showed. That is what the person agreed to. It also has
//!   the hash that the offer carried, and a list that has everything else right is refused for
//!   its hash only if the hash is not the one of that list;
//! - a hasher that has every span gives a hash, and one that has not says which span is missing.
//!
//! Bit 2 of the first byte, for a list, says whether the hash in the offer is replaced by the
//! hash of the list that follows. Without it almost no input gets past the hash, and the checks
//! that come after it would never run on anything the fuzzer found.

#![no_main]

use std::collections::HashSet;

use baton_proto::{
    frame,
    limits::MAX_FILE_BYTES,
    message::Request,
    transfer::{
        Offer, TransferRequest,
        hash::{FileHasher, HashError, SPAN_LEN},
        lane::{FrameHead, LaneHeader},
        list::{ListChecker, ListChunk, ListEntry, ListError, hash_list},
        path,
    },
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((mode, data)) = data.split_first() else {
        return;
    };
    match mode % 4 {
        0 => lane_header(data),
        1 => frame_head(data),
        2 => list(mode >> 2, data),
        _ => hasher(data),
    }
});

fn lane_header(data: &[u8]) {
    let Some(bytes) = data.first_chunk::<{ LaneHeader::LEN }>() else {
        return;
    };
    if let Ok(header) = LaneHeader::from_bytes(bytes) {
        assert_eq!(&header.to_bytes(), bytes);
    }
}

fn frame_head(data: &[u8]) {
    let Some((tag, rest)) = data.split_first() else {
        return;
    };
    let Ok(head) = FrameHead::decode(*tag, rest) else {
        return;
    };
    assert_eq!(FrameHead::rest_len(*tag), Ok(rest.len()));
    // A reason that this version does not know is written as `Other`, so it is the value that
    // comes back, and not the bytes.
    let mut again = Vec::new();
    head.encode_into(&mut again);
    let (tag, rest) = again.split_first().unwrap();
    assert_eq!(FrameHead::decode(*tag, rest), Ok(head));
}

/// Splits input into parts, each after a length of two bytes.
fn parts(mut data: &[u8]) -> impl Iterator<Item = &[u8]> {
    std::iter::from_fn(move || {
        let (len, rest) = data.split_first_chunk::<2>()?;
        let (part, rest) = rest.split_at_checked(usize::from(u16::from_be_bytes(*len)))?;
        data = rest;
        Some(part)
    })
}

/// An offer, and then the pieces of a list that is checked against it, each part after a length.
/// With bit 0 of `flags` set, the offer carries the hash of the list that follows.
fn list(flags: u8, data: &[u8]) {
    let mut parts = parts(data);
    let Some(offer) = parts.next() else {
        return;
    };
    let Ok(Request::Transfer(TransferRequest::Offer(mut offer))) =
        frame::decode_payload::<Request>(offer)
    else {
        return;
    };
    let mut chunks = Vec::new();
    for chunk in parts {
        let Ok(chunk) = frame::decode_payload::<ListChunk>(chunk) else {
            return;
        };
        chunks.push(chunk);
    }
    let entries: Vec<ListEntry> = chunks
        .iter()
        .flat_map(|chunk| chunk.entries.iter().cloned())
        .collect();
    let hash = hash_list(&entries);
    if flags & 1 == 1 {
        offer.list_hash = hash;
    }

    let mut checker = ListChecker::new(&offer).expect("an offer that decoded is valid");
    for chunk in chunks {
        if checker.push(chunk).is_err() {
            return;
        }
    }
    match checker.finish() {
        Ok(listing) => {
            assert_eq!(offer.list_hash, hash);
            assert_eq!(listing.entries(), entries.as_slice());
            assert_what_was_offered(&offer, &entries);
        }
        // Everything else was right, so the hash is what was wrong.
        Err(ListError::HashMismatch) => assert_ne!(offer.list_hash, hash),
        Err(_) => {}
    }
}

/// What an accepted list must be, worked out again from scratch.
fn assert_what_was_offered(offer: &Offer, entries: &[ListEntry]) {
    let mut folders = HashSet::new();
    let (mut files, mut bytes, mut items) = (0_u64, 0_u64, 0_u64);
    for (i, entry) in entries.iter().enumerate() {
        path::check_path(&entry.path).expect("an entry of an accepted list has a valid path");
        if let Some(before) = i.checked_sub(1).and_then(|before| entries.get(before)) {
            assert!(
                path::compare(&before.path, &entry.path).is_lt(),
                "entries are in order"
            );
        }
        let (parent, _) = path::split_name(&entry.path);
        if parent.is_empty() {
            items += 1;
        } else {
            assert!(folders.contains(parent), "an entry is inside a listed folder");
        }
        if entry.folder {
            folders.insert(entry.path.as_str());
        } else {
            files += 1;
            bytes += entry.size;
        }
    }
    assert_eq!(items, u64::from(offer.summary.items));
    assert_eq!(files, u64::from(offer.summary.files));
    assert_eq!(folders.len() as u64, u64::from(offer.summary.folders));
    assert_eq!(bytes, offer.summary.bytes);

    // The top-level items that the preview showed are the first ones of the list.
    let top = entries
        .iter()
        .filter(|entry| path::split_name(&entry.path).0.is_empty());
    for (shown, item) in offer.preview.iter().zip(top) {
        assert_eq!(shown.name, item.path);
        assert_eq!(shown.folder, item.folder);
        if item.folder {
            let prefix = format!("{}/", item.path);
            let inside = entries
                .iter()
                .filter(|entry| !entry.folder && entry.path.starts_with(&prefix));
            assert_eq!(u64::from(shown.files), inside.clone().count() as u64);
            assert_eq!(shown.bytes, inside.map(|entry| entry.size).sum::<u64>());
        } else {
            assert_eq!(shown.bytes, item.size);
        }
    }
}

/// The size of a file, and then values for spans, each as a number and 32 bytes.
fn hasher(data: &[u8]) {
    let Some((size, mut rest)) = data.split_first_chunk::<8>() else {
        return;
    };
    // Small files, where a hasher can be complete, and files up to the largest there is.
    let size = u64::from_be_bytes(*size);
    let size = if size & 1 == 0 {
        (size >> 1) % (40 * SPAN_LEN)
    } else {
        (size >> 1) % (MAX_FILE_BYTES + 1)
    };
    let mut hasher = FileHasher::new(size).expect("a size up to the largest is allowed");
    while let Some((record, after)) = rest.split_first_chunk::<36>() {
        rest = after;
        let (index, value) = record.split_first_chunk::<4>().unwrap();
        let index = u32::from_be_bytes(*index);
        let value = <[u8; 32]>::try_from(value).unwrap();
        let had = hasher.has(index);
        match hasher.insert(index, value) {
            Ok(()) => assert!(!had && index < hasher.span_count()),
            Err(HashError::Duplicate { .. }) => assert!(had),
            Err(HashError::NoSuchSpan { .. }) => assert!(index >= hasher.span_count()),
            Err(other) => panic!("unexpected error from insert: {other}"),
        }
    }
    match hasher.finish() {
        Ok(_) => assert!(hasher.is_complete()),
        Err(HashError::Missing { index }) => {
            assert!(!hasher.has(index) && index < hasher.span_count());
        }
        Err(other) => panic!("unexpected error from finish: {other}"),
    }
}
