// SPDX-License-Identifier: Apache-2.0

//! A viewer on real nodes: what a node says about itself, how it answers the offer of a window and
//! the end of a session, and a window that goes through from the offer to the pictures.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{
    id::SessionId,
    message::{DeclineReason, Request, Response, capability},
    stream::{Codec, Offer, SourceKind, Stop, StopReason, StreamReply, StreamRequest},
};
use baton_stream::{
    control::{Report, SourceMessage, Thermal, Want},
    header::FrameHeader,
};
use iroh::endpoint::Connection;

use super::*;
use crate::{
    Config,
    node::Tuning,
    session::call,
    streaming::SourceEvents,
    testing::{PATIENCE, QUIET, TestNode},
    util::{BoxFuture, lock},
};

/// How long a test waits for the answer to a request. The wait for a person is cut short by the
/// tests that need it to be.
const WAIT: Duration = Duration::from_secs(10);

/// What the application does with an offer.
#[derive(Debug, Clone, Copy)]
pub(super) enum Plan {
    Accept,
    Refuse,
    Block,
    /// Does not answer, like a person who is away.
    Wait,
}

/// What a decoder of a test was told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Told {
    Decode(u64),
    Show(u64),
    Source(SourceMessage),
    Ended,
}

#[derive(Clone, Default)]
pub(super) struct Log(Arc<Mutex<Vec<Told>>>);

impl Log {
    pub(super) fn all(&self) -> Vec<Told> {
        lock(&self.0).clone()
    }

    pub(super) fn has(&self, told: &Told) -> bool {
        self.all().contains(told)
    }
}

struct Recording(Log);

impl Decoder for Recording {
    fn decode(&mut self, frame: &FrameHeader, _picture: Vec<u8>) -> Result<(), DecodeError> {
        lock(&(self.0).0).push(Told::Decode(frame.number));
        Ok(())
    }

    fn show(&mut self, number: u64) {
        lock(&(self.0).0).push(Told::Show(number));
    }

    fn source(&mut self, message: SourceMessage) {
        lock(&(self.0).0).push(Told::Source(message));
    }

    fn state(&self) -> DecoderState {
        DecoderState {
            decode_us: 1500,
            thermal: Thermal::Nominal,
        }
    }

    fn ended(&mut self) {
        lock(&(self.0).0).push(Told::Ended);
    }
}

/// Counts the questions that were taken back from the person: the future was dropped before it
/// answered.
struct TakenBack<'a>(&'a AtomicUsize);

impl Drop for TakenBack<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// An application that does what a test tells it, and writes down what happens to it.
pub(super) struct App {
    plan: Mutex<Plan>,
    /// What the person was shown, in order.
    asked: Mutex<Vec<WindowOffer>>,
    taken_back: AtomicUsize,
    /// The sessions that were started.
    handles: Mutex<Vec<ViewHandle>>,
    /// What the decoders of those sessions were told.
    pub(super) log: Log,
}

impl App {
    pub(super) fn new(plan: Plan) -> Arc<Self> {
        Arc::new(Self {
            plan: Mutex::new(plan),
            asked: Mutex::default(),
            taken_back: AtomicUsize::new(0),
            handles: Mutex::default(),
            log: Log::default(),
        })
    }

    pub(super) fn plan(&self, plan: Plan) {
        *lock(&self.plan) = plan;
    }

    pub(super) fn asked(&self) -> Vec<WindowOffer> {
        lock(&self.asked).clone()
    }

    pub(super) fn taken_back(&self) -> usize {
        self.taken_back.load(Ordering::SeqCst)
    }

    pub(super) fn handle(&self, index: usize) -> ViewHandle {
        lock(&self.handles).remove(index)
    }

    fn viewing(&self) -> Viewing {
        Viewing::new(
            Recording(self.log.clone()),
            2796,
            1290,
            120,
            vec![Codec::H264],
        )
        .unwrap()
        .within(quick())
    }
}

impl WindowHandler for App {
    fn offered<'a>(
        &'a self,
        _from: &'a PairedDevice,
        offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        lock(&self.asked).push(offer.clone());
        let plan = *lock(&self.plan);
        Box::pin(async move {
            match plan {
                Plan::Accept => WindowVerdict::Accept(self.viewing()),
                Plan::Refuse => WindowVerdict::Refuse,
                Plan::Block => WindowVerdict::Block,
                Plan::Wait => {
                    let _taken_back = TakenBack(&self.taken_back);
                    std::future::pending().await
                }
            }
        })
    }

    fn started(&self, _from: &PairedDevice, handle: ViewHandle) {
        lock(&self.handles).push(handle);
    }
}

/// Times shortened, so that a test waits for them.
pub(super) fn quick() -> ViewParams {
    ViewParams {
        report_every: Duration::from_millis(40),
        tick_every: Duration::from_millis(20),
        ..ViewParams::default()
    }
}

pub(super) fn session(byte: u8) -> SessionId {
    SessionId::from_bytes([byte; SessionId::LEN])
}

pub(super) fn offer(byte: u8) -> Offer {
    Offer {
        session: session(byte),
        source: SourceKind::Window,
        title: "Report.pdf".to_owned(),
        app: "Preview".to_owned(),
        width: 1440,
        height: 900,
        control: false,
    }
}

/// A node that shows windows, with an application that does `plan`, and a paired node that has
/// none and offers them.
struct Rig {
    viewer: TestNode,
    source: TestNode,
    app: Arc<App>,
}

impl Rig {
    async fn new(plan: Plan) -> Self {
        Self::with(plan, Tuning::default(), |_| {}).await
    }

    async fn with(plan: Plan, tuning: Tuning, configure: impl FnOnce(&mut Config)) -> Self {
        let app = App::new(plan);
        let viewer = TestNode::configured(configure, |builder| {
            builder.tuning(tuning).window_handler(Arc::clone(&app))
        })
        .await;
        let source = TestNode::start().await;
        source.pair_with(&viewer);
        Self {
            viewer,
            source,
            app,
        }
    }

    /// The connection that the source keeps to the viewer, made if there is none.
    async fn connection(&self) -> Connection {
        self.source
            .node
            .pooled_session(&self.viewer.id)
            .await
            .unwrap()
            .connection
    }

    async fn ask(&self, request: StreamRequest) -> StreamReply {
        ask(&self.connection().await, request).await
    }

    async fn offer(&self, byte: u8) -> StreamReply {
        self.ask(StreamRequest::Offer(offer(byte))).await
    }

    fn streaming(&self) -> Arc<Streaming> {
        self.viewer.node.streaming()
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

/// Sends a request about a window and gives the answer.
pub(super) async fn ask(connection: &Connection, request: StreamRequest) -> StreamReply {
    match call(connection, &Request::Stream(request), WAIT)
        .await
        .unwrap()
    {
        Response::Stream(reply) => reply,
        other => panic!("the answer was not about a window: {other:?}"),
    }
}

pub(super) fn declined(byte: u8, reason: DeclineReason) -> StreamReply {
    StreamReply::Declined(baton_proto::stream::Declined {
        session: session(byte),
        reason,
    })
}

pub(super) fn stopped(byte: u8) -> StreamReply {
    StreamReply::Stopped(baton_proto::stream::Stopped {
        session: session(byte),
    })
}

pub(super) fn stop(byte: u8) -> StreamRequest {
    StreamRequest::Stop(Stop {
        session: session(byte),
        reason: StopReason::Ended,
    })
}

// What a node says it does.

#[tokio::test]
async fn only_a_node_with_a_handler_says_it_shows_windows() {
    let rig = Rig::new(Plan::Refuse).await;
    let session = rig
        .source
        .node
        .pooled_session(&rig.viewer.id)
        .await
        .unwrap();
    assert!(session.peer.supports(capability::STREAM_VIEW));
    // The source offers a window, but it was given nothing to show one with.
    assert!(!session.peer.supports(capability::STREAM_SHARE));

    let session = rig
        .viewer
        .node
        .pooled_session(&rig.source.id)
        .await
        .unwrap();
    assert!(!session.peer.supports(capability::STREAM_VIEW));
}

#[tokio::test]
async fn a_node_without_a_handler_declines_an_offer_and_a_stop_as_unsupported() {
    let (a, b) = (TestNode::start().await, TestNode::start().await);
    a.pair_with(&b);
    let connection = a.node.pooled_session(&b.id).await.unwrap().connection;

    let reply = ask(&connection, StreamRequest::Offer(offer(1))).await;
    assert_eq!(reply, declined(1, DeclineReason::Unsupported));
    // A device that takes no part has no session to stop, and says so.
    assert_eq!(
        ask(&connection, stop(1)).await,
        declined(1, DeclineReason::Unsupported)
    );
}

#[tokio::test]
async fn a_source_of_a_kind_that_this_version_does_not_know_is_declined_as_unsupported() {
    let rig = Rig::new(Plan::Accept).await;
    let reply = rig
        .ask(StreamRequest::Offer(Offer {
            source: SourceKind::Other,
            ..offer(1)
        }))
        .await;
    assert_eq!(reply, declined(1, DeclineReason::Unsupported));
    assert!(rig.app.asked().is_empty(), "the person was asked");
}

// Offers.

#[tokio::test]
async fn an_accepted_offer_starts_a_session_and_the_source_is_told_what_the_viewer_can_take() {
    let rig = Rig::new(Plan::Accept).await;
    let reply = rig.offer(1).await;
    let StreamReply::Accepted(accepted) = reply else {
        panic!("the offer was not accepted: {reply:?}");
    };
    assert_eq!(accepted.session, session(1));
    assert_eq!(
        (accepted.max_width, accepted.max_height, accepted.max_fps),
        (2796, 1290, 120)
    );
    assert_eq!(accepted.codecs, vec![Codec::H264]);

    // The application has the session, and the node has it too.
    assert_eq!(lock(&rig.app.handles).len(), 1);
    assert!(rig.streaming().viewing(&session(1)).is_some());
}

#[tokio::test]
async fn a_refusal_and_a_block_are_declined_with_their_reasons() {
    let rig = Rig::new(Plan::Refuse).await;
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Refused));
    rig.app.plan(Plan::Block);
    assert_eq!(rig.offer(2).await, declined(2, DeclineReason::Blocked));
    assert_eq!(rig.app.asked().len(), 2);
    assert!(rig.streaming().is_empty());
}

#[tokio::test]
async fn the_person_is_shown_text_that_is_clean_and_not_the_raw_text() {
    let rig = Rig::new(Plan::Refuse).await;
    rig.ask(StreamRequest::Offer(Offer {
        title: "Q3\u{202e}\u{0}\ttotals".to_owned(),
        app: "\u{200b}\u{200b}".to_owned(),
        width: 800,
        height: 600,
        control: true,
        ..offer(1)
    }))
    .await;
    rig.ask(StreamRequest::Offer(Offer {
        title: String::new(),
        app: "Preview".to_owned(),
        ..offer(2)
    }))
    .await;

    let asked = rig.app.asked();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].title.as_deref(), Some("Q3 totals"));
    assert_eq!(asked[0].app, None, "nothing visible is left of the name");
    assert_eq!((asked[0].width, asked[0].height), (800, 600));
    assert!(asked[0].control);
    assert_eq!(asked[1].title, None);
    assert_eq!(asked[1].app.as_deref(), Some("Preview"));
    assert!(!asked[1].control);
    // What a window is called does not reach a log.
    let shown = format!("{:?}", asked[0]);
    assert!(!shown.contains("totals"), "{shown}");
}

#[tokio::test]
async fn an_id_seen_before_on_the_connection_is_invalid_and_the_person_is_not_asked_again() {
    let rig = Rig::new(Plan::Refuse).await;
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Refused));
    // The same id again, whatever came of the first offer under it.
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Invalid));
    rig.app.plan(Plan::Accept);
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Invalid));
    assert_eq!(rig.app.asked().len(), 1);
    // A new id is a new offer.
    assert!(matches!(rig.offer(2).await, StreamReply::Accepted(_)));
}

#[tokio::test]
async fn an_offer_while_another_waits_is_rate_limited_and_a_stop_withdraws_the_first() {
    let rig = Rig::new(Plan::Wait).await;
    let connection = rig.connection().await;
    let first = tokio::spawn({
        let connection = connection.clone();
        async move { ask(&connection, StreamRequest::Offer(offer(1))).await }
    });
    rig.until("the person to be asked", |rig| rig.app.asked().len() == 1)
        .await;

    // One offer waits for a person at a time.
    assert_eq!(
        ask(&connection, StreamRequest::Offer(offer(2))).await,
        declined(2, DeclineReason::RateLimited)
    );
    assert_eq!(rig.app.asked().len(), 1);

    // A stop for another session withdraws nothing.
    assert_eq!(ask(&connection, stop(9)).await, stopped(9));
    tokio::time::sleep(QUIET).await;
    assert!(!first.is_finished());
    assert_eq!(rig.app.taken_back(), 0);

    // A stop for the offer withdraws it: the offer is answered as refused, and the question is
    // taken back from the person.
    assert_eq!(ask(&connection, stop(1)).await, stopped(1));
    let reply = tokio::time::timeout(PATIENCE, first)
        .await
        .expect("the offer was not answered")
        .unwrap();
    assert_eq!(reply, declined(1, DeclineReason::Refused));
    assert_eq!(rig.app.taken_back(), 1);

    // The place is free again.
    rig.app.plan(Plan::Accept);
    assert!(matches!(
        ask(&connection, StreamRequest::Offer(offer(3))).await,
        StreamReply::Accepted(_)
    ));
}

#[tokio::test]
async fn a_window_that_is_shown_keeps_other_offers_out_until_it_ends() {
    let rig = Rig::new(Plan::Accept).await;
    assert!(matches!(rig.offer(1).await, StreamReply::Accepted(_)));
    assert_eq!(rig.offer(2).await, declined(2, DeclineReason::RateLimited));

    let handle = rig.app.handle(0);
    handle.end();
    rig.until("the session to be forgotten", |rig| {
        rig.streaming().is_empty()
    })
    .await;
    assert!(rig.app.log.has(&Told::Ended));
    assert!(matches!(rig.offer(3).await, StreamReply::Accepted(_)));
}

#[tokio::test]
async fn an_offer_that_the_person_does_not_answer_in_time_is_refused() {
    let tuning = Tuning {
        approval_timeout: Duration::from_millis(300),
        ..Tuning::default()
    };
    let rig = Rig::with(Plan::Wait, tuning, |_| {}).await;
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Refused));
    assert_eq!(rig.app.taken_back(), 1);
    // And the place is free.
    rig.app.plan(Plan::Block);
    assert_eq!(rig.offer(2).await, declined(2, DeclineReason::Blocked));
}

#[tokio::test]
async fn a_device_that_sends_too_many_items_is_rate_limited() {
    let rig = Rig::with(Plan::Refuse, Tuning::default(), |config| {
        config.handoff.max_items_per_minute = 2;
    })
    .await;
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Refused));
    assert_eq!(rig.offer(2).await, declined(2, DeclineReason::Refused));
    assert_eq!(rig.offer(3).await, declined(3, DeclineReason::RateLimited));
    assert_eq!(rig.app.asked().len(), 2);
}

// The end of a session.

#[tokio::test]
async fn a_stop_ends_the_session_and_one_for_a_session_that_is_not_there_changes_nothing() {
    let rig = Rig::new(Plan::Accept).await;
    assert!(matches!(rig.offer(1).await, StreamReply::Accepted(_)));
    let handle = rig.app.handle(0);

    assert_eq!(rig.ask(stop(9)).await, stopped(9));
    tokio::time::sleep(QUIET).await;
    assert!(!rig.app.log.has(&Told::Ended));

    assert_eq!(rig.ask(stop(1)).await, stopped(1));
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the session did not end"),
        End::Peer
    );
    assert!(rig.app.log.has(&Told::Ended));
    // Stopping changes nothing the second time.
    assert_eq!(rig.ask(stop(1)).await, stopped(1));
}

#[tokio::test]
async fn unpairing_the_source_ends_its_session_and_withdraws_its_offer() {
    let rig = Rig::new(Plan::Accept).await;
    assert!(matches!(rig.offer(1).await, StreamReply::Accepted(_)));
    let handle = rig.app.handle(0);

    rig.streaming().unpair(&rig.source.id);
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the session did not end"),
        End::Local
    );
    assert!(rig.streaming().is_empty());

    // An offer that waits is withdrawn too. The device is still paired here, so it can ask.
    rig.app.plan(Plan::Wait);
    let connection = rig.connection().await;
    let waiting =
        tokio::spawn(async move { ask(&connection, StreamRequest::Offer(offer(2))).await });
    rig.until("the person to be asked", |rig| rig.app.asked().len() == 2)
        .await;
    rig.streaming().unpair(&rig.source.id);
    let reply = tokio::time::timeout(PATIENCE, waiting)
        .await
        .expect("the offer was not answered")
        .unwrap();
    assert_eq!(reply, declined(2, DeclineReason::Refused));
    assert_eq!(rig.app.taken_back(), 1);
}

#[tokio::test]
async fn a_node_that_stops_ends_its_session() {
    let rig = Rig::new(Plan::Accept).await;
    assert!(matches!(rig.offer(1).await, StreamReply::Accepted(_)));
    let handle = rig.app.handle(0);

    rig.viewer.node.shutdown().await;
    tokio::time::timeout(PATIENCE, handle.ended())
        .await
        .expect("the session did not end");
    assert!(rig.app.log.has(&Told::Ended));
}

#[tokio::test]
async fn a_connection_that_ends_gives_back_what_was_kept_for_it() {
    let rig = Rig::new(Plan::Refuse).await;
    let connection = rig.connection().await;
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Refused));
    assert_eq!(lock(&rig.streaming().table).offers.seen.len(), 1);

    connection.close(0u32.into(), b"done");
    rig.until("the ids of the connection to be forgotten", |rig| {
        lock(&rig.streaming().table).offers.seen.is_empty()
    })
    .await;
}

// A window from the offer to the pictures.

/// What the platform of a test source hears.
#[derive(Clone, Default)]
pub(super) struct Heard(Arc<Mutex<Vec<String>>>);

impl Heard {
    pub(super) fn count(&self, what: &str) -> usize {
        lock(&self.0).iter().filter(|each| *each == what).count()
    }
}

pub(super) struct Platform(pub(super) Heard);

impl SourceEvents for Platform {
    fn report(&mut self, _: Report) {
        lock(&(self.0).0).push("report".to_owned());
    }

    fn want(&mut self, _: Want) {
        lock(&(self.0).0).push("want".to_owned());
    }

    fn input(&mut self, _: baton_stream::control::InputBatch, _: Duration) {
        lock(&(self.0).0).push("input".to_owned());
    }

    fn held(&mut self, _: Vec<baton_stream::sender::LtrToken>) {}

    fn recovery(&mut self) {}

    fn ended(&mut self) {
        lock(&(self.0).0).push("ended".to_owned());
    }
}

#[test]
fn a_viewer_cannot_say_it_takes_what_the_protocol_does_not_allow() {
    let make = |width, height, fps, codecs| {
        Viewing::new(Recording(Log::default()), width, height, fps, codecs)
    };
    assert!(make(3840, 3840, 120, vec![Codec::H264]).is_ok());
    for (what, made) in [
        (
            "a codec that is not known",
            make(1440, 900, 60, vec![Codec::Other]),
        ),
        (
            "one among others",
            make(1440, 900, 60, vec![Codec::H264, Codec::Other]),
        ),
        ("no codec", make(1440, 900, 60, vec![])),
        ("nine codecs", make(1440, 900, 60, vec![Codec::H264; 9])),
        ("no width", make(0, 900, 60, vec![Codec::H264])),
        ("too wide", make(3841, 900, 60, vec![Codec::H264])),
        ("too high", make(1440, 3841, 60, vec![Codec::H264])),
        ("no rate", make(1440, 900, 0, vec![Codec::H264])),
        ("too fast", make(1440, 900, 121, vec![Codec::H264])),
    ] {
        assert!(matches!(made, Err(Error::Config(_))), "{what}");
    }
}

#[tokio::test]
async fn an_offer_is_checked_in_the_order_of_the_protocol() {
    let rig = Rig::new(Plan::Refuse).await;
    let connection = rig.connection().await;
    assert_eq!(rig.offer(1).await, declined(1, DeclineReason::Refused));

    rig.app.plan(Plan::Wait);
    let waiting = tokio::spawn({
        let connection = connection.clone();
        async move { ask(&connection, StreamRequest::Offer(offer(2))).await }
    });
    rig.until("the person to be asked", |rig| rig.app.asked().len() == 2)
        .await;
    // An id that was seen is invalid before another offer that waits is looked at.
    assert_eq!(
        ask(&connection, StreamRequest::Offer(offer(1))).await,
        declined(1, DeclineReason::Invalid)
    );
    // A kind that this version does not know is unsupported before the id is looked at.
    assert_eq!(
        ask(
            &connection,
            StreamRequest::Offer(Offer {
                source: SourceKind::Other,
                ..offer(1)
            })
        )
        .await,
        declined(1, DeclineReason::Unsupported)
    );
    // An offer that was turned away because another waits was seen all the same.
    assert_eq!(
        ask(&connection, StreamRequest::Offer(offer(3))).await,
        declined(3, DeclineReason::RateLimited)
    );
    assert_eq!(ask(&connection, stop(2)).await, stopped(2));
    assert_eq!(waiting.await.unwrap(), declined(2, DeclineReason::Refused));
    rig.app.plan(Plan::Accept);
    assert_eq!(
        ask(&connection, StreamRequest::Offer(offer(3))).await,
        declined(3, DeclineReason::Invalid)
    );
    assert_eq!(rig.app.asked().len(), 2);
}

#[tokio::test]
async fn a_stop_from_another_device_neither_ends_a_session_nor_withdraws_an_offer() {
    let rig = Rig::new(Plan::Accept).await;
    let other = TestNode::start().await;
    other.pair_with(&rig.viewer);
    let theirs = other
        .node
        .pooled_session(&rig.viewer.id)
        .await
        .unwrap()
        .connection;

    assert!(matches!(rig.offer(1).await, StreamReply::Accepted(_)));
    let handle = rig.app.handle(0);
    // The other device names the session of the source. It is answered, and nothing changes.
    assert_eq!(ask(&theirs, stop(1)).await, stopped(1));
    tokio::time::sleep(QUIET).await;
    assert!(!rig.app.log.has(&Told::Ended));
    assert!(rig.streaming().viewing(&session(1)).is_some());
    handle.end();
    rig.until("the session to be forgotten", |rig| {
        rig.streaming().is_empty()
    })
    .await;

    rig.app.plan(Plan::Wait);
    let connection = rig.connection().await;
    let waiting =
        tokio::spawn(async move { ask(&connection, StreamRequest::Offer(offer(2))).await });
    rig.until("the person to be asked", |rig| rig.app.asked().len() == 2)
        .await;
    assert_eq!(ask(&theirs, stop(2)).await, stopped(2));
    tokio::time::sleep(QUIET).await;
    assert!(!waiting.is_finished());
    assert_eq!(rig.app.taken_back(), 0);
    // The source withdraws it itself.
    assert_eq!(rig.ask(stop(2)).await, stopped(2));
    assert_eq!(waiting.await.unwrap(), declined(2, DeclineReason::Refused));
}

#[tokio::test]
async fn unpairing_in_the_trust_list_ends_the_session_as_this_device_ending_it() {
    let rig = Rig::new(Plan::Accept).await;
    assert!(matches!(rig.offer(1).await, StreamReply::Accepted(_)));
    let handle = rig.app.handle(0);
    rig.viewer.trust.remove(&rig.source.id).unwrap();
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the session did not end"),
        End::Local
    );
}

/// An application whose person says yes when the test says so.
struct Nod {
    yes: tokio::sync::Notify,
    asked: AtomicUsize,
    handles: Mutex<Vec<ViewHandle>>,
}

impl WindowHandler for Nod {
    fn offered<'a>(
        &'a self,
        _from: &'a PairedDevice,
        _offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            self.yes.notified().await;
            let viewing = Viewing::new(
                Recording(Log::default()),
                2796,
                1290,
                120,
                vec![Codec::H264],
            );
            WindowVerdict::Accept(viewing.unwrap().within(quick()))
        })
    }

    fn started(&self, _from: &PairedDevice, handle: ViewHandle) {
        lock(&self.handles).push(handle);
    }
}

#[tokio::test]
async fn a_stop_that_lands_with_the_yes_withdraws_the_offer() {
    let app = Arc::new(Nod {
        yes: tokio::sync::Notify::new(),
        asked: AtomicUsize::new(0),
        handles: Mutex::default(),
    });
    let viewer = TestNode::with(|builder| builder.window_handler(Arc::clone(&app))).await;
    let source = TestNode::start().await;
    source.pair_with(&viewer);
    let connection = source
        .node
        .pooled_session(&viewer.id)
        .await
        .unwrap()
        .connection;
    let paired = viewer.trust.get(&source.id).unwrap();
    let streaming = viewer.node.streaming();

    // In each round the yes and the Stop are both there when the offer is next looked at. Which
    // one it sees first is chance, so a round that gets it wrong shows up about every other time.
    for byte in 1..=24 {
        let offered = tokio::spawn({
            let connection = connection.clone();
            async move { ask(&connection, StreamRequest::Offer(offer(byte))).await }
        });
        let deadline = Instant::now() + PATIENCE;
        while app.asked.load(Ordering::SeqCst) < usize::from(byte) {
            assert!(Instant::now() < deadline, "the person was not asked");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        // Nothing else runs between the yes and the Stop: a test's runtime has one thread. The
        // Stop is answered here as the session would answer it.
        app.yes.notify_waiters();
        let link = crate::transfer::incoming::Link::mint();
        let stopping = streaming.answer(&paired, &connection, link, stop(byte));
        assert_eq!(stopping.await, stopped(byte));
        assert_eq!(
            offered.await.unwrap(),
            declined(byte, DeclineReason::Refused),
            "round {byte}"
        );
        assert!(streaming.is_empty());
        assert!(lock(&app.handles).is_empty());
    }
}
