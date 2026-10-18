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

use super::{
    End, Picture, ShareOutcome, SourceHandle, Takes, WindowShare,
    answer_tests::{App, Heard, Plan, Platform, Told, ask, declined, offer, stop, stopped},
};
use crate::{
    error::{Error, Result},
    node::Tuning,
    testing::{PATIENCE, QUIET, TestNode},
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
    // The person at the source changes their mind: the call is dropped.
    let given_up = tokio::time::timeout(Duration::from_millis(300), rig.share(window())).await;
    assert!(given_up.is_err(), "the offer was answered");
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
