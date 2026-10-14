// SPDX-License-Identifier: Apache-2.0

//! The compatibility rules of the wire format (docs/protocol.md sections 2 and 5),
//! tested with messages written the way a newer or a different implementation
//! might write them.

// Tests may unwrap and index freely. Clippy exempts #[test] functions and #[cfg(test)]
// modules, but not the helper functions of an integration test.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use baton_proto::{
    Platform, frame,
    message::{DeclineReason, ErrorCode, Hello, Outcome, Ping, Request, Response},
    pair::{PairRejection, PairResponse},
};
use ciborium::{Value, cbor};

fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).unwrap();
    out
}

fn item_id() -> Value {
    Value::Bytes(vec![0x42; 16])
}

fn declined(reason: &Value) -> Vec<u8> {
    encode(
        &cbor!({
            "Handoff" => { "id" => item_id(), "outcome" => { "Declined" => reason } },
        })
        .unwrap(),
    )
}

fn error_with_code(code: &Value) -> Vec<u8> {
    encode(&cbor!({ "Error" => { "code" => code, "message" => "m" } }).unwrap())
}

fn hello_on(platform: &Value) -> Vec<u8> {
    encode(
        &cbor!({
            "Hello" => {
                "protocol_minor" => 0,
                "device" => { "name" => "n", "platform" => platform, "app_version" => "v" },
                "capabilities" => [],
            },
        })
        .unwrap(),
    )
}

fn outcome_of(payload: &[u8]) -> Outcome {
    match frame::decode_payload::<Response>(payload).unwrap() {
        Response::Handoff(reply) => reply.outcome,
        other => panic!("expected a handoff reply, got {other:?}"),
    }
}

#[test]
fn unknown_keys_are_ignored_in_every_struct() {
    let ping =
        encode(&cbor!({ "Ping" => { "nonce" => 1, "later" => [1, 2, { "x" => null }] } }).unwrap());
    assert_eq!(
        frame::decode_payload::<Request>(&ping).unwrap(),
        Request::Ping(Ping { nonce: 1 })
    );

    // Inside a nested struct, and next to the keys that are read.
    let hello = encode(
        &cbor!({
            "Hello" => {
                "later" => 1,
                "protocol_minor" => 3,
                "device" => {
                    "name" => "n", "later" => "x", "platform" => "Linux", "app_version" => "v",
                },
                "capabilities" => ["handoff.link"],
            },
        })
        .unwrap(),
    );
    let Request::Hello(Hello {
        protocol_minor,
        device,
        capabilities,
    }) = frame::decode_payload::<Request>(&hello).unwrap()
    else {
        panic!("not a hello");
    };
    assert_eq!(protocol_minor, 3);
    assert_eq!(device.platform, Platform::Linux);
    assert_eq!(capabilities, ["handoff.link"]);

    let reply = encode(
        &cbor!({
            "Handoff" => { "later" => 1, "id" => item_id(), "outcome" => "Accepted" },
        })
        .unwrap(),
    );
    assert_eq!(outcome_of(&reply), Outcome::Accepted);
}

#[test]
fn unknown_unit_variants_of_extensible_enums_decode_as_other() {
    assert_eq!(
        outcome_of(&declined(&Value::Text("SomethingNew".into()))),
        Outcome::Declined(DeclineReason::Other)
    );

    let Response::Error(reply) =
        frame::decode_payload::<Response>(&error_with_code(&Value::Text("SomethingNew".into())))
            .unwrap()
    else {
        panic!("not an error reply");
    };
    assert_eq!(reply.code, ErrorCode::Other);

    let rejected = encode(&cbor!({ "Rejected" => "SomethingNew" }).unwrap());
    assert_eq!(
        frame::decode_payload::<PairResponse>(&rejected).unwrap(),
        PairResponse::Rejected(PairRejection::Other)
    );

    let Request::Hello(hello) =
        frame::decode_payload::<Request>(&hello_on(&Value::Text("PlanNine".into()))).unwrap()
    else {
        panic!("not a hello");
    };
    assert_eq!(hello.device.platform, Platform::Other);
}

#[test]
fn a_new_variant_that_carries_data_is_an_error_not_other() {
    // Why protocol.md says new variants of the extensible enums must be unit variants.
    let data = || cbor!({ "RetryAfter" => { "secs" => 5 } }).unwrap();
    assert!(frame::decode_payload::<Response>(&declined(&data())).is_err());
    assert!(frame::decode_payload::<Response>(&error_with_code(&data())).is_err());
    assert!(
        frame::decode_payload::<PairResponse>(&encode(&cbor!({ "Rejected" => data() }).unwrap()))
            .is_err()
    );
    assert!(frame::decode_payload::<Request>(&hello_on(&data())).is_err());
}

#[test]
fn enums_that_are_not_extensible_reject_unknown_variants() {
    for payload in [
        encode(&cbor!({ "Frobnicate" => {} }).unwrap()),
        encode(&Value::Text("Frobnicate".into())),
        // A kind of item this version does not know about cannot be decoded at all.
        encode(
            &cbor!({
                "Handoff" => { "id" => item_id(), "item" => { "File" => { "name" => "a" } } },
            })
            .unwrap(),
        ),
    ] {
        assert!(frame::decode_payload::<Request>(&payload).is_err());
    }
    let unknown_response = encode(&cbor!({ "Frobnicate" => {} }).unwrap());
    assert!(frame::decode_payload::<Response>(&unknown_response).is_err());
}

#[test]
fn a_unit_variant_is_its_name_as_text_and_only_null_may_stand_in_for_nothing() {
    // The form the specification defines.
    let text =
        encode(&cbor!({ "Handoff" => { "id" => item_id(), "outcome" => "Accepted" } }).unwrap());
    assert_eq!(outcome_of(&text), Outcome::Accepted);

    // Some serializers write a unit variant as a one-entry map to null. Receivers accept
    // that, senders never produce it.
    let with_null = encode(
        &cbor!({ "Handoff" => { "id" => item_id(), "outcome" => { "Accepted" => null } } })
            .unwrap(),
    );
    assert_eq!(outcome_of(&with_null), Outcome::Accepted);

    // Anything else in that position is an error, wherever it points.
    for content in [cbor!(1), cbor!("id"), cbor!({}), cbor!([])] {
        let payload = encode(
            &cbor!({
                "Handoff" => {
                    "id" => item_id(),
                    "outcome" => { "Accepted" => content.unwrap() },
                },
            })
            .unwrap(),
        );
        assert!(frame::decode_payload::<Response>(&payload).is_err());
    }
}
