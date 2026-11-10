// SPDX-License-Identifier: Apache-2.0

//! A paired device asking another what its person is working on, to continue it
//! (`docs/protocol.md` section 3, Continue).
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use baton_core::{ContinueHandler, ContinueOutcome, Error, PairedDevice, util::BoxFuture};
use baton_proto::{
    ALPN_SESSION, DeviceId,
    message::{
        ClipboardText, ContinueAsk, ContinueItems, ContinueReply, ContinueRequest, DeclineReason,
        Hello, Link, Request, Response, capability,
    },
};
use common::{Device, Raw, Spec, device};
use tokio::sync::Notify;

/// A raw peer that the device was told is paired.
async fn paired_raw(target: &Device) -> (Raw, iroh::endpoint::Connection) {
    let raw = Raw::with_key(iroh::SecretKey::generate()).await;
    target
        .trust
        .pair(raw.id, &common::info("Raw"), common::START)
        .unwrap();
    let connection = raw.dial(target, ALPN_SESSION).await.unwrap();
    (raw, connection)
}

/// Sends `request` on a fresh stream and returns the answer.
async fn ask_raw(connection: &iroh::endpoint::Connection, request: &Request) -> Response {
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    send.write_all(&baton_proto::frame::encode(request).unwrap())
        .await
        .unwrap();
    send.finish().unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(10), recv.read_to_end(1 << 20))
        .await
        .expect("no answer")
        .unwrap();
    baton_proto::frame::decode::<Response>(&reply).unwrap()
}

fn introduction() -> Request {
    Request::Hello(Hello {
        protocol_minor: 0,
        device: common::info("Raw"),
        capabilities: vec![],
    })
}

fn the_question() -> Request {
    Request::Continue(ContinueRequest::Ask(ContinueAsk {}))
}

fn page() -> ContinueItems {
    ContinueItems {
        page: Some(Link {
            url: "https://example.com/article".to_owned(),
            title: Some("An article".to_owned()),
        }),
        clipboard: Some(ClipboardText {
            text: "the copied text".to_owned(),
        }),
    }
}

/// What a device reads when it is asked: whatever the test set, and who asked, in order.
struct Reads {
    gives: Mutex<Result<ContinueItems, DeclineReason>>,
    asked_by: Mutex<Vec<DeviceId>>,
    /// While this is set, a read waits for `release`.
    held: Mutex<bool>,
    release: Notify,
    reading: AtomicUsize,
}

impl Reads {
    fn giving(items: ContinueItems) -> Arc<Self> {
        Arc::new(Self {
            gives: Mutex::new(Ok(items)),
            asked_by: Mutex::new(Vec::new()),
            held: Mutex::new(false),
            release: Notify::new(),
            reading: AtomicUsize::new(0),
        })
    }

    fn will_give(&self, gives: Result<ContinueItems, DeclineReason>) {
        *self.gives.lock().unwrap() = gives;
    }

    fn asked_by(&self) -> Vec<DeviceId> {
        self.asked_by.lock().unwrap().clone()
    }

    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }

    fn let_go(&self) {
        *self.held.lock().unwrap() = false;
        self.release.notify_waiters();
    }
}

impl ContinueHandler for Reads {
    fn items<'a>(
        &'a self,
        asker: &'a PairedDevice,
    ) -> BoxFuture<'a, Result<ContinueItems, DeclineReason>> {
        Box::pin(async move {
            self.asked_by.lock().unwrap().push(asker.id);
            self.reading.fetch_add(1, Ordering::SeqCst);
            loop {
                let released = self.release.notified();
                if !*self.held.lock().unwrap() {
                    break;
                }
                released.await;
            }
            self.gives.lock().unwrap().clone()
        })
    }
}

/// A laptop that gives what its person works on, and a phone that asks, paired.
async fn laptop_and_phone(reads: &Arc<Reads>) -> (Device, Device) {
    let laptop = Spec::new("Laptop")
        .with_continue(Arc::clone(reads) as Arc<dyn ContinueHandler>)
        .start()
        .await;
    let phone = device("Phone").await;
    phone.pair_with(&laptop).await;
    (laptop, phone)
}

/// The person at `laptop` allows, or no longer allows, `phone` to ask.
fn allow(laptop: &Device, phone: &Device, allowed: bool) {
    assert!(
        laptop
            .trust
            .update(&phone.id, |device| device.policy.continuing = allowed)
            .unwrap()
    );
}

#[tokio::test]
async fn a_device_that_the_person_allowed_is_given_the_page_and_the_text() {
    let reads = Reads::giving(page());
    let (laptop, phone) = laptop_and_phone(&reads).await;
    allow(&laptop, &phone, true);

    let outcome = phone.node.continue_from(&laptop.id).await.unwrap();

    assert_eq!(outcome, ContinueOutcome::Items(page()));
    // The laptop read it for the phone, once, and for nobody else.
    assert_eq!(reads.asked_by(), [phone.id]);
}

#[tokio::test]
async fn a_device_that_is_not_allowed_is_told_so_and_nothing_is_read() {
    let reads = Reads::giving(page());
    let (laptop, phone) = laptop_and_phone(&reads).await;

    // Off is what a device just paired has.
    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Declined(DeclineReason::Blocked)
    );
    assert_eq!(reads.asked_by(), []);

    // The person says yes, and then no, and each counts at the next request: nothing was
    // remembered from the one before.
    allow(&laptop, &phone, true);
    assert!(matches!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(_)
    ));
    allow(&laptop, &phone, false);
    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Declined(DeclineReason::Blocked)
    );
    assert_eq!(reads.asked_by(), [phone.id]);
}

#[tokio::test]
async fn what_one_device_may_do_says_nothing_of_another() {
    let reads = Reads::giving(page());
    let (laptop, phone) = laptop_and_phone(&reads).await;
    let tablet = device("Tablet").await;
    tablet.pair_with(&laptop).await;
    allow(&laptop, &phone, true);

    assert!(matches!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(_)
    ));
    assert_eq!(
        tablet.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Declined(DeclineReason::Blocked)
    );
    assert_eq!(reads.asked_by(), [phone.id]);
}

#[tokio::test]
async fn nothing_to_continue_is_an_answer_and_not_a_refusal() {
    let reads = Reads::giving(ContinueItems::default());
    let (laptop, phone) = laptop_and_phone(&reads).await;
    allow(&laptop, &phone, true);

    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(ContinueItems::default())
    );
}

#[tokio::test]
async fn a_device_that_can_read_what_its_person_works_on_says_so_and_one_that_cannot_does_not() {
    let reads = Reads::giving(page());
    let (laptop, phone) = laptop_and_phone(&reads).await;
    allow(&laptop, &phone, true);
    let (_raw, connection) = paired_raw(&laptop).await;
    let Response::Hello(hello) = ask_raw(&connection, &introduction()).await else {
        panic!("not a hello");
    };
    assert!(hello.supports(capability::CONTINUE_SERVE), "{hello:?}");

    let bare = device("Bare").await;
    let (_raw, connection) = paired_raw(&bare).await;
    let Response::Hello(hello) = ask_raw(&connection, &introduction()).await else {
        panic!("not a hello");
    };
    assert!(!hello.supports(capability::CONTINUE_SERVE), "{hello:?}");
}

#[tokio::test]
async fn a_device_that_cannot_read_is_not_asked_and_one_that_is_asked_anyway_says_unsupported() {
    let laptop = device("Laptop").await;
    let phone = device("Phone").await;
    phone.pair_with(&laptop).await;
    allow(&laptop, &phone, true);

    // The phone does not send the request at all, since the laptop did not advertise it.
    let error = phone.node.continue_from(&laptop.id).await.unwrap_err();
    assert!(
        matches!(&error, Error::Unsupported(what) if what == capability::CONTINUE_SERVE),
        "{error:?}"
    );

    // A peer that sends it anyway is answered, not cut off, and nothing is read.
    let (_raw, connection) = paired_raw(&laptop).await;
    assert_eq!(
        ask_raw(&connection, &the_question()).await,
        Response::Continue(ContinueReply::Declined(DeclineReason::Unsupported))
    );
}

#[tokio::test]
async fn a_part_that_the_protocol_would_not_carry_is_left_out_and_the_other_is_given() {
    let reads = Reads::giving(ContinueItems {
        page: Some(Link {
            url: String::new(),
            title: None,
        }),
        clipboard: Some(ClipboardText {
            text: "kept".to_owned(),
        }),
    });
    let (laptop, phone) = laptop_and_phone(&reads).await;
    allow(&laptop, &phone, true);

    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(ContinueItems {
            page: None,
            clipboard: Some(ClipboardText {
                text: "kept".to_owned()
            }),
        })
    );

    reads.will_give(Ok(ContinueItems {
        page: Some(Link {
            url: "https://example.com/".to_owned(),
            title: None,
        }),
        clipboard: Some(ClipboardText {
            text: "x".repeat(baton_proto::limits::MAX_CLIPBOARD_TEXT_BYTES + 1),
        }),
    }));
    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(ContinueItems {
            page: Some(Link {
                url: "https://example.com/".to_owned(),
                title: None,
            }),
            clipboard: None,
        })
    );
}

#[tokio::test]
async fn the_reasons_that_the_handler_gives_are_the_ones_the_request_names() {
    let reads = Reads::giving(page());
    let (laptop, phone) = laptop_and_phone(&reads).await;
    allow(&laptop, &phone, true);

    for (gives, said) in [
        (DeclineReason::Blocked, DeclineReason::Blocked),
        (DeclineReason::Unsupported, DeclineReason::Unsupported),
        (DeclineReason::RateLimited, DeclineReason::RateLimited),
        (DeclineReason::Other, DeclineReason::Other),
        // The ones that mean nothing for this request are a failure to read.
        (DeclineReason::Refused, DeclineReason::Other),
        (DeclineReason::TooLarge, DeclineReason::Other),
        (DeclineReason::NoSpace, DeclineReason::Other),
        (DeclineReason::Invalid, DeclineReason::Other),
    ] {
        reads.will_give(Err(gives));
        assert_eq!(
            phone.node.continue_from(&laptop.id).await.unwrap(),
            ContinueOutcome::Declined(said),
            "{gives:?}"
        );
    }
}

#[tokio::test]
async fn a_read_that_takes_too_long_is_a_failure_to_read_and_is_let_go_of() {
    let reads = Reads::giving(page());
    let mut spec =
        Spec::new("Laptop").with_continue(Arc::clone(&reads) as Arc<dyn ContinueHandler>);
    spec.tuning.continue_prepare = Duration::from_millis(200);
    let laptop = spec.start().await;
    let phone = device("Phone").await;
    phone.pair_with(&laptop).await;
    allow(&laptop, &phone, true);

    reads.hold();
    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Declined(DeclineReason::Other)
    );

    // The read that ran out does not hold the place: the next one is read.
    reads.let_go();
    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(page())
    );
}

#[tokio::test]
async fn one_read_at_a_time_and_another_that_comes_meanwhile_is_told_to_wait() {
    let reads = Reads::giving(page());
    let (laptop, phone) = laptop_and_phone(&reads).await;
    let tablet = device("Tablet").await;
    tablet.pair_with(&laptop).await;
    allow(&laptop, &phone, true);
    allow(&laptop, &tablet, true);

    reads.hold();
    let first = {
        let node = Arc::clone(&phone.node);
        let laptop = laptop.id;
        tokio::spawn(async move { node.continue_from(&laptop).await })
    };
    // Wait until the laptop is reading for the phone.
    tokio::time::timeout(Duration::from_secs(5), async {
        while reads.reading.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the laptop never began to read");

    assert_eq!(
        tablet.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Declined(DeclineReason::RateLimited)
    );

    reads.let_go();
    assert_eq!(
        first.await.unwrap().unwrap(),
        ContinueOutcome::Items(page())
    );
    // The place is free again.
    assert!(matches!(
        tablet.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(_)
    ));
}

#[tokio::test]
async fn a_device_that_asks_too_often_is_told_to_slow_down() {
    let reads = Reads::giving(page());
    let mut spec =
        Spec::new("Laptop").with_continue(Arc::clone(&reads) as Arc<dyn ContinueHandler>);
    spec.config.handoff.max_items_per_minute = 3;
    let laptop = spec.start().await;
    let phone = device("Phone").await;
    phone.pair_with(&laptop).await;
    allow(&laptop, &phone, true);

    for _ in 0..3 {
        assert!(matches!(
            phone.node.continue_from(&laptop.id).await.unwrap(),
            ContinueOutcome::Items(_)
        ));
    }
    assert_eq!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Declined(DeclineReason::RateLimited)
    );
    assert_eq!(reads.asked_by().len(), 3);
}

#[tokio::test]
async fn a_device_that_is_not_allowed_does_not_use_up_what_the_ones_that_are_may_ask() {
    let reads = Reads::giving(page());
    let mut spec =
        Spec::new("Laptop").with_continue(Arc::clone(&reads) as Arc<dyn ContinueHandler>);
    spec.config.handoff.max_items_per_minute = 2;
    let laptop = spec.start().await;
    let phone = device("Phone").await;
    phone.pair_with(&laptop).await;

    // Refused at once, many times over, and none of them counts.
    for _ in 0..5 {
        assert_eq!(
            phone.node.continue_from(&laptop.id).await.unwrap(),
            ContinueOutcome::Declined(DeclineReason::Blocked)
        );
    }
    allow(&laptop, &phone, true);
    for _ in 0..2 {
        assert!(matches!(
            phone.node.continue_from(&laptop.id).await.unwrap(),
            ContinueOutcome::Items(_)
        ));
    }
}

#[tokio::test]
async fn a_device_that_was_unpaired_is_not_answered() {
    let reads = Reads::giving(page());
    let (laptop, phone) = laptop_and_phone(&reads).await;
    allow(&laptop, &phone, true);
    assert!(matches!(
        phone.node.continue_from(&laptop.id).await.unwrap(),
        ContinueOutcome::Items(_)
    ));

    laptop.trust.remove(&phone.id).unwrap();

    assert!(phone.node.continue_from(&laptop.id).await.is_err());
    assert_eq!(reads.asked_by().len(), 1);
}
