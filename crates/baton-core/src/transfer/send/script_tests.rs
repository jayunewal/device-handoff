// SPDX-License-Identifier: Apache-2.0

//! The engine against a receiver that a test scripts.
//!
//! Each test says what the receiver answers to every request and what each lane of each connection
//! does, so that every answer that section 8 lets a receiver give, and every fault it tells the
//! sender about, happens when the test says and in the order it says. The tests with the real
//! receiver (`receiver_tests`) show that the engine and the receiver agree. These show what the
//! engine does with each answer, and in the cases that the real receiver never gets to.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use baton_proto::{
    close_code,
    id::TransferId,
    message::{DeclineReason, ErrorCode},
    transfer::{FailReason, Failure},
};

use super::{
    engine::{self, Sending},
    lane::ListFrames,
    outcome::{Error, Outcome, Why},
    policy::Policy,
    progress::Stage,
    selection::Selection,
    testing::{
        Call, Closed, Connect, Event, Folder, Gate, Kind, Log, Script, Step, Take, accepted,
        cancelled, declined, done, ended, error, id, until, within,
    },
    wire::Wire,
};
use crate::transfer::testing::bytes;

/// A folder with files of these sizes, named `f0`, `f1` and so on, and the walk of all of them.
fn sample(sizes: &[usize]) -> (Folder, Arc<Selection>) {
    let folder = Folder::new();
    let names: Vec<String> = (0..sizes.len()).map(|n| format!("f{n}")).collect();
    for (n, (name, size)) in names.iter().zip(sizes).enumerate() {
        folder.make(name, &bytes(u8::try_from(n).unwrap(), *size));
    }
    let chosen: Vec<&str> = names.iter().map(String::as_str).collect();
    let selection = folder.select(&chosen);
    (folder, selection)
}

/// How many bytes lane 0 writes for the list of `selection`.
fn list_len(selection: &Selection) -> u64 {
    ListFrames::new(selection.entries()).unwrap().len() as u64
}

fn start(
    script: &Arc<Script>,
    selection: &Arc<Selection>,
    policy: Policy,
    id: TransferId,
) -> Sending {
    let wire: Arc<dyn Wire> = Arc::clone(script) as Arc<dyn Wire>;
    engine::start(wire, Arc::clone(selection), policy, id).unwrap()
}

/// What a receiver that takes everything answers.
fn normal(id: TransferId, call: &Call) -> Step {
    match call.kind {
        Kind::Offer => accepted(id, 8),
        Kind::Finish => done(id, Vec::new()),
        Kind::Cancel => cancelled(id),
    }
}

/// A receiver that takes everything, over lanes that take everything.
fn friendly(id: TransferId) -> Script {
    Script::new(move |call| normal(id, call))
}

/// The numbers of the lanes that were opened on connection `conn`, in order of number.
fn lanes_on(log: &Log, conn: usize) -> Vec<u8> {
    let mut lanes: Vec<u8> = log
        .opened()
        .into_iter()
        .filter(|(on, _)| *on == conn)
        .map(|(_, lane)| lane)
        .collect();
    lanes.sort_unstable();
    lanes
}

/// Waits until the engine is pausing before its next attempt.
async fn until_waiting(sending: &Sending) {
    let mut progress = sending.progress();
    within(
        "the engine to wait",
        progress.wait_for(|now| now.stage == Stage::Waiting),
    )
    .await
    .unwrap();
}

// The end that a receiver that takes everything makes.

#[tokio::test]
async fn a_receiver_that_takes_everything_ends_the_transfer_with_its_outcome() {
    let (_folder, selection) = sample(&[10, 20, 0]);
    let me = id(1);
    let failure = Failure {
        file: 1,
        reason: FailReason::Mismatch,
    };
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Finish => done(me, vec![failure]),
        _ => normal(me, call),
    }));
    let sending = start(&script, &selection, Policy::quick(), me);
    let progress = sending.progress();

    let outcome = ended(sending).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(sent.done.failed, [failure]);
    assert_eq!((sent.files, sent.bytes), (3, 30));
    assert_eq!(sent.skipped, Vec::new());
    let now = *progress.borrow();
    assert_eq!(
        (now.stage, now.attempt, now.files, now.bytes, now.sent),
        (Stage::Over, 1, 3, 30, 30)
    );
    let events = script.log().events();
    assert_eq!(
        events[..3],
        [
            Event::Connect(1),
            Event::Request(1, Kind::Offer),
            Event::Open(1, 0)
        ]
    );
    assert_eq!(events.last(), Some(&Event::Request(1, Kind::Finish)));
}

// The offer.

#[tokio::test]
async fn an_offer_that_is_not_answered_is_made_again_on_the_same_connection() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(Script::new(move |call| match (call.kind, call.nth) {
        (Kind::Offer, 0) => Step::Silent,
        _ => normal(me, call),
    }));

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let log = script.log();
    assert_eq!(log.connections(), 1);
    assert_eq!(log.requests(Kind::Offer), 2);
    assert_eq!(log.requests(Kind::Finish), 1);
}

#[tokio::test]
async fn an_offer_that_is_never_answered_ends_the_attempt_and_then_the_transfer() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Offer => Step::Silent,
        _ => normal(me, call),
    }));
    let policy = Policy {
        attempts: 2,
        ..Policy::quick()
    };

    let outcome = ended(start(&script, &selection, policy, me)).await;

    assert_eq!(outcome, Outcome::Failed(Error::GaveUp(Why::Silent)));
    let log = script.log();
    // Three tries on each of two connections, and nothing else.
    assert_eq!(log.connections(), 2);
    assert_eq!(log.requests(Kind::Offer), 6);
    assert_eq!(log.opened(), Vec::new());
}

#[tokio::test]
async fn every_reason_to_decline_ends_the_transfer_before_a_lane_is_opened() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    for reason in [
        DeclineReason::Unsupported,
        DeclineReason::TooLarge,
        DeclineReason::RateLimited,
        DeclineReason::Blocked,
        DeclineReason::Invalid,
        DeclineReason::NoSpace,
        DeclineReason::Refused,
        DeclineReason::Other,
    ] {
        let script = Arc::new(Script::new(move |_| declined(me, reason)));

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert_eq!(outcome, Outcome::Declined(reason));
        let log = script.log();
        assert_eq!(log.opened(), Vec::new());
        assert_eq!(log.connections(), 1);
        assert_eq!(log.requests(Kind::Offer), 1);
        assert_eq!(log.requests(Kind::Finish), 0);
    }
}

#[tokio::test]
async fn an_answer_about_another_transfer_is_not_taken() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let other = id(2);
    let wrong_answers_to_the_offer = [
        accepted(other, 4),
        declined(other, DeclineReason::Refused),
        cancelled(me),
        done(me, Vec::new()),
    ];
    for wrong in wrong_answers_to_the_offer {
        let script = Arc::new(Script::new(move |_| wrong.clone()));

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert!(
            matches!(outcome, Outcome::Failed(Error::Protocol(_))),
            "{outcome:?}"
        );
        assert_eq!(script.log().connections(), 1);
    }
    for wrong in [done(other, Vec::new()), accepted(me, 4), cancelled(me)] {
        let script = Arc::new(Script::new(move |call| match call.kind {
            Kind::Finish => wrong.clone(),
            _ => normal(me, call),
        }));

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert!(
            matches!(outcome, Outcome::Failed(Error::Protocol(_))),
            "{outcome:?}"
        );
        assert_eq!(script.log().connections(), 1);
    }
}

#[tokio::test]
async fn a_receiver_that_fails_inside_is_offered_the_transfer_again_on_a_new_connection() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    for code in [ErrorCode::Internal, ErrorCode::Other] {
        let script = Arc::new(Script::new(move |call| match (call.conn, call.kind) {
            (1, Kind::Offer) => error(code),
            _ => normal(me, call),
        }));

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
        let log = script.log();
        assert_eq!(log.connections(), 2);
        assert_eq!(log.requests(Kind::Offer), 2);
        assert!(log.opened().iter().all(|(conn, _)| *conn == 2));
    }
}

#[tokio::test]
async fn an_offer_that_the_receiver_does_not_understand_is_not_made_again() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    for code in [ErrorCode::Malformed, ErrorCode::NotFound] {
        let script = Arc::new(Script::new(move |_| error(code)));

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert!(
            matches!(outcome, Outcome::Failed(Error::Protocol(_))),
            "{outcome:?}"
        );
        assert_eq!(script.log().connections(), 1);
        assert_eq!(script.log().requests(Kind::Offer), 1);
    }
}

#[tokio::test]
async fn a_connection_that_the_receiver_ends_for_good_ends_the_transfer() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);

    // When connecting.
    let script = Arc::new(friendly(me).connects(|_| Connect::Refused(404)));
    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
    assert_eq!(outcome, Outcome::Failed(Error::Refused(404)));
    assert_eq!(script.log().refused_connections(), 1);

    // When offering.
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Offer => Step::Refused(7),
        _ => normal(me, call),
    }));
    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
    assert_eq!(outcome, Outcome::Failed(Error::Refused(7)));
    assert_eq!(script.log().connections(), 1);

    // When finishing.
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Finish => Step::Refused(8),
        _ => normal(me, call),
    }));
    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
    assert_eq!(outcome, Outcome::Failed(Error::Refused(8)));
    assert_eq!(script.log().connections(), 1);
}

#[tokio::test]
async fn a_connection_that_cannot_be_made_is_asked_for_again() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(friendly(me).connects(|n| match n {
        1 => Connect::Lost,
        2 => Connect::TimedOut,
        _ => Connect::Ok,
    }));

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let events = script.log().events();
    assert_eq!(
        events[..3],
        [
            Event::ConnectFailed(1),
            Event::ConnectFailed(2),
            Event::Connect(3)
        ]
    );
}

// The lanes.

#[tokio::test]
async fn the_lanes_are_as_many_as_the_receiver_allows_and_the_policy_wants() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    // (lanes the receiver allows, lanes the policy wants, lanes opened)
    for (allowed, wanted, opened) in [(1, 4, 1), (2, 4, 2), (8, 4, 4), (8, 2, 2), (8, 8, 8)] {
        let script = Arc::new(Script::new(move |call| match call.kind {
            Kind::Offer => accepted(me, allowed),
            _ => normal(me, call),
        }));
        let policy = Policy {
            lanes: wanted,
            ..Policy::quick()
        };

        let outcome = ended(start(&script, &selection, policy, me)).await;

        assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
        let expected: Vec<u8> = (0..opened.min(6)).collect();
        assert_eq!(lanes_on(&script.log(), 1), expected, "{allowed}, {wanted}");
    }
}

#[tokio::test]
async fn a_transfer_of_one_file_or_none_uses_only_the_lane_that_carries_the_list() {
    let me = id(1);
    let (_folder, one) = sample(&[10]);
    let script = Arc::new(friendly(me));
    ended(start(&script, &one, Policy::quick(), me)).await;
    assert_eq!(lanes_on(&script.log(), 1), [0]);

    let folder = Folder::new();
    folder.make_folder("empty");
    let none = folder.select(&["empty"]);
    let script = Arc::new(friendly(me));
    let outcome = ended(start(&script, &none, Policy::quick(), me)).await;
    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert_eq!(lanes_on(&script.log(), 1), [0]);
}

#[tokio::test]
async fn the_other_lanes_are_opened_only_when_the_list_is_written() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    let list = list_len(&selection);
    let gate = Gate::new();
    let held = gate.clone();
    let script = Arc::new(friendly(me).lanes(move |_, lane| {
        if lane == 0 {
            // The whole list but its last byte.
            Take::GateAfter {
                after: list - 1,
                gate: held.clone(),
            }
        } else {
            Take::All
        }
    }));
    let sending = start(&script, &selection, Policy::quick(), me);
    let log = script.log();

    until("lane 0 to write nearly all the list", || {
        script
            .lane(1, 0)
            .is_some_and(|lane| lane.len() as u64 == list - 1)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(log.opened(), [(1, 0)]);

    gate.open();
    let outcome = ended(sending).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert_eq!(lanes_on(&log, 1), [0, 1, 2, 3]);
    // Lane 0 starts with the list, and no other lane has it.
    let first = script.lane(1, 0).unwrap().bytes();
    assert_eq!(first[0], baton_proto::transfer::lane::TAG_LIST);
    for lane in 1..4 {
        let bytes = script.lane(1, lane).unwrap().bytes();
        assert_ne!(bytes.first(), Some(&baton_proto::transfer::lane::TAG_LIST));
    }
}

#[tokio::test]
async fn a_stop_that_ends_the_transfer_for_good_is_not_followed_by_finish() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    for code in [
        close_code::PROTOCOL_VIOLATION,
        close_code::NOT_PAIRED,
        close_code::TOO_LARGE,
        close_code::CANCELLED,
    ] {
        let script = Arc::new(friendly(me).lanes(move |_, lane| {
            if lane == 0 {
                Take::StopAfter { after: 0, code }
            } else {
                Take::All
            }
        }));

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert_eq!(outcome, Outcome::Stopped(code));
        let log = script.log();
        assert_eq!(log.connections(), 1);
        assert_eq!(log.requests(Kind::Finish), 0);
        assert_eq!(log.requests(Kind::Cancel), 0);
        // The lane is let go of, and not as one that is finished.
        assert_eq!(log.closed(), [(1, 0, Closed::Reset(close_code::TIMEOUT))]);
    }
}

#[tokio::test]
async fn a_stop_of_408_ends_the_attempt_and_the_next_one_goes_on() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    let script = Arc::new(friendly(me).lanes(|conn, lane| {
        if conn == 1 && lane == 0 {
            Take::StopAfter {
                after: 0,
                code: close_code::TIMEOUT,
            }
        } else {
            Take::All
        }
    }));

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let log = script.log();
    assert_eq!(log.connections(), 2);
    // The offer is made again on the new connection, unchanged, and everything is sent again.
    assert_eq!(log.requests(Kind::Offer), 2);
    assert_eq!(lanes_on(&log, 2), [0, 1, 2, 3]);
    assert_eq!(log.requests(Kind::Finish), 1);

    // Attempts that are all stopped like this give up.
    let script = Arc::new(friendly(me).lanes(|_, _| Take::StopAfter {
        after: 0,
        code: close_code::TIMEOUT,
    }));
    let policy = Policy {
        attempts: 3,
        ..Policy::quick()
    };
    let outcome = ended(start(&script, &selection, policy, me)).await;
    assert_eq!(outcome, Outcome::Failed(Error::GaveUp(Why::TimedOut)));
    assert_eq!(script.log().connections(), 3);
}

#[tokio::test]
async fn a_lane_that_fails_ends_the_attempt_and_the_lanes_that_are_going_are_reset_with_408() {
    // A chunk holds at most one span, so each lane has a file of its own to hold on to.
    let (_folder, selection) = sample(&[1 << 20; 6]);
    let me = id(1);
    let list = list_len(&selection);
    let script = Arc::new(friendly(me).lanes(move |_, lane| match lane {
        0 => Take::HangAfter { after: list },
        3 => Take::LoseAfter { after: 0 },
        _ => Take::HangAfter { after: 1 },
    }));
    let policy = Policy {
        attempts: 1,
        ..Policy::quick()
    };

    let outcome = ended(start(&script, &selection, policy, me)).await;

    assert_eq!(outcome, Outcome::Failed(Error::GaveUp(Why::Connection)));
    let log = script.log();
    assert_eq!(lanes_on(&log, 1), [0, 1, 2, 3]);
    let mut closed = log.closed();
    closed.sort_by_key(|(conn, lane, _)| (*conn, *lane));
    let reset = Closed::Reset(close_code::TIMEOUT);
    assert_eq!(
        closed,
        [(1, 0, reset), (1, 1, reset), (1, 2, reset), (1, 3, reset)]
    );
    assert_eq!(log.requests(Kind::Finish), 0);
}

#[tokio::test]
async fn a_lane_that_takes_no_byte_or_cannot_be_opened_ends_the_attempt() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    let policy = Policy {
        attempts: 1,
        ..Policy::quick()
    };
    for (take, why) in [
        (Take::HangAfter { after: 0 }, Why::Stalled),
        (Take::Nothing, Why::Connection),
        (Take::LoseAfter { after: 3 }, Why::Connection),
        (Take::OpenFails, Why::Connection),
        (Take::OpenHangs, Why::Stalled),
    ] {
        let started = Instant::now();
        let script = Arc::new(friendly(me).lanes({
            let take = take.clone();
            move |_, lane| if lane == 0 { take.clone() } else { Take::All }
        }));

        let outcome = ended(start(&script, &selection, policy.clone(), me)).await;

        assert_eq!(outcome, Outcome::Failed(Error::GaveUp(why)), "{take:?}");
        if why == Why::Stalled {
            assert!(started.elapsed() >= policy.stall, "{take:?}");
        }
        assert_eq!(script.log().requests(Kind::Finish), 0);
    }
}

#[tokio::test]
async fn an_attempt_that_stalled_is_followed_by_another() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    let script = Arc::new(friendly(me).lanes(|conn, lane| {
        if conn == 1 && lane == 0 {
            Take::HangAfter { after: 0 }
        } else {
            Take::All
        }
    }));

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert_eq!(script.log().connections(), 2);
    assert_eq!(script.log().requests(Kind::Offer), 2);
}

#[tokio::test]
async fn a_stop_when_the_lane_is_ended_counts_as_a_stop() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);

    // 499: the receiver gave the transfer up, and nothing more is sent.
    let script = Arc::new(friendly(me).lanes(|_, _| Take::StopAtEnd {
        code: close_code::CANCELLED,
    }));
    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
    assert_eq!(outcome, Outcome::Stopped(close_code::CANCELLED));
    assert_eq!(script.log().requests(Kind::Finish), 0);

    // 507: the files that were checked are put in place, and `Finish` still says which.
    let script = Arc::new(friendly(me).lanes(|_, _| Take::StopAtEnd {
        code: close_code::NO_SPACE,
    }));
    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert_eq!(script.log().requests(Kind::Finish), 1);
}

// No room.

#[tokio::test]
async fn a_stop_of_507_is_followed_by_finish_and_the_outcome_it_gets_is_the_end() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    let list = list_len(&selection);
    let no_room = move |_: usize, lane: u8| {
        if lane == 0 {
            Take::StopAfter {
                after: list,
                code: close_code::NO_SPACE,
            }
        } else {
            Take::All
        }
    };
    let failure = Failure {
        file: 0,
        reason: FailReason::NoSpace,
    };

    // The receiver names the files that it has no room for.
    let script = Arc::new(
        Script::new(move |call| match call.kind {
            Kind::Finish => done(me, vec![failure]),
            _ => normal(me, call),
        })
        .lanes(no_room),
    );
    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(sent.done.failed, [failure]);
    let log = script.log();
    assert_eq!((log.connections(), log.requests(Kind::Finish)), (1, 1));
    assert_eq!(log.requests(Kind::Offer), 1);

    // The receiver has nothing of the transfer, so there is nothing to wait for, and no use in
    // offering it again.
    for forgot in [
        error(ErrorCode::NotFound),
        declined(me, DeclineReason::Refused),
    ] {
        let script = Arc::new(
            Script::new(move |call| match call.kind {
                Kind::Finish => forgot.clone(),
                _ => normal(me, call),
            })
            .lanes(no_room),
        );
        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
        assert_eq!(outcome, Outcome::Stopped(close_code::NO_SPACE));
        assert_eq!(script.log().connections(), 1);
    }

    // The receiver does not answer.
    let script = Arc::new(
        Script::new(move |call| match call.kind {
            Kind::Finish => Step::Silent,
            _ => normal(me, call),
        })
        .lanes(no_room),
    );
    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;
    assert_eq!(outcome, Outcome::Failed(Error::NoOutcome));
}

// The end.

#[tokio::test]
async fn finish_is_asked_again_on_the_same_connection_when_the_answer_is_slow() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(Script::new(move |call| match (call.kind, call.nth) {
        (Kind::Finish, 0 | 1) => Step::Silent,
        _ => normal(me, call),
    }));

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let log = script.log();
    assert_eq!(log.connections(), 1);
    assert_eq!(log.requests(Kind::Finish), 3);
    assert_eq!(log.requests(Kind::Offer), 1);
}

#[tokio::test]
async fn a_finish_that_is_never_answered_leaves_the_outcome_unknown() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Finish => Step::Silent,
        _ => normal(me, call),
    }));
    let policy = Policy::quick();

    let outcome = ended(start(&script, &selection, policy.clone(), me)).await;

    // The files are in place or they are not, and the sender does not know which.
    assert_eq!(outcome, Outcome::Failed(Error::NoOutcome));
    let log = script.log();
    assert_eq!(log.connections(), 1);
    assert_eq!(
        log.requests(Kind::Finish),
        usize::try_from(policy.finish_tries).unwrap()
    );
}

#[tokio::test]
async fn a_receiver_that_forgot_the_transfer_is_offered_it_again() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    for forgot in [
        error(ErrorCode::NotFound),
        declined(me, DeclineReason::Refused),
    ] {
        let script = Arc::new(Script::new(move |call| match (call.conn, call.kind) {
            (1, Kind::Finish) => forgot.clone(),
            _ => normal(me, call),
        }));

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
        let log = script.log();
        assert_eq!(log.connections(), 2);
        assert_eq!(log.requests(Kind::Offer), 2);
        // What the receiver forgot is not asked about, but offered, and sent again.
        assert_eq!(log.requests(Kind::Finish), 2);
        assert_eq!(lanes_on(&log, 2), [0, 1, 2, 3]);
    }
}

#[tokio::test]
async fn a_finish_that_the_receiver_does_not_understand_ends_the_transfer() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Finish => error(ErrorCode::Malformed),
        _ => normal(me, call),
    }));

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert!(
        matches!(outcome, Outcome::Failed(Error::Protocol(_))),
        "{outcome:?}"
    );
    assert_eq!(script.log().connections(), 1);
}

// A finish whose answer was never heard.

/// A receiver that takes everything, except that the answers to the `Finish` of each connection are
/// what `finish` says, given the number of the connection and of the request.
fn answering_finish(
    id: TransferId,
    finish: impl Fn(usize, usize) -> Step + Send + Sync + 'static,
) -> Arc<Script> {
    Arc::new(Script::new(move |call| match call.kind {
        Kind::Finish => finish(call.conn, call.nth),
        _ => normal(id, call),
    }))
}

#[tokio::test]
async fn the_outcome_of_a_finish_that_was_not_heard_is_asked_for_before_anything_is_offered() {
    let (_folder, selection) = sample(&[10, 20, 30]);
    let me = id(1);
    let script = answering_finish(me, move |conn, _| match conn {
        1 => Step::Lost,
        _ => done(me, Vec::new()),
    });

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    // What the lost `Finish` said is what the outcome is made of.
    assert_eq!((sent.files, sent.bytes), (3, 60));
    let log = script.log();
    assert_eq!(log.connections(), 2);
    assert_eq!(log.requests(Kind::Offer), 1);
    assert_eq!(log.requests(Kind::Finish), 2);
    assert!(log.opened().iter().all(|(conn, _)| *conn == 1));
}

#[tokio::test]
async fn a_receiver_that_does_not_have_the_outcome_is_offered_the_transfer() {
    let (_folder, selection) = sample(&[10; 6]);
    let me = id(1);
    for probe_gets in [
        error(ErrorCode::NotFound),
        Step::Silent,
        declined(me, DeclineReason::Refused),
        error(ErrorCode::Internal),
    ] {
        let script = answering_finish(me, move |conn, nth| match (conn, nth) {
            (1, _) => Step::Lost,
            (2, 0) => probe_gets.clone(),
            _ => done(me, Vec::new()),
        });

        let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

        assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
        let log = script.log();
        assert_eq!(log.connections(), 2);
        // The offer, and everything again, on the second connection.
        assert_eq!(log.requests(Kind::Offer), 2);
        assert_eq!(lanes_on(&log, 2), [0, 1, 2, 3]);
        assert_eq!(log.requests(Kind::Finish), 3);
    }
}

#[tokio::test]
async fn a_probe_that_is_lost_is_made_again_on_the_next_connection() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = answering_finish(me, move |conn, _| match conn {
        1 | 2 => Step::Lost,
        _ => done(me, Vec::new()),
    });

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let log = script.log();
    assert_eq!(log.connections(), 3);
    assert_eq!(log.requests(Kind::Offer), 1);
}

#[tokio::test]
async fn a_probe_that_the_receiver_ends_the_connection_for_ends_the_transfer() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = answering_finish(me, move |conn, _| match conn {
        1 => Step::Lost,
        _ => Step::Refused(9),
    });

    let outcome = ended(start(&script, &selection, Policy::quick(), me)).await;

    assert_eq!(outcome, Outcome::Failed(Error::Refused(9)));
    assert_eq!(script.log().requests(Kind::Offer), 1);
}

// Giving up.

#[tokio::test]
async fn cancelling_before_anything_was_done_needs_no_connection() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(friendly(me));
    let sending = start(&script, &selection, Policy::quick(), me);

    sending.cancel();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Cancelled { told: true });
    assert_eq!(script.log().events(), Vec::new());
}

#[tokio::test]
async fn cancelling_between_attempts_tells_a_receiver_that_may_know_the_transfer() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let slow = Policy {
        backoff_start: Duration::from_secs(60),
        backoff_max: Duration::from_secs(60),
        patience: Duration::from_secs(600),
        ..Policy::quick()
    };

    // Nothing was offered, so there is nobody to tell and no connection is made.
    let script = Arc::new(friendly(me).connects(|_| Connect::Lost));
    let sending = start(&script, &selection, slow.clone(), me);
    until_waiting(&sending).await;
    sending.cancel();
    let outcome = ended(sending).await;
    assert_eq!(outcome, Outcome::Cancelled { told: true });
    let log = script.log();
    assert_eq!((log.refused_connections(), log.connections()), (1, 0));
    assert_eq!(log.requests(Kind::Cancel), 0);

    // An offer was made, so a connection is made for the one request.
    let offers_go_unanswered = move |call: &Call| match (call.conn, call.kind) {
        (1, Kind::Offer) => Step::Silent,
        _ => normal(me, call),
    };
    let script = Arc::new(Script::new(offers_go_unanswered));
    let sending = start(&script, &selection, slow.clone(), me);
    until_waiting(&sending).await;
    sending.cancel();
    let outcome = ended(sending).await;
    assert_eq!(outcome, Outcome::Cancelled { told: true });
    let log = script.log();
    assert_eq!(log.connections(), 2);
    assert_eq!(log.requests(Kind::Cancel), 1);

    // The connection cannot be made, and the receiver is not known to have been told.
    for fails in [Connect::Lost, Connect::TimedOut] {
        let script = Arc::new(
            Script::new(offers_go_unanswered)
                .connects(move |n| if n == 1 { Connect::Ok } else { fails }),
        );
        let sending = start(&script, &selection, slow.clone(), me);
        until_waiting(&sending).await;
        sending.cancel();
        let outcome = ended(sending).await;
        assert_eq!(outcome, Outcome::Cancelled { told: false });
        assert_eq!(script.log().requests(Kind::Cancel), 0);
    }
}

#[tokio::test]
async fn cancelling_while_the_offer_waits_for_its_answer_tells_the_receiver() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let other = id(2);
    // (what `Cancel` gets, whether the sender can say that it told the receiver)
    for (reply, told) in [
        (cancelled(me), true),
        (declined(me, DeclineReason::Unsupported), true),
        (cancelled(other), false),
        (error(ErrorCode::NotFound), false),
        (Step::Silent, false),
        (Step::Lost, false),
    ] {
        let script = Arc::new(Script::new(move |call| match call.kind {
            Kind::Offer => Step::Never,
            _ => reply.clone(),
        }));
        let sending = start(&script, &selection, Policy::quick(), me);
        let log = script.log();
        until("the offer", || log.requests(Kind::Offer) == 1).await;

        sending.cancel();
        let outcome = ended(sending).await;

        assert_eq!(outcome, Outcome::Cancelled { told });
        assert_eq!(log.requests(Kind::Cancel), 1);
        assert_eq!(log.opened(), Vec::new());
    }
}

#[tokio::test]
async fn cancelling_while_finish_waits_for_its_answer_tells_the_receiver() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Finish => Step::Never,
        _ => normal(me, call),
    }));
    let sending = start(&script, &selection, Policy::quick(), me);
    let log = script.log();
    until("the finish", || log.requests(Kind::Finish) == 1).await;

    sending.cancel();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Cancelled { told: true });
    assert_eq!(log.requests(Kind::Cancel), 1);
}

#[tokio::test]
async fn cancelling_while_lanes_are_going_resets_every_lane_with_499_and_tells_the_receiver() {
    let (_folder, selection) = sample(&[1 << 20; 6]);
    let me = id(1);
    let list = list_len(&selection);
    let script = Arc::new(friendly(me).lanes(move |_, lane| {
        if lane == 0 {
            Take::HangAfter { after: list }
        } else {
            Take::HangAfter { after: 1 }
        }
    }));
    let sending = start(&script, &selection, Policy::quick(), me);
    let log = script.log();
    until("all four lanes", || log.opened().len() == 4).await;

    sending.cancel();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Cancelled { told: true });
    let mut closed = log.closed();
    closed.sort_by_key(|(conn, lane, _)| (*conn, *lane));
    let reset = Closed::Reset(close_code::CANCELLED);
    assert_eq!(
        closed,
        [(1, 0, reset), (1, 1, reset), (1, 2, reset), (1, 3, reset)]
    );
    assert_eq!(log.requests(Kind::Cancel), 1);
    assert_eq!(log.requests(Kind::Finish), 0);
    assert_eq!(log.connections(), 1);
}

#[tokio::test]
async fn dropping_the_handle_gives_the_transfer_up() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(Script::new(move |call| match call.kind {
        Kind::Offer => Step::Never,
        _ => normal(me, call),
    }));
    let sending = start(&script, &selection, Policy::quick(), me);
    let log = script.log();
    until("the offer", || log.requests(Kind::Offer) == 1).await;

    drop(sending);

    until("the receiver to be told", || {
        log.requests(Kind::Cancel) == 1
    })
    .await;
}

// How long the engine keeps trying.

#[tokio::test]
async fn the_attempts_are_no_more_than_the_policy_allows() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    for (attempts, tried) in [(0, 1), (1, 1), (3, 3)] {
        let script = Arc::new(friendly(me).connects(|_| Connect::Lost));
        let policy = Policy {
            attempts,
            backoff_start: Duration::from_millis(1),
            patience: Duration::from_secs(600),
            ..Policy::quick()
        };

        let outcome = ended(start(&script, &selection, policy, me)).await;

        assert_eq!(outcome, Outcome::Failed(Error::GaveUp(Why::Connection)));
        assert_eq!(script.log().refused_connections(), tried);
    }
}

#[tokio::test]
async fn attempts_that_get_nowhere_are_given_up_after_the_patience() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(friendly(me).connects(|_| Connect::Lost));
    let policy = Policy {
        attempts: 1000,
        patience: Duration::from_millis(250),
        backoff_start: Duration::from_millis(50),
        backoff_max: Duration::from_millis(50),
        ..Policy::quick()
    };
    let started = Instant::now();

    let outcome = ended(start(&script, &selection, policy, me)).await;

    assert_eq!(outcome, Outcome::Failed(Error::GaveUp(Why::Connection)));
    assert!(started.elapsed() >= Duration::from_millis(250));
    let tried = script.log().refused_connections();
    assert!((3..=8).contains(&tried), "{tried} attempts");
}

#[tokio::test]
async fn the_pause_between_attempts_doubles_up_to_the_most() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = Arc::new(friendly(me).connects(|_| Connect::Lost));
    let policy = Policy {
        attempts: 5,
        patience: Duration::from_secs(600),
        backoff_start: Duration::from_millis(40),
        backoff_max: Duration::from_millis(160),
        ..Policy::quick()
    };

    ended(start(&script, &selection, policy, me)).await;

    let times: Vec<Instant> = script
        .log()
        .times()
        .into_iter()
        .filter(|(_, event)| matches!(event, Event::ConnectFailed(_)))
        .map(|(time, _)| time)
        .collect();
    assert_eq!(times.len(), 5);
    let gaps: Vec<Duration> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    // A pause is never shorter than it was asked to be, and a little slack is left for the clocks.
    let slack = Duration::from_millis(5);
    for (gap, least) in gaps.iter().zip([40, 80, 160, 160]) {
        assert!(*gap + slack >= Duration::from_millis(least), "{gaps:?}");
    }
}

/// Lanes that take everything and are stopped with 408 when they are ended: every attempt gets the
/// files through, and then fails.
fn moving(id: TransferId) -> Arc<Script> {
    Arc::new(friendly(id).lanes(|_, _| Take::StopAtEnd {
        code: close_code::TIMEOUT,
    }))
}

#[tokio::test]
async fn attempts_that_get_files_moving_are_not_given_up_for_the_patience() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = moving(me);
    let policy = Policy {
        attempts: 6,
        patience: Duration::from_millis(120),
        backoff_start: Duration::from_millis(50),
        backoff_max: Duration::from_millis(50),
        ..Policy::quick()
    };

    let outcome = ended(start(&script, &selection, policy, me)).await;

    // Five pauses of 50 ms are more than the patience, and the last attempt is the sixth.
    assert_eq!(outcome, Outcome::Failed(Error::GaveUp(Why::TimedOut)));
    assert_eq!(script.log().connections(), 6);
}

#[tokio::test]
async fn an_attempt_that_got_files_moving_starts_the_pauses_again() {
    let (_folder, selection) = sample(&[10]);
    let me = id(1);
    let script = moving(me);
    let policy = Policy {
        attempts: 7,
        patience: Duration::from_secs(600),
        backoff_start: Duration::from_millis(25),
        backoff_max: Duration::from_secs(5),
        ..Policy::quick()
    };

    ended(start(&script, &selection, policy, me)).await;

    // Pauses that kept doubling would be 25, 50, 100, 200, 400 and 800 ms.
    let connects: Vec<Instant> = script
        .log()
        .times()
        .into_iter()
        .filter(|(_, event)| matches!(event, Event::Connect(_)))
        .map(|(time, _)| time)
        .collect();
    assert_eq!(connects.len(), 7);
    let last = connects[6] - connects[5];
    assert!(last < Duration::from_millis(400), "{last:?}");
}

#[tokio::test]
async fn progress_tells_the_stage_and_the_attempt_and_counts_again_in_each() {
    let (_folder, selection) = sample(&[10, 20]);
    let me = id(1);
    let script = Arc::new(friendly(me).lanes(|conn, _| {
        if conn == 1 {
            Take::StopAtEnd {
                code: close_code::TIMEOUT,
            }
        } else {
            Take::All
        }
    }));
    let sending = start(&script, &selection, Policy::quick(), me);
    let progress = sending.progress();

    let outcome = ended(sending).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let now = *progress.borrow();
    assert_eq!(
        (now.stage, now.attempt, now.files, now.bytes, now.sent),
        (Stage::Over, 2, 2, 30, 30)
    );
}
