// SPDX-License-Identifier: Apache-2.0

//! A source on real nodes: what a node says about itself when it was set up to show its windows,
//! how it offers a window and starts the session that a yes begins, and what becomes of an offer
//! that is refused, given up, or made while another is going.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use baton_proto::{
    limits::MAX_STREAM_TITLE_BYTES,
    message::{DeclineReason, capability},
    stream::{Codec, StreamRequest},
};
use baton_stream::control::{Config as Format, SourceMessage};
use tokio_util::sync::CancellationToken;

use super::{
    End, Picture, ShareOutcome, SourceHandle, Takes, WindowShare,
    answer_tests::{App, Heard, Plan, Platform, Told, ask, declined, offer, stop, stopped},
};
use crate::{
    error::{Error, Result},
    node::Tuning,
    testing::{PATIENCE, QUIET, TestNode},
    util::lock,
};

/// What the platform of a test makes of what a viewer takes: a picture of 1440 by 900 pixels at
/// 60 frames a second, as far as the viewer takes it.
fn fit(takes: &Takes) -> Option<Format> {
    takes.codecs.contains(&Codec::H264).then(|| Format {
        epoch: 0,
        codec: Codec::H264,
        width: takes.max_width.min(1440),
        height: takes.max_height.min(900),
        fps: takes.max_fps.min(60),
    })
}

fn window() -> WindowShare {
    WindowShare::new("Report.pdf", "Preview", 1440, 900)
}

/// A node that shows windows, with an application that does `plan`, and a paired node that was
/// set up to offer its own.
struct Rig {
    viewer: TestNode,
    source: TestNode,
    app: Arc<App>,
}

impl Rig {
    async fn new(plan: Plan) -> Self {
        Self::with(plan, Tuning::default(), true).await
    }

    async fn with(plan: Plan, tuning: Tuning, control: bool) -> Self {
        let app = App::new(plan);
        let viewer = TestNode::with(|builder| {
            builder
                .tuning(tuning.clone())
                .window_handler(Arc::clone(&app))
        })
        .await;
        let source = TestNode::with(|builder| builder.tuning(tuning).share_windows(control)).await;
        source.pair_with(&viewer);
        Self {
            viewer,
            source,
            app,
        }
    }

    /// Offers `window` as the platform of a test would, and gives what it heard.
    async fn share(&self, window: WindowShare) -> (Result<ShareOutcome>, Heard) {
        let heard = Heard::default();
        let outcome = self
            .source
            .node
            .share(&self.viewer.id, window, Platform(heard.clone()), fit)
            .await;
        (outcome, heard)
    }

    /// Offers the window and gives the session that a yes starts.
    async fn started(&self) -> (SourceHandle, Heard) {
        let (outcome, heard) = self.share(window()).await;
        match outcome.unwrap() {
            ShareOutcome::Started(handle) => (handle, heard),
            other => panic!("the window was not shared: {other:?}"),
        }
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

/// Waits until the person at the viewer has been asked about `waiting`, which must not finish.
async fn asked_about<F: std::future::Future>(rig: &Rig, waiting: std::pin::Pin<&mut F>) {
    tokio::select! {
        _ = waiting => panic!("the offer was answered"),
        () = rig.until("the person to be asked", |rig| !rig.app.asked().is_empty()) => {}
    }
}

// What a node says it does.

#[tokio::test]
async fn a_node_advertises_sharing_only_when_set_up_to_and_taking_input_only_then() {
    let viewer = TestNode::start().await;
    let plain = TestNode::with(|builder| builder.share_windows(false)).await;
    let controlled = TestNode::with(|builder| builder.share_windows(true)).await;
    plain.pair_with(&viewer);
    controlled.pair_with(&viewer);

    // What a device says of itself reaches the one that connects to it.
    let of_plain = viewer.node.pooled_session(&plain.id).await.unwrap();
    assert!(of_plain.peer.supports(capability::STREAM_SHARE));
    assert!(!of_plain.peer.supports(capability::STREAM_CONTROL));
    assert!(!of_plain.peer.supports(capability::STREAM_VIEW));
    let of_controlled = viewer.node.pooled_session(&controlled.id).await.unwrap();
    assert!(of_controlled.peer.supports(capability::STREAM_SHARE));
    assert!(of_controlled.peer.supports(capability::STREAM_CONTROL));

    // The viewer was given nothing to show windows with, and says none of it.
    let of_viewer = plain.node.pooled_session(&viewer.id).await.unwrap();
    assert!(!of_viewer.peer.supports(capability::STREAM_SHARE));
    assert!(!of_viewer.peer.supports(capability::STREAM_CONTROL));
    assert!(!of_viewer.peer.supports(capability::STREAM_VIEW));
}

#[tokio::test]
async fn a_node_that_only_shares_declines_an_offer_and_answers_a_stop() {
    let rig = Rig::new(Plan::Accept).await;
    // The viewer offers a window to the source, which was not given anything to show one with.
    rig.viewer.pair_with(&rig.source);
    let connection = rig
        .viewer
        .node
        .pooled_session(&rig.source.id)
        .await
        .unwrap()
        .connection;
    assert_eq!(
        ask(&connection, StreamRequest::Offer(offer(1))).await,
        declined(1, DeclineReason::Unsupported)
    );
    // It takes part in window streaming, so it knows there is nothing to stop.
    assert_eq!(ask(&connection, stop(1)).await, stopped(1));
}

// Offering a window.

#[tokio::test]
async fn a_window_goes_through_from_the_offer_to_the_pictures_and_the_reports() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, heard) = rig.started().await;

    // The person at the viewer was shown the window, and nothing was said of control.
    let asked = rig.app.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].title.as_deref(), Some("Report.pdf"));
    assert_eq!(asked[0].app.as_deref(), Some("Preview"));
    assert_eq!((asked[0].width, asked[0].height), (1440, 900));
    assert!(!asked[0].control);

    // The source says what the picture is, and the viewer reports.
    rig.until("the viewer to hear the picture format", |rig| {
        rig.app
            .log
            .all()
            .iter()
            .any(|told| matches!(told, Told::Source(SourceMessage::Config(_))))
    })
    .await;
    let deadline = Instant::now() + PATIENCE;
    while heard.count("report") == 0 {
        assert!(Instant::now() < deadline, "the source heard no report");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // A picture is decoded and shown.
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
    rig.until("the picture to be decoded and shown", |rig| {
        rig.app.log.has(&Told::Decode(0)) && rig.app.log.has(&Told::Show(0))
    })
    .await;

    // The person closes the window on the viewer, and the source hears of it.
    rig.app.handle(0).end();
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the source did not hear that the session ended"),
        End::Peer
    );
    assert!(heard.count("ended") >= 1);
}

#[tokio::test]
async fn a_window_that_grants_control_says_so_to_the_person() {
    let rig = Rig::new(Plan::Accept).await;
    let (outcome, _) = rig.share(window().with_control()).await;
    assert!(matches!(outcome.unwrap(), ShareOutcome::Started(_)));
    assert!(rig.app.asked()[0].control);
}

#[tokio::test]
async fn a_person_who_says_no_or_a_policy_that_blocks_is_not_an_error() {
    let rig = Rig::new(Plan::Refuse).await;
    let (outcome, heard) = rig.share(window()).await;
    assert!(matches!(
        outcome.unwrap(),
        ShareOutcome::Declined(DeclineReason::Refused)
    ));
    // The platform was not told of a session that never began.
    assert_eq!(heard.count("ended"), 0);

    rig.app.plan(Plan::Block);
    let (outcome, _) = rig.share(window()).await;
    assert!(matches!(
        outcome.unwrap(),
        ShareOutcome::Declined(DeclineReason::Blocked)
    ));

    // Neither kept the place, so a window can be offered again, and this time it is accepted.
    rig.app.plan(Plan::Accept);
    let (outcome, _) = rig.share(window()).await;
    assert!(matches!(outcome.unwrap(), ShareOutcome::Started(_)));
}

#[tokio::test]
async fn a_person_who_does_not_answer_in_time_is_a_no() {
    let tuning = Tuning {
        approval_timeout: Duration::from_millis(300),
        ..Tuning::default()
    };
    let rig = Rig::with(Plan::Wait, tuning, true).await;
    let (outcome, _) = rig.share(window()).await;
    assert!(matches!(
        outcome.unwrap(),
        ShareOutcome::Declined(DeclineReason::Refused)
    ));
    // The question was taken back from the person.
    assert_eq!(rig.app.taken_back(), 1);
}

#[tokio::test]
async fn giving_up_while_the_person_decides_takes_the_question_back() {
    let rig = Rig::new(Plan::Wait).await;
    let mut offer = Box::pin(rig.share(window()));
    asked_about(&rig, offer.as_mut()).await;
    // The person at the source changes their mind: the call is dropped.
    drop(offer);
    assert_eq!(rig.app.asked().len(), 1);
    rig.until("the question to be taken back", |rig| {
        rig.app.taken_back() == 1
    })
    .await;

    // The viewer has no offer waiting any more, and neither does the source.
    rig.app.plan(Plan::Accept);
    let (outcome, _) = rig.share(window()).await;
    assert!(matches!(outcome.unwrap(), ShareOutcome::Started(_)));
}

#[tokio::test]
async fn an_offer_made_straight_after_one_was_given_up_is_taken() {
    let rig = Rig::new(Plan::Wait).await;
    let mut first = Box::pin(rig.share(window()));
    asked_about(&rig, first.as_mut()).await;
    // The person at the source picks another window: the first call is dropped and the second
    // made in the same breath.
    drop(first);
    rig.app.plan(Plan::Accept);
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_offer_made_straight_after_one_was_given_up_by_another_thread_is_taken() {
    let rig = Rig::new(Plan::Wait).await;
    let mut first = Box::pin(rig.share(window()));
    asked_about(&rig, first.as_mut()).await;
    // The call is dropped by a thread that is not in the runtime, as it is when an application
    // polls it from one place and gives up from another.
    std::thread::scope(|scope| {
        scope.spawn(move || drop(first)).join().unwrap();
    });
    rig.app.plan(Plan::Accept);
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_offer_made_straight_after_a_session_ended_is_taken() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, _) = rig.started().await;
    // The person at the source switches to another window while one is shown.
    handle.end();
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_offer_made_straight_after_the_handle_was_dropped_is_taken() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, _) = rig.started().await;
    drop(handle);
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_offer_made_straight_after_the_platform_ended_the_session_is_taken() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, _) = rig.started().await;
    // The platform calls the handle from a thread of its own, which is not in the runtime.
    std::thread::spawn(move || handle.end()).join().unwrap();
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn an_offer_made_straight_after_the_platform_dropped_the_handle_is_taken() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, _) = rig.started().await;
    // The platform lets go of the handle from a thread of its own too.
    std::thread::spawn(move || drop(handle)).join().unwrap();
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_session_that_the_viewer_ended_is_not_stopped_again() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, heard) = rig.started().await;
    // The session runs: the viewer has opened its control stream and reported.
    rig.until("the viewer to report", |_| heard.count("report") > 0)
        .await;
    rig.app.handle(0).end();
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the source did not hear that the session ended"),
        End::Peer
    );
    // The viewer let go of its place when it ended the session: no `Stop` goes out, and the next
    // offer has nothing to wait for.
    let kept = lock(&rig.source.node.streaming().table).stops.len();
    assert_eq!(kept, 0, "{kept} stops are kept");
}

#[tokio::test]
async fn a_second_window_is_not_offered_while_one_is_offered_or_shared() {
    let rig = Rig::new(Plan::Wait).await;
    let mut first = Box::pin(rig.share(window()));
    let second = async {
        rig.until("the person to be asked", |rig| !rig.app.asked().is_empty())
            .await;
        rig.share(window()).await.0
    };
    tokio::select! {
        _ = &mut first => panic!("the first offer was answered"),
        outcome = second => assert!(matches!(outcome, Err(Error::Busy)), "{outcome:?}"),
    }
    // Giving the first up lets go of its place, and the viewer takes the question back.
    drop(first);
    rig.until("the question to be taken back", |rig| {
        rig.app.taken_back() == 1
    })
    .await;

    rig.app.plan(Plan::Accept);
    let (handle, _) = rig.started().await;
    let (outcome, _) = rig.share(window()).await;
    assert!(matches!(outcome, Err(Error::Busy)), "{outcome:?}");
    // The viewer shows one window at a time, so it takes the next after the first has gone.
    handle.end();
    rig.until("the viewer's session to end", |rig| {
        rig.app.log.has(&Told::Ended)
    })
    .await;
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn the_connection_is_held_for_as_long_as_the_session_runs() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, _) = rig.started().await;
    let session = rig
        .source
        .node
        .pooled_session(&rig.viewer.id)
        .await
        .unwrap();
    assert_eq!(session.holds(), 1);

    handle.end();
    let deadline = Instant::now() + PATIENCE;
    while session.holds() != 0 {
        assert!(Instant::now() < deadline, "the hold was not let go of");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn the_connection_is_held_while_the_person_decides() {
    let rig = Rig::new(Plan::Wait).await;
    let mut first = Box::pin(rig.share(window()));
    asked_about(&rig, first.as_mut()).await;
    let session = rig
        .source
        .node
        .pooled_session(&rig.viewer.id)
        .await
        .unwrap();
    assert_eq!(session.holds(), 1, "the offer that waits holds nothing");
    drop(first);
    let deadline = Instant::now() + PATIENCE;
    while session.holds() != 0 {
        assert!(Instant::now() < deadline, "the hold was not let go of");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn the_connection_stays_held_while_the_session_runs() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, heard) = rig.started().await;
    // The session has run for a while: the viewer has reported.
    rig.until("the viewer to report", |_| heard.count("report") > 0)
        .await;
    let session = rig
        .source
        .node
        .pooled_session(&rig.viewer.id)
        .await
        .unwrap();
    assert_eq!(session.holds(), 1, "the running session holds nothing");
    drop(handle);
}

#[tokio::test]
async fn unpairing_another_device_leaves_an_offer_that_waits() {
    let rig = Rig::new(Plan::Wait).await;
    let other = TestNode::start().await;
    rig.source.pair_with(&other);
    let mut waiting = Box::pin(rig.share(window()));
    asked_about(&rig, waiting.as_mut()).await;
    rig.source.trust.remove(&other.id).unwrap();
    tokio::select! {
        (outcome, _) = &mut waiting => panic!("the offer ended: {outcome:?}"),
        () = tokio::time::sleep(QUIET) => {}
    }
    assert_eq!(rig.app.taken_back(), 0);
}

#[tokio::test]
async fn a_viewer_that_takes_nothing_is_stopped_at_once() {
    let rig = Rig::new(Plan::Accept).await;
    let outcome = rig
        .source
        .node
        .share(&rig.viewer.id, window(), Platform(Heard::default()), |_| {
            None
        })
        .await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Unmatched)),
        "{outcome:?}"
    );
    // The `Stop` ends the viewer's side, well before it would end on its own for want of the
    // source's control stream.
    let view = rig.app.handle(0);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), view.ended())
            .await
            .expect("the viewer was not stopped"),
        End::Peer
    );
}

#[tokio::test]
async fn a_format_beyond_what_the_viewer_takes_stops_the_viewer_at_once() {
    let rig = Rig::new(Plan::Accept).await;
    let outcome = rig
        .source
        .node
        .share(
            &rig.viewer.id,
            window(),
            Platform(Heard::default()),
            |takes| {
                fit(takes).map(|format| Format {
                    width: takes.max_width + 1,
                    ..format
                })
            },
        )
        .await;
    assert!(matches!(outcome, Err(Error::Config(_))), "{outcome:?}");
    let view = rig.app.handle(0);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), view.ended())
            .await
            .expect("the viewer was not stopped"),
        End::Peer
    );
}

// The `Stop`s that went before an offer.

#[tokio::test]
async fn an_offer_waits_for_a_stop_that_went_to_the_same_device_and_for_no_other() {
    let rig = Rig::new(Plan::Accept).await;
    let streaming = rig.source.node.streaming();
    let (to, other) = (rig.viewer.id, TestNode::start().await.id);
    let answered = CancellationToken::new();
    lock(&streaming.table).stops.push((to, answered.clone()));

    // A `Stop` to one device is nothing to an offer to another.
    tokio::time::timeout(PATIENCE, streaming.settled(&other, Duration::from_secs(60)))
        .await
        .expect("an offer to another device waited");
    // To the same device it is: nothing is settled until the answer is in.
    let waiting = streaming.settled(&to, Duration::from_secs(60));
    tokio::pin!(waiting);
    tokio::select! {
        () = &mut waiting => panic!("the offer did not wait for the Stop"),
        () = tokio::time::sleep(QUIET) => {}
    }
    answered.cancel();
    tokio::time::timeout(PATIENCE, waiting)
        .await
        .expect("the offer went on waiting after the Stop was answered");
}

#[tokio::test]
async fn an_offer_does_not_wait_for_a_stop_longer_than_it_is_given() {
    let rig = Rig::new(Plan::Accept).await;
    let streaming = rig.source.node.streaming();
    lock(&streaming.table)
        .stops
        .push((rig.viewer.id, CancellationToken::new()));
    let began = Instant::now();
    tokio::time::timeout(
        PATIENCE,
        streaming.settled(&rig.viewer.id, Duration::from_millis(200)),
    )
    .await
    .expect("the offer waited for longer than it was given");
    let waited = began.elapsed();
    assert!(waited >= Duration::from_millis(200), "{waited:?}");
}

#[tokio::test]
async fn an_offer_is_made_only_once_the_stop_before_it_is_answered() {
    let rig = Rig::new(Plan::Wait).await;
    let streaming = rig.source.node.streaming();
    // A `Stop` that went to the viewer a moment ago, and is not answered yet.
    let answered = CancellationToken::new();
    lock(&streaming.table)
        .stops
        .push((rig.viewer.id, answered.clone()));
    let mut offer = Box::pin(rig.share(window()));
    tokio::select! {
        (outcome, _) = &mut offer => panic!("the offer ended: {outcome:?}"),
        () = tokio::time::sleep(QUIET) => {}
    }
    assert!(
        rig.app.asked().is_empty(),
        "the offer went out before the Stop was answered"
    );
    answered.cancel();
    asked_about(&rig, offer.as_mut()).await;
}

#[tokio::test]
async fn an_offer_waits_for_every_stop_that_went_before_it() {
    let rig = Rig::new(Plan::Accept).await;
    let streaming = rig.source.node.streaming();
    let (first, second) = (CancellationToken::new(), CancellationToken::new());
    {
        let mut table = lock(&streaming.table);
        table.stops.push((rig.viewer.id, first.clone()));
        table.stops.push((rig.viewer.id, second.clone()));
    }
    let waiting = streaming.settled(&rig.viewer.id, Duration::from_secs(60));
    tokio::pin!(waiting);
    first.cancel();
    tokio::select! {
        () = &mut waiting => panic!("the offer did not wait for the second Stop"),
        () = tokio::time::sleep(QUIET) => {}
    }
    second.cancel();
    tokio::time::timeout(PATIENCE, waiting)
        .await
        .expect("the offer went on waiting after both Stops were answered");
}

#[tokio::test]
async fn a_stop_that_is_answered_is_not_kept() {
    let rig = Rig::new(Plan::Accept).await;
    let streaming = rig.source.node.streaming();
    let (handle, _) = rig.started().await;
    handle.end();
    // The answer to the `Stop` that the end sent settles it, and the next one finds it gone.
    streaming.settled(&rig.viewer.id, PATIENCE).await;
    let (handle, _) = rig.started().await;
    handle.end();
    streaming.settled(&rig.viewer.id, PATIENCE).await;
    let (handle, _) = rig.started().await;
    drop(handle);
    streaming.settled(&rig.viewer.id, PATIENCE).await;
    // Only the last is left, and it is answered: the table is tidied when the next is added.
    let kept = lock(&streaming.table).stops.len();
    assert!(kept <= 1, "{kept} stops are kept");
}

// What is not offered.

#[tokio::test]
async fn a_device_that_shows_no_windows_is_not_offered_one() {
    let plain = TestNode::start().await;
    let source = TestNode::with(|builder| builder.share_windows(false)).await;
    source.pair_with(&plain);
    let outcome = source
        .node
        .share(&plain.id, window(), Platform(Heard::default()), fit)
        .await;
    assert!(
        matches!(&outcome, Err(Error::Unsupported(what)) if what == capability::STREAM_VIEW),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_device_that_is_not_paired_is_not_offered_a_window() {
    let rig = Rig::new(Plan::Accept).await;
    let stranger = TestNode::start().await;
    let outcome = rig
        .source
        .node
        .share(&stranger.id, window(), Platform(Heard::default()), fit)
        .await;
    assert!(matches!(outcome, Err(Error::NotPaired(_))), "{outcome:?}");
}

#[tokio::test]
async fn a_node_that_was_not_set_up_to_share_or_to_take_input_offers_nothing() {
    let rig = Rig::new(Plan::Accept).await;
    // The viewer was not set up to show its windows.
    let outcome = rig
        .viewer
        .node
        .share(&rig.source.id, window(), Platform(Heard::default()), fit)
        .await;
    assert!(matches!(outcome, Err(Error::Config(_))), "{outcome:?}");

    // The source cannot grant control, because it takes no input.
    let rig = Rig::with(Plan::Accept, Tuning::default(), false).await;
    let (outcome, _) = rig.share(window().with_control()).await;
    assert!(matches!(outcome, Err(Error::Config(_))), "{outcome:?}");
    // Nothing reached the person, and the place is free for a window without control.
    assert_eq!(rig.app.asked().len(), 0);
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_window_beyond_a_limit_is_not_offered() {
    let rig = Rig::new(Plan::Accept).await;
    for (what, window) in [
        (
            "a title that is too long",
            WindowShare::new("a".repeat(MAX_STREAM_TITLE_BYTES + 1), "Preview", 1440, 900),
        ),
        (
            "a width of 0",
            WindowShare::new("Report.pdf", "Preview", 0, 900),
        ),
        (
            "a height beyond the most",
            WindowShare::new("Report.pdf", "Preview", 1440, u32::MAX),
        ),
    ] {
        let (outcome, _) = rig.share(window).await;
        assert!(
            matches!(outcome, Err(Error::BadItem(_))),
            "{what}: {outcome:?}"
        );
    }
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.app.asked().len(), 0);
}

// When the viewer takes nothing the platform can make.

#[tokio::test]
async fn a_viewer_that_takes_nothing_the_platform_can_make_is_told_the_session_failed() {
    let rig = Rig::new(Plan::Accept).await;
    let heard = Heard::default();
    let outcome = rig
        .source
        .node
        .share(&rig.viewer.id, window(), Platform(heard.clone()), |_| None)
        .await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Unmatched)),
        "{outcome:?}"
    );
    // The viewer had made its side of the session, and the Stop ended it.
    rig.until("the viewer's session to end", |rig| {
        rig.app.log.has(&Told::Ended)
    })
    .await;
    assert_eq!(heard.count("ended"), 0);
    // The source's place is free again.
    let (outcome, _) = rig.share(window()).await;
    assert!(
        matches!(outcome, Ok(ShareOutcome::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_picture_format_beyond_what_the_viewer_takes_is_not_started() {
    let rig = Rig::new(Plan::Accept).await;
    let outcome = rig
        .source
        .node
        .share(
            &rig.viewer.id,
            window(),
            Platform(Heard::default()),
            |takes| {
                fit(takes).map(|format| Format {
                    width: takes.max_width + 1,
                    ..format
                })
            },
        )
        .await;
    assert!(matches!(outcome, Err(Error::Config(_))), "{outcome:?}");
    rig.until("the viewer's session to end", |rig| {
        rig.app.log.has(&Told::Ended)
    })
    .await;
}

#[tokio::test]
async fn a_new_picture_format_beyond_what_the_viewer_takes_is_refused_and_changes_nothing() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, _) = rig.started().await;
    let beyond = Format {
        epoch: 0,
        codec: Codec::H264,
        width: 2796 + 1,
        height: 900,
        fps: 60,
    };
    assert!(matches!(handle.new_epoch(beyond), Err(Error::Config(_))));
    // A format within what the viewer takes is.
    handle
        .new_epoch(Format {
            width: 1000,
            ..beyond
        })
        .unwrap();
}

// When the other device goes.

#[tokio::test]
async fn unpairing_the_viewer_ends_the_session() {
    let rig = Rig::new(Plan::Accept).await;
    let (handle, heard) = rig.started().await;
    rig.source.trust.remove(&rig.viewer.id).unwrap();
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the session did not end"),
        End::Local
    );
    assert!(heard.count("ended") >= 1);
}

#[tokio::test]
async fn unpairing_the_viewer_gives_up_an_offer_that_waits() {
    let rig = Rig::new(Plan::Wait).await;
    let waiting = rig.share(window());
    tokio::pin!(waiting);
    let unpair = async {
        rig.until("the person to be asked", |rig| !rig.app.asked().is_empty())
            .await;
        rig.source.trust.remove(&rig.viewer.id).unwrap();
        std::future::pending::<()>().await;
    };
    let (outcome, _) = tokio::select! {
        outcome = &mut waiting => outcome,
        () = unpair => unreachable!(),
    };
    assert!(matches!(outcome, Err(Error::NotPaired(_))), "{outcome:?}");
}

#[tokio::test]
async fn stopping_the_node_gives_up_an_offer_that_waits_and_ends_the_session() {
    let rig = Rig::new(Plan::Wait).await;
    let waiting = rig.share(window());
    tokio::pin!(waiting);
    let stopping = async {
        rig.until("the person to be asked", |rig| !rig.app.asked().is_empty())
            .await;
        rig.source.node.shutdown().await;
        std::future::pending::<()>().await;
    };
    let (outcome, _) = tokio::select! {
        outcome = &mut waiting => outcome,
        () = stopping => unreachable!(),
    };
    assert!(matches!(outcome, Err(Error::Stopped)), "{outcome:?}");

    let rig = Rig::new(Plan::Accept).await;
    let (handle, _) = rig.started().await;
    rig.source.node.shutdown().await;
    assert_eq!(
        tokio::time::timeout(PATIENCE, handle.ended())
            .await
            .expect("the session did not end"),
        End::Local
    );
}

#[test]
fn a_window_is_not_written_to_a_log() {
    let shown = format!("{:?}", WindowShare::new("Salary.xlsx", "Numbers", 800, 600));
    assert!(!shown.contains("Salary"), "{shown}");
    assert!(!shown.contains("Numbers"), "{shown}");
}
