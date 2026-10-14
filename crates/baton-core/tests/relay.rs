// SPDX-License-Identifier: Apache-2.0

//! Devices that cannot reach each other directly, and so go through a relay.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;

use baton_core::link::PathKind;
use baton_proto::message::{ClipboardText, Item, Outcome};
use common::{Spec, approve};
use iroh::test_utils::run_relay_server;

#[tokio::test]
async fn devices_with_no_direct_path_pair_and_hand_things_over_through_a_relay() {
    let (_map, relay, _server) = run_relay_server().await.unwrap();
    let relay = relay.to_string();

    // Neither device has an address the other could dial: only the relay connects them.
    let a = Spec::new("Laptop").through_relay(&relay).start().await;
    let b = Spec::new("Phone").through_relay(&relay).start().await;
    assert_eq!(a.node.status().direct_addrs, Vec::new());

    let offer = a.node.offer_pairing(approve).await.unwrap();
    assert!(offer.ticket().addrs.is_empty(), "{:?}", offer.ticket());
    assert!(
        offer.ticket().relay.is_some(),
        "the ticket should name the relay: {:?}",
        offer.ticket()
    );
    b.node.join_pairing(offer.ticket(), |_| {}).await.unwrap();
    offer.wait().await.unwrap();

    let report = b.node.ping(&a.id).await.unwrap();
    assert_eq!(report.path, PathKind::Relay);

    let sent = b
        .node
        .send(
            &a.id,
            Item::ClipboardText(ClipboardText {
                text: "over the relay".into(),
            }),
        )
        .await
        .unwrap();
    assert_eq!(sent, Outcome::Accepted);
    a.inbox.wait_for(1).await;

    // The relay is remembered, so the pairing survives a restart without the ticket.
    let known = b.trust.get(&a.id).unwrap();
    assert!(known.relay.is_some(), "{known:?}");
}
