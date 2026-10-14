// SPDX-License-Identifier: Apache-2.0

//! What strangers, and devices that have been unpaired, can and cannot do.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use baton_core::Identity;
use baton_proto::{
    ALPN_PAIR, ALPN_SESSION,
    message::{ClipboardText, Item},
};
use common::{Raw, Spec, device, pair};
use iroh::{SecretKey, endpoint::ConnectionError};

fn closed_with(error: &ConnectionError) -> Option<u64> {
    match error {
        ConnectionError::ApplicationClosed(close) => Some(close.error_code.into_inner()),
        _ => None,
    }
}

#[tokio::test]
async fn a_stranger_is_turned_away_before_it_can_say_anything() {
    let a = device("Laptop").await;
    let stranger = Raw::new().await;

    match stranger.dial(&a, ALPN_SESSION).await {
        // The refusal can arrive as part of connecting...
        Err(error) => assert!(error.contains("403"), "{error}"),
        // ...or right after.
        Ok(connection) => {
            let ended = tokio::time::timeout(Duration::from_secs(5), connection.closed())
                .await
                .expect("the connection should be closed straight away");
            assert_eq!(closed_with(&ended), Some(403), "{ended}");
        }
    }
    assert_eq!(a.node.connected_peers(), Vec::new());
    assert_eq!(a.inbox.items(), Vec::new());
}

#[tokio::test]
async fn a_stranger_cannot_get_a_request_answered_even_by_racing_the_refusal() {
    let a = device("Laptop").await;
    let stranger = Raw::new().await;
    let Ok(connection) = stranger.dial(&a, ALPN_SESSION).await else {
        return; // refused while connecting, which is the point
    };
    // Whatever is sent now must not be handled.
    if let Ok((mut send, _recv)) = connection.open_bi().await {
        let request = baton_proto::message::Request::Handoff(baton_proto::message::Handoff {
            id: baton_proto::ItemId::from_bytes([1; 16]),
            item: Item::ClipboardText(ClipboardText {
                text: "sneaky".into(),
            }),
        });
        let _ = send
            .write_all(&baton_proto::frame::encode(&request).unwrap())
            .await;
        let _ = send.finish();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(a.inbox.items(), Vec::new());
}

#[tokio::test]
async fn many_strangers_leave_nothing_behind() {
    let a = device("Laptop").await;
    for _ in 0..15 {
        let stranger = Raw::new().await;
        if let Ok(connection) = stranger.dial(&a, ALPN_SESSION).await {
            let _ = tokio::time::timeout(Duration::from_secs(5), connection.closed()).await;
        }
    }
    assert_eq!(a.node.connected_peers(), Vec::new());
    assert!(a.trust.is_empty());
}

#[tokio::test]
async fn protocols_this_version_does_not_speak_are_refused() {
    let a = device("Laptop").await;
    let stranger = Raw::new().await;
    assert!(stranger.dial(&a, b"baton/9").await.is_err());
    assert!(stranger.dial(&a, b"something-else").await.is_err());
}

#[tokio::test]
async fn a_stranger_may_open_the_pairing_protocol_but_gets_nothing_from_it() {
    let a = device("Laptop").await;
    let stranger = Raw::new().await;
    // No offer is open, so there is nothing to redeem. The connection is allowed
    // (anyone must be able to start pairing); the answer to any request is no.
    let connection = stranger.dial(&a, ALPN_PAIR).await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let request = baton_proto::pair::PairRequest {
        device: common::info("Intruder"),
        proof: baton_proto::pair::PairProof::compute(
            &baton_proto::pair::PairSecret::from_bytes([1; 32]),
            &a.id,
            &stranger.id,
            &[0; 32],
        ),
    };
    send.write_all(&baton_proto::frame::encode(&request).unwrap())
        .await
        .unwrap();
    send.finish().unwrap();
    let bytes = recv.read_to_end(4096).await.unwrap();
    let response: baton_proto::pair::PairResponse = baton_proto::frame::decode(&bytes).unwrap();
    assert_eq!(
        response,
        baton_proto::pair::PairResponse::Rejected(baton_proto::pair::PairRejection::Invalid)
    );
    assert!(a.trust.is_empty());
}

#[tokio::test]
async fn a_device_unpaired_between_the_check_and_the_registration_is_cut_off_anyway() {
    // Unpairing closes the connections the registry knows about. A connection that has
    // passed the gate's first look at the list of paired devices but is not registered
    // yet is invisible to it, so the gate has to look again once it is registered.
    // The hook unpairs the device at exactly that moment, and then gives the rest of the
    // node time to react to the unpairing, which finds nothing to close yet.
    let trust: Arc<OnceLock<baton_core::TrustStore>> = Arc::new(OnceLock::new());
    let mut spec = Spec::new("Laptop");
    spec.before_registering = Some(Arc::new({
        let trust = Arc::clone(&trust);
        move |peer| {
            let trust = Arc::clone(&trust);
            Box::pin(async move {
                trust
                    .get()
                    .expect("set before anyone connects")
                    .remove(&peer)
                    .unwrap();
                for _ in 0..20 {
                    tokio::task::yield_now().await;
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
        }
    }));
    let a = spec.start().await;
    trust.set(a.trust.clone()).unwrap();

    // A device that is really paired, and later a bare client holding its key.
    let identity = Identity::from_secret_bytes(&[7; 32]);
    let phone = Spec::new("Phone")
        .start_with(tempfile::tempdir().unwrap(), &identity)
        .await;
    phone.pair_with(&a).await;
    let phone_id = phone.id;
    drop(phone.stop().await);
    assert!(a.trust.contains(&phone_id));

    let client = Raw::with_key(SecretKey::from_bytes(&[7; 32])).await;
    match client.dial(&a, ALPN_SESSION).await {
        Err(error) => assert!(error.contains("403"), "{error}"),
        Ok(connection) => {
            // No request is made. If the connection were registered as if nothing had
            // happened, it would sit there idle for as long as the client likes.
            let ended = tokio::time::timeout(Duration::from_secs(5), connection.closed())
                .await
                .expect("the connection should be closed without any request being made");
            assert_eq!(closed_with(&ended), Some(403), "{ended}");
        }
    }
    assert!(!a.trust.contains(&phone_id));
    assert_eq!(a.node.connected_peers(), Vec::new());
}

#[tokio::test]
async fn a_flood_of_pairing_connections_does_not_disturb_paired_devices() {
    let (a, b) = pair().await;
    b.node
        .send(
            &a.id,
            Item::ClipboardText(ClipboardText { text: "one".into() }),
        )
        .await
        .unwrap();

    // An attacker holds every pairing slot it can get.
    let attacker = Raw::new().await;
    let mut held = Vec::new();
    for _ in 0..30 {
        let connection = attacker.dial(&a, ALPN_PAIR).await.unwrap();
        // Beyond the slots there are, the device closes the connection straight away,
        // and whether that is noticed before or after this call is a matter of timing.
        let stream = connection.open_bi().await.ok();
        held.push((connection, stream));
    }

    // Devices that are already paired are not affected.
    b.node
        .send(
            &a.id,
            Item::ClipboardText(ClipboardText { text: "two".into() }),
        )
        .await
        .unwrap();
    a.node.ping(&b.id).await.unwrap();
    a.inbox.wait_for(2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_connection_attempts_from_one_place_is_answered_one_way_or_the_other() {
    // Enough at once to exceed what a device lets through without asking senders to
    // prove their address. Those are sent a retry token, which a real client answers
    // by itself, so nobody should be left hanging: each attempt either reaches the
    // gate (and is refused there, being a stranger) or is shed at the door.
    let a = Arc::new(device("Laptop").await);
    let attacker = Arc::new(Raw::new().await);

    let attempts: Vec<_> = (0..200)
        .map(|_| {
            let (a, attacker) = (Arc::clone(&a), Arc::clone(&attacker));
            tokio::spawn(async move { attacker.dial(&a, ALPN_SESSION).await.map(|_| ()) })
        })
        .collect();
    for attempt in attempts {
        if let Err(error) = attempt.await.unwrap() {
            let at_the_gate = error.contains("403");
            let shed = error.contains("refused");
            assert!(at_the_gate || shed, "unexpected failure: {error}");
        }
    }

    // And the device is none the worse for it.
    let friend = device("Phone").await;
    friend.pair_with(&a).await;
    friend.node.ping(&a.id).await.unwrap();
}
