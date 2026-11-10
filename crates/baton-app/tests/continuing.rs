// SPDX-License-Identifier: Apache-2.0

//! What a person continues on another device, the way a screen drives it: a Mac whose platform
//! reads the page in front in the browser and the text on the clipboard, and a phone that asks. The
//! Mac's platform is played by the test, which answers [`Event::ContinueAsked`] as it likes.
//!
//! Both apps run on this computer and keep to its own addresses, so the tests run the same on a
//! laptop and in CI. Every wait has a limit, so a hang fails the test instead of stalling the run.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{path::Path, time::Duration};

use baton_app::{
    App, ContinueItems, ContinueOutcome, ContinuePage, ErrorKind, Event, Events, Options,
};
use baton_core::Identity;
use tempfile::TempDir;

/// Long enough for a slow machine, short enough that a hang does not take the run with it.
const WAIT: Duration = Duration::from_secs(30);

/// Long enough for something that should not happen to have happened.
const QUIET: Duration = Duration::from_millis(500);

/// One device: an app, and the folder it keeps its files in.
struct Dev {
    app: App,
    dir: TempDir,
    serves: bool,
}

impl Dev {
    /// A device that reads what its person works on when it is asked, if `serves`.
    async fn start(name: &str, serves: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), name);
        let app = App::start(options(dir.path(), serves)).await.unwrap();
        Self { app, dir, serves }
    }

    fn id(&self) -> String {
        self.app.me().id
    }

    /// The app is stopped and started again over the same folder, as when a program is closed and
    /// opened again.
    async fn restarted(self) -> Self {
        self.app.stop().await;
        let app = App::start(options(self.dir.path(), self.serves))
            .await
            .unwrap();
        Self { app, ..self }
    }
}

fn options(dir: &Path, serves: bool) -> Options {
    let mut options = Options::new(dir);
    options.serve_continue = serves;
    options
}

/// Settings that keep the device to this computer.
fn write_settings(dir: &Path, name: &str) {
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "device_name = \"{name}\"\n\n[network]\nrelay = \"disabled\"\nlan_discovery = false\n\
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

/// A Mac that reads, a phone that asks, and the two paired. The Mac showed the ticket, as it does.
struct Pair {
    mac: Dev,
    phone: Dev,
}

impl Pair {
    async fn new() -> Self {
        Self::with_mac(true).await
    }

    async fn with_mac(serves: bool) -> Self {
        let mac = Dev::start("Studio Mac", serves).await;
        let phone = Dev::start("Pocket Phone", false).await;
        pair(&mac, &phone).await;
        Self { mac, phone }
    }

    /// The person at the Mac lets the phone ask what they are working on.
    fn allow(&self) {
        self.mac
            .app
            .set_continue_access(&self.phone.id(), true)
            .unwrap();
    }

    /// The phone asks the Mac, and gives how it came out.
    async fn asks(&self) -> baton_app::Result<ContinueOutcome> {
        tokio::time::timeout(WAIT, self.phone.app.continue_from(&self.mac.id()))
            .await
            .expect("the question was never answered")
    }

    /// The phone asks the Mac while the platform of the Mac answers with `read`. The number of the
    /// request that the platform was given, and the name that it was told, come back with the
    /// outcome.
    async fn asks_and_is_read_for(
        &self,
        read: Option<ContinueItems>,
    ) -> (ContinueOutcome, (u64, String)) {
        let mut events = self.mac.app.events();
        let app = self.mac.app.clone();
        let platform = tokio::spawn(async move {
            let (id, from) = next_matching(&mut events, |event| match event {
                Event::ContinueAsked { id, from } => Some((id, from)),
                _ => None,
            })
            .await;
            assert!(app.answer_continue(id, read), "the request was not waiting");
            (id, from)
        });
        let outcome = self.asks().await.unwrap();
        (outcome, platform.await.unwrap())
    }

    /// The Mac is not read for, and says nothing of it for as long as it takes to be sure.
    async fn is_not_read_for(&self, events: &mut Events) {
        let read = tokio::time::timeout(QUIET, async {
            loop {
                match events.next().await {
                    Some(Event::ContinueAsked { .. }) => return,
                    Some(_) => {}
                    // The app is gone, and there is nothing more to wait for.
                    None => std::future::pending().await,
                }
            }
        })
        .await;
        assert!(read.is_err(), "the platform was asked to read");
    }
}

fn the_page() -> ContinuePage {
    ContinuePage {
        url: "https://example.com/report?q=1".to_owned(),
        title: Some("The report".to_owned()),
    }
}

fn what_is_open() -> ContinueItems {
    ContinueItems {
        page: Some(the_page()),
        clipboard: Some("a line that was copied".to_owned()),
    }
}

#[tokio::test]
async fn a_phone_that_is_allowed_is_given_what_the_person_at_the_mac_works_on() {
    let pair = Pair::new().await;
    pair.allow();

    let (outcome, (_, from)) = pair.asks_and_is_read_for(Some(what_is_open())).await;

    assert_eq!(outcome, ContinueOutcome::Items(what_is_open()));
    // The platform is told which device asks, by the name that is shown for it.
    assert_eq!(from, "Pocket Phone");
}

#[tokio::test]
async fn each_part_is_given_alone_and_a_mac_with_nothing_open_gives_nothing() {
    let pair = Pair::new().await;
    pair.allow();

    let only_the_page = ContinueItems {
        page: Some(ContinuePage {
            url: "https://example.com/".to_owned(),
            title: None,
        }),
        clipboard: None,
    };
    let (outcome, _) = pair.asks_and_is_read_for(Some(only_the_page.clone())).await;
    assert_eq!(outcome, ContinueOutcome::Items(only_the_page));

    let only_the_text = ContinueItems {
        page: None,
        clipboard: Some("only this".to_owned()),
    };
    let (outcome, _) = pair.asks_and_is_read_for(Some(only_the_text.clone())).await;
    assert_eq!(outcome, ContinueOutcome::Items(only_the_text));

    // A locked Mac, or one with no page and no text, is not an error: there is nothing to continue.
    let (outcome, _) = pair
        .asks_and_is_read_for(Some(ContinueItems::default()))
        .await;
    assert_eq!(outcome, ContinueOutcome::Items(ContinueItems::default()));
}

#[tokio::test]
async fn a_phone_is_turned_away_until_the_person_allows_it_and_the_mac_is_not_read_for_it() {
    let pair = Pair::new().await;
    let mut events = pair.mac.app.events();
    assert!(!pair.mac.app.continue_access(&pair.phone.id()).unwrap());

    // Just paired: nothing is given, and nothing is read.
    assert_eq!(pair.asks().await.unwrap(), ContinueOutcome::NotAllowed);
    pair.is_not_read_for(&mut events).await;

    pair.allow();
    assert!(pair.mac.app.continue_access(&pair.phone.id()).unwrap());
    let (outcome, _) = pair.asks_and_is_read_for(Some(what_is_open())).await;
    assert_eq!(outcome, ContinueOutcome::Items(what_is_open()));

    // The person takes it back, and the next ask is turned away without a read.
    pair.mac
        .app
        .set_continue_access(&pair.phone.id(), false)
        .unwrap();
    let mut events = pair.mac.app.events();
    assert_eq!(pair.asks().await.unwrap(), ContinueOutcome::NotAllowed);
    pair.is_not_read_for(&mut events).await;
}

#[tokio::test]
async fn a_platform_that_could_not_read_is_a_mac_that_cannot_give_it_just_now() {
    let pair = Pair::new().await;
    pair.allow();

    let (outcome, _) = pair.asks_and_is_read_for(None).await;

    assert_eq!(outcome, ContinueOutcome::Unavailable);
    // The next ask is read for as ever.
    let (outcome, _) = pair.asks_and_is_read_for(Some(what_is_open())).await;
    assert_eq!(outcome, ContinueOutcome::Items(what_is_open()));
}

#[tokio::test]
async fn a_mac_that_was_not_started_to_read_says_it_cannot() {
    let pair = Pair::with_mac(false).await;
    // What the person said does not change what the device can do.
    pair.allow();
    let mut events = pair.mac.app.events();

    assert_eq!(pair.asks().await.unwrap(), ContinueOutcome::Unsupported);
    pair.is_not_read_for(&mut events).await;
}

#[tokio::test]
async fn a_second_ask_while_the_first_is_being_read_is_told_to_wait() {
    let pair = Pair::new().await;
    pair.allow();
    let mut events = pair.mac.app.events();
    let first = {
        let app = pair.phone.app.clone();
        let mac = pair.mac.id();
        tokio::spawn(async move { app.continue_from(&mac).await })
    };
    let (id, _) = next_matching(&mut events, |event| match event {
        Event::ContinueAsked { id, from } => Some((id, from)),
        _ => None,
    })
    .await;

    // One question at a time is all that anybody needs, and the second is not read for.
    assert_eq!(pair.asks().await.unwrap(), ContinueOutcome::Busy);
    pair.is_not_read_for(&mut events).await;

    assert!(pair.mac.app.answer_continue(id, Some(what_is_open())));
    let outcome = tokio::time::timeout(WAIT, first)
        .await
        .expect("the first question was never answered")
        .unwrap()
        .unwrap();
    assert_eq!(outcome, ContinueOutcome::Items(what_is_open()));
}

#[tokio::test]
async fn an_answer_is_taken_once_and_only_for_a_request_that_is_waiting() {
    let pair = Pair::new().await;
    pair.allow();

    let (outcome, (id, _)) = pair.asks_and_is_read_for(Some(what_is_open())).await;
    assert_eq!(outcome, ContinueOutcome::Items(what_is_open()));

    // Answered already, and never asked.
    assert!(!pair.mac.app.answer_continue(id, Some(what_is_open())));
    assert!(!pair.mac.app.answer_continue(id + 1000, None));
}

#[tokio::test]
async fn a_phone_that_gives_up_is_not_waited_for() {
    let pair = Pair::new().await;
    pair.allow();
    let mut events = pair.mac.app.events();
    let asking = {
        let app = pair.phone.app.clone();
        let mac = pair.mac.id();
        tokio::spawn(async move { app.continue_from(&mac).await })
    };
    let (id, _) = next_matching(&mut events, |event| match event {
        Event::ContinueAsked { id, from } => Some((id, from)),
        _ => None,
    })
    .await;

    // The person leaves the screen: the ask is dropped on the phone, and the Mac is not asked to
    // wait for it. What the platform read afterwards is taken or turned away, and nothing is kept
    // that a later ask could be given.
    asking.abort();
    let _ = asking.await;
    let _ = pair.mac.app.answer_continue(id, Some(what_is_open()));

    // A new ask is read for afresh, whatever became of the last.
    let (outcome, (later, _)) = pair
        .asks_and_is_read_for(Some(ContinueItems {
            page: None,
            clipboard: Some("a newer line".to_owned()),
        }))
        .await;
    assert_ne!(later, id);
    assert_eq!(
        outcome,
        ContinueOutcome::Items(ContinueItems {
            page: None,
            clipboard: Some("a newer line".to_owned()),
        })
    );
}

#[tokio::test]
async fn a_mac_that_nobody_is_watching_cannot_be_read_and_says_so_at_once() {
    let pair = Pair::new().await;
    pair.allow();

    // No screen listens to the events of the Mac: nothing could read, and nobody waits for the time
    // that a read would have taken.
    let started = std::time::Instant::now();
    let outcome = pair.asks().await.unwrap();

    assert_eq!(outcome, ContinueOutcome::Unavailable);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the answer waited for a platform that is not there"
    );
}

#[tokio::test]
async fn the_person_allows_a_paired_device_and_only_a_paired_device() {
    let pair = Pair::new().await;
    let app = &pair.mac.app;

    let phone = pair.phone.id();
    for allowed in [true, false, true] {
        app.set_continue_access(&phone, allowed).unwrap();
        assert_eq!(app.continue_access(&phone).unwrap(), allowed);
    }

    // A device that is not paired is allowed nothing, and cannot be.
    let stranger = Identity::generate().device_id().to_string();
    assert_eq!(
        app.continue_access(&stranger).unwrap_err().kind(),
        ErrorKind::NotPaired
    );
    assert_eq!(
        app.set_continue_access(&stranger, true).unwrap_err().kind(),
        ErrorKind::NotPaired
    );
    assert_eq!(
        app.set_continue_access("not an id", true)
            .unwrap_err()
            .kind(),
        ErrorKind::NotPaired
    );
    // And a device that is not paired is not asked.
    assert_eq!(
        pair.phone
            .app
            .continue_from(&stranger)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::NotPaired
    );
}

#[tokio::test]
async fn what_the_person_allowed_is_kept_when_the_app_is_opened_again_and_goes_with_the_pairing() {
    let Pair { mac, phone } = Pair::new().await;
    mac.app.set_continue_access(&phone.id(), true).unwrap();

    let mac = mac.restarted().await;
    assert!(mac.app.continue_access(&phone.id()).unwrap());

    // Unpairing forgets the device and what it was allowed: pairing the same phone again starts
    // from nothing.
    mac.app.unpair(&phone.id()).unwrap();
    assert_eq!(
        mac.app.continue_access(&phone.id()).unwrap_err().kind(),
        ErrorKind::NotPaired
    );
    pair(&mac, &phone).await;
    assert!(!mac.app.continue_access(&phone.id()).unwrap());
}

#[test]
fn what_was_read_is_not_printed_in_the_logs() {
    let items = ContinueItems {
        page: Some(ContinuePage {
            url: "https://example.com/?token=SECRET".to_owned(),
            title: Some("A private title".to_owned()),
        }),
        clipboard: Some("hunter2".to_owned()),
    };
    let printed = format!("{items:?} {:?}", ContinueOutcome::Items(items.clone()));
    for secret in ["SECRET", "private", "hunter2", "example.com"] {
        assert!(!printed.contains(secret), "{printed}");
    }
    // The sizes are enough to see that something was read.
    assert!(printed.contains("clipboard_bytes"), "{printed}");
    assert!(printed.contains("url_bytes"), "{printed}");
}
