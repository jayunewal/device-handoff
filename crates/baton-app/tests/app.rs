// SPDX-License-Identifier: Apache-2.0

//! The app layer driven the way a screen drives it: two apps on this computer, each with a
//! folder of its own.
//!
//! The settings need nothing beyond this computer's own addresses (no relay, no discovery, no
//! port mapping), so the tests run the same on a laptop and in CI. The apps find each other
//! through the addresses in the pairing ticket, as they would on a network where discovery is
//! blocked. Every wait has a limit, so a hang fails the test instead of stalling the run.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{path::Path, task::Poll, time::Duration};

use baton_app::{
    App, ErrorKind, Event, Events, OfferOutcome, Options, Platform, ReceivedItem, Route, settings,
};
use tempfile::TempDir;
use tokio::sync::oneshot;

/// Long enough for a slow machine, short enough that a hang does not take the run with it.
const WAIT: Duration = Duration::from_secs(30);

/// One device: an app, and the folder it keeps its files in.
struct Dev {
    app: App,
    /// Kept so the folder lives as long as the device does.
    _dir: TempDir,
}

impl Dev {
    async fn start(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), Some(name), false);
        let app = App::start(Options::new(dir.path())).await.unwrap();
        Self { app, _dir: dir }
    }

    fn id(&self) -> String {
        self.app.me().id
    }
}

/// Settings that keep the device to this computer.
fn write_settings(dir: &Path, name: Option<&str>, lan_discovery: bool) {
    let name = name.map_or_else(String::new, |name| format!("device_name = \"{name}\"\n"));
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "{name}\n[network]\nrelay = \"disabled\"\nlan_discovery = {lan_discovery}\n\
             port_mapping = false\n"
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

/// Starts joining `offerer`'s ticket on `joiner`. Returns the join, and the code that the
/// joiner showed.
fn join(
    joiner: &Dev,
    ticket: &str,
) -> (
    tokio::task::JoinHandle<baton_app::Result<baton_app::Device>>,
    oneshot::Receiver<String>,
) {
    let app = joiner.app.clone();
    let ticket = ticket.to_owned();
    let (shown, code) = oneshot::channel();
    let task = tokio::spawn(async move {
        app.join_pairing(&ticket, move |code| {
            let _ = shown.send(code);
        })
        .await
    });
    (task, code)
}

/// Pairs the two the way two people would: the offerer shows a ticket, the joiner enters it
/// (pasted along with the command it came in, as it often is), and the offerer says yes.
async fn pair(offerer: &Dev, joiner: &Dev) {
    let mut events = offerer.app.events();
    let offer = offerer.app.offer_pairing().await.unwrap();
    let (task, code) = join(joiner, &format!("baton pair join {}\n", offer.ticket));

    let request = next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;
    let shown = tokio::time::timeout(WAIT, code).await.unwrap().unwrap();
    assert_eq!(
        request.code, shown,
        "the two devices must show the same code"
    );
    assert_eq!(request.device_id, joiner.id());
    assert_eq!(request.fingerprint, joiner.app.me().fingerprint);
    assert!(offerer.app.answer_pairing(request.id, true));

    let paired_with = task.await.unwrap().unwrap();
    assert_eq!(paired_with.id, offerer.id());
    let outcome = next_matching(&mut events, |event| match event {
        Event::OfferEnded(outcome) => Some(outcome),
        _ => None,
    })
    .await;
    let OfferOutcome::Paired(paired) = outcome else {
        panic!("the offer ended without pairing: {outcome:?}");
    };
    assert_eq!(paired.id, joiner.id());
}

#[tokio::test]
async fn two_devices_pair_and_hand_each_other_things() {
    let a = Dev::start("Device A").await;
    let b = Dev::start("Device B").await;
    assert!(a.app.devices().is_empty(), "a new app has no devices");
    assert_eq!(a.app.me().name, "Device A");
    let here = if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Linux
    };
    assert_eq!(a.app.me().platform, here);

    let mut a_events = a.app.events();
    pair(&a, &b).await;
    next_matching(&mut a_events, |event| {
        (event == Event::DevicesChanged).then_some(())
    })
    .await;

    let [seen_by_a] = &a.app.devices()[..] else {
        panic!("A should have one device");
    };
    assert_eq!(
        (seen_by_a.name.as_str(), &seen_by_a.id),
        ("Device B", &b.id())
    );
    let [seen_by_b] = &b.app.devices()[..] else {
        panic!("B should have one device");
    };
    assert_eq!(
        (seen_by_b.name.as_str(), &seen_by_b.id),
        ("Device A", &a.id())
    );

    // B hands A a link and then some text, and A's screen hears of both.
    let reach = b.app.ping(&a.id()).await.unwrap();
    assert_ne!(reach.route, Route::Relay);
    b.app
        .send_link(&a.id(), "https://example.com/path?q=1", Some("An example"))
        .await
        .unwrap();
    let link = next_matching(&mut a_events, |event| match event {
        Event::Received(received) => Some(received),
        _ => None,
    })
    .await;
    assert_eq!(
        (link.from_name.as_str(), &link.from_id),
        ("Device B", &b.id())
    );
    assert_eq!(
        link.item,
        ReceivedItem::Link {
            url: "https://example.com/path?q=1".to_owned(),
            title: Some("An example".to_owned()),
        }
    );
    b.app.send_text(&a.id(), "hello from B").await.unwrap();
    let text = next_matching(&mut a_events, |event| match event {
        Event::Received(received) => Some(received),
        _ => None,
    })
    .await;
    assert_eq!(
        text.item,
        ReceivedItem::Text {
            text: "hello from B".to_owned()
        }
    );

    // The inbox keeps both, newest first, until they are removed.
    let inbox = a.app.inbox().unwrap();
    assert_eq!(
        inbox.iter().map(|r| r.number).collect::<Vec<_>>(),
        [text.number, link.number]
    );
    assert!(a.app.remove_received(text.number).unwrap());
    assert!(!a.app.remove_received(text.number).unwrap());
    assert_eq!(a.app.inbox().unwrap(), [link]);
    assert_eq!(a.app.clear_inbox().unwrap(), 1);
    assert!(a.app.inbox().unwrap().is_empty(), "clearing leaves nothing");

    // Forgetting a device is heard of too.
    a.app.unpair(&b.id()).unwrap();
    next_matching(&mut a_events, |event| {
        (event == Event::DevicesChanged).then_some(())
    })
    .await;
    assert!(a.app.devices().is_empty(), "an unpaired device is gone");
    assert_eq!(
        a.app.unpair(&b.id()).unwrap_err().kind(),
        ErrorKind::NotPaired
    );
}

#[tokio::test]
async fn a_device_that_is_declined_is_not_paired() {
    let a = Dev::start("Device A").await;
    let b = Dev::start("Device B").await;
    let mut events = a.app.events();
    let offer = a.app.offer_pairing().await.unwrap();
    let (task, _code) = join(&b, &offer.ticket);

    let request = next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;
    assert!(a.app.answer_pairing(request.id, false));
    assert!(
        !a.app.answer_pairing(request.id, true),
        "an answer counts once"
    );

    assert_eq!(
        task.await.unwrap().unwrap_err().kind(),
        ErrorKind::PairingFailed
    );
    let outcome = next_matching(&mut events, |event| match event {
        Event::OfferEnded(outcome) => Some(outcome),
        _ => None,
    })
    .await;
    assert_eq!(outcome, OfferOutcome::Declined);
    assert!(a.app.devices().is_empty(), "a declined device is not kept");
    assert!(b.app.devices().is_empty(), "a declined device is not kept");
}

#[tokio::test]
async fn withdrawing_an_offer_takes_the_question_off_the_screen() {
    let a = Dev::start("Device A").await;
    let b = Dev::start("Device B").await;
    let mut events = a.app.events();
    let offer = a.app.offer_pairing().await.unwrap();
    let (task, _code) = join(&b, &offer.ticket);
    let request = next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;

    a.app.cancel_offer();
    let (mut question_ended, mut offer_ended) = (false, false);
    next_matching(&mut events, |event| {
        match event {
            Event::PairingRequestEnded { id } if id == request.id => question_ended = true,
            Event::OfferEnded(OfferOutcome::Cancelled) => offer_ended = true,
            _ => {}
        }
        (question_ended && offer_ended).then_some(())
    })
    .await;
    assert!(!a.app.answer_pairing(request.id, true));
    task.await.unwrap().unwrap_err();
    assert!(
        a.app.devices().is_empty(),
        "a withdrawn offer pairs nothing"
    );

    // The ticket is spent, and withdrawing again changes nothing.
    a.app.cancel_offer();
    let (second, _code) = join(&b, &offer.ticket);
    second.await.unwrap().unwrap_err();
}

#[tokio::test]
async fn a_new_offer_replaces_the_old_one_quietly() {
    let a = Dev::start("Device A").await;
    let b = Dev::start("Device B").await;
    let mut events = a.app.events();
    let old = a.app.offer_pairing().await.unwrap();
    let new = a.app.offer_pairing().await.unwrap();
    assert_ne!(old.ticket, new.ticket);

    // Only the offer that is showing can be used, and the end of the old one is not reported
    // as the end of the new one.
    let (task, _code) = join(&b, &old.ticket);
    task.await.unwrap().unwrap_err();
    a.app.cancel_offer();
    let outcome = next_matching(&mut events, |event| match event {
        Event::OfferEnded(outcome) => Some(outcome),
        _ => None,
    })
    .await;
    assert_eq!(outcome, OfferOutcome::Cancelled);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), events.next())
            .await
            .is_err(),
        "the old offer must not report a second end"
    );
}

#[tokio::test]
async fn joining_can_be_given_up() {
    let a = Dev::start("Device A").await;
    let b = Dev::start("Device B").await;
    let mut events = a.app.events();
    let offer = a.app.offer_pairing().await.unwrap();
    let (task, code) = join(&b, &offer.ticket);
    // Give up only once both screens are showing the code, so the test does not depend on
    // which of the two gets there first.
    let request = next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;
    tokio::time::timeout(WAIT, code).await.unwrap().unwrap();

    b.app.cancel_join();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PairingFailed);
    assert!(error.message().contains("cancelled"), "{error}");
    // A's question goes away because the device that asked has left.
    next_matching(&mut events, |event| match event {
        Event::PairingRequestEnded { id } if id == request.id => Some(()),
        _ => None,
    })
    .await;
    assert!(!a.app.answer_pairing(request.id, true));
    assert!(a.app.devices().is_empty(), "giving up pairs nothing");
}

#[tokio::test]
async fn text_that_is_not_a_ticket_is_refused_before_anything_is_sent() {
    let a = Dev::start("Device A").await;
    for text in ["", "hello", "BATON1:", "BATON1:!!!!", "BATON2:AAAA"] {
        let error = a.app.join_pairing(text, |_| {}).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadTicket, "{text:?}: {error}");
        assert!(!error.message().contains('\n'));
    }
    // A device cannot pair with itself.
    let own = a.app.offer_pairing().await.unwrap();
    let error = a.app.join_pairing(&own.ticket, |_| {}).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::BadTicket, "{error}");
}

#[tokio::test]
async fn things_that_cannot_be_sent_are_refused_with_a_reason() {
    let a = Dev::start("Device A").await;
    let nobody = "0".repeat(64);
    for id in [nobody.as_str(), "not an id", ""] {
        let error = a.app.send_text(id, "hello").await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotPaired, "{id:?}");
        assert_eq!(
            a.app.ping(id).await.unwrap_err().kind(),
            ErrorKind::NotPaired
        );
    }
    // What is wrong with the thing itself is said first.
    for text in ["", "   \n"] {
        let error = a.app.send_text(&nobody, text).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadItem);
    }
    for url in ["", "example.com", "not a url", "/just/a/path"] {
        let error = a.app.send_link(&nobody, url, None).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadItem, "{url:?}");
    }
}

#[tokio::test]
async fn a_stopped_app_says_so_and_lets_go_of_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device A"), false);
    let first = App::start(Options::new(dir.path())).await.unwrap();
    let id = first.me().id;

    // One folder, one running app.
    let second = App::start(Options::new(dir.path())).await.unwrap_err();
    assert_eq!(second.kind(), ErrorKind::AlreadyRunning);

    let mut events = first.events();
    first.stop().await;
    first.stop().await;
    assert!(events.next().await.is_none());
    assert!(events.next().await.is_none());
    assert_eq!(first.inbox().unwrap_err().kind(), ErrorKind::Stopped);
    assert_eq!(
        first.offer_pairing().await.unwrap_err().kind(),
        ErrorKind::Stopped
    );
    assert_eq!(
        first.send_text(&id, "hello").await.unwrap_err().kind(),
        ErrorKind::Stopped
    );
    assert_eq!(
        first.ping(&id).await.unwrap_err().kind(),
        ErrorKind::Stopped
    );

    // The same device starts again with the same identity.
    let again = App::start(Options::new(dir.path())).await.unwrap();
    assert_eq!(again.me().id, id);
    again.stop().await;
}

#[tokio::test]
async fn the_name_a_person_chose_is_the_name_devices_see() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), None, false);
    assert_eq!(settings::device_name(dir.path()).unwrap(), None);

    settings::set_device_name(dir.path(), "Kitchen iPad").unwrap();
    assert_eq!(
        settings::device_name(dir.path()).unwrap().as_deref(),
        Some("Kitchen iPad")
    );
    // A name with nothing in it is the person's to fix, not damage to the folder, and what
    // the screen shows must not give the folder's path away.
    for name in ["", "  ", "\u{200b}"] {
        let error = settings::set_device_name(dir.path(), name).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadName, "{name:?}");
        assert!(error.message().contains("name"), "{error}");
        assert!(
            !error.message().contains(&*dir.path().to_string_lossy()),
            "{error}"
        );
    }
    assert_eq!(
        settings::device_name(dir.path()).unwrap().as_deref(),
        Some("Kitchen iPad"),
        "a name that was refused changes nothing"
    );

    let app = App::start(Options::new(dir.path())).await.unwrap();
    assert_eq!(app.me().name, "Kitchen iPad");
    app.stop().await;
}

#[tokio::test]
async fn discovery_on_the_local_network_can_be_switched_off_for_a_run() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device A"), true);
    let mut options = Options::new(dir.path());
    options.lan_discovery = Some(false);
    let app = App::start(options).await.unwrap();
    assert!(!app.network().lan_discovery);
    app.stop().await;
}

#[tokio::test]
async fn a_folder_that_cannot_be_used_is_reported_as_storage() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device A"), false);
    std::fs::write(
        dir.path().join("devices.json"),
        "this is not a list of devices",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(
            dir.path().join("devices.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let error = App::start(Options::new(dir.path())).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Storage, "{error}");
    assert!(!error.message().contains('\n'));
}

#[tokio::test]
async fn withdrawing_an_offer_just_after_a_yes_reports_what_happened() {
    // A sheet that closes as the person taps Yes withdraws the offer in the same breath.
    // Whichever of the two reached the node first, the screen must be told what happened to
    // the devices: a yes that was claimed cannot be taken back, and a withdrawal that came
    // first is not a pairing.
    for _ in 0..3 {
        let a = Dev::start("Device A").await;
        let b = Dev::start("Device B").await;
        let mut events = a.app.events();
        let offer = a.app.offer_pairing().await.unwrap();
        let (task, _code) = join(&b, &offer.ticket);
        let request = next_matching(&mut events, |event| match event {
            Event::PairingRequested(request) => Some(request),
            _ => None,
        })
        .await;

        assert!(a.app.answer_pairing(request.id, true));
        a.app.cancel_offer();

        let outcome = next_matching(&mut events, |event| match event {
            Event::OfferEnded(outcome) => Some(outcome),
            _ => None,
        })
        .await;
        let joined = task.await.unwrap();
        match outcome {
            OfferOutcome::Paired(device) => {
                assert_eq!(device.id, b.id());
                assert_eq!(joined.unwrap().id, a.id());
                assert_eq!(a.app.devices().len(), 1);
                assert_eq!(b.app.devices().len(), 1);
            }
            OfferOutcome::Cancelled => {
                joined.unwrap_err();
                assert!(
                    a.app.devices().is_empty(),
                    "reported as cancelled, but paired"
                );
                assert!(
                    b.app.devices().is_empty(),
                    "reported as cancelled, but paired"
                );
            }
            other => panic!("the offer ended some other way: {other:?}"),
        }
    }
}

#[tokio::test]
async fn an_app_that_is_dropped_while_a_question_is_showing_lets_go_of_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device A"), false);
    let a = App::start(Options::new(dir.path())).await.unwrap();
    let b = Dev::start("Device B").await;
    let mut events = a.events();
    let offer = a.offer_pairing().await.unwrap();
    let (task, _code) = join(&b, &offer.ticket);
    next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;

    // The question is on the screen and nobody has answered. The screen goes away, and the
    // folder is free at once: the question does not keep the app running.
    drop(events);
    drop(a);
    let again = App::start(Options::new(dir.path())).await.unwrap();
    again.stop().await;
    // The device that asked finds out only when its connection runs out, which takes a
    // while, so it is not waited for.
    task.abort();
}

#[tokio::test]
async fn an_app_that_is_dropped_while_an_offer_is_showing_ends_its_events() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device A"), false);
    let a = App::start(Options::new(dir.path())).await.unwrap();
    let mut events = a.events();
    a.offer_pairing().await.unwrap();

    // Nobody calls `stop`. The offer is waiting for a withdrawal that nobody is left to
    // make, and it must not keep the stream of events open.
    drop(a);
    let ended = tokio::time::timeout(WAIT, events.next())
        .await
        .expect("the events never ended");
    assert!(ended.is_none(), "{ended:?}");

    // The stream can end while the task that holds the offer is still waiting, so look at the
    // tasks too. This test has a runtime of its own and the app is all that ran on it, so once
    // the offer's task has ended none are left.
    let metrics = tokio::runtime::Handle::current().metrics();
    tokio::time::timeout(WAIT, async {
        while metrics.num_alive_tasks() > 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a task of the dropped app is still running");

    let again = App::start(Options::new(dir.path())).await.unwrap();
    again.stop().await;
}

#[tokio::test]
async fn a_second_stop_returns_only_when_the_folder_is_free() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device A"), false);
    let app = App::start(Options::new(dir.path())).await.unwrap();

    // The first call is given up on as soon as it has begun, as when a screen is closed
    // while it waits. The stopping goes on without it.
    let mut first = Box::pin(app.stop());
    std::future::poll_fn(|cx| {
        let _ = first.as_mut().poll(cx);
        Poll::Ready(())
    })
    .await;
    drop(first);
    app.stop().await;
    let again = App::start(Options::new(dir.path())).await.unwrap();
    again.stop().await;

    // Two calls at once are both held until it is done.
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device B"), false);
    let app = App::start(Options::new(dir.path())).await.unwrap();
    tokio::join!(app.stop(), app.stop());
    let again = App::start(Options::new(dir.path())).await.unwrap();
    again.stop().await;
}

#[tokio::test]
async fn an_offer_that_is_being_made_as_the_app_stops_is_not_handed_out() {
    // With a relay set, the node waits a moment for it before it makes the offer. This one
    // is not there, so the offer is being made for the whole of that moment.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "device_name = \"Device A\"\n[network]\nrelay = \"custom\"\n\
         relay_urls = [\"http://127.0.0.1:9\"]\nlan_discovery = false\nport_mapping = false\n",
    )
    .unwrap();
    let app = App::start(Options::new(dir.path())).await.unwrap();
    let offering = {
        let app = app.clone();
        tokio::spawn(async move { app.offer_pairing().await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !offering.is_finished(),
        "the offer should still be in the making"
    );

    app.stop().await;
    let error = tokio::time::timeout(WAIT, offering)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Stopped, "{error}");
}
