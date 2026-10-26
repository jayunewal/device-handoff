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
    Platform, SessionId, TransferId, frame,
    message::{DeclineReason, ErrorCode, Hello, Outcome, Ping, Request, Response, capability},
    pair::{PairRejection, PairRequest, PairResponse},
    stream::{Codec, SourceKind, StopReason, StreamReply, StreamRequest, Wanted, WhyNot},
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

fn session_id() -> Value {
    Value::Bytes(vec![0x5a; 16])
}

#[test]
fn a_device_that_takes_part_in_window_streaming_says_so_in_its_capabilities() {
    // The capabilities are what a source checks before it makes an offer, and what a viewer
    // checks before it shows a control: a device that does not know the request would answer it
    // with an error, and an offer is not worth that.
    assert_eq!(capability::STREAM_SHARE, "stream.share");
    assert_eq!(capability::STREAM_VIEW, "stream.view");
    assert_eq!(capability::STREAM_CONTROL, "stream.control");
    assert_eq!(capability::STREAM_PRIVATE, "stream.private");
    // And a source that starts a session when a paired device asks it to.
    assert_eq!(capability::STREAM_START, "stream.start");
}

#[test]
fn a_device_that_sends_or_plays_the_sound_of_a_screen_says_so_in_its_capabilities() {
    // A new kind of stream and new control messages are sent only to a device that advertised
    // the capability (section 5): a source checks `stream.hear` before it writes `Sound`, `Silent`
    // or a packet, and a viewer checks `stream.sound` before it writes `Hear`.
    assert_eq!(capability::STREAM_SOUND, "stream.sound");
    assert_eq!(capability::STREAM_HEAR, "stream.hear");
}

#[test]
fn a_device_that_can_be_unlocked_or_unlocks_with_a_face_says_so_in_its_capabilities() {
    // Messages about an unlock are sent only to a device that advertised the capability
    // (section 5): a viewer checks `unlock.target` before it writes `Unlock`, and a source checks
    // `unlock.approve` before it asks for a key or sends a question.
    assert_eq!(capability::UNLOCK_TARGET, "unlock.target");
    assert_eq!(capability::UNLOCK_APPROVE, "unlock.approve");
}

#[test]
fn a_display_is_a_kind_of_source_that_this_version_knows() {
    // The kind is the name of the variant, as text, and it is not the kind that is read for a
    // name this version does not know.
    let offer = |source: &str| {
        encode(
            &cbor!({
                "Stream" => {
                    "Offer" => {
                        "session" => session_id(),
                        "source" => source,
                        "title" => "",
                        "app" => "",
                        "width" => 3024,
                        "height" => 1964,
                    },
                },
            })
            .unwrap(),
        )
    };
    let Request::Stream(StreamRequest::Offer(display)) =
        frame::decode_payload::<Request>(&offer("Display")).unwrap()
    else {
        panic!("not an offer");
    };
    assert_eq!(display.source, SourceKind::Display);
    assert_eq!((display.title.as_str(), display.app.as_str()), ("", ""));
    assert_eq!((display.width, display.height), (3024, 1964));
    let Request::Stream(StreamRequest::Offer(window)) =
        frame::decode_payload::<Request>(&offer("Window")).unwrap()
    else {
        panic!("not an offer");
    };
    assert_eq!(window.source, SourceKind::Window);

    // And it is written the same way.
    let written = frame::encode(&Request::Stream(StreamRequest::Offer(display))).unwrap();
    assert!(
        written
            .windows(b"Display".len())
            .any(|bytes| bytes == b"Display"),
        "the kind is written as its name"
    );
}

#[test]
fn unknown_keys_are_ignored_in_the_messages_of_a_stream() {
    // The offer, with a key from a later version, and without `control`, which means false.
    let offer = encode(
        &cbor!({
            "Stream" => {
                "Offer" => {
                    "later" => [1, 2, { "x" => null }],
                    "session" => session_id(),
                    "source" => "Window",
                    "title" => "Report.pdf",
                    "app" => "Preview",
                    "width" => 1440,
                    "height" => 900,
                },
            },
        })
        .unwrap(),
    );
    let Request::Stream(StreamRequest::Offer(offer)) =
        frame::decode_payload::<Request>(&offer).unwrap()
    else {
        panic!("not an offer");
    };
    assert_eq!(offer.session, SessionId::from_bytes([0x5a; 16]));
    assert_eq!(offer.source, SourceKind::Window);
    assert_eq!((offer.width, offer.height), (1440, 900));
    assert!(!offer.control);
    assert!(!offer.private);

    // The answer, in the same way.
    let accepted = encode(
        &cbor!({
            "Stream" => {
                "Accepted" => {
                    "later" => "x",
                    "session" => session_id(),
                    "max_width" => 2732,
                    "max_height" => 2048,
                    "max_fps" => 60,
                    "codecs" => ["H264"],
                },
            },
        })
        .unwrap(),
    );
    let Response::Stream(StreamReply::Accepted(accepted)) =
        frame::decode_payload::<Response>(&accepted).unwrap()
    else {
        panic!("not an answer");
    };
    assert_eq!(accepted.max_fps, 60);
    assert_eq!(accepted.codecs, [Codec::H264]);

    // And the end of a session, and the reply to it.
    let stop = encode(
        &cbor!({
            "Stream" => {
                "Stop" => { "later" => 1, "session" => session_id(), "reason" => "Ended" },
            },
        })
        .unwrap(),
    );
    let Request::Stream(StreamRequest::Stop(stop)) =
        frame::decode_payload::<Request>(&stop).unwrap()
    else {
        panic!("not a stop");
    };
    assert_eq!(stop.reason, StopReason::Ended);
    let stopped = encode(
        &cbor!({ "Stream" => { "Stopped" => { "later" => 1, "session" => session_id() } } })
            .unwrap(),
    );
    let Response::Stream(StreamReply::Stopped(stopped)) =
        frame::decode_payload::<Response>(&stopped).unwrap()
    else {
        panic!("not a reply to a stop");
    };
    assert_eq!(stopped.session, SessionId::from_bytes([0x5a; 16]));
}

#[test]
fn unknown_keys_are_ignored_in_the_messages_that_start_a_session() {
    // The start, with a key from a later version and without `control`, which means false.
    let start = encode(
        &cbor!({
            "Stream" => {
                "Start" => { "later" => [1, 2], "session" => session_id(), "wanted" => "Screen" },
            },
        })
        .unwrap(),
    );
    let Request::Stream(StreamRequest::Start(start)) =
        frame::decode_payload::<Request>(&start).unwrap()
    else {
        panic!("not a start");
    };
    assert_eq!(start.session, SessionId::from_bytes([0x5a; 16]));
    assert_eq!(start.wanted, Wanted::Screen);
    assert!(!start.control);

    // And the two answers to it.
    let starting = encode(
        &cbor!({ "Stream" => { "Starting" => { "later" => 1, "session" => session_id() } } })
            .unwrap(),
    );
    let Response::Stream(StreamReply::Starting(starting)) =
        frame::decode_payload::<Response>(&starting).unwrap()
    else {
        panic!("not a reply to a start");
    };
    assert_eq!(starting.session, SessionId::from_bytes([0x5a; 16]));
    let not_started = encode(
        &cbor!({
            "Stream" => {
                "NotStarted" => { "later" => 1, "session" => session_id(), "why" => "Busy" },
            },
        })
        .unwrap(),
    );
    let Response::Stream(StreamReply::NotStarted(not_started)) =
        frame::decode_payload::<Response>(&not_started).unwrap()
    else {
        panic!("not a refusal of a start");
    };
    assert_eq!(not_started.why, WhyNot::Busy);
    // The reason for a source that cannot hold its keyboard and pointer is written by its name.
    let held = encode(
        &cbor!({
            "Stream" => {
                "NotStarted" => {
                    "session" => session_id(), "why" => "NeedsInputPermission",
                },
            },
        })
        .unwrap(),
    );
    let Response::Stream(StreamReply::NotStarted(held)) =
        frame::decode_payload::<Response>(&held).unwrap()
    else {
        panic!("not a refusal of a start");
    };
    assert_eq!(held.why, WhyNot::NeedsInputPermission);
}

#[test]
fn a_kind_a_stream_does_not_know_is_other_and_one_with_data_is_an_error() {
    let offer = |source: Value| {
        encode(
            &cbor!({
                "Stream" => {
                    "Offer" => {
                        "session" => session_id(), "source" => source, "title" => "t",
                        "app" => "a", "width" => 10, "height" => 10,
                    },
                },
            })
            .unwrap(),
        )
    };
    let accepted = |codec: Value| {
        encode(
            &cbor!({
                "Stream" => {
                    "Accepted" => {
                        "session" => session_id(), "max_width" => 10, "max_height" => 10,
                        "max_fps" => 30, "codecs" => [codec, "H264"],
                    },
                },
            })
            .unwrap(),
        )
    };
    let stop = |reason: Value| {
        encode(
            &cbor!({ "Stream" => { "Stop" => { "session" => session_id(), "reason" => reason } } })
                .unwrap(),
        )
    };
    let declined = |reason: Value| {
        encode(
            &cbor!({
                "Stream" => { "Declined" => { "session" => session_id(), "reason" => reason } },
            })
            .unwrap(),
        )
    };

    let start = |wanted: Value| {
        encode(
            &cbor!({
                "Stream" => { "Start" => { "session" => session_id(), "wanted" => wanted } },
            })
            .unwrap(),
        )
    };
    let not_started = |why: Value| {
        encode(
            &cbor!({
                "Stream" => { "NotStarted" => { "session" => session_id(), "why" => why } },
            })
            .unwrap(),
        )
    };

    // What a later version adds is a unit variant, and this one reads it as Other: a source it
    // does not know is declined as unsupported, a codec is one that it will not choose.
    let Request::Stream(StreamRequest::Offer(other)) =
        frame::decode_payload::<Request>(&offer(Value::Text("Desktop".into()))).unwrap()
    else {
        panic!("not an offer");
    };
    assert_eq!(other.source, SourceKind::Other);
    let Response::Stream(StreamReply::Accepted(other)) =
        frame::decode_payload::<Response>(&accepted(Value::Text("Av1".into()))).unwrap()
    else {
        panic!("not an answer");
    };
    assert_eq!(other.codecs, [Codec::Other, Codec::H264]);
    let Request::Stream(StreamRequest::Stop(other)) =
        frame::decode_payload::<Request>(&stop(Value::Text("SomethingNew".into()))).unwrap()
    else {
        panic!("not a stop");
    };
    assert_eq!(other.reason, StopReason::Other);
    let Response::Stream(StreamReply::Declined(other)) =
        frame::decode_payload::<Response>(&declined(Value::Text("SomethingNew".into()))).unwrap()
    else {
        panic!("not a decline");
    };
    assert_eq!(other.reason, DeclineReason::Other);
    // What a viewer wants that this version does not know is something it can say no to, and a
    // reason for a no that it does not know is one it can still show as a no.
    let Request::Stream(StreamRequest::Start(other)) =
        frame::decode_payload::<Request>(&start(Value::Text("Window".into()))).unwrap()
    else {
        panic!("not a start");
    };
    assert_eq!(other.wanted, Wanted::Other);
    let Response::Stream(StreamReply::NotStarted(other)) =
        frame::decode_payload::<Response>(&not_started(Value::Text("Asleep".into()))).unwrap()
    else {
        panic!("not a refusal of a start");
    };
    assert_eq!(other.why, WhyNot::Other);

    // One that carries data cannot be read at all, which is why new ones must not.
    let data = || cbor!({ "Window" => { "id" => 1 } }).unwrap();
    assert!(frame::decode_payload::<Request>(&offer(data())).is_err());
    assert!(frame::decode_payload::<Response>(&accepted(data())).is_err());
    assert!(frame::decode_payload::<Request>(&stop(data())).is_err());
    assert!(frame::decode_payload::<Response>(&declined(data())).is_err());
    assert!(frame::decode_payload::<Request>(&start(data())).is_err());
    assert!(frame::decode_payload::<Response>(&not_started(data())).is_err());
}

#[test]
fn a_kind_of_stream_request_this_version_does_not_know_is_an_error() {
    // Requests are not extensible: a device asks only what the capabilities say is understood.
    let later =
        encode(&cbor!({ "Stream" => { "Pause" => { "session" => session_id() } } }).unwrap());
    assert!(frame::decode_payload::<Request>(&later).is_err());
    let later =
        encode(&cbor!({ "Stream" => { "Paused" => { "session" => session_id() } } }).unwrap());
    assert!(frame::decode_payload::<Response>(&later).is_err());
}

fn offer_that_says(unlock: Option<Value>) -> Vec<u8> {
    let mut offer = cbor!({
        "session" => session_id(), "source" => "Display", "title" => "", "app" => "",
        "width" => 3024, "height" => 1964, "control" => true, "private" => true,
    })
    .unwrap();
    if let (Some(unlock), Value::Map(fields)) = (unlock, &mut offer) {
        fields.push((Value::Text("unlock".into()), unlock));
    }
    encode(&cbor!({ "Stream" => { "Offer" => offer } }).unwrap())
}

#[test]
fn an_offer_that_does_not_say_unlock_does_not_offer_one() {
    // What a message leaves out stays as a session starts, which is the strict way: the viewer
    // has no unlock to ask for.
    let read = |unlock: Option<Value>| {
        let Request::Stream(StreamRequest::Offer(offer)) =
            frame::decode_payload::<Request>(&offer_that_says(unlock)).unwrap()
        else {
            panic!("not an offer");
        };
        offer
    };
    assert!(!read(None).unlock);
    assert!(!read(Some(Value::Bool(false))).unlock);
    assert!(read(Some(Value::Bool(true))).unlock);
    // A word that is not a yes or a no is not a yes.
    for wrong in [Value::from(1), Value::Text("true".into()), Value::Null] {
        assert!(
            frame::decode_payload::<Request>(&offer_that_says(Some(wrong.clone()))).is_err(),
            "{wrong:?}"
        );
    }
}

#[test]
fn unknown_keys_are_ignored_in_the_messages_of_an_unlock() {
    use baton_proto::unlock::{KeyRequest, Sealing, UnlockReply, UnlockRequest};

    // The request for a key, with a key from a later version, without `renew` (which means
    // false) and with it.
    let request = |renew: Option<bool>| {
        let mut fields = vec![(Value::Text("later".into()), cbor!([1, 2]).unwrap())];
        if let Some(renew) = renew {
            fields.push((Value::Text("renew".into()), Value::Bool(renew)));
        }
        encode(&cbor!({ "Unlock" => { "Key" => Value::Map(fields) } }).unwrap())
    };
    for (renew, read) in [(None, false), (Some(false), false), (Some(true), true)] {
        assert_eq!(
            frame::decode_payload::<Request>(&request(renew)).unwrap(),
            Request::Unlock(UnlockRequest::Key(KeyRequest { renew: read })),
            "{renew:?}"
        );
    }
    // `renew` is a yes or a no and nothing else.
    for wrong in [Value::from(1), Value::Text("true".into()), Value::Null] {
        let bad = encode(&cbor!({ "Unlock" => { "Key" => { "renew" => wrong } } }).unwrap());
        assert!(frame::decode_payload::<Request>(&bad).is_err());
    }

    // The answer, in the same way.
    let reply = encode(
        &cbor!({
            "Unlock" => {
                "Key" => {
                    "later" => { "x" => null },
                    "sealing" => "EciesP256",
                    "public" => Value::Bytes(vec![4; 65]),
                },
            },
        })
        .unwrap(),
    );
    let Response::Unlock(UnlockReply::Key(key)) =
        frame::decode_payload::<Response>(&reply).unwrap()
    else {
        panic!("not a key");
    };
    assert_eq!(key.sealing, Sealing::EciesP256);
    assert_eq!(key.public.as_bytes(), [4; 65]);

    // The words of a refusal are those of any other request.
    let declined =
        |reason: Value| encode(&cbor!({ "Unlock" => { "Declined" => reason } }).unwrap());
    assert_eq!(
        frame::decode_payload::<Response>(&declined(Value::Text("Unsupported".into()))).unwrap(),
        Response::Unlock(UnlockReply::Declined(DeclineReason::Unsupported))
    );
}

#[test]
fn a_way_of_sealing_that_an_unlock_does_not_know_is_other_and_one_with_data_is_an_error() {
    use baton_proto::unlock::{Sealing, UnlockReply};

    let key = |sealing: Value| {
        encode(
            &cbor!({
                "Unlock" => { "Key" => { "sealing" => sealing, "public" => Value::Bytes(vec![1]) } },
            })
            .unwrap(),
        )
    };
    let Response::Unlock(UnlockReply::Key(other)) =
        frame::decode_payload::<Response>(&key(Value::Text("Lattice".into()))).unwrap()
    else {
        panic!("not a key");
    };
    assert_eq!(other.sealing, Sealing::Other);
    // A reason for a refusal that it does not know is one it can still show as a no.
    let declined = encode(&cbor!({ "Unlock" => { "Declined" => "SomethingNew" } }).unwrap());
    assert_eq!(
        frame::decode_payload::<Response>(&declined).unwrap(),
        Response::Unlock(UnlockReply::Declined(DeclineReason::Other))
    );
    // One that carries data cannot be read at all, which is why new ones must not.
    let data = cbor!({ "Lattice" => { "n" => 1 } }).unwrap();
    assert!(frame::decode_payload::<Response>(&key(data.clone())).is_err());
    let declined = encode(&cbor!({ "Unlock" => { "Declined" => data } }).unwrap());
    assert!(frame::decode_payload::<Response>(&declined).is_err());
}

#[test]
fn a_kind_of_unlock_request_this_version_does_not_know_is_an_error() {
    // Requests are not extensible: a device asks only what the capabilities say is understood.
    let later = encode(&cbor!({ "Unlock" => { "Forget" => {} } }).unwrap());
    assert!(frame::decode_payload::<Request>(&later).is_err());
    let later = encode(&cbor!({ "Unlock" => { "Forgotten" => {} } }).unwrap());
    assert!(frame::decode_payload::<Response>(&later).is_err());
    // And the two directions do not read each other's messages.
    let key = encode(&cbor!({ "Unlock" => { "Key" => {} } }).unwrap());
    assert!(frame::decode_payload::<Request>(&key).is_ok());
    assert!(frame::decode_payload::<Response>(&key).is_err());
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
