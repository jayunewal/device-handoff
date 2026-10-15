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
    Platform, TransferId, frame,
    message::{DeclineReason, ErrorCode, Hello, Outcome, Ping, Request, Response, capability},
    pair::{PairRejection, PairRequest, PairResponse},
    transfer::{FailReason, TransferReply, TransferRequest, list::ListChunk},
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

fn transfer_id() -> Value {
    Value::Bytes(vec![0x24; 16])
}

#[test]
fn a_device_that_takes_part_in_transfers_says_so_in_its_capabilities() {
    // The capability is what a sender checks before it makes an offer: a device that does
    // not know the request would answer it with an error, and an offer is not worth that.
    assert_eq!(capability::TRANSFER_FILES, "transfer.files");
}

#[test]
fn unknown_keys_are_ignored_in_the_messages_of_a_transfer() {
    // Next to the keys that are read: in the request, in the summary and in a preview item.
    // The keys that stand for nothing (`folder`, `files`) are absent, which means false and 0.
    let offer = encode(
        &cbor!({
            "Transfer" => {
                "Offer" => {
                    "later" => [1, 2, { "x" => null }],
                    "id" => transfer_id(),
                    "summary" => {
                        "items" => 1, "files" => 1, "folders" => 0, "bytes" => 5, "later" => "x",
                    },
                    "preview" => [{ "name" => "a.txt", "bytes" => 5, "later" => null }],
                    "list_hash" => Value::Bytes(vec![0x11; 32]),
                },
            },
        })
        .unwrap(),
    );
    let Request::Transfer(TransferRequest::Offer(offer)) =
        frame::decode_payload::<Request>(&offer).unwrap()
    else {
        panic!("not an offer");
    };
    assert_eq!(offer.id, TransferId::from_bytes([0x24; 16]));
    assert_eq!(offer.list_hash.as_bytes(), &[0x11; 32]);
    assert_eq!(offer.summary.bytes, 5);
    assert_eq!(offer.preview[0].name, "a.txt");
    assert!(!offer.preview[0].folder);
    assert_eq!(offer.preview[0].files, 0);

    // A piece of the list: unknown keys in the piece and in an entry, and no time.
    let list = encode(
        &cbor!({
            "later" => true,
            "entries" => [{ "path" => "a.txt", "size" => 5, "later" => [] }],
        })
        .unwrap(),
    );
    let list = frame::decode_payload::<ListChunk>(&list).unwrap();
    assert_eq!(list.entries[0].path, "a.txt");
    assert_eq!(list.entries[0].size, 5);
    assert_eq!(list.entries[0].modified_ms, None);

    // And in the outcome of a transfer, and in what it names.
    let done = encode(
        &cbor!({
            "Transfer" => {
                "Done" => {
                    "id" => transfer_id(),
                    "later" => 1,
                    "failed" => [{ "file" => 3, "reason" => "Skipped", "later" => 1 }],
                },
            },
        })
        .unwrap(),
    );
    let Response::Transfer(TransferReply::Done(done)) =
        frame::decode_payload::<Response>(&done).unwrap()
    else {
        panic!("not an outcome");
    };
    assert_eq!(done.failed[0].file, 3);
    assert_eq!(done.more_failed, 0);
}

#[test]
fn a_reason_a_transfer_does_not_know_is_other_and_one_with_data_is_an_error() {
    let declined = |reason: Value| {
        encode(
            &cbor!({
                "Transfer" => { "Declined" => { "id" => transfer_id(), "reason" => reason } },
            })
            .unwrap(),
        )
    };
    let done = |reason: Value| {
        encode(
            &cbor!({
                "Transfer" => {
                    "Done" => { "id" => transfer_id(), "failed" => [{ "file" => 1, "reason" => reason }] },
                },
            })
            .unwrap(),
        )
    };

    // Reasons that a later version adds are unit variants, and an older one reads them as Other.
    let Response::Transfer(TransferReply::Declined(declined_other)) =
        frame::decode_payload::<Response>(&declined(Value::Text("SomethingNew".into()))).unwrap()
    else {
        panic!("not a decline");
    };
    assert_eq!(declined_other.reason, DeclineReason::Other);
    // The two reasons that transfers added to the ones items had.
    for (name, reason) in [
        ("NoSpace", DeclineReason::NoSpace),
        ("Refused", DeclineReason::Refused),
    ] {
        let Response::Transfer(TransferReply::Declined(known)) =
            frame::decode_payload::<Response>(&declined(Value::Text(name.into()))).unwrap()
        else {
            panic!("not a decline");
        };
        assert_eq!(known.reason, reason);
    }
    let Response::Transfer(TransferReply::Done(done_other)) =
        frame::decode_payload::<Response>(&done(Value::Text("SomethingNew".into()))).unwrap()
    else {
        panic!("not an outcome");
    };
    assert_eq!(done_other.failed[0].reason, FailReason::Other);

    // A reason that carries data cannot be read at all, which is why new ones must not.
    let data = || cbor!({ "RetryAfter" => { "secs" => 5 } }).unwrap();
    assert!(frame::decode_payload::<Response>(&declined(data())).is_err());
    assert!(frame::decode_payload::<Response>(&done(data())).is_err());
}

#[test]
fn a_kind_of_transfer_request_this_version_does_not_know_is_an_error() {
    // Requests are not extensible: a sender asks only what the capabilities say is understood.
    let later = encode(&cbor!({ "Transfer" => { "Pause" => { "id" => transfer_id() } } }).unwrap());
    assert!(frame::decode_payload::<Request>(&later).is_err());
    let later =
        encode(&cbor!({ "Transfer" => { "Paused" => { "id" => transfer_id() } } }).unwrap());
    assert!(frame::decode_payload::<Response>(&later).is_err());
}

// The fuzz targets start from the seeds in `fuzz/seeds/decode_payload`, and a seed that is no
// longer a message (a rule was added that it breaks, a field was renamed) quietly stops the
// fuzzer from starting deep inside the decoders. The name of a seed says what it must be.
#[test]
fn the_committed_fuzz_seeds_are_still_the_messages_they_are_named_for() {
    let seeds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/decode_payload");
    let mut checked = 0;
    for entry in std::fs::read_dir(&seeds).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let bytes = std::fs::read(&path).unwrap();
        let decodes = if name.starts_with("request-") {
            frame::decode_payload::<Request>(&bytes).is_ok()
        } else if name.starts_with("response-") {
            frame::decode_payload::<Response>(&bytes).is_ok()
        } else if name == "pair-request" {
            frame::decode_payload::<PairRequest>(&bytes).is_ok()
        } else if name.starts_with("pair-") {
            frame::decode_payload::<PairResponse>(&bytes).is_ok()
        } else if name == "list-chunk" {
            frame::decode_payload::<ListChunk>(&bytes).is_ok()
        } else if name.starts_with("malformed-") {
            // Made to be refused, to show the fuzzer what a near miss looks like.
            continue;
        } else {
            panic!("the seed {name} has a name that says nothing about what it is");
        };
        assert!(
            decodes,
            "the seed {name} is not the message it is named for"
        );
        checked += 1;
    }
    assert!(checked >= 15, "only {checked} seeds were checked");
}
