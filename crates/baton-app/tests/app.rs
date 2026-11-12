// SPDX-License-Identifier: Apache-2.0

//! The app layer driven the way a screen drives it: two apps on this computer, each with a
//! folder of its own.
//!
//! The settings need nothing beyond this computer's own addresses (no relay, no discovery, no
//! port mapping), so the tests run the same on a laptop and in CI. The apps find each other
//! through the addresses in the pairing ticket, as they would on a network where discovery is
//! blocked. Every wait has a limit, so a hang fails the test instead of stalling the run.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    task::Poll,
    time::Duration,
};

use baton_app::{
    App, ErrorKind, Event, Events, FileSend, FilesSent, OfferOutcome, Options, Platform,
    ReceivedItem, Receiving, Route, SendStage, TransferResult, settings,
};
use tempfile::TempDir;
use tokio::sync::oneshot;

/// Long enough for a slow machine, short enough that a hang does not take the run with it.
const WAIT: Duration = Duration::from_secs(30);

/// One device: an app, and the folder it keeps its files in.
struct Dev {
    app: App,
    /// The folder that the device keeps its state in, kept so it lives as long as the device does.
    dir: TempDir,
    /// The folder that files are received in, for a device that takes files.
    received: Option<TempDir>,
}

impl Dev {
    async fn start(name: &str) -> Self {
        Self::start_with(name, false).await
    }

    /// A device that takes files, into a folder of its own.
    async fn start_taking_files(name: &str) -> Self {
        Self::start_with(name, true).await
    }

    async fn start_with(name: &str, takes_files: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), Some(name), false);
        let received = takes_files.then(|| tempfile::tempdir().unwrap());
        let mut options = Options::new(dir.path());
        options.receive_dir = received.as_ref().map(|folder| folder.path().to_path_buf());
        let app = App::start(options).await.unwrap();
        Self { app, dir, received }
    }

    fn id(&self) -> String {
        self.app.me().id
    }

    /// The folder that received files are put in.
    fn received(&self) -> &Path {
        self.received
            .as_ref()
            .expect("a device that takes files")
            .path()
    }

    /// The app is stopped and started again over the same folder, as when a program is closed and
    /// opened again.
    async fn restarted(self) -> Self {
        self.app.stop().await;
        let mut options = Options::new(self.dir.path());
        options.receive_dir = self
            .received
            .as_ref()
            .map(|folder| folder.path().to_path_buf());
        let app = App::start(options).await.unwrap();
        Self { app, ..self }
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

// Files.

/// What is in `root`, by path from it: a folder is `None`, and a file has its bytes. The hidden
/// folder that Baton keeps what is still arriving in is left out.
fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, at: &Path, found: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in std::fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            if at == root
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".baton-incoming-")
            {
                continue;
            }
            let path = entry.path();
            let shown = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            if entry.file_type().unwrap().is_dir() {
                found.insert(shown, None);
                walk(root, &path, found);
            } else {
                found.insert(shown, Some(std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(root, root, &mut found);
    found
}

/// A file, and a folder with a file and a folder inside it, ready to be sent: 100 017 bytes in
/// three files. The folder it lives in is kept by the caller.
fn sample_files() -> (TempDir, Vec<PathBuf>) {
    let source = tempfile::tempdir().unwrap();
    let docs = source.path().join("docs");
    std::fs::create_dir_all(docs.join("sub")).unwrap();
    std::fs::write(source.path().join("hello.txt"), b"hello from B").unwrap();
    std::fs::write(docs.join("a.txt"), b"first").unwrap();
    std::fs::write(docs.join("sub").join("b.bin"), vec![7; 100_000]).unwrap();
    let paths = vec![source.path().join("hello.txt"), docs];
    (source, paths)
}

/// What [`sample_files`] comes to in the folder that receives it.
fn sample_tree() -> BTreeMap<String, Option<Vec<u8>>> {
    BTreeMap::from([
        ("docs".to_owned(), None),
        ("docs/a.txt".to_owned(), Some(b"first".to_vec())),
        ("docs/sub".to_owned(), None),
        ("docs/sub/b.bin".to_owned(), Some(vec![7; 100_000])),
        ("hello.txt".to_owned(), Some(b"hello from B".to_vec())),
    ])
}

/// One file with the name `name`, ready to be sent.
fn one_file(name: &str, content: &[u8]) -> (TempDir, Vec<PathBuf>) {
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join(name);
    std::fs::write(&path, content).unwrap();
    (source, vec![path])
}

/// Waits for a device to be asked whether it takes files.
async fn question(events: &mut Events) -> baton_app::TransferRequest {
    next_matching(events, |event| match event {
        Event::TransferRequested(request) => Some(request),
        _ => None,
    })
    .await
}

/// Waits for the end of a transfer that is being sent.
async fn finished(sending: &FileSend) -> baton_app::Result<FilesSent> {
    tokio::time::timeout(WAIT, sending.outcome())
        .await
        .expect("the transfer never ended")
}

/// Whether no event of the kind `is` comes for a little while.
async fn none_comes(events: &mut Events, is: impl Fn(&Event) -> bool) -> bool {
    tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            match events.next().await {
                Some(event) if is(&event) => return,
                Some(_) => {}
                None => std::future::pending::<()>().await,
            }
        }
    })
    .await
    .is_err()
}

#[tokio::test]
async fn files_are_sent_when_the_person_says_yes_and_arrive_as_they_were() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    assert_eq!(a.app.receiving(), Receiving::On);
    assert_eq!(b.app.receiving(), Receiving::Off);
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let (_source, paths) = sample_files();

    let sending = b.app.send_files(&a.id(), paths).await.unwrap();

    // The person is asked who wants to send what, and nothing is kept before they answer.
    let request = question(&mut a_events).await;
    assert_eq!(
        (request.device_name.as_str(), &request.device_id),
        ("Device B", &b.id())
    );
    assert_eq!(
        (request.items, request.files, request.folders, request.bytes),
        (2, 3, 2, 100_017)
    );
    let shown: Vec<_> = request
        .preview
        .iter()
        .map(|item| (item.name.as_str(), item.folder, item.files, item.bytes))
        .collect();
    assert_eq!(
        shown,
        [("docs", true, 2, 100_005), ("hello.txt", false, 0, 12)]
    );
    assert!(tree(a.received()).is_empty(), "nothing is kept before yes");

    assert!(a.app.answer_transfer(request.id, true));
    assert!(
        !a.app.answer_transfer(request.id, true),
        "an answer counts once"
    );

    let sent = finished(&sending).await.unwrap();
    assert_eq!(
        sent,
        FilesSent {
            files: 3,
            bytes: 100_017,
            failed: 0,
            left_out: 0
        }
    );
    let end = next_matching(&mut a_events, |event| match event {
        Event::TransferEnded(end) => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end.request, request.id);
    assert_eq!(end.result, TransferResult::Received { failed: 0 });
    assert_eq!(
        (end.device_name.as_str(), end.files, end.bytes),
        ("Device B", 3, 100_017)
    );
    assert_eq!(end.preview, request.preview);
    assert_eq!(tree(a.received()), sample_tree());
    // The end says where the items are, so that a screen can take the person to them: the top of
    // what was offered, in the order it was offered, and not every file in a folder.
    assert_eq!(
        end.saved,
        [a.received().join("docs"), a.received().join("hello.txt")]
    );

    // The sender's screen has what it needs to show a finished bar, and asking again gets the
    // same answer.
    let progress = sending.progress();
    assert_eq!(progress.stage, SendStage::Over);
    assert_eq!(
        (progress.files, progress.bytes, progress.sent),
        (3, 100_017, 100_017)
    );
    assert_eq!(
        progress.attempt, 1,
        "nothing went wrong, so nothing was tried again"
    );
    assert_eq!(finished(&sending).await.unwrap(), sent);
}

#[tokio::test]
async fn files_the_person_declines_are_not_kept_and_the_sender_is_told() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let (_source, paths) = sample_files();

    let sending = b.app.send_files(&a.id(), paths).await.unwrap();
    let request = question(&mut a_events).await;
    assert!(a.app.answer_transfer(request.id, false));

    let error = finished(&sending).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined, "{error}");
    assert!(
        error.message().contains("Device A did not take the files"),
        "{error}"
    );
    assert!(tree(a.received()).is_empty(), "a no keeps nothing");
    assert!(
        none_comes(&mut a_events, |event| {
            matches!(event, Event::TransferEnded(_))
        })
        .await,
        "a transfer that was not taken has no end to show"
    );
}

#[tokio::test]
async fn a_sender_that_gives_up_takes_the_question_off_the_screen() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let (_source, paths) = sample_files();
    let sending = b.app.send_files(&a.id(), paths).await.unwrap();
    let request = question(&mut a_events).await;

    sending.cancel();

    next_matching(&mut a_events, |event| match event {
        Event::TransferRequestEnded { id } if id == request.id => Some(()),
        _ => None,
    })
    .await;
    assert!(
        !a.app.answer_transfer(request.id, true),
        "a question that was taken back cannot be answered"
    );
    let error = finished(&sending).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled, "{error}");
    assert!(tree(a.received()).is_empty());
}

#[tokio::test]
async fn a_send_that_is_given_up_while_it_goes_is_dropped_and_leaves_no_files() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    // Large enough that it cannot be over before the cancel is made. It is a hole in the
    // file system, so it costs nothing to make.
    let source = tempfile::tempdir().unwrap();
    let big = source.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(64 << 20)
        .unwrap();
    let sending = b.app.send_files(&a.id(), vec![big]).await.unwrap();
    let request = question(&mut a_events).await;
    assert!(a.app.answer_transfer(request.id, true));

    tokio::time::timeout(WAIT, async {
        while sending.progress().sent == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("no byte was sent");
    assert_eq!(sending.progress().stage, SendStage::Sending);
    sending.cancel();

    let error = finished(&sending).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled, "{error}");
    let end = next_matching(&mut a_events, |event| match event {
        Event::TransferEnded(end) => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end.request, request.id);
    assert_eq!(end.result, TransferResult::Dropped);
    assert!(end.saved.is_empty(), "nothing is kept, so nothing is shown");
    assert!(tree(a.received()).is_empty(), "half a file is never kept");
}

#[tokio::test]
async fn the_places_of_one_transfer_are_not_the_places_of_another() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    // A transfer that is still going when the other one is over: large enough that it cannot be
    // over first, and a hole in the file system, so it costs nothing to make.
    let source = tempfile::tempdir().unwrap();
    let big = source.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(256 << 20)
        .unwrap();
    let going = b.app.send_files(&a.id(), vec![big]).await.unwrap();
    let going_request = question(&mut a_events).await;
    assert!(a.app.answer_transfer(going_request.id, true));
    tokio::time::timeout(WAIT, async {
        while going.progress().sent == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("no byte was sent");

    let (_small_source, paths) = one_file("small.txt", b"small");
    let small = b.app.send_files(&a.id(), paths).await.unwrap();
    let small_request = question(&mut a_events).await;
    assert!(a.app.answer_transfer(small_request.id, true));
    finished(&small).await.unwrap();
    let small_end = next_matching(&mut a_events, |event| match event {
        Event::TransferEnded(end) => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(small_end.request, small_request.id);
    assert_eq!(small_end.saved, [a.received().join("small.txt")]);
    assert_eq!(
        going.progress().stage,
        SendStage::Sending,
        "the other transfer is still going"
    );

    // The one that is still going is given up, and keeps nothing. The place of the file that
    // arrived meanwhile is not its place.
    going.cancel();
    let error = finished(&going).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled, "{error}");
    let going_end = next_matching(&mut a_events, |event| match event {
        Event::TransferEnded(end) => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(going_end.request, going_request.id);
    assert_eq!(going_end.result, TransferResult::Dropped);
    assert!(
        going_end.saved.is_empty(),
        "nothing is kept, so nothing is shown"
    );
    assert_eq!(
        tree(a.received()),
        BTreeMap::from([("small.txt".to_owned(), Some(b"small".to_vec()))])
    );
}

#[tokio::test]
async fn letting_go_of_a_send_gives_the_transfer_up() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let (_source, paths) = sample_files();
    let sending = b.app.send_files(&a.id(), paths).await.unwrap();
    let request = question(&mut a_events).await;

    // A screen that goes away takes its transfer with it.
    drop(sending);

    next_matching(&mut a_events, |event| match event {
        Event::TransferRequestEnded { id } if id == request.id => Some(()),
        _ => None,
    })
    .await;
    assert!(!a.app.answer_transfer(request.id, true));
    assert!(tree(a.received()).is_empty());
}

#[tokio::test]
async fn an_app_that_is_opened_again_tells_the_receiver_to_let_go_of_what_it_left() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    // Large enough that it cannot be over before the app is closed. It is a hole in the file
    // system, so it costs nothing to make.
    let source = tempfile::tempdir().unwrap();
    let big = source.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(128 << 20)
        .unwrap();
    let sending = b.app.send_files(&a.id(), vec![big]).await.unwrap();
    let request = question(&mut a_events).await;
    assert!(a.app.answer_transfer(request.id, true));
    tokio::time::timeout(WAIT, async {
        while sending.progress().sent == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("no byte was sent");

    // The person closes the app in the middle of the transfer. Nothing is told to the receiver,
    // which keeps what it was sent until it lets go of it by itself.
    b.app.stop().await;
    let error = finished(&sending).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled, "{error}");
    assert!(
        none_comes(&mut a_events, |event| matches!(
            event,
            Event::TransferEnded(_)
        ))
        .await,
        "the receiver was not told"
    );

    // Opened again, the app gives up what it left there as soon as it reaches the device.
    let b = b.restarted().await;

    let end = next_matching(&mut a_events, |event| match event {
        Event::TransferEnded(end) => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end.request, request.id);
    assert_eq!(end.result, TransferResult::Dropped);
    assert!(tree(a.received()).is_empty(), "half a file is never kept");
    // Starting again did not change who the device is, and the receiver takes files from it again.
    let (_source, paths) = one_file("after.txt", b"after the restart");
    let sending = b.app.send_files(&a.id(), paths).await.unwrap();
    let request = question(&mut a_events).await;
    assert!(a.app.answer_transfer(request.id, true));
    finished(&sending).await.unwrap();
    assert_eq!(
        tree(a.received()),
        BTreeMap::from([("after.txt".to_owned(), Some(b"after the restart".to_vec()))])
    );
}

#[tokio::test]
async fn a_file_that_is_gone_when_its_turn_comes_is_counted_on_both_devices() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let source = tempfile::tempdir().unwrap();
    let keep = source.path().join("keep.txt");
    let gone = source.path().join("gone.txt");
    std::fs::write(&keep, b"still here").unwrap();
    std::fs::write(&gone, b"not for long").unwrap();
    let sending = b
        .app
        .send_files(&a.id(), vec![keep, gone.clone()])
        .await
        .unwrap();
    let request = question(&mut a_events).await;

    // The files are read only after the person says yes.
    std::fs::remove_file(&gone).unwrap();
    assert!(a.app.answer_transfer(request.id, true));

    let sent = finished(&sending).await.unwrap();
    assert_eq!(
        sent,
        FilesSent {
            files: 2,
            bytes: 22,
            failed: 1,
            left_out: 0
        }
    );
    let end = next_matching(&mut a_events, |event| match event {
        Event::TransferEnded(end) => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end.result, TransferResult::Received { failed: 1 });
    assert_eq!(
        tree(a.received()),
        BTreeMap::from([("keep.txt".to_owned(), Some(b"still here".to_vec()))])
    );
    assert_eq!(
        end.saved,
        [a.received().join("keep.txt")],
        "a file that did not arrive is not shown"
    );
}

#[tokio::test]
async fn a_name_that_is_taken_gets_a_number_and_the_end_says_where_the_file_went() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    std::fs::write(a.received().join("hello.txt"), b"was here before").unwrap();
    let (_source, paths) = one_file("hello.txt", b"hello from B");

    let sending = b.app.send_files(&a.id(), paths).await.unwrap();
    let request = question(&mut a_events).await;
    assert!(a.app.answer_transfer(request.id, true));

    finished(&sending).await.unwrap();
    let end = next_matching(&mut a_events, |event| match event {
        Event::TransferEnded(end) => Some(end),
        _ => None,
    })
    .await;
    // What the person was shown is the name it was sent with, and what they look for is the name
    // it has now.
    assert_eq!(end.preview[0].name, "hello.txt");
    assert_eq!(end.saved, [a.received().join("hello 2.txt")]);
    assert_eq!(
        std::fs::read(a.received().join("hello.txt")).unwrap(),
        b"was here before"
    );
    assert_eq!(
        std::fs::read(&end.saved[0]).unwrap(),
        b"hello from B",
        "the path is the file that arrived"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn things_that_are_left_out_of_a_folder_are_counted() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let source = tempfile::tempdir().unwrap();
    let docs = source.path().join("docs");
    std::fs::create_dir(&docs).unwrap();
    std::fs::write(docs.join("a.txt"), b"first").unwrap();
    // A link inside a folder is never followed or sent.
    std::os::unix::fs::symlink("a.txt", docs.join("link")).unwrap();

    let sending = b.app.send_files(&a.id(), vec![docs]).await.unwrap();
    let request = question(&mut a_events).await;
    assert_eq!((request.files, request.folders), (1, 1));
    assert!(a.app.answer_transfer(request.id, true));

    let sent = finished(&sending).await.unwrap();
    assert_eq!(
        sent,
        FilesSent {
            files: 1,
            bytes: 5,
            failed: 0,
            left_out: 1
        }
    );
    assert_eq!(
        tree(a.received()),
        BTreeMap::from([
            ("docs".to_owned(), None),
            ("docs/a.txt".to_owned(), Some(b"first".to_vec())),
        ])
    );
}

#[tokio::test]
async fn two_questions_at_once_are_answered_one_by_one() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let (_one, one) = one_file("one.txt", b"the first");
    let (_two, two) = one_file("two.txt", b"the second");
    let first = b.app.send_files(&a.id(), one).await.unwrap();
    let second = b.app.send_files(&a.id(), two).await.unwrap();

    let asked = [question(&mut a_events).await, question(&mut a_events).await];
    assert_ne!(asked[0].id, asked[1].id);
    let named = |name: &str| {
        asked
            .iter()
            .find(|request| request.preview[0].name == name)
            .unwrap()
            .id
    };
    assert!(a.app.answer_transfer(named("two.txt"), false));
    assert!(a.app.answer_transfer(named("one.txt"), true));

    assert_eq!(finished(&first).await.unwrap().files, 1);
    assert_eq!(
        finished(&second).await.unwrap_err().kind(),
        ErrorKind::Declined
    );
    assert_eq!(
        tree(a.received()),
        BTreeMap::from([("one.txt".to_owned(), Some(b"the first".to_vec()))])
    );
}

#[tokio::test]
async fn names_are_cleaned_before_the_person_sees_them() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    // A right-to-left override makes the end of a name read as something else.
    let (_source, paths) = one_file("photo\u{202E}gnp.exe", b"x");
    let sending = b.app.send_files(&a.id(), paths).await.unwrap();

    let request = question(&mut a_events).await;

    assert_eq!(request.preview[0].name, "photognp.exe");
    assert!(a.app.answer_transfer(request.id, false));
    finished(&sending).await.unwrap_err();
}

#[tokio::test]
async fn a_device_that_takes_no_files_says_so_to_the_sender() {
    let a = Dev::start("Device A").await;
    let b = Dev::start("Device B").await;
    assert_eq!(a.app.receiving(), Receiving::Off);
    pair(&a, &b).await;
    let (_source, paths) = sample_files();

    let sending = b.app.send_files(&a.id(), paths).await.unwrap();

    let error = finished(&sending).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined, "{error}");
    assert!(
        error.message().contains("Device A did not take the files"),
        "{error}"
    );
}

#[tokio::test]
async fn a_folder_that_cannot_be_used_leaves_the_device_running_and_taking_no_files() {
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), Some("Device A"), false);
    let nowhere = tempfile::tempdir().unwrap();
    let mut options = Options::new(dir.path());
    options.receive_dir = Some(nowhere.path().join("not-there"));
    let a = Dev {
        app: App::start(options).await.unwrap(),
        dir,
        received: None,
    };
    let b = Dev::start("Device B").await;
    assert_eq!(
        a.app.receiving(),
        Receiving::Unusable("The folder for received files is not there.".to_owned())
    );
    pair(&a, &b).await;
    let (_source, paths) = sample_files();

    let sending = b.app.send_files(&a.id(), paths).await.unwrap();

    let error = finished(&sending).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined, "{error}");
    assert!(
        nowhere.path().read_dir().unwrap().next().is_none(),
        "no folder was made for the files"
    );
}

#[tokio::test]
async fn files_that_cannot_be_sent_are_refused_before_anything_is_offered() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let (source, paths) = sample_files();

    // A device that is not paired.
    let stranger = "0".repeat(64);
    let error = b.app.send_files(&stranger, paths).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotPaired, "{error}");

    // Nothing chosen, and something that is not there.
    for chosen in [Vec::new(), vec![source.path().join("gone.txt")]] {
        let error = b.app.send_files(&a.id(), chosen).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadItem, "{error}");
    }
    assert!(
        none_comes(&mut a.app.events(), |event| {
            matches!(event, Event::TransferRequested(_))
        })
        .await,
        "nothing was offered"
    );
}

#[tokio::test]
async fn stopping_the_app_turns_a_question_that_is_showing_into_a_no() {
    let a = Dev::start_taking_files("Device A").await;
    let b = Dev::start("Device B").await;
    pair(&a, &b).await;
    let mut a_events = a.app.events();
    let (_source, paths) = sample_files();
    let _sending = b.app.send_files(&a.id(), paths).await.unwrap();
    let request = question(&mut a_events).await;

    a.app.stop().await;

    assert!(!a.app.answer_transfer(request.id, true));
    assert!(tree(a.received()).is_empty());
    let (_other, paths) = one_file("late.txt", b"x");
    assert_eq!(
        a.app.send_files(&b.id(), paths).await.unwrap_err().kind(),
        ErrorKind::Stopped
    );
}
