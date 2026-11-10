// SPDX-License-Identifier: Apache-2.0

//! What waits in the outbox for a device that cannot be reached, the way a screen drives it: two
//! apps on this computer, each with a folder of its own (`docs/adr/0016-outbox.md`).
//!
//! A device that is away is played by one that no longer has the other paired: it answers the
//! other's call by closing the connection, at once, which the sender cannot tell from a device that
//! is not there. The tests need no wait for a call that goes unanswered, and the device comes back
//! when the two are paired again. The settings keep both to this computer's own addresses, so the
//! tests run the same on a laptop and in CI. Every wait has a limit, so a hang fails the test
//! instead of stalling the run.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{path::Path, time::Duration};

use baton_app::{App, ErrorKind, Event, Events, Options, OutboxEnd, ReceivedItem, SendResult};
use tempfile::TempDir;

/// Long enough for a slow machine, short enough that a hang does not take the run with it.
const WAIT: Duration = Duration::from_mins(1);

/// Long enough for something that should not happen to have happened.
const QUIET: Duration = Duration::from_millis(700);

/// One device: an app, and the folder it keeps its files in.
struct Dev {
    app: App,
    dir: TempDir,
}

impl Dev {
    async fn start(name: &str) -> Self {
        Self::start_with(name, "").await
    }

    /// The same, with `more` added to the settings, such as `[handoff]` and what is in it.
    async fn start_with(name: &str, more: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), name, more);
        let app = App::start(Options::new(dir.path())).await.unwrap();
        Self { app, dir }
    }

    fn id(&self) -> String {
        self.app.me().id
    }

    /// The app is stopped and started again over the same folder, as when a program is closed and
    /// opened again.
    async fn restarted(self) -> Self {
        self.app.stop().await;
        let app = App::start(Options::new(self.dir.path())).await.unwrap();
        Self { app, ..self }
    }
}

/// Settings that keep the device to this computer, and `more` after them.
fn write_settings(dir: &Path, name: &str, more: &str) {
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "device_name = \"{name}\"\n\n[network]\nrelay = \"disabled\"\nlan_discovery = false\n\
             port_mapping = false\n\n{more}"
        ),
    )
    .unwrap();
}

/// Waits for an event `pick` likes, and returns what it made of it.
async fn next_matching<T>(events: &mut Events, mut pick: impl FnMut(Event) -> Option<T>) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.next().await.expect("the app stopped while waiting");
            if let Some(found) = pick(event) {
                return found;
            }
        }
    })
    .await
    .expect("the event never came")
}

/// Whether no event `unwanted` likes comes within a short time.
async fn none_comes(events: &mut Events, mut unwanted: impl FnMut(&Event) -> bool) -> bool {
    tokio::time::timeout(QUIET, async {
        loop {
            match events.next().await {
                Some(event) if unwanted(&event) => return,
                Some(_) => {}
                None => std::future::pending::<()>().await,
            }
        }
    })
    .await
    .is_err()
}

/// Pairs the two the way two people would: the offerer shows a ticket, the joiner enters it, and
/// the offerer says yes.
async fn pair(offerer: &Dev, joiner: &Dev) {
    let mut events = offerer.app.events();
    let offer = offerer.app.offer_pairing().await.unwrap();
    let app = joiner.app.clone();
    let ticket = offer.ticket.clone();
    let task = tokio::spawn(async move { app.join_pairing(&ticket, |_| {}).await });
    let request = next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;
    assert!(offerer.app.answer_pairing(request.id, true));
    task.await.unwrap().unwrap();
    next_matching(&mut events, |event| match event {
        Event::OfferEnded(_) => Some(()),
        _ => None,
    })
    .await;
}

/// A sender, and a device that it cannot reach: the device is paired with it, and then forgets it.
async fn sender_and_a_device_that_is_away() -> (Dev, Dev) {
    sender_with_settings_and_a_device_that_is_away("").await
}

/// The same, with `more` added to the settings of the sender.
async fn sender_with_settings_and_a_device_that_is_away(more: &str) -> (Dev, Dev) {
    let sender = Dev::start_with("Studio Mac", more).await;
    let away = Dev::start("Pocket Phone").await;
    pair(&sender, &away).await;
    away.app.unpair(&sender.id()).unwrap();
    (sender, away)
}

#[tokio::test]
async fn a_link_for_a_device_that_is_away_waits_and_goes_when_the_device_is_back() {
    let (sender, away) = sender_and_a_device_that_is_away().await;
    let mut events = sender.app.events();

    let result = sender
        .app
        .send_link(
            &away.id(),
            "https://example.com/private?token=SECRET",
            Some("A private title"),
        )
        .await
        .unwrap();
    assert_eq!(result, SendResult::Waiting);
    next_matching(&mut events, |event| {
        matches!(event, Event::OutboxChanged).then_some(())
    })
    .await;

    // What waits is told by what it is, and never by what it says.
    let waiting = sender.app.outbox();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].device_id, away.id());
    assert_eq!(waiting[0].device_name, "Pocket Phone");
    assert_eq!(waiting[0].summary, "a link (example.com)");
    assert_eq!(waiting[0].expires_at, waiting[0].queued_at + 24 * 60 * 60);
    let printed = format!("{waiting:?}");
    for secret in ["SECRET", "private", "token"] {
        assert!(!printed.contains(secret), "{printed}");
    }
    assert_eq!(away.app.inbox().unwrap(), Vec::new());

    // The two are paired again, as two people would.
    pair(&sender, &away).await;

    let settled = next_matching(&mut events, |event| match event {
        Event::OutboxSettled(settled) => Some(settled),
        _ => None,
    })
    .await;
    assert_eq!(settled.end, OutboxEnd::Delivered);
    assert_eq!(settled.device_name, "Pocket Phone");
    assert_eq!(
        settled.message,
        "Sent a link (example.com) to Pocket Phone."
    );
    assert_eq!(sender.app.outbox(), Vec::new());

    let inbox = away.app.inbox().unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].from_name, "Studio Mac");
    match &inbox[0].item {
        ReceivedItem::Link { url, title } => {
            assert_eq!(url, "https://example.com/private?token=SECRET");
            assert_eq!(title.as_deref(), Some("A private title"));
        }
        other @ ReceivedItem::Text { .. } => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn what_waits_is_still_there_when_the_app_is_opened_again_and_goes_then() {
    let (sender, away) = sender_and_a_device_that_is_away().await;

    assert_eq!(
        sender.app.send_text(&away.id(), "first").await.unwrap(),
        SendResult::Waiting
    );
    assert_eq!(
        sender.app.send_text(&away.id(), "second").await.unwrap(),
        SendResult::Waiting
    );
    let before = sender.app.outbox();
    assert_eq!(before.len(), 2);

    // The person closes Baton and opens it again: what waited is in the file.
    let sender = sender.restarted().await;
    let after = sender.app.outbox();
    assert_eq!(
        after, before,
        "the same items, in the same order, with the same numbers"
    );

    // Opened again, it is sent as soon as the device is back, oldest first.
    let mut events = sender.app.events();
    pair(&sender, &away).await;
    for _ in 0..2 {
        let settled = next_matching(&mut events, |event| match event {
            Event::OutboxSettled(settled) => Some(settled),
            _ => None,
        })
        .await;
        assert_eq!(settled.end, OutboxEnd::Delivered);
    }
    assert_eq!(sender.app.outbox(), Vec::new());
    let mut texts: Vec<_> = away
        .app
        .inbox()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.number, entry.item))
        .collect();
    texts.sort_by_key(|(number, _)| *number);
    assert_eq!(
        texts.into_iter().map(|(_, item)| item).collect::<Vec<_>>(),
        [
            ReceivedItem::Text {
                text: "first".to_owned()
            },
            ReceivedItem::Text {
                text: "second".to_owned()
            },
        ],
        "the one that waited longer arrived first"
    );

    // What was sent is not sent again by the next run.
    let sender = sender.restarted().await;
    assert_eq!(sender.app.outbox(), Vec::new());
    assert_eq!(away.app.inbox().unwrap().len(), 2);
}

#[tokio::test]
async fn the_person_can_give_up_on_what_waits_and_unpairing_the_device_drops_the_rest() {
    let (sender, away) = sender_and_a_device_that_is_away().await;
    for text in ["one", "two", "three"] {
        assert_eq!(
            sender.app.send_text(&away.id(), text).await.unwrap(),
            SendResult::Waiting
        );
    }
    let waiting = sender.app.outbox();
    assert_eq!(waiting.len(), 3);
    let mut events = sender.app.events();

    // One is given up on, and only that one.
    assert!(sender.app.cancel_outbox(waiting[1].id).unwrap());
    assert!(
        !sender.app.cancel_outbox(waiting[1].id).unwrap(),
        "it is not there any more"
    );
    assert!(!sender.app.cancel_outbox(9_999).unwrap());
    next_matching(&mut events, |event| {
        matches!(event, Event::OutboxChanged).then_some(())
    })
    .await;
    let ids: Vec<_> = sender.app.outbox().iter().map(|item| item.id).collect();
    assert_eq!(ids, [waiting[0].id, waiting[2].id]);

    // The device is unpaired: nothing is sent to a device that is not paired, and nothing waits
    // for one.
    sender.app.unpair(&away.id()).unwrap();
    assert_eq!(sender.app.outbox(), Vec::new());
    next_matching(&mut events, |event| {
        matches!(event, Event::OutboxChanged).then_some(())
    })
    .await;

    // The person who cancelled, or unpaired, is told nothing of what they did, and the device never
    // got any of it.
    assert!(
        none_comes(&mut events, |event| matches!(
            event,
            Event::OutboxSettled(_)
        ))
        .await,
        "nothing is said of what the person did"
    );
    assert_eq!(away.app.inbox().unwrap(), Vec::new());
}

#[tokio::test]
async fn no_more_than_twenty_items_wait_for_one_device_and_the_person_is_told() {
    let (sender, away) = sender_and_a_device_that_is_away().await;
    for n in 0..20 {
        assert_eq!(
            sender
                .app
                .send_text(&away.id(), &format!("item {n}"))
                .await
                .unwrap(),
            SendResult::Waiting,
            "item {n}"
        );
    }

    let error = sender
        .app
        .send_text(&away.id(), "one too many")
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unreachable, "{error}");
    assert!(
        error
            .message()
            .contains("nothing more can wait for it: 20 items already do"),
        "{error}"
    );
    assert!(!error.message().contains('\n'));
    assert_eq!(
        sender.app.outbox().len(),
        20,
        "it was not put with the others"
    );

    // Making room makes room.
    let first = sender.app.outbox()[0].id;
    assert!(sender.app.cancel_outbox(first).unwrap());
    assert_eq!(
        sender
            .app
            .send_text(&away.id(), "now there is room")
            .await
            .unwrap(),
        SendResult::Waiting
    );
}

#[tokio::test]
async fn a_device_that_answers_is_told_so_and_what_it_will_not_take_does_not_wait() {
    let sender = Dev::start("Studio Mac").await;
    let device = Dev::start_with("Pocket Phone", "[handoff]\nmax_clipboard_bytes = 1024\n").await;
    pair(&sender, &device).await;

    // A device that took it is not one that it waits for.
    assert_eq!(
        sender.app.send_text(&device.id(), "small").await.unwrap(),
        SendResult::Delivered
    );
    assert_eq!(device.app.inbox().unwrap().len(), 1);
    assert_eq!(sender.app.outbox(), Vec::new());

    // A device that answered and would not take it says so at once, and waiting would not change
    // that: nothing waits for it.
    let error = sender
        .app
        .send_text(&device.id(), &"x".repeat(2_000))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined, "{error}");
    assert!(
        error
            .message()
            .contains("Pocket Phone did not take the text"),
        "{error}"
    );
    assert_eq!(sender.app.outbox(), Vec::new());
    assert_eq!(device.app.inbox().unwrap().len(), 1);
}

#[tokio::test]
async fn text_that_is_more_than_the_settings_of_this_device_allow_is_told_and_does_not_wait() {
    // The limit is the sender's own, and it is held before anything is dialed: that the device is
    // away changes nothing about what is wrong with the text, and waiting would not either.
    let (sender, away) =
        sender_with_settings_and_a_device_that_is_away("[handoff]\nmax_clipboard_bytes = 1024\n")
            .await;

    let error = sender
        .app
        .send_text(&away.id(), &"x".repeat(2_000))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::BadItem, "{error}");
    assert_eq!(sender.app.outbox(), Vec::new());

    // Within the limit it waits, as for any device that is away: the size is what told them apart.
    assert_eq!(
        sender
            .app
            .send_text(&away.id(), &"x".repeat(1_000))
            .await
            .unwrap(),
        SendResult::Waiting
    );
    assert_eq!(sender.app.outbox().len(), 1);
}

#[tokio::test]
async fn nothing_is_queued_for_what_is_wrong_with_the_item_or_the_device_or_a_stopped_app() {
    let (sender, away) = sender_and_a_device_that_is_away().await;

    // What is wrong with the item is told, and nothing waits for it.
    assert_eq!(
        sender
            .app
            .send_text(&away.id(), "  \n")
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::BadItem
    );
    assert_eq!(
        sender
            .app
            .send_link(&away.id(), "not a link", None)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::BadItem
    );
    // A device that is not paired is told, and nothing waits for it.
    assert_eq!(
        sender
            .app
            .send_text(&"0".repeat(64), "hello")
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::NotPaired
    );
    assert_eq!(sender.app.outbox(), Vec::new());

    // A stopped app says so, and queues nothing: it would be sent when Baton opens again, which the
    // person did not ask for after closing it.
    sender.app.stop().await;
    assert_eq!(
        sender
            .app
            .send_text(&away.id(), "too late")
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::Stopped
    );
    assert_eq!(
        sender.app.cancel_outbox(1).unwrap_err().kind(),
        ErrorKind::Stopped
    );
    let sender = Dev {
        app: App::start(Options::new(sender.dir.path())).await.unwrap(),
        ..sender
    };
    assert_eq!(sender.app.outbox(), Vec::new());
}
