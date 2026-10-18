// SPDX-License-Identifier: Apache-2.0

//! Paired devices talking to each other, and what happens when they stop being paired.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use baton_core::{Error, Identity, link::PathKind};
use baton_proto::{
    ALPN_SESSION, DeviceId, SessionId, TransferId,
    message::{
        ClipboardText, DeclineReason, Hello, Item, Link, Outcome, Request, Response, capability,
    },
    stream::{SourceKind, Stop, StopReason, StreamReply, StreamRequest},
    transfer::{
        Cancel, Declined, Finish, Offer, PreviewItem, Summary, TransferReply, TransferRequest,
        list::ListHash,
    },
};
use common::{Raw, Spec, device, eventually, pair};

fn link(url: &str) -> Item {
    Item::Link(Link {
        url: url.to_owned(),
        title: None,
    })
}

fn text(text: &str) -> Item {
    Item::ClipboardText(ClipboardText {
        text: text.to_owned(),
    })
}

#[tokio::test]
async fn a_paired_device_answers_pings_and_reports_the_path() {
    let (a, b) = pair().await;
    let report = b.node.ping(&a.id).await.unwrap();
    assert!(report.rtt < Duration::from_secs(5));
    // Both devices are on this machine, which counts as the local network.
    assert_eq!(report.path, PathKind::Lan);
    // And the other direction works without any further setup.
    a.node.ping(&b.id).await.unwrap();
}

#[tokio::test]
async fn links_and_clipboard_text_arrive_in_order_with_the_senders_identity() {
    let (a, b) = pair().await;
    assert_eq!(
        b.node
            .send(&a.id, link("https://example.com/one"))
            .await
            .unwrap(),
        Outcome::Accepted
    );
    assert_eq!(
        b.node.send(&a.id, text("two")).await.unwrap(),
        Outcome::Accepted
    );
    a.inbox.wait_for(2).await;
    let items = a.inbox.items();
    assert_eq!(items[0], (b.id, link("https://example.com/one")));
    assert_eq!(items[1], (b.id, text("two")));
}

#[tokio::test]
async fn items_go_both_ways_over_one_pairing() {
    let (a, b) = pair().await;
    a.node.send(&b.id, text("to the phone")).await.unwrap();
    b.node.send(&a.id, text("to the laptop")).await.unwrap();
    a.inbox.wait_for(1).await;
    b.inbox.wait_for(1).await;
    assert_eq!(a.inbox.items()[0].1, text("to the laptop"));
    assert_eq!(b.inbox.items()[0].1, text("to the phone"));
}

#[tokio::test]
async fn a_second_item_reuses_the_connection() {
    let (a, b) = pair().await;
    b.node.send(&a.id, text("one")).await.unwrap();
    b.node.send(&a.id, text("two")).await.unwrap();
    b.node.ping(&a.id).await.unwrap();
    // One connection serves everything, seen from either end.
    assert_eq!(b.node.open_connections(&a.id), 1);
    assert_eq!(a.node.open_connections(&b.id), 1);
    a.inbox.wait_for(2).await;
}

#[tokio::test]
async fn a_kind_the_receiver_does_not_take_is_refused_before_it_is_sent() {
    let mut spec = Spec::new("Laptop");
    spec.accepts = &[capability::LINK];
    let a = spec.start().await;
    let b = device("Phone").await;
    b.pair_with(&a).await;

    let error = b.node.send(&a.id, text("secret")).await.unwrap_err();
    assert!(matches!(error, Error::Unsupported(_)), "{error}");
    // Links are still fine.
    b.node
        .send(&a.id, link("https://example.com"))
        .await
        .unwrap();
    a.inbox.wait_for(1).await;
    assert_eq!(a.inbox.items().len(), 1);
}

#[tokio::test]
async fn the_sender_will_not_send_more_than_its_own_limit() {
    let mut spec = Spec::new("Phone");
    spec.config.handoff.max_clipboard_bytes = 16;
    let b = spec.start().await;
    let a = device("Laptop").await;
    b.pair_with(&a).await;

    b.node.send(&a.id, text(&"x".repeat(16))).await.unwrap();
    let error = b.node.send(&a.id, text(&"x".repeat(17))).await.unwrap_err();
    assert!(matches!(error, Error::BadItem(_)), "{error}");
}

#[tokio::test]
async fn the_receiver_enforces_its_own_limits_whatever_the_sender_thinks() {
    let mut spec = Spec::new("Laptop");
    spec.config.handoff.max_clipboard_bytes = 16;
    spec.config.handoff.max_items_per_minute = 3;
    let a = spec.start().await;
    let b = device("Phone").await;
    b.pair_with(&a).await;

    assert_eq!(
        b.node.send(&a.id, text(&"x".repeat(100))).await.unwrap(),
        Outcome::Declined(DeclineReason::TooLarge)
    );
    for i in 0..3 {
        assert_eq!(
            b.node
                .send(&a.id, text(&format!("item {i}")))
                .await
                .unwrap(),
            Outcome::Accepted
        );
    }
    assert_eq!(
        b.node.send(&a.id, text("one too many")).await.unwrap(),
        Outcome::Declined(DeclineReason::RateLimited)
    );
    a.inbox.wait_for(3).await;
    assert_eq!(
        a.inbox.items().len(),
        3,
        "declined items must not be handled"
    );
}

#[tokio::test]
async fn sending_to_a_device_that_is_not_paired_fails_without_touching_the_network() {
    let a = device("Laptop").await;
    let stranger = DeviceId::from_bytes([9; 32]);
    let error = a.node.send(&stranger, text("hello")).await.unwrap_err();
    assert!(
        matches!(error, Error::NotPaired(id) if id == stranger),
        "{error}"
    );
    let error = a.node.ping(&stranger).await.unwrap_err();
    assert!(matches!(error, Error::NotPaired(_)), "{error}");
}

#[tokio::test]
async fn a_device_that_reaches_another_is_remembered_with_where_it_came_from() {
    let (a, b) = pair().await;
    a.trust
        .update(&b.id, |device| device.addrs.clear())
        .unwrap();

    b.node.ping(&a.id).await.unwrap();
    eventually("the address to be refreshed", || {
        !a.trust.get(&b.id).unwrap().addrs.is_empty()
    })
    .await;
}

#[tokio::test]
async fn a_paired_device_that_renames_itself_is_flagged_and_not_believed() {
    let a = device("Laptop").await;
    let identity = Identity::generate();
    let b = Spec::new("Phone")
        .start_with(tempfile::tempdir().unwrap(), &identity)
        .await;
    b.pair_with(&a).await;
    let b_id = b.id;
    assert_eq!(a.trust.get(&b_id).unwrap().display_name(), "Phone");

    // The same device comes back calling itself something else, which is what a rename
    // looks like, or what an attacker with the device's key and a better name looks like.
    let dir = b.stop().await;
    let renamed = Spec::new("Bank manager").start_with(dir, &identity).await;
    // What a peer says about itself is saved at most once a minute.
    a.clock.advance(120);
    renamed.node.ping(&a.id).await.unwrap();

    eventually("the new name to be noticed", || {
        a.trust.get(&b_id).unwrap().name_changed
    })
    .await;
    let known = a.trust.get(&b_id).unwrap();
    assert_eq!(known.name, "Bank manager", "what it says it is called");
    assert_eq!(
        known.display_name(),
        "Phone",
        "what the person knows it as does not change by itself"
    );
}

#[tokio::test]
async fn unpairing_closes_open_connections_at_once() {
    let (a, b) = pair().await;
    b.node.ping(&a.id).await.unwrap();
    assert_eq!(a.node.connected_peers(), vec![b.id]);

    a.trust.remove(&b.id).unwrap();
    eventually("the connection to close", || {
        a.node.connected_peers().is_empty()
    })
    .await;
    eventually("the other side to notice", || {
        b.node.connected_peers().is_empty()
    })
    .await;
}

#[tokio::test]
async fn a_device_that_was_unpaired_cannot_come_back_without_pairing_again() {
    let (a, b) = pair().await;
    b.node.send(&a.id, text("before")).await.unwrap();
    a.trust.remove(&b.id).unwrap();

    // B still thinks it is paired. A's answer is a clear refusal.
    let error = b.node.send(&a.id, text("after")).await.unwrap_err();
    assert!(matches!(error, Error::Closed { code: 403, .. }), "{error}");
    a.inbox.wait_for(1).await;
    assert_eq!(a.inbox.items().len(), 1);

    // Pairing again restores it.
    b.pair_with(&a).await;
    b.node.send(&a.id, text("again")).await.unwrap();
    a.inbox.wait_for(2).await;
}

#[tokio::test]
async fn unpairing_by_the_sender_stops_sends_immediately() {
    let (a, b) = pair().await;
    b.node.send(&a.id, text("one")).await.unwrap();
    b.trust.remove(&a.id).unwrap();
    let error = b.node.send(&a.id, text("two")).await.unwrap_err();
    assert!(matches!(error, Error::NotPaired(_)), "{error}");
    eventually("the connection to close", || {
        b.node.connected_peers().is_empty()
    })
    .await;
}

#[tokio::test]
async fn unused_connections_are_closed_and_reopened_on_demand() {
    let mut spec = Spec::new("Phone");
    spec.tuning.idle_connection = Duration::from_millis(300);
    let b = spec.start().await;
    let a = device("Laptop").await;
    b.pair_with(&a).await;

    b.node.send(&a.id, text("one")).await.unwrap();
    assert_eq!(a.node.connected_peers(), vec![b.id]);
    eventually("the idle connection to be closed", || {
        a.node.connected_peers().is_empty()
    })
    .await;

    b.node.send(&a.id, text("two")).await.unwrap();
    a.inbox.wait_for(2).await;
}

#[tokio::test]
async fn shutting_down_closes_connections_and_is_safe_to_repeat() {
    let (a, b) = pair().await;
    b.node.ping(&a.id).await.unwrap();
    a.node.shutdown().await;
    a.node.shutdown().await;
    eventually("the other side to notice", || {
        b.node.connected_peers().is_empty()
    })
    .await;
    let error = b.node.send(&a.id, text("anyone there")).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::Network { .. } | Error::Timeout(_) | Error::Closed { .. }
        ),
        "{error}"
    );
}

/// Dials `target` as a device that is paired with it, but speaks by hand.
async fn paired_raw(target: &common::Device) -> (Raw, iroh::endpoint::Connection) {
    let secret = iroh::SecretKey::generate();
    let raw = Raw::with_key(secret).await;
    target
        .trust
        .pair(raw.id, &common::info("Raw"), common::START)
        .unwrap();
    let connection = raw.dial(target, ALPN_SESSION).await.unwrap();
    (raw, connection)
}

/// Sends `bytes` on a fresh stream and returns whatever comes back.
async fn exchange_raw(
    connection: &iroh::endpoint::Connection,
    bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let (mut send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
    send.write_all(bytes).await.map_err(|e| e.to_string())?;
    send.finish().map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(10), recv.read_to_end(1 << 20))
        .await
        .map_err(|_| "no answer".to_owned())?
        .map_err(|e| e.to_string())
}

fn framed(request: &Request) -> Vec<u8> {
    baton_proto::frame::encode(request).unwrap()
}

#[tokio::test]
async fn hand_made_requests_get_the_same_answers_as_real_ones() {
    let a = device("Laptop").await;
    let (_raw, connection) = paired_raw(&a).await;
    let reply = exchange_raw(
        &connection,
        &framed(&Request::Ping(baton_proto::message::Ping { nonce: 77 })),
    )
    .await
    .unwrap();
    let response: Response = baton_proto::frame::decode(&reply).unwrap();
    assert_eq!(
        response,
        Response::Pong(baton_proto::message::Ping { nonce: 77 })
    );
}

#[tokio::test]
async fn a_device_that_does_not_take_part_in_transfers_declines_every_request_of_one() {
    let a = device("Laptop").await;
    let (_raw, connection) = paired_raw(&a).await;

    // It does not say that it does, so a sender has no reason to ask.
    let hello = Request::Hello(Hello {
        protocol_minor: 0,
        device: common::info("Raw"),
        capabilities: vec![],
    });
    let reply = exchange_raw(&connection, &framed(&hello)).await.unwrap();
    let Response::Hello(hello) = baton_proto::frame::decode::<Response>(&reply).unwrap() else {
        panic!("not a hello");
    };
    assert!(
        !hello
            .capabilities
            .iter()
            .any(|name| name == capability::TRANSFER_FILES),
        "{:?}",
        hello.capabilities
    );

    // A sender that asks anyway, with requests that are valid, is answered, not cut off.
    let id = TransferId::from_bytes([5; 16]);
    let offer = Offer {
        id,
        summary: Summary {
            items: 1,
            files: 1,
            folders: 0,
            bytes: 5,
        },
        preview: vec![PreviewItem {
            name: "a.txt".into(),
            folder: false,
            files: 0,
            bytes: 5,
        }],
        list_hash: ListHash::from_bytes([6; ListHash::LEN]),
    };
    for request in [
        TransferRequest::Offer(offer),
        TransferRequest::Finish(Finish {
            id,
            files: 1,
            bytes: 5,
        }),
        TransferRequest::Cancel(Cancel { id }),
    ] {
        let reply = exchange_raw(&connection, &framed(&Request::Transfer(request)))
            .await
            .unwrap();
        assert_eq!(
            baton_proto::frame::decode::<Response>(&reply).unwrap(),
            Response::Transfer(TransferReply::Declined(Declined {
                id,
                reason: DeclineReason::Unsupported,
            }))
        );
    }
}

#[tokio::test]
async fn a_device_that_does_not_take_part_in_window_streaming_declines_every_request_of_it() {
    let a = device("Laptop").await;
    let (_raw, connection) = paired_raw(&a).await;

    // It neither shares a window nor shows one, and it does not say that it does.
    let hello = Request::Hello(Hello {
        protocol_minor: 0,
        device: common::info("Raw"),
        capabilities: vec![],
    });
    let reply = exchange_raw(&connection, &framed(&hello)).await.unwrap();
    let Response::Hello(hello) = baton_proto::frame::decode::<Response>(&reply).unwrap() else {
        panic!("not a hello");
    };
    assert!(
        !hello
            .capabilities
            .iter()
            .any(|name| name.starts_with("stream.")),
        "{:?}",
        hello.capabilities
    );

    // A device that asks anyway, with requests that are valid, is answered, not cut off. The
    // answer to a stop is a decline too, as it is for the requests of a transfer.
    let session = SessionId::from_bytes([7; 16]);
    for request in [
        StreamRequest::Offer(baton_proto::stream::Offer {
            session,
            source: SourceKind::Window,
            title: "Report.pdf".into(),
            app: "Preview".into(),
            width: 1440,
            height: 900,
            control: true,
        }),
        StreamRequest::Stop(Stop {
            session,
            reason: StopReason::Ended,
        }),
    ] {
        let reply = exchange_raw(&connection, &framed(&Request::Stream(request)))
            .await
            .unwrap();
        assert_eq!(
            baton_proto::frame::decode::<Response>(&reply).unwrap(),
            Response::Stream(StreamReply::Declined(baton_proto::stream::Declined {
                session,
                reason: DeclineReason::Unsupported,
            }))
        );
    }
}

#[tokio::test]
async fn bad_requests_are_answered_or_cut_off_and_the_connection_survives() {
    let a = device("Laptop").await;
    let (_raw, connection) = paired_raw(&a).await;

    // A length header over the limit: the stream is reset with 400, nothing is read.
    let error = exchange_raw(&connection, &common::header(u32::MAX))
        .await
        .unwrap_err();
    assert!(error.contains("400"), "{error}");

    // A zero-length frame is just as wrong.
    let error = exchange_raw(&connection, &common::header(0))
        .await
        .unwrap_err();
    assert!(error.contains("400"), "{error}");

    // A well-framed payload that is not a request is answered with an error.
    let junk = b"definitely not CBOR";
    let mut bytes = common::header(u32::try_from(junk.len()).unwrap()).to_vec();
    bytes.extend_from_slice(junk);
    let reply = exchange_raw(&connection, &bytes).await.unwrap();
    let response: Response = baton_proto::frame::decode(&reply).unwrap();
    assert!(
        matches!(&response, Response::Error(e) if e.code == baton_proto::message::ErrorCode::Malformed),
        "{response:?}"
    );

    // A request that breaks a field limit is too.
    let too_long = Request::Handoff(baton_proto::message::Handoff {
        id: baton_proto::ItemId::from_bytes([1; 16]),
        item: link(&format!("https://example.com/{}", "x".repeat(10_000))),
    });
    let mut encoded = Vec::new();
    ciborium::into_writer(&too_long, &mut encoded).unwrap();
    let mut bytes = common::header(u32::try_from(encoded.len()).unwrap()).to_vec();
    bytes.extend_from_slice(&encoded);
    let reply = exchange_raw(&connection, &bytes).await.unwrap();
    let response: Response = baton_proto::frame::decode(&reply).unwrap();
    assert!(matches!(response, Response::Error(_)), "{response:?}");

    // After all that, the connection still does its job.
    let reply = exchange_raw(
        &connection,
        &framed(&Request::Ping(baton_proto::message::Ping { nonce: 1 })),
    )
    .await
    .unwrap();
    assert!(matches!(
        baton_proto::frame::decode::<Response>(&reply).unwrap(),
        Response::Pong(_)
    ));
    assert_eq!(a.inbox.items(), Vec::new());
}

#[tokio::test]
async fn a_request_that_never_finishes_is_cut_off() {
    let mut spec = Spec::new("Laptop");
    spec.tuning.request_deadline = Duration::from_millis(300);
    let a = spec.start().await;
    let (_raw, connection) = paired_raw(&a).await;

    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    // Promise ten bytes and send two.
    send.write_all(&common::header(10)).await.unwrap();
    send.write_all(&[0, 0]).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(5), recv.read_to_end(1024))
        .await
        .expect("the receiver should give up")
        .unwrap_err();
    assert!(error.to_string().contains("408"), "{error}");
}

#[tokio::test]
async fn a_device_cannot_open_streams_the_protocol_does_not_use() {
    let a = device("Laptop").await;
    let (_raw, connection) = paired_raw(&a).await;
    // Unidirectional streams are not allowed at all, so opening one cannot succeed.
    let attempt = tokio::time::timeout(Duration::from_millis(500), connection.open_uni()).await;
    assert!(attempt.is_err(), "a unidirectional stream was opened");
}

#[tokio::test]
async fn a_paired_devices_connection_is_given_room_for_bulk_data_once_it_is_known() {
    let a = device("Laptop").await;
    let (_raw, connection) = paired_raw(&a).await;
    // A device announces a small window to everyone and raises it once it knows the peer is
    // paired, which it tells the peer in a MAX_DATA frame. Nothing else would send one on a
    // connection that has carried no data.
    eventually("the device to raise the connection window", || {
        connection.stats().frame_rx.max_data > 0
    })
    .await;
}
