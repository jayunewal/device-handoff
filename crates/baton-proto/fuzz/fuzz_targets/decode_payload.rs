// SPDX-License-Identifier: Apache-2.0

//! Feeds arbitrary bytes to every message decoder.
//!
//! Besides "never panics", it checks two properties that a plain crash search
//! would miss:
//!
//! - a decoder accepts a payload only if it is exactly one well-formed CBOR
//!   item, judged by an independent checker;
//! - whatever decodes can be encoded again and decodes to the same message.

#![no_main]

use baton_proto::{
    frame,
    message::{Message, Request, Response},
    pair::{PairRequest, PairResponse},
};
use libfuzzer_sys::fuzz_target;

#[path = "../../tests/support/strict_cbor.rs"]
mod reference;

fn round_trips<M: Message + PartialEq + std::fmt::Debug>(data: &[u8]) -> bool {
    let Ok(message) = frame::decode_payload::<M>(data) else {
        return false;
    };
    let frame = frame::encode(&message).expect("a message that decoded can be encoded");
    assert_eq!(
        frame::decode::<M>(&frame).expect("an encoded message decodes"),
        message
    );
    true
}

fuzz_target!(|data: &[u8]| {
    let mut accepted = round_trips::<Request>(data);
    accepted |= round_trips::<Response>(data);
    accepted |= round_trips::<PairResponse>(data);
    // `PairRequest` carries a proof, which deliberately cannot be compared with `==`.
    if let Ok(request) = frame::decode_payload::<PairRequest>(data) {
        accepted = true;
        let frame = frame::encode(&request).expect("a message that decoded can be encoded");
        let again = frame::decode::<PairRequest>(&frame).expect("an encoded message decodes");
        assert_eq!(again.device, request.device);
        assert_eq!(again.proof.as_bytes(), request.proof.as_bytes());
    }
    assert!(
        !accepted || reference::is_one_item(data),
        "accepted a payload that is not one CBOR item: {}",
        data_encoding::HEXLOWER.encode(data)
    );

    // Whole frames, header included.
    let _ = frame::decode::<Request>(data);
    let _ = frame::decode::<Response>(data);
});
