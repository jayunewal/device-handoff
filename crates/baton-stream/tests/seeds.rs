// SPDX-License-Identifier: Apache-2.0

//! The seeds of the `stream` fuzz target (`crates/baton-proto/fuzz/seeds/stream`) are still what
//! they are named for.
//!
//! The target starts from them, and a seed that stops being what it was, because a rule was added
//! that it breaks or a field was renamed, quietly stops the fuzzer from starting deep inside the
//! readers. The first byte of a seed picks the part, as in the target, and the rest is what the part
//! reads. Each seed is listed here with what the part must make of it: refuse it, read it and write
//! it in another form, or read it and write the same bytes, which is what a seed named for a
//! message must do if it is the message as the reference sends it. The table has no seed missing
//! and none that is not a file.

// Tests may unwrap and index freely. Clippy exempts #[test] functions and #[cfg(test)]
// modules, but not the helper functions of an integration test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{collections::BTreeSet, path::Path};

use baton_proto::{frame, message::Message};
use baton_stream::{
    control::{SourceMessage, ViewerMessage},
    header::{FrameHeader, PREFIX_LEN, decode_prefix, encode_prefix},
};

#[path = "../../baton-proto/tests/support/strict_cbor.rs"]
mod reference;

/// What a part makes of a seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// It reads it, and writes the same bytes back: the seed is what the reference sends.
    Written,
    /// It reads it, and writes it in another form: a field or a value of a later version, or a
    /// form that the reference accepts and does not write.
    Read,
    /// It refuses it, for the reason that the name gives.
    Refused,
}
use Outcome::{Read, Refused, Written};

#[rustfmt::skip]
const SEEDS: &[(&str, Outcome)] = &[
    ("header-and-a-picture-behind-it", Written),
    ("header-cut-short", Refused),
    ("header-delta", Written),
    ("header-depends-on-a-later-frame", Refused),
    ("header-key-frame", Written),
    ("header-key-frame-and-recovery-frame", Refused),
    ("header-key-frame-that-depends-on-another", Refused),
    ("header-of-a-lane", Refused),
    ("header-recovery-frame", Written),
    ("header-reserved-flag", Refused),
    ("source-config", Written),
    ("source-config-a-field-of-a-later-version", Read),
    ("source-config-codec-of-a-later-version", Read),
    ("source-config-fps-too-high", Refused),
    ("source-config-height-too-high", Refused),
    ("source-config-width-0", Refused),
    ("source-message-nobody-knows", Refused),
    ("source-notice", Written),
    ("source-notice-of-no-events", Refused),
    ("source-notice-reason-of-a-later-version", Read),
    ("source-pause", Written),
    ("source-pause-reason-of-a-later-version", Read),
    ("source-resume", Written),
    ("source-resume-as-a-map", Read),
    ("source-sends-a-message-of-the-viewer", Refused),
    ("start-control-stream", Written),
    ("start-frame-of-video", Written),
    ("start-lane-of-a-transfer", Written),
    ("viewer-input-64-events", Written),
    ("viewer-input-65-events", Refused),
    ("viewer-input-key", Written),
    ("viewer-input-key-code-too-high", Refused),
    ("viewer-input-key-without-text", Written),
    ("viewer-input-modifiers-that-are-not-defined", Refused),
    ("viewer-input-no-events", Refused),
    ("viewer-input-pointer", Written),
    ("viewer-input-position-not-a-number", Refused),
    ("viewer-input-position-out-of-range", Refused),
    ("viewer-input-scroll", Written),
    ("viewer-input-scroll-too-far", Refused),
    ("viewer-input-text-of-20-units", Written),
    ("viewer-input-text-of-22-units", Refused),
    ("viewer-input-text-only", Written),
    ("viewer-input-text-only-that-comes-up", Refused),
    ("viewer-input-text-only-without-text", Refused),
    ("viewer-input-text-too-long", Refused),
    ("viewer-input-text-with-a-control-character", Refused),
    ("viewer-keyframe", Written),
    ("viewer-keyframe-of-the-biggest-frame", Written),
    ("viewer-report", Written),
    ("viewer-report-before-the-first-picture", Written),
    ("viewer-report-thermal-state-of-a-later-version", Read),
    ("viewer-sends-a-message-of-the-source", Refused),
    ("viewer-want", Written),
    ("viewer-want-width-0", Refused),
];

/// The part that a seed is for, from the first byte, as the target picks it.
fn part(name: &str) -> u8 {
    match name.split('-').next() {
        Some("start") => 0,
        Some("header") => 1,
        Some("source") => 2,
        Some("viewer") => 3,
        _ => panic!("the seed {name} has a name that says nothing about what it is for"),
    }
}

fn start(data: &[u8]) -> Outcome {
    let Some(bytes) = data.first_chunk::<PREFIX_LEN>() else {
        return Refused;
    };
    let (kind, session) = decode_prefix(bytes);
    assert_eq!(&encode_prefix(kind, session), bytes);
    Written
}

fn header(data: &[u8]) -> Outcome {
    let Ok((header, rest)) = FrameHeader::parse(data) else {
        return Refused;
    };
    assert_eq!(rest.len(), data.len() - FrameHeader::LEN);
    let written = header
        .encode()
        .expect("a header that was read can be written");
    assert_eq!(written.as_slice(), &data[..FrameHeader::LEN]);
    assert_eq!(FrameHeader::parse(&written).unwrap().0, header);
    Written
}

fn message<M: Message + PartialEq + std::fmt::Debug>(data: &[u8]) -> Outcome {
    let Ok(message) = frame::decode_payload::<M>(data) else {
        return Refused;
    };
    assert!(
        reference::is_one_item(data),
        "accepted a payload that is not one CBOR item: {}",
        data_encoding::HEXLOWER.encode(data)
    );
    message
        .validate()
        .expect("what decodes passes its own checks");
    let written = frame::encode(&message).expect("a message that decoded can be encoded");
    assert_eq!(
        frame::decode::<M>(&written).expect("an encoded message decodes"),
        message
    );
    if written[frame::HEADER_LEN..] == *data {
        Written
    } else {
        Read
    }
}

#[test]
fn the_committed_fuzz_seeds_are_still_what_they_are_named_for() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../baton-proto/fuzz/seeds/stream");
    let mut files = BTreeSet::new();
    for entry in std::fs::read_dir(&directory).unwrap() {
        let entry = entry.unwrap();
        files.insert(entry.file_name().into_string().unwrap());
    }
    let listed: BTreeSet<String> = SEEDS.iter().map(|(name, _)| (*name).to_owned()).collect();
    let differ: Vec<_> = listed.symmetric_difference(&files).collect();
    assert!(
        differ.is_empty(),
        "the table and the seed files differ in {differ:?}"
    );

    for (name, expected) in SEEDS {
        let bytes = std::fs::read(directory.join(name)).unwrap();
        let (mode, data) = bytes.split_first().expect("a seed is not empty");
        assert_eq!(mode % 4, part(name), "the seed {name} picks another part");
        let got = match mode % 4 {
            0 => start(data),
            1 => header(data),
            2 => message::<SourceMessage>(data),
            _ => message::<ViewerMessage>(data),
        };
        assert_eq!(
            got, *expected,
            "the seed {name} is not what it is named for"
        );
    }
}
