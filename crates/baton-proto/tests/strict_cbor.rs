// SPDX-License-Identifier: Apache-2.0

//! The decoders never accept a payload that is not exactly one well-formed CBOR
//! item.
//!
//! This is checked against an independent reference (`support/strict_cbor.rs`)
//! on payloads made by damaging valid messages: every byte replaced, removed or
//! inserted, and every prefix. Random bytes almost never form CBOR, so they
//! cannot reach deep into the decoders; damaged valid messages can.

// Tests may unwrap and index freely. Clippy exempts #[test] functions and #[cfg(test)]
// modules, but not the helper functions of an integration test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "support/strict_cbor.rs"]
mod reference;

use baton_proto::{
    DeviceId, DeviceInfo, ItemId, Platform, frame,
    message::{
        ClipboardText, DeclineReason, ErrorCode, ErrorReply, Handoff, HandoffReply, Hello, Item,
        Link, Outcome, Ping, Request, Response,
    },
    pair::{PairProof, PairRejection, PairRequest, PairResponse, PairSecret},
};

fn device() -> DeviceInfo {
    DeviceInfo {
        name: "Kitchen laptop".into(),
        platform: Platform::Linux,
        app_version: "0.1.0".into(),
    }
}

/// Valid payloads of every message type, covering every variant and field kind.
fn valid_payloads() -> Vec<(&'static str, Vec<u8>)> {
    fn payload<M: baton_proto::message::Message>(message: &M) -> Vec<u8> {
        frame::encode(message).unwrap()[frame::HEADER_LEN..].to_vec()
    }
    let id = ItemId::from_bytes([0x42; 16]);
    let hello = Hello {
        protocol_minor: 0,
        device: device(),
        capabilities: vec!["handoff.link".into(), "handoff.clipboard.text".into()],
    };
    let proof = PairProof::compute(
        &PairSecret::from_bytes([1; 32]),
        &DeviceId::from_bytes([2; 32]),
        &DeviceId::from_bytes([3; 32]),
        &[9; 32],
    );
    vec![
        ("request hello", payload(&Request::Hello(hello.clone()))),
        (
            "request ping",
            payload(&Request::Ping(Ping { nonce: u64::MAX })),
        ),
        (
            "request link",
            payload(&Request::Handoff(Handoff {
                id,
                item: Item::Link(Link {
                    url: "https://example.com/a?b=c".into(),
                    title: Some("Example".into()),
                }),
            })),
        ),
        (
            "request clipboard",
            payload(&Request::Handoff(Handoff {
                id,
                item: Item::ClipboardText(ClipboardText {
                    text: "hello".into(),
                }),
            })),
        ),
        ("response hello", payload(&Response::Hello(hello))),
        ("response pong", payload(&Response::Pong(Ping { nonce: 7 }))),
        (
            "response accepted",
            payload(&Response::Handoff(HandoffReply {
                id,
                outcome: Outcome::Accepted,
            })),
        ),
        (
            "response declined",
            payload(&Response::Handoff(HandoffReply {
                id,
                outcome: Outcome::Declined(DeclineReason::TooLarge),
            })),
        ),
        (
            "response error",
            payload(&Response::Error(ErrorReply {
                code: ErrorCode::Malformed,
                message: "bad".into(),
            })),
        ),
        (
            "pair request",
            payload(&PairRequest {
                device: device(),
                proof,
            }),
        ),
        (
            "pair accepted",
            payload(&PairResponse::Accepted { device: device() }),
        ),
        (
            "pair rejected",
            payload(&PairResponse::Rejected(PairRejection::Declined)),
        ),
    ]
}

/// Decodes `bytes` as every message type and checks the invariant.
fn check(label: &str, bytes: &[u8]) {
    let accepted = frame::decode_payload::<Request>(bytes).is_ok()
        || frame::decode_payload::<Response>(bytes).is_ok()
        || frame::decode_payload::<PairRequest>(bytes).is_ok()
        || frame::decode_payload::<PairResponse>(bytes).is_ok();
    assert!(
        !accepted || reference::is_one_item(bytes),
        "{label}: accepted a payload that is not one CBOR item: {}",
        data_encoding::HEXLOWER.encode(bytes)
    );
}

#[test]
fn valid_messages_are_accepted_by_the_reference_too() {
    for (name, payload) in valid_payloads() {
        assert!(reference::is_one_item(&payload), "{name}");
    }
}

#[test]
fn no_damaged_message_is_accepted_unless_it_is_still_cbor() {
    // Bytes that change the shape of an item: every major type with small, large
    // and indefinite lengths, simple values and the break code.
    const SHAPES: [u8; 24] = [
        0x00, 0x01, 0x18, 0x1f, 0x20, 0x40, 0x41, 0x5f, 0x60, 0x61, 0x7f, 0x80, 0x81, 0x9f, 0xa0,
        0xa1, 0xa2, 0xbf, 0xc0, 0xf4, 0xf6, 0xf7, 0xf8, 0xff,
    ];
    for (name, payload) in valid_payloads() {
        for i in 0..payload.len() {
            for &b in &SHAPES {
                let mut replaced = payload.clone();
                replaced[i] = b;
                check(&format!("{name}: byte {i} -> {b:#04x}"), &replaced);

                let mut inserted = payload.clone();
                inserted.insert(i, b);
                check(&format!("{name}: insert {b:#04x} at {i}"), &inserted);
            }
            let mut removed = payload.clone();
            removed.remove(i);
            check(&format!("{name}: remove byte {i}"), &removed);
            check(&format!("{name}: cut at {i}"), &payload[..i]);
        }
    }
}
