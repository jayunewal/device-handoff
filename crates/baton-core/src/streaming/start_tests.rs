// SPDX-License-Identifier: Apache-2.0

//! Sessions that the viewer starts, on real nodes: what a source checks before it gets ready and
//! what it answers, the offer that it makes then, and the viewer that takes that offer, and no
//! other, without asking its person.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use baton_proto::{
    id::SessionId,
    message::{DeclineReason, capability},
    stream::{
        NotStarted, Offer, SourceKind, Start, Starting, Stop, StopReason, Stopped, StreamReply,
        StreamRequest,
    },
};
use baton_stream::{
    input::{Button, Modifiers, PointerAction},
    input_queue::{Input, Refused},
};
use iroh::endpoint::Connection;
use tokio::sync::mpsc;

use super::{
    End, Picture, ShareOutcome, SourceHandle, StartHandler, StartOutcome, StartRequest,
    StartVerdict, ViewHandle, Wanted, WhyNot, WindowShare,
    answer_tests::{
        App, Heard, Plan, Platform, Spoken, Told, ask, declined, session, stop, stopped,
    },
    offer_tests::{fit, window},
    sound_tests::{aac, packet},
};
use crate::{
    Config, Identity, NodeBuilder, StartPolicy, TrustStore,
    error::{Error, Result},
    node::Tuning,
    testing::{PATIENCE, QUIET, TestNode},
    trust::PairedDevice,
    util::{BoxFuture, lock},
};

/// What the application of a source does when it is asked to start a session.
#[derive(Debug, Clone, Copy)]
enum Ready {
    /// Says that it will, and gives the request to the test, which makes the offer or does not.
    Yes,
    /// Says that it will not, for this reason.
    No(WhyNot),
    /// Does not answer, like a program that waits for a permission.
    Hang,
}

struct Prepares {
    ready: Mutex<Ready>,
    /// What it was asked for: what is wanted, and the control that the request could grant.
    asked: Mutex<Vec<(Wanted, bool)>>,
    requests: mpsc::UnboundedSender<StartRequest>,
}

impl Prepares {
    fn new(ready: Ready) -> (Arc<Self>, mpsc::UnboundedReceiver<StartRequest>) {
        let (requests, receiver) = mpsc::unbounded_channel();
        let prepares = Arc::new(Self {
            ready: Mutex::new(ready),
            asked: Mutex::default(),
            requests,
        });
        (prepares, receiver)
    }

    fn ready(&self, ready: Ready) {
        *lock(&self.ready) = ready;
    }

    fn asked(&self) -> Vec<(Wanted, bool)> {
        lock(&self.asked).clone()
    }
}

impl StartHandler for Prepares {
    fn start<'a>(
        &'a self,
        _from: &'a PairedDevice,
        request: StartRequest,
    ) -> BoxFuture<'a, StartVerdict> {
        lock(&self.asked).push((request.wanted, request.control));
        let ready = *lock(&self.ready);
        Box::pin(async move {
            match ready {
                // The place is held for as long as the test holds the request.
                Ready::Yes => {
                    let _ = self.requests.send(request);
                    StartVerdict::Starting
                }
                Ready::No(why) => StartVerdict::NotStarted(why),
                Ready::Hang => std::future::pending().await,
            }
        })
    }
}

fn start_under(session: SessionId, control: bool) -> StreamRequest {
    StreamRequest::Start(Start {
        session,
        wanted: Wanted::Screen,
        control,
    })
}

fn start_of(byte: u8, control: bool) -> StreamRequest {
    start_under(session(byte), control)
}

fn starting(byte: u8) -> StreamReply {
    StreamReply::Starting(Starting {
        session: session(byte),
    })
}

fn not_started_for(session: SessionId, why: WhyNot) -> StreamReply {
    StreamReply::NotStarted(NotStarted { session, why })
}

fn not_started(byte: u8, why: WhyNot) -> StreamReply {
    not_started_for(session(byte), why)
}

fn declined_for(session: SessionId, reason: DeclineReason) -> StreamReply {
    StreamReply::Declined(baton_proto::stream::Declined { session, reason })
}

fn stop_for(session: SessionId) -> StreamRequest {
    StreamRequest::Stop(Stop {
        session,
        reason: StopReason::Ended,
    })
}

fn stopped_for(session: SessionId) -> StreamReply {
    StreamReply::Stopped(Stopped { session })
}

/// A whole display, which is what a start asks for.
fn screen() -> WindowShare {
    WindowShare::new("", "", 3024, 1964).whole_display()
}

/// The offer of a display, written by hand.
fn display_offer(session: SessionId, control: bool) -> StreamRequest {
    StreamRequest::Offer(Offer {
        session,
        source: SourceKind::Display,
        title: String::new(),
        app: String::new(),
        width: 3024,
        height: 1964,
        control,
        private: false,
        unlock: false,
    })
}

/// The offer of a display that is controlled and kept private, written by hand.
fn private_display_offer(session: SessionId) -> StreamRequest {
    match display_offer(session, true) {
        StreamRequest::Offer(offer) => StreamRequest::Offer(Offer {
            private: true,
            ..offer
        }),
        other => other,
    }
}

fn viewing_of(outcome: Result<StartOutcome>) -> ViewHandle {
    match outcome {
        Ok(StartOutcome::Started(handle)) => handle,
        other => panic!("the session was not started: {other:?}"),
    }
}

fn source_of(shared: Result<ShareOutcome>) -> SourceHandle {
    match shared {
        Ok(ShareOutcome::Started(handle)) => handle,
        other => panic!("the screen was not shared: {other:?}"),
    }
}

/// A node that shows the screens of others and starts sessions with them, and a paired node that
/// shares its own screen and starts a session when it is asked to, as far as the person there
/// allowed that.
struct Rig {
    /// The device in the hand of the person.
    phone: TestNode,
    /// The device whose screen is looked at.
    mac: TestNode,
    /// The application of the phone. Its plan is what the person says to an offer that they are
    /// asked about.
    app: Arc<App>,
    /// The application of the Mac.
    prepares: Arc<Prepares>,
    requests: tokio::sync::Mutex<mpsc::UnboundedReceiver<StartRequest>>,
}

impl Rig {
    async fn new(plan: Plan, allowed: StartPolicy) -> Self {
        Self::with(plan, allowed, Tuning::default(), |_| {}).await
    }

    /// `configure` changes the settings of the Mac.
    async fn with(
        plan: Plan,
        allowed: StartPolicy,
        tuning: Tuning,
        configure: impl FnOnce(&mut Config),
    ) -> Self {
        Self::taking(plan, allowed, tuning, true, configure).await
    }

    /// The same, with a Mac that takes input from a viewer if `takes_input`, and does not
    /// otherwise.
    async fn taking(
        plan: Plan,
        allowed: StartPolicy,
        tuning: Tuning,
        takes_input: bool,
        configure: impl FnOnce(&mut Config),
    ) -> Self {
        Self::build(plan, allowed, tuning, takes_input, false, configure).await
    }

    /// The same, with a phone that plays sound and a Mac that was set up to send it.
    async fn sounding(plan: Plan, allowed: StartPolicy) -> Self {
        Self::build(plan, allowed, Tuning::default(), true, true, |_| {}).await
    }

    async fn build(
        plan: Plan,
        allowed: StartPolicy,
        tuning: Tuning,
        takes_input: bool,
        sound: bool,
        configure: impl FnOnce(&mut Config),
    ) -> Self {
        let app = App::new(plan);
        let (prepares, requests) = Prepares::new(Ready::Yes);
        let phone = TestNode::with(|builder| {
            let builder = builder
                .tuning(tuning.clone())
                .window_handler(Arc::clone(&app));
            if sound { builder.play_sound() } else { builder }
        })
        .await;
        let mac = TestNode::configured(configure, |builder| {
            let builder = builder
                .tuning(tuning)
                .share_windows(takes_input)
                .start_handler(Arc::clone(&prepares));
            if sound {
                builder.share_sound()
            } else {
                builder
            }
        })
        .await;
        mac.pair_with(&phone);
        mac.trust
            .update(&phone.id, |device| device.policy.start = allowed)
            .unwrap();
        Self {
            phone,
            mac,
            app,
            prepares,
            requests: tokio::sync::Mutex::new(requests),
        }
    }

    /// Changes what the person at the Mac allows the phone.
    fn allow(&self, allowed: StartPolicy) {
        self.mac
            .trust
            .update(&self.phone.id, |device| device.policy.start = allowed)
            .unwrap();
    }

    /// The connection that the phone keeps to the Mac, made if there is none.
    async fn connection(&self) -> Connection {
        self.phone
            .node
            .pooled_session(&self.mac.id)
            .await
            .unwrap()
            .connection
    }

    /// The connection that the Mac keeps to the phone, made if there is none.
    async fn mac_connection(&self) -> Connection {
        self.mac
            .node
            .pooled_session(&self.phone.id)
            .await
            .unwrap()
            .connection
    }

    /// A request from the phone to the Mac, written by hand.
    async fn ask(&self, request: StreamRequest) -> StreamReply {
        ask(&self.connection().await, request).await
    }

    /// The request that the application of the Mac was given when it said that it would start.
    async fn next_request(&self) -> StartRequest {
        tokio::time::timeout(PATIENCE, async { self.requests.lock().await.recv().await })
            .await
            .expect("the application of the source was not asked")
            .expect("the application of the source was dropped")
    }

    /// Starts a session as a person at the phone would.
    async fn start(&self, control: bool) -> Result<StartOutcome> {
        self.phone
            .node
            .start(&self.mac.id, Wanted::Screen, control, self.app.viewing())
            .await
    }

    /// Makes the offer for the next request as the platform of the Mac would, and gives what it
    /// heard.
    async fn mac_shares(&self) -> (Result<ShareOutcome>, Heard) {
        let request = self.next_request().await;
        let window = if request.control {
            screen().with_control()
        } else {
            screen()
        };
        let heard = Heard::default();
        let shared = self
            .mac
            .node
            .share_asked(request.slot, window, Platform(heard.clone()), fit)
            .await;
        (shared, heard)
    }

    /// Starts a session at the phone and makes the offer at the Mac.
    async fn started(&self, control: bool) -> (Result<StartOutcome>, Result<ShareOutcome>, Heard) {
        let (outcome, (shared, heard)) = tokio::join!(self.start(control), self.mac_shares());
        (outcome, shared, heard)
    }

    /// The session that the start of the phone waits for the offer of, if it waits.
    fn waiting_for(&self) -> Option<SessionId> {
        lock(&self.phone.node.streaming().table)
            .starting
            .as_ref()
            .map(|pending| pending.session)
    }

    /// Whether the Mac keeps the place of a session that it is to start or offer.
    fn mac_holds_a_place(&self) -> bool {
        lock(&self.mac.node.streaming().table).sending.is_some()
    }

    /// The session that runs on the Mac, if one does.
    fn running_on_mac(&self) -> Option<SessionId> {
        lock(&self.mac.node.streaming().table)
            .sessions
            .keys()
            .next()
            .copied()
    }

    /// Waits until `condition` holds.
    async fn until(&self, what: &str, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !condition(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

/// A picture is sent and shown, so that a position on the viewer has a picture to be in.
async fn show_a_picture(rig: &Rig, handle: &SourceHandle) {
    handle.begin().expect("no picture was wanted");
    handle
        .sent(Picture {
            bytes: vec![0x42; 4096],
            referenced: true,
            ltr: None,
            capture_time: 1000,
        })
        .unwrap()
        .expect("the frame was dropped");
    rig.until("the picture to be shown", |rig| {
        rig.app.log.has(&Told::Show(0))
    })
    .await;
}

// What a node says it does, and what it is set up to do.

#[tokio::test]
async fn a_node_advertises_starting_only_when_it_was_given_something_to_start_with() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Off).await;
    let plain = TestNode::with(|builder| builder.share_windows(true)).await;
    plain.pair_with(&rig.phone);

    let of_mac = rig.phone.node.pooled_session(&rig.mac.id).await.unwrap();
    assert!(of_mac.peer.supports(capability::STREAM_SHARE));
    assert!(of_mac.peer.supports(capability::STREAM_START));
    // A node that shares its screen and was given nothing to start with does not say it starts.
    let of_plain = rig.phone.node.pooled_session(&plain.id).await.unwrap();
    assert!(of_plain.peer.supports(capability::STREAM_SHARE));
    assert!(!of_plain.peer.supports(capability::STREAM_START));
    // The phone shows what others share. It is not asked to start anything.
    let of_phone = rig.mac.node.pooled_session(&rig.phone.id).await.unwrap();
    assert!(of_phone.peer.supports(capability::STREAM_VIEW));
    assert!(!of_phone.peer.supports(capability::STREAM_START));
}

#[tokio::test]
async fn a_way_to_start_needs_a_node_that_shares_its_windows() {
    let dir = tempfile::tempdir().unwrap();
    let identity = Identity::generate();
    let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
    let (prepares, _requests) = Prepares::new(Ready::Yes);
    let built = NodeBuilder::new(&identity, trust, Config::default())
        .start_handler(prepares)
        .bind()
        .await;
    assert!(
        matches!(built, Err(Error::Config(_))),
        "a node that cannot share was given a way to start"
    );
}

#[tokio::test]
async fn a_node_that_takes_no_part_declines_a_start_and_one_that_starts_nothing_says_so() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Off).await;
    let nothing = TestNode::start().await;
    let sharer = TestNode::with(|builder| builder.share_windows(true)).await;
    for node in [&nothing, &sharer] {
        node.pair_with(&rig.phone);
        // The person there allowed it, so that is not the reason.
        node.trust
            .update(&rig.phone.id, |device| {
                device.policy.start = StartPolicy::Control;
            })
            .unwrap();
    }
    let to_nothing = rig.phone.node.pooled_session(&nothing.id).await.unwrap();
    let to_sharer = rig.phone.node.pooled_session(&sharer.id).await.unwrap();
    assert_eq!(
        ask(&to_nothing.connection, start_of(1, false)).await,
        declined(1, DeclineReason::Unsupported)
    );
    assert_eq!(
        ask(&to_sharer.connection, start_of(1, false)).await,
        not_started(1, WhyNot::Unsupported)
    );
}

// What a source checks.

#[tokio::test]
async fn a_device_that_the_person_did_not_allow_is_refused_and_nothing_gets_ready() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Off).await;
    assert_eq!(
        rig.ask(start_of(1, false)).await,
        not_started(1, WhyNot::NotAllowed)
    );
    assert_eq!(
        rig.ask(start_of(2, true)).await,
        not_started(2, WhyNot::NotAllowed)
    );
    assert!(rig.prepares.asked().is_empty(), "the application was asked");

    // The setting is read each time: when the person turns it on, the device can start.
    rig.allow(StartPolicy::Look);
    assert_eq!(rig.ask(start_of(3, false)).await, starting(3));
    assert_eq!(rig.prepares.asked(), [(Wanted::Screen, false)]);

    // And when they turn it off, it cannot.
    drop(rig.next_request().await);
    rig.allow(StartPolicy::Off);
    assert_eq!(
        rig.ask(start_of(4, false)).await,
        not_started(4, WhyNot::NotAllowed)
    );
    assert_eq!(rig.prepares.asked().len(), 1);
}

#[tokio::test]
async fn the_control_a_start_may_have_is_what_the_person_allowed_and_the_viewer_wished_for() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let cases = [
        (StartPolicy::Look, false, false),
        (StartPolicy::Look, true, false),
        (StartPolicy::Control, false, false),
        (StartPolicy::Control, true, true),
    ];
    for (byte, (allowed, wished, granted)) in (1..).zip(cases) {
        rig.allow(allowed);
        assert_eq!(rig.ask(start_of(byte, wished)).await, starting(byte));
        let request = rig.next_request().await;
        let said = format!("allowed {allowed:?}, wished for {wished}");
        assert_eq!(request.control, granted, "{said}");
        assert_eq!(request.slot.control(), granted, "{said}");
        assert_eq!(request.wanted, Wanted::Screen);
        assert_eq!(request.slot.peer(), rig.phone.id);
    }
}

#[tokio::test]
async fn a_source_that_takes_no_input_grants_no_control_whatever_the_person_allowed() {
    // Without a bound, a start that cannot be offered would wait out the thirty seconds.
    let tuning = Tuning {
        start_wait: Duration::from_secs(5),
        ..Tuning::default()
    };
    let rig = Rig::taking(Plan::Refuse, StartPolicy::Control, tuning, false, |_| {}).await;
    let (outcome, shared, _) = rig.started(true).await;
    let viewing = viewing_of(outcome);
    let _source = source_of(shared);
    // The application was not told that the viewer may control, so the offer it made was not
    // refused for asking for what the node cannot take, and the viewer was shown a screen to look
    // at.
    assert_eq!(rig.prepares.asked(), [(Wanted::Screen, false)]);
    assert!(!viewing.control());
}

#[tokio::test]
async fn what_this_version_does_not_know_is_unsupported_and_nothing_gets_ready() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Control).await;
    let reply = rig
        .ask(StreamRequest::Start(Start {
            session: session(1),
            wanted: Wanted::Other,
            control: false,
        }))
        .await;
    assert_eq!(reply, not_started(1, WhyNot::Unsupported));
    assert!(rig.prepares.asked().is_empty(), "the application was asked");
}

#[tokio::test]
async fn the_answer_of_the_application_is_the_answer_and_a_no_lets_go_of_the_place() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    rig.prepares.ready(Ready::No(WhyNot::NeedsPermission));
    assert_eq!(
        rig.ask(start_of(1, false)).await,
        not_started(1, WhyNot::NeedsPermission)
    );
    rig.prepares.ready(Ready::No(WhyNot::Unavailable));
    assert_eq!(
        rig.ask(start_of(2, false)).await,
        not_started(2, WhyNot::Unavailable)
    );
    // The place was let go of each time, so the next start finds it free.
    rig.prepares.ready(Ready::Yes);
    assert_eq!(rig.ask(start_of(3, false)).await, starting(3));
}

#[tokio::test]
async fn an_application_that_is_slow_to_get_ready_is_cut_off_and_lets_go_of_the_place() {
    let tuning = Tuning {
        start_prepare: Duration::from_millis(300),
        ..Tuning::default()
    };
    let rig = Rig::with(Plan::Refuse, StartPolicy::Look, tuning, |_| {}).await;
    rig.prepares.ready(Ready::Hang);
    assert_eq!(
        rig.ask(start_of(1, false)).await,
        not_started(1, WhyNot::Failed)
    );
    rig.prepares.ready(Ready::Yes);
    assert_eq!(rig.ask(start_of(2, false)).await, starting(2));
}

#[tokio::test]
async fn an_id_seen_before_on_the_connection_is_invalid_and_on_another_it_is_a_retry() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let first = rig.connection().await;
    assert_eq!(ask(&first, start_of(1, false)).await, starting(1));
    // On the same connection nothing is repeated under an id.
    assert_eq!(
        ask(&first, start_of(1, false)).await,
        not_started(1, WhyNot::Invalid)
    );

    // On another connection the same start is a retry, because the answer may have been lost with
    // the connection that it was sent on: it is answered as the first was, the application is not
    // asked again, and the place is not taken twice.
    first.close(0u32.into(), b"a test");
    rig.until("the connection to be closed", |_| {
        first.close_reason().is_some()
    })
    .await;
    let second = rig.connection().await;
    assert_eq!(ask(&second, start_of(1, false)).await, starting(1));
    assert_eq!(rig.prepares.asked().len(), 1);
    // A start under another id meets the place that the first holds.
    assert_eq!(
        ask(&second, start_of(2, false)).await,
        not_started(2, WhyNot::Busy)
    );
}

#[tokio::test]
async fn a_start_is_a_retry_only_for_the_device_that_made_it() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    // A second device that the person at the Mac allowed as well.
    let other = TestNode::start().await;
    other.pair_with(&rig.mac);
    rig.mac
        .trust
        .update(&other.id, |device| device.policy.start = StartPolicy::Look)
        .unwrap();
    let theirs = other
        .node
        .pooled_session(&rig.mac.id)
        .await
        .unwrap()
        .connection;

    // The place is taken for the phone, and the offer is not made yet. The same id from another
    // device is not the retry of that start: it finds the place taken, and nothing is said to it
    // as if it had started anything.
    assert_eq!(rig.ask(start_of(1, false)).await, starting(1));
    assert_eq!(
        ask(&theirs, start_of(1, false)).await,
        not_started(1, WhyNot::Busy)
    );
    assert_eq!(rig.prepares.asked().len(), 1);
    drop(rig.next_request().await);

    // The session runs. Its id is not free for another device either, and is not a start of that
    // device that went through.
    let (outcome, shared, _) = rig.started(false).await;
    let _viewing = viewing_of(outcome);
    let _source = source_of(shared);
    let running = rig.running_on_mac().expect("no session runs on the Mac");
    assert_eq!(
        ask(&theirs, start_under(running, false)).await,
        not_started_for(running, WhyNot::Invalid)
    );
    assert_eq!(rig.prepares.asked().len(), 2);
}

#[tokio::test]
async fn a_start_while_another_goes_or_an_offer_is_made_is_busy() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    assert_eq!(rig.ask(start_of(1, false)).await, starting(1));
    let request = rig.next_request().await;
    assert_eq!(
        rig.ask(start_of(2, false)).await,
        not_started(2, WhyNot::Busy)
    );
    // The person at the Mac cannot offer a window of their own meanwhile either.
    let shared = rig
        .mac
        .node
        .share(&rig.phone.id, window(), Platform(Heard::default()), fit)
        .await;
    assert!(matches!(shared, Err(Error::Busy)), "{shared:?}");
    assert_eq!(rig.prepares.asked().len(), 1);

    // When the place is let go of, a start goes on.
    drop(request);
    assert_eq!(rig.ask(start_of(3, false)).await, starting(3));
}

#[tokio::test]
async fn too_many_starts_in_a_minute_are_rate_limited_and_a_refused_start_costs_nothing() {
    let rig = Rig::with(
        Plan::Refuse,
        StartPolicy::Off,
        Tuning::default(),
        |config| {
            config.handoff.max_items_per_minute = 2;
        },
    )
    .await;
    for byte in 1..=5 {
        assert_eq!(
            rig.ask(start_of(byte, false)).await,
            not_started(byte, WhyNot::NotAllowed)
        );
    }
    rig.allow(StartPolicy::Look);
    rig.prepares.ready(Ready::No(WhyNot::Unavailable));
    for byte in 6..=7 {
        assert_eq!(
            rig.ask(start_of(byte, false)).await,
            not_started(byte, WhyNot::Unavailable)
        );
    }
    assert_eq!(
        rig.ask(start_of(8, false)).await,
        not_started(8, WhyNot::RateLimited)
    );
    assert_eq!(rig.prepares.asked().len(), 2);
}

#[tokio::test]
async fn a_stop_takes_back_the_asking_whose_offer_waits_and_no_other_stop_does() {
    // The offer is made to a phone that is not waiting for it, so the person is asked, and does
    // not answer.
    let rig = Rig::new(Plan::Wait, StartPolicy::Look).await;
    let other = TestNode::start().await;
    other.pair_with(&rig.mac);
    let theirs = other
        .node
        .pooled_session(&rig.mac.id)
        .await
        .unwrap()
        .connection;

    assert_eq!(rig.ask(start_of(1, false)).await, starting(1));
    let request = rig.next_request().await;
    let sharing = rig
        .mac
        .node
        .share_asked(request.slot, screen(), Platform(Heard::default()), fit);
    tokio::pin!(sharing);
    tokio::select! {
        outcome = &mut sharing => panic!("the offer was answered: {outcome:?}"),
        () = rig.until("the person to be asked", |rig| rig.app.asked().len() == 1) => {}
    }

    // A stop for another session takes nothing back, and neither does one from another device.
    assert_eq!(rig.ask(stop(9)).await, stopped(9));
    assert_eq!(ask(&theirs, stop(1)).await, stopped(1));
    tokio::select! {
        outcome = &mut sharing => panic!("the offer was given up: {outcome:?}"),
        () = tokio::time::sleep(QUIET) => {}
    }
    assert_eq!(rig.app.taken_back(), 0);

    // The stop of the device that asked takes it back, and the question with it.
    assert_eq!(rig.ask(stop(1)).await, stopped(1));
    let outcome = tokio::time::timeout(PATIENCE, &mut sharing)
        .await
        .expect("the offer was not given up");
    assert!(
        matches!(outcome, Ok(ShareOutcome::Cancelled)),
        "{outcome:?}"
    );
    rig.until("the question to be taken back", |rig| {
        rig.app.taken_back() == 1
    })
    .await;
    // The place is free again.
    assert_eq!(rig.ask(start_of(2, false)).await, starting(2));
}

#[tokio::test]
async fn the_slot_grants_no_more_control_than_the_person_allowed() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    assert_eq!(rig.ask(start_of(1, true)).await, starting(1));
    let request = rig.next_request().await;
    assert!(!request.control);
    let shared = rig
        .mac
        .node
        .share_asked(
            request.slot,
            screen().with_control(),
            Platform(Heard::default()),
            fit,
        )
        .await;
    assert!(matches!(shared, Err(Error::Config(_))), "{shared:?}");
    // The place was let go of when the call failed.
    assert_eq!(rig.ask(start_of(2, false)).await, starting(2));
}

// A session that the viewer starts.

#[tokio::test]
async fn a_session_the_viewer_starts_runs_and_nobody_is_asked() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let (outcome, shared, heard) = rig.started(false).await;
    let viewing = viewing_of(outcome);
    let source = source_of(shared);

    // The person at the phone asked when they started, and was not asked again. Had they been,
    // the plan of the test would have refused, and nothing would be running.
    assert!(rig.app.asked().is_empty(), "the person was asked");
    assert!(!viewing.control());
    assert!(rig.waiting_for().is_none());

    // The platform of the Mac hears the reports of the phone, and a picture is shown.
    rig.until("the source to hear a report", |_| heard.count("report") > 0)
        .await;
    show_a_picture(&rig, &source).await;

    // The person closes the screen at the phone, and the Mac hears of it.
    viewing.end();
    assert_eq!(
        tokio::time::timeout(PATIENCE, source.ended())
            .await
            .expect("the source did not hear that the session ended"),
        End::Peer
    );
}

#[tokio::test]
async fn sound_goes_through_a_session_the_viewer_starts() {
    let rig = Rig::sounding(Plan::Refuse, StartPolicy::Look).await;
    let (outcome, shared, heard) = rig.started(false).await;
    let viewing = viewing_of(outcome);
    let source = source_of(shared);

    // The Mac advertised sound, and the start took the offer of a whole display as one with it.
    assert!(viewing.can_hear());
    assert!(viewing.hear(true));
    rig.until("the platform to be asked for sound", |_| {
        heard.count("hear") == 1
    })
    .await;
    source.begin_sound(aac()).unwrap();
    rig.until("the speaker to be told the form", |rig| {
        rig.app.speech.all() == [Spoken::Sound(aac())]
    })
    .await;
    assert_eq!(source.sound(packet(1, 0)).unwrap(), Some(0));
    rig.until("the packet to be played", |rig| {
        rig.app.speech.played().len() == 1
    })
    .await;
    assert_eq!(rig.app.speech.played(), [(0, true, 0, vec![1; 300])]);

    // The person turns it off, and the platform is told to let the sound of the screen play.
    assert!(viewing.hear(false));
    rig.until("the platform to be told", |_| heard.count("hush") == 1)
        .await;
}

#[tokio::test]
async fn control_comes_with_the_wish_and_the_allowance_and_with_neither_alone() {
    for (allowed, wished, granted) in [
        (StartPolicy::Look, false, false),
        (StartPolicy::Look, true, false),
        (StartPolicy::Control, false, false),
        (StartPolicy::Control, true, true),
    ] {
        let said = format!("allowed {allowed:?}, wished for {wished}");
        let rig = Rig::new(Plan::Refuse, allowed).await;
        let (outcome, shared, heard) = rig.started(wished).await;
        let viewing = viewing_of(outcome);
        let source = source_of(shared);
        assert_eq!(viewing.control(), granted, "{said}");
        show_a_picture(&rig, &source).await;

        let tap = Input::Pointer {
            x: 0.25,
            y: 0.75,
            modifiers: Modifiers::NONE,
            action: PointerAction::Down(Button::Left),
        };
        if granted {
            viewing.input(tap).unwrap();
            rig.until("the source to hear the input", |_| heard.count("input") > 0)
                .await;
        } else {
            assert_eq!(viewing.input(tap), Err(Refused::NotGranted), "{said}");
            tokio::time::sleep(QUIET).await;
            assert_eq!(heard.count("input"), 0, "{said}");
        }
    }
}

#[tokio::test]
async fn a_device_that_is_not_allowed_is_told_so_and_can_start_once_it_is() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Off).await;
    let outcome = rig.start(false).await;
    assert!(
        matches!(outcome, Ok(StartOutcome::NotStarted(WhyNot::NotAllowed))),
        "{outcome:?}"
    );
    assert!(rig.waiting_for().is_none(), "the place was not let go of");

    rig.allow(StartPolicy::Look);
    let (outcome, shared, _) = rig.started(false).await;
    let _viewing = viewing_of(outcome);
    let _source = source_of(shared);
}

#[tokio::test]
async fn a_device_that_does_not_start_sessions_is_not_asked_to() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let plain = TestNode::with(|builder| builder.share_windows(true)).await;
    plain.pair_with(&rig.phone);
    let outcome = rig
        .phone
        .node
        .start(&plain.id, Wanted::Screen, false, rig.app.viewing())
        .await;
    assert!(
        matches!(&outcome, Err(Error::Unsupported(what)) if what == capability::STREAM_START),
        "{outcome:?}"
    );
    assert!(rig.waiting_for().is_none(), "the place was not let go of");
}

#[tokio::test]
async fn an_offer_that_nobody_asked_for_is_asked_about_as_ever() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    // A start that came to nothing leaves nothing behind that could take a later offer.
    rig.prepares.ready(Ready::No(WhyNot::Unavailable));
    let outcome = rig.start(false).await;
    assert!(
        matches!(outcome, Ok(StartOutcome::NotStarted(WhyNot::Unavailable))),
        "{outcome:?}"
    );
    assert!(rig.waiting_for().is_none());

    // The person at the Mac offers a window of their own, and the person at the phone is asked,
    // and says no.
    let shared = rig
        .mac
        .node
        .share(&rig.phone.id, window(), Platform(Heard::default()), fit)
        .await;
    assert!(
        matches!(shared, Ok(ShareOutcome::Declined(DeclineReason::Refused))),
        "{shared:?}"
    );
    assert_eq!(rig.app.asked().len(), 1);
}

#[tokio::test]
async fn an_offer_from_another_device_or_under_another_id_is_not_taken_for_the_answer() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    // A device that the phone is paired with too, and that offers it a display by hand.
    let other = TestNode::with(|builder| builder.share_windows(false)).await;
    other.pair_with(&rig.phone);
    let from_other = other
        .node
        .pooled_session(&rig.phone.id)
        .await
        .unwrap()
        .connection;
    let from_mac = rig.mac_connection().await;

    let offering = async {
        let request = rig.next_request().await;
        let asked_for = rig.waiting_for().expect("the start does not wait");
        // The id that the phone waits for, from a device it did not ask.
        assert_eq!(
            ask(&from_other, display_offer(asked_for, false)).await,
            declined_for(asked_for, DeclineReason::RateLimited)
        );
        // The device it asked, under another id.
        assert_eq!(
            ask(&from_mac, display_offer(session(0x77), false)).await,
            declined(0x77, DeclineReason::RateLimited)
        );
        assert!(rig.app.asked().is_empty(), "the person was asked");
        assert_eq!(rig.waiting_for(), Some(asked_for), "the start was taken");

        // The offer that answers it is taken.
        rig.mac
            .node
            .share_asked(request.slot, screen(), Platform(Heard::default()), fit)
            .await
    };
    let (outcome, shared) = tokio::join!(rig.start(false), offering);
    let _viewing = viewing_of(outcome);
    let _source = source_of(shared);
    assert!(rig.app.asked().is_empty(), "the person was asked");
}

#[tokio::test]
async fn only_the_device_that_was_asked_takes_the_start_back_and_only_under_its_id() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let other = TestNode::with(|builder| builder.share_windows(false)).await;
    other.pair_with(&rig.phone);
    let from_other = other
        .node
        .pooled_session(&rig.phone.id)
        .await
        .unwrap()
        .connection;
    let from_mac = rig.mac_connection().await;

    let stopping = async {
        let request = rig.next_request().await;
        let asked_for = rig.waiting_for().expect("the start does not wait");
        // The id that the phone waits for, from a device it did not ask.
        assert_eq!(
            ask(&from_other, stop_for(asked_for)).await,
            stopped_for(asked_for)
        );
        // The device it asked, for another session.
        assert_eq!(ask(&from_mac, stop(0x66)).await, stopped(0x66));
        tokio::time::sleep(QUIET).await;
        assert_eq!(
            rig.waiting_for(),
            Some(asked_for),
            "the start was taken back"
        );

        // The device it asked, for the session it waits for, takes the start back.
        assert_eq!(
            ask(&from_mac, stop_for(asked_for)).await,
            stopped_for(asked_for)
        );
        request
    };
    let (outcome, _request) = tokio::join!(rig.start(false), stopping);
    assert!(
        matches!(outcome, Ok(StartOutcome::NotStarted(WhyNot::Failed))),
        "{outcome:?}"
    );
    assert!(rig.waiting_for().is_none());
}

#[tokio::test]
async fn an_offer_of_another_kind_than_was_asked_for_is_declined_and_ends_the_start() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    // The phone asked for a screen, and the Mac offers a window.
    let offering = async {
        let request = rig.next_request().await;
        rig.mac
            .node
            .share_asked(request.slot, window(), Platform(Heard::default()), fit)
            .await
    };
    let (outcome, shared) = tokio::join!(rig.start(false), offering);
    assert!(
        matches!(outcome, Ok(StartOutcome::NotStarted(WhyNot::Failed))),
        "{outcome:?}"
    );
    assert!(
        matches!(shared, Ok(ShareOutcome::Declined(DeclineReason::Invalid))),
        "{shared:?}"
    );
    assert!(rig.app.asked().is_empty(), "the person was asked");
    assert!(rig.waiting_for().is_none());
}

#[tokio::test]
async fn a_viewer_takes_no_more_control_than_the_person_wished_for() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Control).await;
    // A source that offers control that was not wished for, by hand.
    let offering = async {
        let request = rig.next_request().await;
        assert!(!request.control);
        let reply = ask(
            &rig.mac_connection().await,
            display_offer(request.slot.session, true),
        )
        .await;
        assert!(matches!(reply, StreamReply::Accepted(_)), "{reply:?}");
        request
    };
    let (outcome, _request) = tokio::join!(rig.start(false), offering);
    let viewing = viewing_of(outcome);
    assert!(!viewing.control());
    // Nothing was said of a private screen either.
    assert!(!viewing.private());
}

#[tokio::test]
async fn a_screen_that_the_source_keeps_private_is_private_for_the_viewer_that_asked_for_it() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Control).await;
    let offering = async {
        let request = rig.next_request().await;
        assert!(request.control);
        let reply = ask(
            &rig.mac_connection().await,
            private_display_offer(request.slot.session),
        )
        .await;
        assert!(matches!(reply, StreamReply::Accepted(_)), "{reply:?}");
        request
    };
    let (outcome, _request) = tokio::join!(rig.start(true), offering);
    let viewing = viewing_of(outcome);
    assert!(viewing.control());
    assert!(viewing.private());
    // The viewer can say what the people at the source may do.
    assert!(viewing.lift(true, false));
    assert!(viewing.lift(false, false));
}

#[tokio::test]
async fn a_start_whose_offer_does_not_come_gives_up_and_a_late_offer_is_asked_about() {
    let tuning = Tuning {
        start_wait: Duration::from_millis(300),
        ..Tuning::default()
    };
    let rig = Rig::with(Plan::Refuse, StartPolicy::Look, tuning, |_| {}).await;
    // The source says that it will offer, and does not.
    let (outcome, request) = tokio::join!(rig.start(false), rig.next_request());
    assert!(matches!(outcome, Err(Error::Timeout(_))), "{outcome:?}");
    assert!(rig.waiting_for().is_none());

    // The phone told the source to give the offer up.
    rig.until("the source to be told", |_| {
        request.slot.reservation.is_withdrawn()
    })
    .await;

    // An offer that was on its way meanwhile answers nothing, so the person is asked, as for any
    // offer.
    let session = request.slot.session;
    let late = ask(&rig.mac_connection().await, display_offer(session, false)).await;
    assert_eq!(late, declined_for(session, DeclineReason::Refused));
    assert_eq!(rig.app.asked().len(), 1);

    // And when the source gets to it, there is nothing to offer: the asking was taken back.
    let shared = rig
        .mac
        .node
        .share_asked(request.slot, screen(), Platform(Heard::default()), fit)
        .await;
    assert!(matches!(shared, Ok(ShareOutcome::Cancelled)), "{shared:?}");
    assert_eq!(rig.app.asked().len(), 1);
}

#[tokio::test]
async fn giving_up_a_start_tells_the_source_to_give_up_the_offer() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let from_mac = rig.mac_connection().await;
    // That the offer is not made is told by the cancellation being looked at first. If it were
    // not, the offer would go out in about half of the tries, so one is not enough.
    for _ in 0..8 {
        let mut waiting = Box::pin(rig.start(false));
        let request = tokio::select! {
            outcome = &mut waiting => panic!("the start was answered: {outcome:?}"),
            request = rig.next_request() => request,
        };
        // The person at the phone gives up while the source gets ready: the call is dropped.
        drop(waiting);
        rig.until("the source to be told", |_| {
            request.slot.reservation.is_withdrawn()
        })
        .await;
        assert!(rig.waiting_for().is_none());

        let shared = rig
            .mac
            .node
            .share_asked(request.slot, screen(), Platform(Heard::default()), fit)
            .await;
        assert!(matches!(shared, Ok(ShareOutcome::Cancelled)), "{shared:?}");
        // A request on the same connection is handled after an offer that went out before it, so
        // the phone has dealt with that offer, if there was one, before the next try starts a
        // start of its own that would turn the offer away.
        assert_eq!(ask(&from_mac, stop(0x77)).await, stopped(0x77));
    }
    // Nothing was offered, so the person was not asked about an offer that nobody wanted.
    assert!(rig.app.asked().is_empty(), "the person was asked");
    // Both places are free, so another start goes through.
    assert_eq!(rig.ask(start_of(0x42, false)).await, starting(0x42));
}

#[tokio::test]
async fn a_start_that_is_taken_back_while_the_application_gets_ready_lets_go_of_the_place_at_once()
{
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    // The application of the Mac waits, like one that waits for a permission.
    rig.prepares.ready(Ready::Hang);
    let mut waiting = Box::pin(rig.start(false));
    tokio::select! {
        outcome = &mut waiting => panic!("the start was answered: {outcome:?}"),
        () = rig.until("the application to be asked", |rig| !rig.prepares.asked().is_empty()) => {}
    }
    assert!(rig.mac_holds_a_place());

    // The person at the phone gives up. The Mac hears it, and does not leave the place to an
    // application that gets ready for nobody.
    drop(waiting);
    rig.until("the Mac to let go of the place", |rig| {
        !rig.mac_holds_a_place()
    })
    .await;

    // So the person can ask again at once, and is not told that the Mac is busy.
    rig.prepares.ready(Ready::Yes);
    let (outcome, shared, _) = rig.started(false).await;
    let _viewing = viewing_of(outcome);
    let _source = source_of(shared);
}

#[tokio::test]
async fn a_start_straight_after_one_that_was_given_up_waits_for_the_source_to_let_go() {
    let tuning = Tuning {
        start_wait: Duration::from_millis(300),
        ..Tuning::default()
    };
    let rig = Rig::with(Plan::Refuse, StartPolicy::Look, tuning, |_| {}).await;
    // The source says that it will offer, and does not.
    let (outcome, request) = tokio::join!(rig.start(false), rig.next_request());
    assert!(matches!(outcome, Err(Error::Timeout(_))), "{outcome:?}");
    // Its application lets go of the place when it hears that the asking was taken back, as one
    // that was about to make the offer does.
    let letting_go = tokio::spawn(async move {
        request.slot.reservation.cancelled().await;
        drop(request);
    });

    // The `Stop` that took the asking back is on its way. The next start waits for it to be
    // answered, or the source would still hold the place and turn it away as busy.
    let (outcome, shared, _) = rig.started(false).await;
    let _viewing = viewing_of(outcome);
    let _source = source_of(shared);
    letting_go.await.unwrap();
}

#[tokio::test]
async fn a_start_is_busy_while_another_waits_or_a_window_is_shown() {
    let rig = Rig::new(Plan::Accept, StartPolicy::Look).await;
    let request = {
        let mut first = Box::pin(rig.start(false));
        let request = tokio::select! {
            outcome = &mut first => panic!("the start was answered: {outcome:?}"),
            request = rig.next_request() => request,
        };
        // The viewer has one place.
        let second = rig.start(false).await;
        assert!(matches!(second, Err(Error::Busy)), "{second:?}");
        request
    };
    // Giving the first up lets go of its place at the phone, and the request lets go of the one
    // at the Mac.
    rig.until("the place to be let go of", |rig| {
        rig.waiting_for().is_none()
    })
    .await;
    drop(request);

    // A window that the Mac offered and the person accepted is shown. Starting waits for it.
    let shared = rig
        .mac
        .node
        .share(&rig.phone.id, window(), Platform(Heard::default()), fit)
        .await;
    let _source = source_of(shared);
    let outcome = rig.start(false).await;
    assert!(matches!(outcome, Err(Error::Busy)), "{outcome:?}");
}

#[tokio::test]
async fn a_start_is_busy_while_the_person_is_asked_about_an_offer() {
    // The Mac offers a window of its own, and the person at the phone is asked, and does not
    // answer.
    let rig = Rig::new(Plan::Wait, StartPolicy::Look).await;
    let offering = rig
        .mac
        .node
        .share(&rig.phone.id, window(), Platform(Heard::default()), fit);
    tokio::pin!(offering);
    tokio::select! {
        outcome = &mut offering => panic!("the offer was answered: {outcome:?}"),
        () = rig.until("the person to be asked", |rig| rig.app.asked().len() == 1) => {}
    }

    // A start does not take the place of that question, and the Mac is not asked to get ready.
    let outcome = rig.start(false).await;
    assert!(matches!(outcome, Err(Error::Busy)), "{outcome:?}");
    assert!(rig.prepares.asked().is_empty(), "the Mac was asked");
}

#[tokio::test]
async fn letting_go_of_a_start_that_was_answered_leaves_the_place_of_the_next_one() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let streaming = rig.phone.node.streaming();
    let expect = |byte| {
        streaming.expect_start(
            rig.mac.id,
            session(byte),
            Wanted::Screen,
            false,
            rig.app.viewing(),
        )
    };
    let first = expect(1).unwrap();
    // The offer came and was taken, so the place is not the start's any more.
    assert!(lock(&streaming.table).starting.take().is_some());
    let second = expect(2).unwrap();

    // The first start is over, and it must not take the place that the second holds.
    drop(first);
    assert_eq!(rig.waiting_for(), Some(session(2)));
    drop(second);
    assert_eq!(rig.waiting_for(), None);
}

#[tokio::test]
async fn a_session_the_viewer_started_gives_back_its_places_when_it_ends() {
    let rig = Rig::new(Plan::Refuse, StartPolicy::Look).await;
    let (outcome, shared, _) = rig.started(false).await;
    let viewing = viewing_of(outcome);
    let source = source_of(shared);
    // While it runs, the Mac starts no other.
    assert_eq!(
        rig.ask(start_of(0x51, false)).await,
        not_started(0x51, WhyNot::Busy)
    );

    viewing.end();
    tokio::time::timeout(PATIENCE, source.ended())
        .await
        .expect("the source did not hear that the session ended");
    rig.until("both ends to forget the session", |rig| {
        rig.phone.node.streaming().is_empty() && rig.mac.node.streaming().is_empty()
    })
    .await;
    let (outcome, shared, _) = rig.started(false).await;
    let _viewing = viewing_of(outcome);
    let _source = source_of(shared);
}

#[tokio::test]
async fn unpairing_the_source_or_stopping_the_node_gives_up_a_start_that_waits() {
    // Without the line that drops a start that waits, the call would wait out this time.
    let tuning = Tuning {
        start_wait: Duration::from_secs(5),
        ..Tuning::default()
    };
    for stopping in [false, true] {
        let rig = Rig::with(Plan::Refuse, StartPolicy::Look, tuning.clone(), |_| {}).await;
        // The source says that it will offer, and does not.
        let waiting = rig.start(false);
        tokio::pin!(waiting);
        let ending = async {
            let _request = rig.next_request().await;
            // The phone has the answer, and waits for the offer, before the source goes away.
            // Before that the call fails as the connection closes, and says nothing of the wait.
            tokio::time::sleep(QUIET).await;
            if stopping {
                rig.phone.node.shutdown().await;
            } else {
                rig.phone.trust.remove(&rig.mac.id).unwrap();
            }
            std::future::pending::<()>().await;
        };
        let outcome = tokio::select! {
            outcome = &mut waiting => outcome,
            () = ending => unreachable!(),
        };
        if stopping {
            assert!(matches!(outcome, Err(Error::Stopped)), "{outcome:?}");
        } else {
            assert!(matches!(outcome, Err(Error::NotPaired(_))), "{outcome:?}");
        }
    }
}
