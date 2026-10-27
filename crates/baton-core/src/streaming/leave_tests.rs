//! Leaving the screen as it is, on real nodes: what a Mac offers, what the end that the person at
//! the iPad chose does at the Mac, and the ends that never leave it.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use baton_proto::stream::{Offer, StreamReply, StreamRequest};
use tokio::{sync::mpsc, time::timeout};

use super::{
    End, ShareOutcome, SourceHandle, StartHandler, StartOutcome, StartRequest, StartVerdict,
    ViewHandle, Wanted, WindowShare,
    answer_tests::{App, Heard, Plan, Platform, ask, offer},
    offer_tests::{fit, window},
};
use crate::{
    Error, StartPolicy,
    node::NodeBuilder,
    testing::{PATIENCE, TestNode},
    trust::PairedDevice,
    util::BoxFuture,
};

/// A window that is controlled and kept private.
fn private_window() -> WindowShare {
    window().with_control().with_private()
}

/// Waits until `condition` holds.
async fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// How often the platform of the Mac was told of an ordinary end, and of an end that leaves the
/// screen as it is.
fn ends_heard(heard: &Heard) -> (usize, usize) {
    (heard.count("ended"), heard.count("left"))
}

/// A Mac that keeps its screen private, and an iPad that is shown it.
struct Rig {
    mac: TestNode,
    ipad: TestNode,
    app: Arc<App>,
    heard: Heard,
}

impl Rig {
    /// A Mac that can leave its screen as it is if `leaving`.
    async fn new(leaving: bool) -> Self {
        Self::built(|builder| {
            if leaving {
                builder.allow_leave()
            } else {
                builder
            }
        })
        .await
    }

    /// A Mac whose builder `setup` has changed.
    async fn built(setup: impl FnOnce(NodeBuilder) -> NodeBuilder) -> Self {
        let app = App::new(Plan::Accept);
        let ipad = TestNode::with(|builder| builder.window_handler(Arc::clone(&app))).await;
        let mac =
            TestNode::with(|builder| setup(builder.share_windows(true).private_sessions())).await;
        mac.pair_with(&ipad);
        Self {
            mac,
            ipad,
            app,
            heard: Heard::default(),
        }
    }

    /// The Mac shares `window` with the iPad, and the person there takes it. Returns once the
    /// Mac has heard the iPad on its control stream: an end that comes before has no stream to
    /// be said on, and the Mac ends the session by itself when it hears nothing for long enough.
    async fn share(&self, window: WindowShare) -> (SourceHandle, ViewHandle) {
        let shared = self
            .mac
            .node
            .share(&self.ipad.id, window, Platform(self.heard.clone()), fit)
            .await;
        let Ok(ShareOutcome::Started(source)) = shared else {
            panic!("the window was not shared: {shared:?}");
        };
        until("the Mac to hear the iPad", || {
            self.heard.count("report") > 0
        })
        .await;
        (source, self.app.handle(0))
    }

    /// Waits until the Mac has ended the session, and says how.
    async fn ended(&self, source: &SourceHandle) -> End {
        timeout(PATIENCE, source.ended())
            .await
            .expect("the session did not end")
    }
}

// What the Mac offers.

#[tokio::test]
async fn a_private_session_is_offered_with_leave_only_by_a_node_that_allows_it() {
    for (what, leaving, window, expected) in [
        (
            "a private session of a node that allows it",
            true,
            private_window(),
            true,
        ),
        (
            "a private session of a node that does not",
            false,
            private_window(),
            false,
        ),
        (
            "a session that is not private, of a node that allows it",
            true,
            window().with_control(),
            false,
        ),
    ] {
        let rig = Rig::new(leaving).await;
        let (_source, viewer) = rig.share(window).await;
        assert_eq!(viewer.can_leave(), expected, "{what}");
    }
}

#[tokio::test]
async fn leaving_the_screen_as_it_is_needs_the_device_to_keep_its_screen_private() {
    for refused in [
        TestNode::refused(NodeBuilder::allow_leave).await,
        TestNode::refused(|builder| builder.share_windows(true).allow_leave()).await,
    ] {
        assert!(matches!(refused, Error::Config(_)), "{refused:?}");
    }
}

/// Says yes to every start, and gives the request to the test.
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

#[tokio::test]
async fn a_session_that_the_viewer_starts_is_offered_leave_as_one_the_source_chooses_is() {
    let (requests, mut asked) = mpsc::unbounded_channel();
    let rig = Rig::built(|builder| builder.allow_leave().start_handler(Agrees(requests))).await;
    rig.mac
        .trust
        .update(&rig.ipad.id, |device| {
            device.policy.start = StartPolicy::Control;
        })
        .unwrap();

    let started = rig
        .ipad
        .node
        .start(&rig.mac.id, Wanted::Screen, true, rig.app.viewing());
    let offers = async {
        let request = timeout(PATIENCE, asked.recv())
            .await
            .expect("the Mac was not asked to start")
            .expect("the Mac was dropped");
        rig.mac
            .node
            .share_asked(
                request.slot,
                private_window().whole_display(),
                Platform(rig.heard.clone()),
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
    assert!(view.can_leave());
}

#[tokio::test]
async fn the_viewer_takes_leave_only_when_the_offer_says_it_of_a_private_session() {
    // What the offer said, whether the session is private and whether it says leave, and
    // whether the person may leave then.
    for (said, private, leave, may) in [
        ("a private session that may be left", true, true, true),
        ("an offer that does not say leave", true, false, false),
        ("a session that is not private", false, true, false),
    ] {
        let rig = Rig::new(true).await;
        let connection = rig
            .mac
            .node
            .pooled_session(&rig.ipad.id)
            .await
            .unwrap()
            .connection;
        let offer = Offer {
            private,
            leave,
            ..offer(1)
        };
        let reply = ask(&connection, StreamRequest::Offer(offer)).await;
        assert!(
            matches!(reply, StreamReply::Accepted(_)),
            "{said}: {reply:?}"
        );
        assert_eq!(rig.app.handle(0).can_leave(), may, "{said}");
    }
}

// The end that the person chose.

#[tokio::test]
async fn the_person_who_leaves_the_screen_as_it_is_has_the_mac_end_the_session_without_the_lock() {
    let rig = Rig::new(true).await;
    let (source, viewer) = rig.share(private_window()).await;
    assert!(viewer.can_leave());

    assert!(viewer.end_leaving());

    assert_eq!(rig.ended(&source).await, End::Peer);
    // Told once, of the leaving, and not of an ordinary end.
    assert_eq!(ends_heard(&rig.heard), (0, 1));
    until("both ends to forget the session", || {
        rig.mac.node.streaming().is_empty() && rig.ipad.node.streaming().is_empty()
    })
    .await;
}

#[tokio::test]
async fn the_person_who_ends_the_session_any_other_way_has_the_mac_lock() {
    for how in ["ends it", "lets go of it"] {
        let rig = Rig::new(true).await;
        let (source, viewer) = rig.share(private_window()).await;
        match how {
            "ends it" => viewer.end(),
            _ => drop(viewer),
        }
        assert_eq!(rig.ended(&source).await, End::Peer, "{how}");
        assert_eq!(ends_heard(&rig.heard), (1, 0), "{how}");
    }
}

#[tokio::test]
async fn the_mac_that_ends_the_session_itself_locks() {
    let rig = Rig::new(true).await;
    let (source, viewer) = rig.share(private_window()).await;
    source.end();
    assert_eq!(rig.ended(&source).await, End::Local);
    assert_eq!(ends_heard(&rig.heard), (1, 0));
    assert_eq!(timeout(PATIENCE, viewer.ended()).await, Ok(End::Peer));
}

#[tokio::test]
async fn a_person_who_cannot_leave_the_screen_as_it_is_has_the_mac_lock_when_they_try() {
    for (what, leaving, window) in [
        ("a node that does not allow it", false, private_window()),
        (
            "a session that is not private",
            true,
            window().with_control(),
        ),
    ] {
        let rig = Rig::new(leaving).await;
        let (source, viewer) = rig.share(window).await;
        assert!(!viewer.can_leave(), "{what}");
        assert!(!viewer.end_leaving(), "{what}");
        assert_eq!(rig.ended(&source).await, End::Peer, "{what}");
        assert_eq!(ends_heard(&rig.heard), (1, 0), "{what}");
    }
}

#[tokio::test]
async fn the_choice_of_a_person_whose_session_the_mac_ended_first_changes_nothing() {
    let rig = Rig::new(true).await;
    let (source, viewer) = rig.share(private_window()).await;
    source.end();
    assert_eq!(rig.ended(&source).await, End::Local);
    assert_eq!(timeout(PATIENCE, viewer.ended()).await, Ok(End::Peer));
    assert!(!viewer.end_leaving());
    assert_eq!(ends_heard(&rig.heard), (1, 0));
}
