// SPDX-License-Identifier: Apache-2.0

//! Unlocking on real nodes: what a node says about it, the key that a device gives for the unlock
//! to be sealed to, how an offer comes to say that the viewer may unlock, and an unlock that goes
//! through the record and the history that the list of paired devices keeps.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{
    DeviceId, PROTOCOL_MINOR,
    limits::MAX_SEALING_KEY_BYTES,
    message::{DeclineReason, Hello, Ping, Response, capability},
    stream::{Offer, StreamReply, StreamRequest},
    unlock::{BoxKey, KeyRequest, Sealing, SealingKey, UnlockKey, UnlockReply, UnlockRequest},
};
use baton_stream::{
    control::{InputBatch, PauseReason, Report, Want},
    sender::LtrToken,
};
use tokio::sync::mpsc;

use super::{
    KeyOutcome, Question, Settled, ShareOutcome, Sharer, SourceEvents, StartHandler, StartOutcome,
    StartRequest, StartVerdict, Streaming, UnlockApprover, UnlockAttempt, UnlockFinish,
    UnlockOutcome, Unlocker, ViewHandle, Viewer, Wanted, WindowHandler, WindowOffer, WindowShare,
    WindowVerdict,
    answer_tests::{App, Heard, Plan, Platform, ask, offer},
    offer_tests::{fit, window},
    unlock::{Concluded, Finish, UnlockSetup},
};
use crate::{
    Error, StartPolicy, TrustStore,
    clock::SystemClock,
    node::{NodeBuilder, Tuning},
    testing::{PATIENCE, TestNode, info, unlocks},
    trust::PairedDevice,
    unlock_store::Unlocks,
    util::{BoxFuture, lock},
};

const SECOND: Duration = Duration::from_secs(1);

/// Waits until `condition` holds.
async fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// A key of the kind that this version seals to. Its seed tells it from another.
fn key_of(seed: u8) -> UnlockKey {
    UnlockKey {
        sealing: Sealing::EciesP256,
        public: SealingKey::new(unlocks::point(seed)),
    }
}

/// The key of a box. Its byte tells it from another.
fn box_key(byte: u8) -> BoxKey {
    BoxKey::from_bytes([byte; BoxKey::LEN])
}

/// A device that is paired, as the list of paired devices of a node keeps it.
fn paired(byte: u8) -> PairedDevice {
    let dir = tempfile::tempdir().unwrap();
    let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
    let id = DeviceId::from_bytes([byte; DeviceId::LEN]);
    trust.pair(id, &info(), 1).unwrap();
    trust.get(&id).unwrap()
}

/// A whole display, which is what a start asks for.
fn screen() -> WindowShare {
    WindowShare::new("", "", 3024, 1964).whole_display()
}

/// A screen that is controlled and kept private.
fn private_screen() -> WindowShare {
    screen().with_control().with_private()
}

fn asks_for_a_key(renew: bool) -> UnlockRequest {
    UnlockRequest::Key(KeyRequest { renew })
}

/// How the attempts of the history ended, the latest first.
fn finishes(trust: &TrustStore) -> Vec<UnlockFinish> {
    trust
        .unlock_history()
        .into_iter()
        .map(|entry| entry.finish)
        .collect()
}

// The application of a viewer that gives the key.

/// Counts the questions that were taken back from the person: the future was dropped before it
/// answered.
struct TakenBack<'a> {
    count: &'a AtomicUsize,
    answered: bool,
}

impl Drop for TakenBack<'_> {
    fn drop(&mut self) {
        if !self.answered {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// What the application of a viewer that unlocks others with the face of its person does when it
/// is asked for a key.
struct Keys {
    /// The answer, or none if it gives none, like a person who is away.
    answer: Mutex<Option<Result<UnlockKey, DeclineReason>>>,
    /// How long it thinks about it first.
    delay: Mutex<Duration>,
    /// The sources that asked, and whether they wished the key to be renewed.
    asked: Mutex<Vec<(DeviceId, bool)>>,
    taken_back: AtomicUsize,
}

impl Keys {
    fn answering(answer: Option<Result<UnlockKey, DeclineReason>>) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            delay: Mutex::new(Duration::ZERO),
            asked: Mutex::default(),
            taken_back: AtomicUsize::new(0),
        })
    }

    fn giving(key: UnlockKey) -> Arc<Self> {
        Self::answering(Some(Ok(key)))
    }

    fn hanging() -> Arc<Self> {
        Self::answering(None)
    }

    fn answer(&self, answer: Option<Result<UnlockKey, DeclineReason>>) {
        *lock(&self.answer) = answer;
    }

    fn delay(&self, delay: Duration) {
        *lock(&self.delay) = delay;
    }

    fn asked(&self) -> Vec<(DeviceId, bool)> {
        lock(&self.asked).clone()
    }

    fn taken_back(&self) -> usize {
        self.taken_back.load(Ordering::SeqCst)
    }
}

impl UnlockApprover for Keys {
    fn key<'a>(
        &'a self,
        source: &'a PairedDevice,
        renew: bool,
    ) -> BoxFuture<'a, Result<UnlockKey, DeclineReason>> {
        lock(&self.asked).push((source.id, renew));
        let answer = lock(&self.answer).clone();
        let delay = *lock(&self.delay);
        Box::pin(async move {
            let mut question = TakenBack {
                count: &self.taken_back,
                answered: false,
            };
            tokio::time::sleep(delay).await;
            let answer = match answer {
                Some(answer) => answer,
                None => std::future::pending().await,
            };
            question.answered = true;
            answer
        })
    }
}

// The platform of a viewer, and of a source.

/// What the platform of a viewer was told.
#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Open,
    Cancel,
    Settled(Settled),
    Ended,
}

#[derive(Default)]
struct Faces {
    seen: Vec<Seen>,
    /// The questions that the face has not answered, oldest first.
    held: VecDeque<Question>,
}

/// The face of the person at a viewer, and what asks for it.
#[derive(Clone, Default)]
struct Face(Arc<Mutex<Faces>>);

impl Face {
    fn seen(&self) -> Vec<Seen> {
        lock(&self.0).seen.clone()
    }

    /// The oldest question that the face has not answered, once the platform is asked.
    async fn asked(&self) -> Question {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let held = lock(&self.0).held.pop_front();
            if let Some(question) = held {
                return question;
            }
            assert!(Instant::now() < deadline, "the face was asked nothing");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Waits until the platform has been told this many things.
    async fn until_seen(&self, count: usize) {
        until("the platform to be told", || self.seen().len() >= count).await;
    }
}

struct Looking(Face);

impl Looking {
    fn note(&self, seen: Seen) {
        lock(&(self.0).0).seen.push(seen);
    }
}

impl Unlocker for Looking {
    fn open(&mut self, question: Question) {
        self.note(Seen::Open);
        lock(&(self.0).0).held.push_back(question);
    }

    fn cancel(&mut self) {
        self.note(Seen::Cancel);
    }

    fn settled(&mut self, settled: Settled) {
        self.note(Seen::Settled(settled));
    }

    fn ended(&mut self) {
        self.note(Seen::Ended);
    }
}

/// The application of a viewer whose person unlocks with their face: it does what the application
/// of a test does with an offer, and gives the sessions that it shows an unlocker.
struct Faced {
    app: Arc<App>,
    face: Face,
}

impl WindowHandler for Faced {
    fn offered<'a>(
        &'a self,
        from: &'a PairedDevice,
        offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        let verdict = self.app.offered(from, offer);
        Box::pin(async move {
            match verdict.await {
                WindowVerdict::Accept(viewing) => {
                    WindowVerdict::Accept(viewing.with_unlocker(Looking(self.face.clone())))
                }
                other => other,
            }
        })
    }

    fn started(&self, from: &PairedDevice, handle: ViewHandle) {
        self.app.started(from, handle);
    }
}

/// The platform of a source that keeps its screen private, and gives the test the attempts to
/// unlock it, which it would type the password for.
struct Typist {
    attempts: mpsc::UnboundedSender<UnlockAttempt>,
}

impl SourceEvents for Typist {
    fn report(&mut self, _: Report) {}

    fn want(&mut self, _: Want) {}

    fn input(&mut self, _: InputBatch, _: Duration) {}

    fn held(&mut self, _: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn private_in_place(&mut self) -> bool {
        true
    }

    fn unlock(&mut self, attempt: UnlockAttempt) {
        let _ = self.attempts.send(attempt);
    }

    fn ended(&mut self) {}
}

async fn next_attempt(attempts: &mut mpsc::UnboundedReceiver<UnlockAttempt>) -> UnlockAttempt {
    tokio::time::timeout(PATIENCE, attempts.recv())
        .await
        .expect("the platform was given no attempt")
        .expect("the platform was dropped")
}

/// The application of a Mac that says yes to every start, and gives the request to the test.
struct Agrees(mpsc::UnboundedSender<StartRequest>);

impl StartHandler for Agrees {
    fn start<'a>(
        &'a self,
        _from: &'a PairedDevice,
        request: StartRequest,
    ) -> BoxFuture<'a, StartVerdict> {
        let _ = self.0.send(request);
        Box::pin(std::future::ready(StartVerdict::Starting))
    }
}

// A Mac whose person set the unlock up for an iPad, and the iPad.

struct Rig {
    mac: TestNode,
    ipad: TestNode,
    app: Arc<App>,
    face: Face,
    keys: Arc<Keys>,
}

impl Rig {
    /// A Mac that keeps its screen private and can be unlocked, and an iPad that unlocks it.
    async fn new() -> Self {
        Self::built(Tuning::default(), true, NodeBuilder::allow_unlock).await
    }

    /// The same, with the time limits of `tuning` on both, an iPad that unlocks others if
    /// `approving`, and a Mac whose builder `setup` has changed.
    async fn built(
        tuning: Tuning,
        approving: bool,
        setup: impl FnOnce(NodeBuilder) -> NodeBuilder,
    ) -> Self {
        let app = App::new(Plan::Accept);
        let face = Face::default();
        let keys = Keys::giving(key_of(0xd1));
        let ipad = TestNode::with(|builder| {
            let builder = builder.tuning(tuning.clone()).window_handler(Faced {
                app: Arc::clone(&app),
                face: face.clone(),
            });
            if approving {
                builder.approve_unlocks(Arc::clone(&keys))
            } else {
                builder
            }
        })
        .await;
        let mac = TestNode::with(|builder| {
            setup(
                builder
                    .tuning(tuning)
                    .share_windows(true)
                    .private_sessions(),
            )
        })
        .await;
        mac.pair_with(&ipad);
        // The person at the Mac set the unlock up for the iPad.
        mac.trust
            .set_up_unlock(&ipad.id, mac.id, unlocks::new_unlock(1), 1)
            .unwrap();
        Self {
            mac,
            ipad,
            app,
            face,
            keys,
        }
    }

    /// The iPad, as the list of paired devices of the Mac keeps it.
    fn ipad_at_the_mac(&self) -> PairedDevice {
        self.mac.trust.get(&self.ipad.id).unwrap()
    }

    /// Whether the person at the iPad may unlock the screen, when the Mac offers `window`.
    async fn may_unlock_in(&self, window: WindowShare) -> bool {
        let shared = self
            .mac
            .node
            .share(&self.ipad.id, window, Platform(Heard::default()), fit)
            .await;
        let Ok(ShareOutcome::Started(_source)) = shared else {
            panic!("the window was not shared: {shared:?}");
        };
        self.app.handle(0).can_unlock()
    }

    /// Whether the person at the iPad may unlock the screen, when the Mac makes `offer` by hand.
    async fn may_unlock_after(&self, offer: Offer) -> bool {
        let connection = self
            .mac
            .node
            .pooled_session(&self.ipad.id)
            .await
            .unwrap()
            .connection;
        let reply = ask(&connection, StreamRequest::Offer(offer)).await;
        assert!(matches!(reply, StreamReply::Accepted(_)), "{reply:?}");
        self.app.handle(0).can_unlock()
    }
}

// What a node says about it.

/// Whether `node` says to `to`, which connects to it, that it can be unlocked, and that it
/// unlocks others.
async fn says(node: &TestNode, to: &TestNode) -> (bool, bool) {
    let hello = to.node.pooled_session(&node.id).await.unwrap().peer;
    (
        hello.supports(capability::UNLOCK_TARGET),
        hello.supports(capability::UNLOCK_APPROVE),
    )
}

#[tokio::test]
async fn a_node_advertises_being_unlocked_and_unlocking_others_only_when_set_up_to() {
    let probe = TestNode::start().await;
    let plain = TestNode::with(|builder| builder.share_windows(true).private_sessions()).await;
    let target = TestNode::with(|builder| {
        builder
            .share_windows(true)
            .private_sessions()
            .allow_unlock()
    })
    .await;
    let approver = TestNode::with(|builder| {
        builder
            .window_handler(App::new(Plan::Refuse))
            .approve_unlocks(Keys::giving(key_of(1)))
    })
    .await;
    for node in [&plain, &target, &approver] {
        node.pair_with(&probe);
    }

    assert_eq!(says(&plain, &probe).await, (false, false));
    assert_eq!(says(&target, &probe).await, (true, false));
    assert_eq!(says(&approver, &probe).await, (false, true));
    // The probe was set up to do neither, and says neither.
    assert_eq!(says(&probe, &target).await, (false, false));
}

#[tokio::test]
async fn being_unlocked_needs_private_sessions_and_unlocking_others_needs_windows_to_show() {
    for refused in [
        TestNode::refused(NodeBuilder::allow_unlock).await,
        TestNode::refused(|builder| builder.share_windows(true).allow_unlock()).await,
        TestNode::refused(|builder| builder.approve_unlocks(Keys::giving(key_of(1)))).await,
    ] {
        assert!(matches!(refused, Error::Config(_)), "{refused:?}");
    }
}

// What a viewer answers to a request for a key.

/// The sessions of a viewer whose application `keys` gives the keys, that lets `charge` decide
/// whether a device may send one more item, and gives a person `approval` to answer.
fn approving(
    keys: &Arc<Keys>,
    approval: Duration,
    charge: impl Fn(&DeviceId) -> bool + Send + Sync + 'static,
) -> Arc<Streaming> {
    let viewer =
        Viewer::new(App::new(Plan::Refuse), charge, approval).approving(Arc::new(Arc::clone(keys)));
    Streaming::taking_part(Some(viewer), None)
}

#[tokio::test]
async fn a_device_that_unlocks_nobody_declines_a_request_for_a_key_as_unsupported() {
    let source = paired(1);
    // Not a viewer at all, and a viewer that was given nobody to ask.
    let viewer = Viewer::new(App::new(Plan::Refuse), |_| true, SECOND);
    for streaming in [Streaming::new(), Streaming::taking_part(Some(viewer), None)] {
        assert!(!streaming.approves_unlock());
        assert_eq!(
            streaming
                .answer_unlock(&source, asks_for_a_key(false))
                .await,
            UnlockReply::Declined(DeclineReason::Unsupported)
        );
    }
}

#[tokio::test]
async fn the_key_that_the_application_gives_goes_to_the_source_with_the_wish_to_renew_it() {
    let keys = Keys::giving(key_of(0xd1));
    let streaming = approving(&keys, SECOND, |_| true);
    let source = paired(1);
    assert!(streaming.approves_unlock());
    for renew in [false, true, false] {
        assert_eq!(
            streaming
                .answer_unlock(&source, asks_for_a_key(renew))
                .await,
            UnlockReply::Key(key_of(0xd1))
        );
    }
    assert_eq!(
        keys.asked(),
        [(source.id, false), (source.id, true), (source.id, false)]
    );
}

#[tokio::test]
async fn a_key_that_the_application_will_not_give_is_declined_for_the_reason_it_gave() {
    let source = paired(1);
    for (given, said) in [
        (DeclineReason::Refused, DeclineReason::Refused),
        (DeclineReason::Blocked, DeclineReason::Blocked),
        (DeclineReason::Unsupported, DeclineReason::Unsupported),
        (DeclineReason::RateLimited, DeclineReason::RateLimited),
        // A reason that this version does not know is not one to pass on.
        (DeclineReason::Other, DeclineReason::Unsupported),
    ] {
        let keys = Keys::answering(Some(Err(given)));
        let streaming = approving(&keys, SECOND, |_| true);
        assert_eq!(
            streaming
                .answer_unlock(&source, asks_for_a_key(false))
                .await,
            UnlockReply::Declined(said),
            "{given:?}"
        );
    }
}

#[tokio::test]
async fn a_key_that_cannot_be_passed_on_is_declined_as_unsupported() {
    let source = paired(1);
    let point = unlocks::point(0xd1);
    let mut compressed = point[..33].to_vec();
    compressed[0] = 0x02;
    let mut not_uncompressed = point.clone();
    not_uncompressed[0] = 0x03;
    for (what, sealing, public) in [
        ("a way of sealing not known", Sealing::Other, point.clone()),
        ("no key", Sealing::EciesP256, Vec::new()),
        (
            "a point too short",
            Sealing::EciesP256,
            point[..64].to_vec(),
        ),
        ("a compressed point", Sealing::EciesP256, compressed),
        (
            "a point too long",
            Sealing::EciesP256,
            [point.clone(), vec![0]].concat(),
        ),
        (
            "a point not uncompressed",
            Sealing::EciesP256,
            not_uncompressed,
        ),
        (
            "a key beyond the limit",
            Sealing::EciesP256,
            vec![4; MAX_SEALING_KEY_BYTES + 1],
        ),
    ] {
        let key = UnlockKey {
            sealing,
            public: SealingKey::new(public),
        };
        let keys = Keys::giving(key);
        let streaming = approving(&keys, SECOND, |_| true);
        assert_eq!(
            streaming
                .answer_unlock(&source, asks_for_a_key(false))
                .await,
            UnlockReply::Declined(DeclineReason::Unsupported),
            "{what}"
        );
        // The person was asked, and the key was looked at after.
        assert_eq!(keys.asked().len(), 1, "{what}");
    }
}

#[tokio::test]
async fn a_device_that_sent_too_many_items_is_rate_limited_and_the_person_is_not_asked() {
    let keys = Keys::giving(key_of(0xd1));
    let charged = Arc::new(AtomicUsize::new(0));
    let streaming = approving(&keys, SECOND, {
        let charged = Arc::clone(&charged);
        move |_| charged.fetch_add(1, Ordering::SeqCst) < 1
    });
    let source = paired(1);
    assert!(matches!(
        streaming
            .answer_unlock(&source, asks_for_a_key(false))
            .await,
        UnlockReply::Key(_)
    ));
    assert_eq!(
        streaming
            .answer_unlock(&source, asks_for_a_key(false))
            .await,
        UnlockReply::Declined(DeclineReason::RateLimited)
    );
    // Each request counts as an item, and the second did not get as far as the person.
    assert_eq!(charged.load(Ordering::SeqCst), 2);
    assert_eq!(keys.asked().len(), 1);
}

#[tokio::test]
async fn one_question_about_a_key_waits_for_the_person_at_a_time() {
    let keys = Keys::hanging();
    let streaming = approving(&keys, Duration::from_mins(1), |_| true);
    let source = paired(1);
    let first = tokio::spawn({
        let (streaming, source) = (Arc::clone(&streaming), source.clone());
        async move {
            streaming
                .answer_unlock(&source, asks_for_a_key(false))
                .await
        }
    });
    until("the person to be asked", || keys.asked().len() == 1).await;

    // A second question does not wait behind the first. It is turned away.
    assert_eq!(
        streaming.answer_unlock(&source, asks_for_a_key(true)).await,
        UnlockReply::Declined(DeclineReason::RateLimited)
    );
    assert_eq!(keys.asked().len(), 1);

    // The first is dropped, as when the connection of the source ends, and the place is free.
    first.abort();
    until("the question to be taken back", || keys.taken_back() == 1).await;
    keys.answer(Some(Ok(key_of(0xd1))));
    assert!(matches!(
        streaming
            .answer_unlock(&source, asks_for_a_key(false))
            .await,
        UnlockReply::Key(_)
    ));
}

#[tokio::test]
async fn a_person_who_takes_longer_than_the_time_to_answer_is_taken_as_saying_no() {
    let keys = Keys::hanging();
    let streaming = approving(&keys, Duration::from_millis(100), |_| true);
    let source = paired(1);
    assert_eq!(
        streaming
            .answer_unlock(&source, asks_for_a_key(false))
            .await,
        UnlockReply::Declined(DeclineReason::Refused)
    );
    // The question was taken back from the application, and the place is free.
    assert_eq!(keys.taken_back(), 1);
    keys.answer(Some(Ok(key_of(0xd1))));
    assert!(matches!(
        streaming
            .answer_unlock(&source, asks_for_a_key(false))
            .await,
        UnlockReply::Key(_)
    ));
}

#[test]
fn what_a_device_answers_to_a_request_for_a_key_is_a_key_a_refusal_or_an_error() {
    let outcome = |reply| KeyOutcome::of(Response::Unlock(reply));
    assert_eq!(
        outcome(UnlockReply::Key(key_of(0xd1))).unwrap(),
        KeyOutcome::Given(key_of(0xd1))
    );
    for reason in [
        DeclineReason::Refused,
        DeclineReason::Blocked,
        DeclineReason::Other,
    ] {
        assert_eq!(
            outcome(UnlockReply::Declined(reason)).unwrap(),
            KeyOutcome::Declined(reason)
        );
    }
    // A key of a kind that this version does not know cannot be sealed to.
    let unknown = UnlockKey {
        sealing: Sealing::Other,
        public: SealingKey::new(unlocks::point(0xd1)),
    };
    assert!(matches!(
        outcome(UnlockReply::Key(unknown)),
        Err(Error::Unsupported(_))
    ));
    // And a key that is not one of the kind it names breaks the protocol.
    for public in [
        Vec::new(),
        unlocks::point(0xd1)[..64].to_vec(),
        vec![4; 300],
    ] {
        let key = UnlockKey {
            sealing: Sealing::EciesP256,
            public: SealingKey::new(public),
        };
        assert!(matches!(
            outcome(UnlockReply::Key(key)),
            Err(Error::Protocol(_))
        ));
    }
    // So does an answer about something else.
    assert!(matches!(
        KeyOutcome::of(Response::Pong(Ping { nonce: 1 })),
        Err(Error::Protocol(_))
    ));
}

// The key, between two nodes.

#[tokio::test]
async fn the_mac_is_given_the_key_that_the_ipad_made_for_it_and_may_ask_for_a_new_one() {
    let rig = Rig::new().await;
    for renew in [false, true] {
        assert_eq!(
            rig.mac.node.unlock_key(&rig.ipad.id, renew).await.unwrap(),
            KeyOutcome::Given(key_of(0xd1))
        );
    }
    assert_eq!(rig.keys.asked(), [(rig.mac.id, false), (rig.mac.id, true)]);
}

#[tokio::test]
async fn an_ipad_that_will_not_give_a_key_says_why() {
    let rig = Rig::new().await;
    for reason in [
        DeclineReason::Refused,
        DeclineReason::Blocked,
        DeclineReason::Unsupported,
    ] {
        rig.keys.answer(Some(Err(reason)));
        assert_eq!(
            rig.mac.node.unlock_key(&rig.ipad.id, false).await.unwrap(),
            KeyOutcome::Declined(reason)
        );
    }
}

#[tokio::test]
async fn a_key_is_asked_for_by_a_node_that_can_be_unlocked_of_a_device_that_unlocks() {
    // The Mac was not set up to be unlocked.
    let rig = Rig::built(Tuning::default(), true, |builder| builder).await;
    let asked = rig.mac.node.unlock_key(&rig.ipad.id, false).await;
    assert!(matches!(asked, Err(Error::Config(_))), "{asked:?}");

    // The iPad does not unlock others, and the Mac does not ask it.
    let rig = Rig::built(Tuning::default(), false, NodeBuilder::allow_unlock).await;
    let asked = rig.mac.node.unlock_key(&rig.ipad.id, false).await;
    assert!(matches!(asked, Err(Error::Unsupported(_))), "{asked:?}");

    // A device that is not paired.
    let rig = Rig::new().await;
    let stranger = DeviceId::from_bytes([9; DeviceId::LEN]);
    let asked = rig.mac.node.unlock_key(&stranger, false).await;
    assert!(matches!(asked, Err(Error::NotPaired(_))), "{asked:?}");
    assert_eq!(rig.keys.asked(), []);
}

#[tokio::test]
async fn the_mac_waits_for_the_person_at_the_ipad_as_long_as_a_person_may_take() {
    // The time for an answer to an ordinary request is shorter than what the person takes.
    let tuning = Tuning {
        reply_deadline: Duration::from_millis(400),
        approval_timeout: Duration::from_secs(10),
        ..Tuning::default()
    };
    let rig = Rig::built(tuning, true, NodeBuilder::allow_unlock).await;
    rig.keys.delay(Duration::from_millis(1200));
    assert_eq!(
        rig.mac.node.unlock_key(&rig.ipad.id, false).await.unwrap(),
        KeyOutcome::Given(key_of(0xd1))
    );
}

#[tokio::test]
async fn a_person_at_the_ipad_who_does_not_answer_in_time_has_refused() {
    let tuning = Tuning {
        approval_timeout: Duration::from_millis(300),
        ..Tuning::default()
    };
    let rig = Rig::built(tuning, true, NodeBuilder::allow_unlock).await;
    rig.keys.answer(None);
    assert_eq!(
        rig.mac.node.unlock_key(&rig.ipad.id, false).await.unwrap(),
        KeyOutcome::Declined(DeclineReason::Refused)
    );
    assert_eq!(rig.keys.taken_back(), 1);
}

// What an offer says.

/// The sessions of a Mac that can be unlocked, over the list `trust` of its paired devices.
fn unlockable(trust: TrustStore, mac: DeviceId) -> Arc<Streaming> {
    Streaming::taking_part(
        None,
        Some(Sharer {
            control: true,
            private: true,
            starter: None,
            sound: false,
            unlock: Some(Unlocks::new(trust, Arc::new(SystemClock), mac)),
            leave: false,
        }),
    )
}

/// What a viewer says of itself, when it unlocks others or when it does not.
fn hello_of(approving: bool) -> Hello {
    Hello {
        protocol_minor: PROTOCOL_MINOR,
        device: info(),
        capabilities: if approving {
            vec![capability::UNLOCK_APPROVE.to_owned()]
        } else {
            Vec::new()
        },
    }
}

#[tokio::test]
async fn a_session_is_offered_an_unlock_only_when_private_and_set_up_for_a_viewer_that_unlocks() {
    let dir = tempfile::tempdir().unwrap();
    let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
    let mac = DeviceId::from_bytes([0x4d; DeviceId::LEN]);
    let (ipad, other) = (paired(1), paired(2));
    for device in [&ipad, &other] {
        trust.pair(device.id, &info(), 1).unwrap();
    }
    trust
        .set_up_unlock(&ipad.id, mac, unlocks::new_unlock(1), 1)
        .unwrap();
    let streaming = unlockable(trust.clone(), mac);

    let offered = |viewer: &PairedDevice, hello: &Hello, private: bool| {
        streaming.unlock_setup(viewer, hello, private)
    };
    assert!(matches!(
        offered(&ipad, &hello_of(true), true),
        UnlockSetup::Offered(_)
    ));
    // The session is not private.
    assert!(matches!(
        offered(&ipad, &hello_of(true), false),
        UnlockSetup::NotOffered
    ));
    // The viewer did not say that it unlocks others, and is not told of an unlock.
    assert!(matches!(
        offered(&ipad, &hello_of(false), true),
        UnlockSetup::Unsupported
    ));
    // The person set nothing up for this device.
    assert!(matches!(
        offered(&other, &hello_of(true), true),
        UnlockSetup::NotOffered
    ));

    // A node that cannot be unlocked, and a node that shares nothing.
    for streaming in [
        Streaming::taking_part(
            None,
            Some(Sharer {
                control: true,
                private: true,
                starter: None,
                sound: false,
                unlock: None,
                leave: false,
            }),
        ),
        Streaming::new(),
    ] {
        assert!(matches!(
            streaming.unlock_setup(&ipad, &hello_of(true), true),
            UnlockSetup::Unsupported
        ));
    }
}

#[tokio::test]
async fn the_unlock_that_a_session_is_offered_reads_the_record_as_it_is_now() {
    let dir = tempfile::tempdir().unwrap();
    let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
    let mac = DeviceId::from_bytes([0x4d; DeviceId::LEN]);
    let ipad = paired(1);
    trust.pair(ipad.id, &info(), 1).unwrap();
    trust
        .set_up_unlock(&ipad.id, mac, unlocks::new_unlock(1), 1)
        .unwrap();
    let streaming = unlockable(trust.clone(), mac);
    let UnlockSetup::Offered(store) = streaming.unlock_setup(&ipad, &hello_of(true), true) else {
        panic!("no unlock was offered");
    };
    assert_eq!(store.record().unwrap().sealed, unlocks::sealed(1));

    // The person sets it up again while the session goes on: the session sees the new one.
    trust
        .set_up_unlock(&ipad.id, mac, unlocks::new_unlock(2), 2)
        .unwrap();
    assert_eq!(store.record().unwrap().sealed, unlocks::sealed(2));

    // And when a password is rejected the setup is over, for this session and the next.
    store.concluded(Concluded {
        finish: Finish::Done(UnlockOutcome::Rejected),
        sealed: Some(unlocks::sealed(2)),
    });
    assert!(store.record().is_none());
    assert!(matches!(
        streaming.unlock_setup(&ipad, &hello_of(true), true),
        UnlockSetup::NotOffered
    ));
}

#[tokio::test]
async fn an_offer_says_unlock_only_for_a_private_window_that_the_person_set_the_unlock_up_for() {
    let private = || window().with_control().with_private();
    // Everything as it should be.
    assert!(Rig::new().await.may_unlock_in(private()).await);
    // The window is not a private one.
    assert!(
        !Rig::new()
            .await
            .may_unlock_in(window().with_control())
            .await
    );

    // The person turned the unlock off.
    let rig = Rig::new().await;
    rig.mac.trust.turn_off_unlock(&rig.ipad.id).unwrap();
    assert!(!rig.may_unlock_in(private()).await);

    // A password that the system did not accept ended the setup.
    let rig = Rig::new().await;
    rig.mac
        .trust
        .amend(&rig.ipad.id, |device, _| {
            device
                .unlock
                .as_mut()
                .map(|unlock| unlock.end(&unlocks::sealed(1)))
        })
        .unwrap();
    assert!(!rig.may_unlock_in(private()).await);

    // The unlock was set up for another device.
    let rig = Rig::new().await;
    let other = TestNode::start().await;
    rig.mac.know(&other);
    rig.mac.trust.turn_off_unlock(&rig.ipad.id).unwrap();
    rig.mac
        .trust
        .set_up_unlock(&other.id, rig.mac.id, unlocks::new_unlock(2), 1)
        .unwrap();
    assert!(!rig.may_unlock_in(private()).await);

    // The record was made on another device and copied here: the box is bound to that one.
    let rig = Rig::new().await;
    let elsewhere = DeviceId::from_bytes([9; DeviceId::LEN]);
    rig.mac
        .trust
        .set_up_unlock(&rig.ipad.id, elsewhere, unlocks::new_unlock(2), 1)
        .unwrap();
    assert!(!rig.may_unlock_in(private()).await);
}

#[tokio::test]
async fn an_offer_says_unlock_only_to_an_ipad_that_unlocks_and_from_a_mac_that_can_be_unlocked() {
    // The iPad was not set up to unlock others, so it is not told that it may.
    let rig = Rig::built(Tuning::default(), false, NodeBuilder::allow_unlock).await;
    assert!(
        !rig.may_unlock_in(window().with_control().with_private())
            .await
    );

    // The Mac was not set up to be unlocked, though the person left a record in the list.
    let rig = Rig::built(Tuning::default(), true, |builder| builder).await;
    assert!(
        !rig.may_unlock_in(window().with_control().with_private())
            .await
    );
}

#[tokio::test]
async fn the_viewer_takes_an_unlock_only_when_the_offer_the_source_and_the_session_all_say_so() {
    // What was said, whether the source can be unlocked and the viewer unlocks, whether the
    // session is private, whether the offer says unlock, and whether the person may unlock.
    for (said, unlockable, approving, private, unlock, may) in [
        ("all of it", true, true, true, true, true),
        (
            "an offer that does not say unlock",
            true,
            true,
            true,
            false,
            false,
        ),
        (
            "a session that is not private",
            true,
            true,
            false,
            true,
            false,
        ),
        (
            "a source that never said it can be unlocked",
            false,
            true,
            true,
            true,
            false,
        ),
        (
            "a viewer that unlocks nobody",
            true,
            false,
            true,
            true,
            false,
        ),
    ] {
        let rig = Rig::built(Tuning::default(), approving, |builder| {
            if unlockable {
                builder.allow_unlock()
            } else {
                builder
            }
        })
        .await;
        let offer = Offer {
            private,
            unlock,
            ..offer(1)
        };
        assert_eq!(rig.may_unlock_after(offer).await, may, "{said}");
    }
}

#[tokio::test]
async fn a_session_that_the_viewer_starts_is_offered_an_unlock_as_one_the_source_chooses_is() {
    let (requests, mut asked) = mpsc::unbounded_channel();
    let rig = Rig::built(Tuning::default(), true, |builder| {
        builder.allow_unlock().start_handler(Agrees(requests))
    })
    .await;
    rig.mac
        .trust
        .update(&rig.ipad.id, |device| {
            device.policy.start = StartPolicy::Control;
        })
        .unwrap();

    let viewing = rig.app.viewing().with_unlocker(Looking(rig.face.clone()));
    let started = rig
        .ipad
        .node
        .start(&rig.mac.id, Wanted::Screen, true, viewing);
    let offers = async {
        let request = tokio::time::timeout(PATIENCE, asked.recv())
            .await
            .expect("the Mac was not asked to start")
            .expect("the Mac was dropped");
        rig.mac
            .node
            .share_asked(
                request.slot,
                private_screen(),
                Platform(Heard::default()),
                fit,
            )
            .await
    };
    let (started, shared) = tokio::join!(started, offers);
    let Ok(StartOutcome::Started(view)) = started else {
        panic!("the session was not started: {started:?}");
    };
    let Ok(ShareOutcome::Started(_source)) = shared else {
        panic!("the screen was not shared: {shared:?}");
    };
    assert!(view.can_unlock());
}

#[tokio::test]
async fn what_a_viewer_knows_of_the_source_on_a_connection_is_forgotten_when_it_ends() {
    let rig = Rig::new().await;
    let learnt = || {
        lock(&rig.ipad.node.streaming().table)
            .offers
            .unlocking
            .len()
    };
    assert_eq!(learnt(), 0);
    // The Mac connects, and the iPad learns that it can be unlocked.
    rig.mac.node.pooled_session(&rig.ipad.id).await.unwrap();
    until("the iPad to learn it", || learnt() == 1).await;

    rig.mac.node.shutdown().await;
    until("the iPad to forget it", || learnt() == 0).await;
}

// An unlock, from the face to the list of paired devices.

/// A Mac that shows its locked screen, with the platform that types for it, and the session at the
/// iPad.
struct Unlocking {
    rig: Rig,
    source: crate::streaming::SourceHandle,
    view: ViewHandle,
    attempts: mpsc::UnboundedReceiver<UnlockAttempt>,
}

impl Unlocking {
    async fn new() -> Self {
        let rig = Rig::new().await;
        let (typist, attempts) = mpsc::unbounded_channel();
        let shared = rig
            .mac
            .node
            .share(
                &rig.ipad.id,
                window().with_control().with_private(),
                Typist { attempts: typist },
                fit,
            )
            .await;
        let Ok(ShareOutcome::Started(source)) = shared else {
            panic!("the window was not shared: {shared:?}");
        };
        let view = rig.app.handle(0);
        assert!(view.can_unlock());
        // The screen of the Mac is locked.
        source.pause(PauseReason::Locked);
        Self {
            rig,
            source,
            view,
            attempts,
        }
    }

    /// The person asks, the face is asked and agrees, and the platform of the Mac is given the
    /// attempt, which opens the box with the key.
    async fn attempt(&mut self) -> UnlockAttempt {
        assert!(self.view.unlock());
        let question = self.rig.face.asked().await;
        assert_eq!(question.sealed(), &unlocks::sealed(1));
        assert_eq!(question.source(), self.rig.mac.id);
        question.opened(box_key(0x42));
        let attempt = next_attempt(&mut self.attempts).await;
        assert_eq!(attempt.key(), &box_key(0x42));
        attempt
    }
}

#[tokio::test]
async fn an_unlock_that_goes_through_is_kept_in_the_list_of_paired_devices_for_good() {
    let mut unlocking = Unlocking::new().await;
    let mut attempt = unlocking.attempt().await;
    let rig = &unlocking.rig;
    // The attempt is about the box that the person set up, for this iPad.
    assert_eq!(attempt.boxed(), unlocks::boxed(1));
    assert_eq!(
        (attempt.target(), attempt.viewer()),
        (rig.mac.id, rig.ipad.id)
    );

    // The platform keeps the box that it makes for the next unlock, and types the password.
    attempt.keep(unlocks::boxed(2), unlocks::sealed(2)).unwrap();
    attempt.done(UnlockOutcome::Unlocked);
    rig.face.until_seen(2).await;
    assert_eq!(
        rig.face.seen(),
        [
            Seen::Open,
            Seen::Settled(Settled::Said(UnlockOutcome::Unlocked))
        ]
    );

    // The new box and the new sealed key replaced the old, and the attempt is in the history.
    let kept = |trust: &TrustStore, mac, ipad| {
        trust
            .get(&ipad)
            .unwrap()
            .unlock
            .unwrap()
            .record(mac, ipad)
            .unwrap()
    };
    let record = kept(&rig.mac.trust, rig.mac.id, rig.ipad.id);
    assert_eq!(
        (record.boxed, record.sealed),
        (unlocks::boxed(2), unlocks::sealed(2))
    );
    assert_eq!(
        finishes(&rig.mac.trust),
        [UnlockFinish::Done(UnlockOutcome::Unlocked)]
    );
    let history = rig.mac.trust.unlock_history();
    assert_eq!(
        (history[0].device, history[0].name.as_str()),
        (rig.ipad.id, "Device")
    );

    // A program that is closed and opened again finds both.
    let Unlocking { rig, .. } = unlocking;
    let Rig { mac, ipad, .. } = rig;
    let mac = mac
        .restarted(|builder| {
            builder
                .share_windows(true)
                .private_sessions()
                .allow_unlock()
        })
        .await;
    let record = kept(&mac.trust, mac.id, ipad.id);
    assert_eq!(
        (record.boxed, record.sealed),
        (unlocks::boxed(2), unlocks::sealed(2))
    );
    assert_eq!(
        finishes(&mac.trust),
        [UnlockFinish::Done(UnlockOutcome::Unlocked)]
    );
}

#[tokio::test]
async fn a_password_that_the_system_rejects_ends_the_setup_in_the_list_of_paired_devices() {
    let mut unlocking = Unlocking::new().await;
    let mut attempt = unlocking.attempt().await;
    attempt.keep(unlocks::boxed(2), unlocks::sealed(2)).unwrap();
    attempt.done(UnlockOutcome::Rejected);
    let rig = &unlocking.rig;
    rig.face.until_seen(2).await;
    assert_eq!(
        rig.face.seen()[1],
        Seen::Settled(Settled::Said(UnlockOutcome::Rejected))
    );

    // The setup is over, and it is written down before the iPad is told.
    assert!(!rig.ipad_at_the_mac().can_unlock(&rig.mac.id));
    assert_eq!(
        finishes(&rig.mac.trust),
        [UnlockFinish::Done(UnlockOutcome::Rejected)]
    );

    // The next try is turned away, without the face being asked.
    assert!(unlocking.view.unlock());
    rig.face.until_seen(3).await;
    assert_eq!(
        rig.face.seen()[2],
        Seen::Settled(Settled::Said(UnlockOutcome::NotSetUp))
    );

    // And the sessions that come after are not offered an unlock.
    let hello = rig
        .mac
        .node
        .pooled_session(&rig.ipad.id)
        .await
        .unwrap()
        .peer;
    assert!(matches!(
        rig.mac
            .node
            .streaming()
            .unlock_setup(&rig.ipad_at_the_mac(), &hello, true),
        UnlockSetup::NotOffered
    ));
}

#[tokio::test]
async fn a_person_who_declines_the_face_lets_the_next_try_come_and_one_who_lost_the_key_does_not() {
    let unlocking = Unlocking::new().await;
    let rig = &unlocking.rig;
    assert!(unlocking.view.unlock());
    rig.face
        .asked()
        .await
        .declined(super::CancelReason::Declined);
    until("the attempt to be written down", || {
        finishes(&rig.mac.trust) == [UnlockFinish::Cancelled(super::CancelReason::Declined)]
    })
    .await;
    assert!(rig.ipad_at_the_mac().can_unlock(&rig.mac.id));

    // Asked again, the face is asked again, and a key that is lost ends the setup.
    assert!(unlocking.view.unlock());
    rig.face
        .asked()
        .await
        .declined(super::CancelReason::KeyLost);
    until("the setup to end", || {
        !rig.ipad_at_the_mac().can_unlock(&rig.mac.id)
    })
    .await;
    assert_eq!(
        finishes(&rig.mac.trust),
        [
            UnlockFinish::Cancelled(super::CancelReason::KeyLost),
            UnlockFinish::Cancelled(super::CancelReason::Declined),
        ]
    );
    drop(unlocking.source);
}
