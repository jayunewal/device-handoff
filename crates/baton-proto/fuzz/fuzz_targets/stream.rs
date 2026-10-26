// SPDX-License-Identifier: Apache-2.0

//! Feeds arbitrary bytes to the parts of window streaming that read what a device on the network
//! sends: the start of a stream, the header of a frame of video, the header of a packet of sound,
//! the messages on the two control streams, and the picture of a frame. It also feeds the sample of
//! an encoder to the part that makes a picture of it, which is the only part of the source that
//! takes bytes it did not write, and the parameter set of a key frame to the part that reads the
//! size of its pictures.
//!
//! The first byte of the input picks the part. Besides "never panics", each part is checked for
//! what it promises:
//!
//! - a start or a header, of a frame or of a packet of sound, that is read can be written again, to
//!   the same bytes, and read again, to the same value;
//! - a message is accepted only if an independent CBOR checker agrees that its payload is exactly
//!   one well-formed item, what decodes passes its own checks, and it can be written again and
//!   read again, to the same message;
//! - a picture that is read leaves a sample that a decoder takes, and when the sample is made into
//!   a picture again, the picture is read as the same sample; a picture that is refused leaves the
//!   output as it was;
//! - a sample that is made into a picture is read back as the units that were written, and one that
//!   is refused leaves the output as it was;
//! - a parameter set gives a size that is not nothing, and one that is cut short gives that size or
//!   none.

#![no_main]

use baton_proto::{frame, message::Message};
use baton_stream::{
    control::{SourceMessage, ViewerMessage},
    h264,
    header::{FrameHeader, PREFIX_LEN, decode_prefix, encode_prefix},
    sound::PacketHeader,
};
use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/strict_cbor.rs"]
mod reference;

fuzz_target!(|data: &[u8]| {
    let Some((mode, data)) = data.split_first() else {
        return;
    };
    match mode % 8 {
        0 => start(data),
        1 => header(data),
        2 => message::<SourceMessage>(data),
        3 => message::<ViewerMessage>(data),
        4 => picture(data),
        5 => sample(data),
        6 => size(data),
        _ => packet(data),
    }
});

/// The parameter sets that a platform would hand over with a sample, for the part that makes a
/// picture of one.
const SPS: &[u8] = &[0x67, 0x42, 0xc0, 0x0a, 0xda, 0x7b, 0x01, 0x10];
const PPS: &[u8] = &[0x68, 0xce, 0x0f, 0xc8];

fn start(data: &[u8]) {
    let Some(bytes) = data.first_chunk::<PREFIX_LEN>() else {
        return;
    };
    let (kind, session) = decode_prefix(bytes);
    assert_eq!(&encode_prefix(kind, session), bytes);
}

fn header(data: &[u8]) {
    let Ok((header, rest)) = FrameHeader::parse(data) else {
        return;
    };
    assert_eq!(rest.len(), data.len() - FrameHeader::LEN);
    let written = header
        .encode()
        .expect("a header that was read can be written");
    assert_eq!(written.as_slice(), &data[..FrameHeader::LEN]);
    let again = FrameHeader::parse(&written).expect("a header that was written can be read");
    assert_eq!(again.0, header);
}

fn packet(data: &[u8]) {
    let Ok((header, rest)) = PacketHeader::parse(data) else {
        return;
    };
    assert_eq!(rest.len(), data.len() - PacketHeader::LEN);
    let written = header.encode();
    assert_eq!(written.as_slice(), &data[..PacketHeader::LEN]);
    let again = PacketHeader::parse(&written).expect("a header that was written can be read");
    assert_eq!(again.0, header);
}

fn message<M: Message + PartialEq + std::fmt::Debug>(data: &[u8]) {
    // Whole frames, header included.
    let _ = frame::decode::<M>(data);
    let Ok(message) = frame::decode_payload::<M>(data) else {
        return;
    };
    assert!(
        reference::is_one_item(data),
        "accepted a payload that is not one CBOR item: {}",
        data_encoding::HEXLOWER.encode(data)
    );
    message
        .validate()
        .expect("what decodes passes its own checks");
    let frame = frame::encode(&message).expect("a message that decoded can be encoded");
    assert_eq!(
        frame::decode::<M>(&frame).expect("an encoded message decodes"),
        message
    );
}

fn picture(data: &[u8]) {
    let mut sample = Vec::new();
    let Ok(taken) = h264::annex_b_to_avcc(data, &mut sample) else {
        assert!(sample.is_empty(), "a refused picture left something behind");
        return;
    };
    // Back to a picture, with the parameter sets that came with it. A key frame without both
    // kinds of them is refused, and so are units that the wire could not tell apart.
    let sets: Vec<&[u8]> = taken.parameter_sets().collect();
    let mut wire = Vec::new();
    let Ok(summary) = h264::avcc_to_annex_b(&sample, h264::AVCC_LENGTH_SIZE, &sets, &mut wire)
    else {
        assert!(wire.is_empty(), "a refused sample left something behind");
        return;
    };
    assert_eq!(summary.idr, taken.idr);
    let mut again = Vec::new();
    let back = h264::annex_b_to_avcc(&wire, &mut again)
        .expect("a picture that was written can be read");
    assert_eq!(again, sample);
    assert_eq!(back.idr, taken.idr);
    if taken.idr {
        assert_eq!(back, taken);
    }
}

/// The input is the length of the length field, the length of a parameter set that may be
/// nonsense, that set, and the sample.
fn sample(data: &[u8]) {
    let Some(([length_size, cut], rest)) = data.split_first_chunk::<2>() else {
        return;
    };
    let length_size = usize::from(*length_size % 6);
    let (candidate, sample) = rest.split_at(usize::from(*cut) % (rest.len() + 1));
    let fixed: [&[u8]; 2] = [SPS, PPS];
    let with_the_candidate_first: [&[u8]; 2] = [candidate, PPS];
    let with_the_candidate_last: [&[u8]; 2] = [SPS, candidate];
    for sets in [fixed, with_the_candidate_first, with_the_candidate_last] {
        let mut wire = Vec::new();
        let Ok(summary) = h264::avcc_to_annex_b(sample, length_size, &sets, &mut wire) else {
            assert!(wire.is_empty(), "a refused sample left something behind");
            continue;
        };
        let mut again = Vec::new();
        let back = h264::annex_b_to_avcc(&wire, &mut again)
            .expect("a picture that was written can be read");
        assert_eq!(back.idr, summary.idr);
        assert!(back.sps.len() + back.pps.len() >= usize::from(summary.idr) * 2);
    }
}

/// The input is a sequence parameter set, as a key frame carries it.
fn size(data: &[u8]) {
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
}
