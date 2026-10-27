// SPDX-License-Identifier: Apache-2.0

//! The seeds of the `stream` fuzz target (`crates/baton-proto/fuzz/seeds/stream`) are still what
//! they are named for.
//!
//! The target starts from them, and a seed that stops being what it was, because a rule was added
//! that it breaks or a field was renamed, quietly stops the fuzzer from starting deep inside the
//! readers. The first byte of a seed picks the part, as in the target, and the rest is what the part
//! reads. Each seed is listed here with what the part must make of it: refuse it, read it and write
//! it in another form, read it and write the same bytes, which is what a seed named for a message
//! must do if it is the message as the reference sends it, or give the size of the picture that a
//! parameter set describes. The table has no seed missing and none that is not a file.

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
    h264,
    header::{FrameHeader, PREFIX_LEN, decode_prefix, encode_prefix},
    sound::PacketHeader,
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
    /// It gives the width and the height of the picture that a sequence parameter set describes.
    Sized(u32, u32),
}
use Outcome::{Read, Refused, Sized, Written};

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
    ("picture-256-units", Written),
    ("picture-257-units", Refused),
    ("picture-delta-frame-from-an-encoder", Written),
    ("picture-key-frame-from-an-encoder", Written),
    ("picture-key-frame-without-parameter-sets", Read),
    ("picture-parameter-sets-only", Refused),
    ("picture-something-before-the-start-code", Refused),
    ("picture-start-codes-of-three-bytes-and-zeros-around-them", Written),
    ("picture-unit-with-the-forbidden-bit", Refused),
    ("picture-without-a-start-code", Refused),
    ("sample-cut-short", Refused),
    ("sample-delta-frame", Written),
    ("sample-empty-unit", Refused),
    ("sample-key-frame", Written),
    ("sample-key-frame-and-a-candidate-set-that-is-a-slice", Written),
    ("sample-key-frame-in-lengths-of-two-bytes", Written),
    ("sample-key-frame-with-parameter-sets-of-its-own", Written),
    ("sample-length-of-3-bytes", Refused),
    ("sample-of-parameter-sets-alone", Refused),
    ("sample-unit-that-ends-in-a-zero", Refused),
    ("sample-unit-with-a-start-code-inside", Refused),
    ("sample-unit-with-the-forbidden-bit", Refused),
    ("size-255-offsets-of-picture-order", Sized(1920, 1080)),
    ("size-256-offsets-of-picture-order", Refused),
    ("size-bit-depth-of-15-bits", Refused),
    ("size-chroma-format-4", Refused),
    ("size-constrained-baseline", Sized(176, 144)),
    ("size-cropped-away", Refused),
    ("size-cut-short", Refused),
    ("size-high-1920-by-1080-cropped-at-the-bottom", Sized(1920, 1080)),
    ("size-high-3840-by-2160", Sized(3840, 2160)),
    ("size-high-4-2-2", Sized(320, 240)),
    ("size-high-4-4-4-cropped", Sized(326, 242)),
    ("size-interlaced", Sized(320, 240)),
    ("size-main-64-by-48", Sized(64, 48)),
    ("size-monochrome", Sized(320, 240)),
    ("size-number-of-31-zeros-that-nothing-reads", Sized(1920, 1080)),
    ("size-number-of-32-zeros", Refused),
    ("size-number-that-does-not-end", Refused),
    ("size-picture-order-type-3", Refused),
    ("size-picture-parameter-set", Refused),
    ("size-scaling-lists-in-4-2-0", Sized(320, 240)),
    ("size-scaling-lists-in-4-4-4-cropped", Sized(316, 233)),
    ("size-wider-than-32-bits", Refused),
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
    ("source-sends-a-hear", Refused),
    ("source-sends-a-message-of-the-viewer", Refused),
    ("source-sends-an-unlock", Refused),
    ("source-sends-an-unlock-opened", Refused),
    ("source-silent", Written),
    ("source-silent-reason-of-a-later-version", Read),
    ("source-sound", Written),
    ("source-sound-a-field-of-a-later-version", Read),
    ("source-sound-channels-3", Refused),
    ("source-sound-codec-of-a-later-version", Read),
    ("source-sound-rate-too-high", Refused),
    ("source-sound-rate-too-low", Refused),
    ("source-sound-samples-too-few", Refused),
    ("source-sound-samples-too-many", Refused),
    ("source-unlock-done", Written),
    ("source-unlock-done-outcome-of-a-later-version", Read),
    ("source-unlock-done-without-a-number", Refused),
    ("source-unlock-done-without-an-outcome", Refused),
    ("source-unlock-sealed", Written),
    ("source-unlock-sealed-challenge-too-long", Refused),
    ("source-unlock-sealed-challenge-too-short", Refused),
    ("source-unlock-sealed-nothing-sealed", Refused),
    ("source-unlock-sealed-too-long", Refused),
    ("source-unlock-sealed-without-a-number", Refused),
    ("start-control-stream", Written),
    ("start-frame-of-video", Written),
    ("start-lane-of-a-transfer", Written),
    ("start-sound-packet", Written),
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
    ("sound-first-packet", Written),
    ("sound-header-cut-short", Refused),
    ("sound-header-of-a-frame-of-video", Refused),
    ("sound-packet-and-the-sound-behind-it", Written),
    ("sound-reserved-flag", Refused),
    ("viewer-hear", Written),
    ("viewer-hear-off", Written),
    ("viewer-hear-without-saying-whether", Refused),
    ("viewer-keyframe", Written),
    ("viewer-keyframe-of-the-biggest-frame", Written),
    ("viewer-lift", Written),
    ("viewer-lift-a-field-of-a-later-version", Read),
    ("viewer-lift-input-not-a-bool", Refused),
    ("viewer-lift-nothing-said", Read),
    ("viewer-report", Written),
    ("viewer-report-before-the-first-picture", Written),
    ("viewer-report-thermal-state-of-a-later-version", Read),
    ("viewer-sends-a-message-of-the-source", Refused),
    ("viewer-sends-a-sound", Refused),
    ("viewer-sends-an-unlock-done", Refused),
    ("viewer-sends-an-unlock-sealed", Refused),
    ("viewer-unlock", Written),
    ("viewer-unlock-as-a-map-with-null", Refused),
    ("viewer-unlock-as-bare-text", Refused),
    ("viewer-unlock-cancelled", Written),
    ("viewer-unlock-cancelled-reason-of-a-later-version", Read),
    ("viewer-unlock-number-not-a-number", Refused),
    ("viewer-unlock-opened", Written),
    ("viewer-unlock-opened-challenge-too-long", Refused),
    ("viewer-unlock-opened-key-too-short", Refused),
    ("viewer-unlock-opened-without-a-key", Refused),
    ("viewer-unlock-the-biggest-number", Written),
    ("viewer-unlock-without-a-number", Refused),
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
        Some("picture") => 4,
        Some("sample") => 5,
        Some("size") => 6,
        Some("sound") => 7,
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

fn packet(data: &[u8]) -> Outcome {
    let Ok((header, rest)) = PacketHeader::parse(data) else {
        return Refused;
    };
    assert_eq!(rest.len(), data.len() - PacketHeader::LEN);
    let written = header.encode();
    assert_eq!(written.as_slice(), &data[..PacketHeader::LEN]);
    assert_eq!(PacketHeader::parse(&written).unwrap().0, header);
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

/// A picture off the wire is made into a sample for a decoder, and the sample into a picture again.
fn picture(data: &[u8]) -> Outcome {
    let mut sample = Vec::new();
    let Ok(taken) = h264::annex_b_to_avcc(data, &mut sample) else {
        assert!(sample.is_empty(), "a refused picture left something behind");
        return Refused;
    };
    let sets: Vec<&[u8]> = taken.parameter_sets().collect();
    let mut wire = Vec::new();
    let Ok(summary) = h264::avcc_to_annex_b(&sample, h264::AVCC_LENGTH_SIZE, &sets, &mut wire)
    else {
        assert!(wire.is_empty(), "a refused sample left something behind");
        return Read;
    };
    assert_eq!(summary.idr, taken.idr);
    let mut again = Vec::new();
    let back = h264::annex_b_to_avcc(&wire, &mut again).unwrap();
    assert_eq!(again, sample);
    assert_eq!(back.idr, taken.idr);
    if taken.idr {
        assert_eq!(back, taken);
    }
    Written
}

/// What the part of the target for a sample makes of it with the parameter sets of the platform
/// (the first of its three tries, which is the one that a seed is named for), and the same checks.
fn sample(data: &[u8]) -> Outcome {
    const SPS: &[u8] = &[0x67, 0x42, 0xc0, 0x0a, 0xda, 0x7b, 0x01, 0x10];
    const PPS: &[u8] = &[0x68, 0xce, 0x0f, 0xc8];
    let ([length_size, cut], rest) = data.split_first_chunk::<2>().expect("a sample has a head");
    let length_size = usize::from(*length_size % 6);
    let (candidate, sample) = rest.split_at(usize::from(*cut) % (rest.len() + 1));
    let fixed: [&[u8]; 2] = [SPS, PPS];
    let with_the_candidate_first: [&[u8]; 2] = [candidate, PPS];
    let with_the_candidate_last: [&[u8]; 2] = [SPS, candidate];
    let mut outcome = None;
    for sets in [fixed, with_the_candidate_first, with_the_candidate_last] {
        let mut wire = Vec::new();
        let got = match h264::avcc_to_annex_b(sample, length_size, &sets, &mut wire) {
            Err(_) => {
                assert!(wire.is_empty(), "a refused sample left something behind");
                Refused
            }
            Ok(summary) => {
                let mut again = Vec::new();
                let back = h264::annex_b_to_avcc(&wire, &mut again).unwrap();
                assert_eq!(back.idr, summary.idr);
                assert!(back.sps.len() + back.pps.len() >= usize::from(summary.idr) * 2);
                Written
            }
        };
        outcome.get_or_insert(got);
    }
    outcome.unwrap()
}

/// What the part of the target for the size of a picture makes of a sequence parameter set, and the
/// same checks: a size is never nothing, and a set that is cut short gives that size or none.
fn size(data: &[u8]) -> Outcome {
    let size = h264::picture_size(data);
    if let Some((width, height)) = size {
        assert!(width > 0 && height > 0);
    }
    for length in 0..data.len().min(96) {
        let cut = h264::picture_size(&data[..length]);
        assert!(
            cut.is_none() || cut == size,
            "a set cut at {length} gave another size"
        );
    }
    size.map_or(Refused, |(width, height)| Sized(width, height))
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
        assert_eq!(mode % 8, part(name), "the seed {name} picks another part");
        let got = match mode % 8 {
            0 => start(data),
            1 => header(data),
            2 => message::<SourceMessage>(data),
            3 => message::<ViewerMessage>(data),
            4 => picture(data),
            5 => sample(data),
            6 => size(data),
            _ => packet(data),
        };
        assert_eq!(
            got, *expected,
            "the seed {name} is not what it is named for"
        );
    }
}
