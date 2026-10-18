// SPDX-License-Identifier: Apache-2.0

//! Feeds arbitrary bytes to the parts of window streaming that read what a device on the network
//! sends: the start of a stream, the header of a frame of video, and the messages on the two
//! control streams.
//!
//! The first byte of the input picks the part. Besides "never panics", each part is checked for
//! what it promises:
//!
//! - a start or a header that is read can be written again, to the same bytes, and read again, to
//!   the same value;
//! - a message is accepted only if an independent CBOR checker agrees that its payload is exactly
//!   one well-formed item, what decodes passes its own checks, and it can be written again and
//!   read again, to the same message.

#![no_main]

use baton_proto::{frame, message::Message};
use baton_stream::{
    control::{SourceMessage, ViewerMessage},
    header::{FrameHeader, PREFIX_LEN, decode_prefix, encode_prefix},
};
use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/strict_cbor.rs"]
mod reference;

fuzz_target!(|data: &[u8]| {
    let Some((mode, data)) = data.split_first() else {
        return;
    };
    match mode % 4 {
        0 => start(data),
        1 => header(data),
        2 => message::<SourceMessage>(data),
        _ => message::<ViewerMessage>(data),
    }
});

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
