// SPDX-License-Identifier: Apache-2.0

//! A device that keeps what it receives in an inbox, and what that looks like from the sender.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use baton_core::{
    Inbox, InboxHandler, InboxItem,
    policy::{ClipboardPolicy, LinkPolicy},
};
use baton_proto::message::{ClipboardText, DeclineReason, Item, Link, Outcome};
use common::{Device, START, Spec, device};
use tempfile::TempDir;

fn link(url: &str) -> Item {
    Item::Link(Link {
        url: url.to_owned(),
        title: Some("A page".to_owned()),
    })
}

fn text(text: &str) -> Item {
    Item::ClipboardText(ClipboardText {
        text: text.to_owned(),
    })
}

/// A receiver that keeps everything in `inbox_dir`, and a sender paired with it.
async fn receiver_and_sender(inbox_dir: &TempDir) -> (Device, Device, Inbox) {
    let inbox = Inbox::load(inbox_dir.path().join("inbox.json"), 200).unwrap();
    let spec = Spec::new("Laptop");
    let handler = InboxHandler::new(inbox.clone()).with_clock(Arc::clone(&spec.clock));
    let receiver = spec.with_handler(handler).start().await;
    let sender = device("Phone").await;
    sender.pair_with(&receiver).await;
    (receiver, sender, inbox)
}

#[tokio::test]
async fn what_a_device_sends_is_kept_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (receiver, sender, inbox) = receiver_and_sender(&dir).await;

    assert_eq!(
        sender
            .node
            .send(&receiver.id, link("https://example.com/one"))
            .await
            .unwrap(),
        Outcome::Accepted
    );
    assert_eq!(
        sender.node.send(&receiver.id, text("two")).await.unwrap(),
        Outcome::Accepted
    );

    let kept = inbox.list().unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!((kept[0].number, kept[1].number), (1, 2));
    assert_eq!(kept[0].from, sender.id);
    assert_eq!(kept[0].from_name, "Phone");
    assert_eq!(kept[0].received_at, START);
    assert_eq!(
        kept[0].item,
        InboxItem::Link {
            url: "https://example.com/one".to_owned(),
            title: Some("A page".to_owned()),
        }
    );
    assert_eq!(
        kept[1].item,
        InboxItem::Text {
            text: "two".to_owned()
        }
    );

    // The device stops and starts again: the inbox is still there.
    receiver.stop().await;
    let again = Inbox::load(dir.path().join("inbox.json"), 200).unwrap();
    assert_eq!(again.list().unwrap(), kept);
}

#[tokio::test]
async fn the_receivers_settings_for_the_sender_decide_what_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let (receiver, sender, inbox) = receiver_and_sender(&dir).await;

    receiver
        .trust
        .update(&sender.id, |paired| {
            paired.policy.links = LinkPolicy::Ignore;
        })
        .unwrap();
    assert_eq!(
        sender
            .node
            .send(&receiver.id, link("https://example.com/"))
            .await
            .unwrap(),
        Outcome::Declined(DeclineReason::Blocked)
    );
    // Only links were switched off.
    assert_eq!(
        sender
            .node
            .send(&receiver.id, text("still welcome"))
            .await
            .unwrap(),
        Outcome::Accepted
    );

    receiver
        .trust
        .update(&sender.id, |paired| {
            paired.policy.links = LinkPolicy::Open;
            paired.policy.clipboard = ClipboardPolicy::Ignore;
        })
        .unwrap();
    // A link set to open is kept for now, and text set to be ignored is not.
    assert_eq!(
        sender
            .node
            .send(&receiver.id, link("https://example.com/open"))
            .await
            .unwrap(),
        Outcome::Accepted
    );
    assert_eq!(
        sender
            .node
            .send(&receiver.id, text("not any more"))
            .await
            .unwrap(),
        Outcome::Declined(DeclineReason::Blocked)
    );

    let kept: Vec<_> = inbox
        .list()
        .unwrap()
        .into_iter()
        .map(|entry| entry.item)
        .collect();
    assert_eq!(kept.len(), 2);
    assert!(matches!(&kept[0], InboxItem::Text { text } if text == "still welcome"));
    assert!(matches!(&kept[1], InboxItem::Link { url, .. } if url == "https://example.com/open"));
}

#[tokio::test]
async fn a_burst_of_items_is_kept_whole_and_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (receiver, sender, inbox) = receiver_and_sender(&dir).await;
    let mut heard = inbox.subscribe();

    for n in 0..12 {
        let item = link(&format!("https://example.com/{n}"));
        assert_eq!(
            sender.node.send(&receiver.id, item).await.unwrap(),
            Outcome::Accepted
        );
    }
    let urls: Vec<_> = inbox
        .list()
        .unwrap()
        .into_iter()
        .map(|entry| match entry.item {
            InboxItem::Link { url, .. } => url,
            InboxItem::Text { .. } => unreachable!(),
        })
        .collect();
    let expected: Vec<_> = (0..12)
        .map(|n| format!("https://example.com/{n}"))
        .collect();
    assert_eq!(urls, expected);
    for n in 1..=12 {
        assert_eq!(heard.recv().await.unwrap().number, n);
    }
}

#[tokio::test]
async fn a_link_that_is_not_well_formed_is_declined_to_the_sender() {
    let dir = tempfile::tempdir().unwrap();
    let (receiver, sender, inbox) = receiver_and_sender(&dir).await;

    // A link with a space in it, which the sender's own checks allow.
    assert_eq!(
        sender
            .node
            .send(&receiver.id, link("https://example.com/a b"))
            .await
            .unwrap(),
        Outcome::Declined(DeclineReason::Invalid)
    );
    assert_eq!(inbox.list().unwrap(), Vec::new());
}
