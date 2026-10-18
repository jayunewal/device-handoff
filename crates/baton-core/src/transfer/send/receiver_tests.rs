// SPDX-License-Identifier: Apache-2.0

//! The engine against the real table of incoming transfers, in memory.
//!
//! The wire in these tests is the table itself: a request is made to it, and a lane is a pipe that
//! its own lane reader reads. So what the engine sends is checked by the receiver that will meet it,
//! the files that come out are the files that were sent, and the faults of section 8 (a connection
//! that goes, a lane that is stopped, an answer that is lost) happen to a receiver that deals with
//! them as it will for real. The tests with a scripted receiver (`script_tests`) cover the answers
//! that this one never gives.

use std::{
    collections::BTreeMap,
    fs,
    time::{Duration, Instant},
};

use baton_proto::{
    close_code,
    message::DeclineReason,
    transfer::{FailReason, Failure, lane::SkipReason},
};

use super::{
    outcome::Outcome,
    policy::Policy,
    progress::Stage,
    testing::{
        Closed, Fate, Fault, Gate, Kind, Rig, ended, held, sender_device, steady, tree, until,
    },
    wire::WireError,
};
use crate::transfer::{
    incoming::{Ending, Verdict},
    testing::bytes,
};

/// What a transfer leaves in a folder: a folder is `None`, and a file holds its content.
type Tree = BTreeMap<String, Option<Vec<u8>>>;

/// The tree of the entries, a file with its content and a folder without.
fn expect(entries: &[(&str, Option<&[u8]>)]) -> Tree {
    entries
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.map(<[u8]>::to_vec)))
        .collect()
}

/// The lanes that were opened on connection `conn`, in order of number.
fn lanes_on(rig: &Rig, conn: usize) -> Vec<u8> {
    let mut lanes: Vec<u8> = rig
        .wire
        .log()
        .opened()
        .into_iter()
        .filter(|(on, _)| *on == conn)
        .map(|(_, lane)| lane)
        .collect();
    lanes.sort_unstable();
    lanes
}

// What arrives.

#[tokio::test]
async fn a_file_arrives_with_the_content_it_was_sent_with() {
    let rig = Rig::new();
    let content = bytes(1, 300_000);
    rig.files.make("hello.bin", &content);
    let selection = rig.files.select(&["hello.bin"]);
    let id = rig.id();
    let sending = rig.start(&selection, steady(), id);
    assert_eq!(sending.id(), id);
    let progress = sending.progress();

    let outcome = ended(sending).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty() && sent.done.more_failed == 0);
    assert_eq!((sent.files, sent.bytes), (1, 300_000));
    assert_eq!(
        tree(&rig.received),
        expect(&[("hello.bin", Some(&content))])
    );
    assert_eq!(rig.person.questions(), 1);
    let now = *progress.borrow();
    assert_eq!(
        (now.stage, now.attempt, now.files, now.bytes, now.sent),
        (Stage::Over, 1, 1, 300_000, 300_000)
    );
    // The receiver heard of the end once, and says what the sender was told.
    until("the receiver to hear the end", || {
        rig.person.endings().len() == 1
    })
    .await;
    assert_eq!(rig.person.endings()[0].1, Ending::Done(sent.done));
}

#[tokio::test]
async fn folders_empty_files_and_files_of_many_spans_arrive_as_they_were() {
    let rig = Rig::new();
    let one_span = bytes(2, 1 << 20);
    let two_spans = bytes(3, 2 << 20);
    let a_byte_more = bytes(4, (1 << 20) + 1);
    let small = bytes(5, 40);
    rig.files.make("trip/notes.txt", &small);
    rig.files.make("trip/photos/one.jpg", &one_span);
    rig.files.make("trip/photos/two.jpg", &two_spans);
    rig.files.make("trip/photos/three.jpg", &a_byte_more);
    rig.files.make("trip/empty.txt", b"");
    rig.files.make_folder("trip/nothing");
    rig.files.make("top.bin", &small);
    let selection = rig.files.select(&["trip", "top.bin"]);
    let sending = rig.start(&selection, steady(), rig.id());

    let outcome = ended(sending).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    assert_eq!(
        tree(&rig.received),
        expect(&[
            ("trip", None),
            ("trip/empty.txt", Some(b"")),
            ("trip/nothing", None),
            ("trip/notes.txt", Some(&small)),
            ("trip/photos", None),
            ("trip/photos/one.jpg", Some(&one_span)),
            ("trip/photos/three.jpg", Some(&a_byte_more)),
            ("trip/photos/two.jpg", Some(&two_spans)),
            ("top.bin", Some(&small)),
        ])
    );
    // There is enough to carry for four lanes, and they are all used.
    assert_eq!(lanes_on(&rig, 1), [0, 1, 2, 3]);
}

#[tokio::test]
async fn a_list_of_more_than_one_frame_arrives_whole() {
    let rig = Rig::new();
    let mut expected = Vec::new();
    for n in 0..1100_u16 {
        let name = format!("many/f{n:04}");
        let content = bytes(7, usize::from(n % 50));
        rig.files.make(&name, &content);
        expected.push((name, content));
    }
    let selection = rig.files.select(&["many"]);

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    assert_eq!(sent.files, 1100);
    let got = tree(&rig.received);
    assert_eq!(got.len(), 1101);
    for (name, content) in expected {
        assert_eq!(got.get(&name), Some(&Some(content)), "{name}");
    }
}

#[tokio::test]
async fn files_that_changed_or_went_since_the_walk_are_skipped_and_the_rest_arrives() {
    let rig = Rig::new();
    let kept = bytes(1, 700);
    rig.files.make("a", &kept);
    let grew = rig.files.make("b", &bytes(2, 700));
    let gone = rig.files.make("c", &bytes(3, 700));
    let selection = rig.files.select(&["a", "b", "c"]);
    let position = |name: &str| {
        let at = selection
            .entries()
            .iter()
            .position(|entry| entry.path == name)
            .unwrap();
        u32::try_from(at).unwrap()
    };
    fs::write(&grew, bytes(9, 701)).unwrap();
    fs::remove_file(&gone).unwrap();

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    let mut skipped = sent.skipped.clone();
    skipped.sort_by_key(|(file, _)| *file);
    assert_eq!(
        skipped,
        [
            (position("b"), SkipReason::Changed),
            (position("c"), SkipReason::Missing)
        ]
    );
    let mut failed = sent.done.failed.clone();
    failed.sort_by_key(|failure| failure.file);
    assert_eq!(
        failed,
        [
            Failure {
                file: position("b"),
                reason: FailReason::Skipped
            },
            Failure {
                file: position("c"),
                reason: FailReason::Skipped
            }
        ]
    );
    assert_eq!(tree(&rig.received), expect(&[("a", Some(&kept))]));
}

#[tokio::test]
async fn a_person_who_says_no_ends_the_transfer_and_nothing_arrives() {
    let rig = Rig::new();
    rig.files.make("a", &bytes(1, 100));
    let selection = rig.files.select(&["a"]);
    for (verdict, reason) in [
        (Verdict::Refuse, DeclineReason::Refused),
        (Verdict::Block, DeclineReason::Blocked),
    ] {
        rig.person.says(verdict);

        let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

        assert_eq!(outcome, Outcome::Declined(reason));
    }
    let log = rig.wire.log();
    assert_eq!(log.opened(), Vec::new());
    assert_eq!(log.requests(Kind::Finish), 0);
    assert!(tree(&rig.received).is_empty());
}

// Giving up.

#[tokio::test]
async fn cancelling_while_the_person_decides_takes_the_question_back() {
    let rig = Rig::new();
    rig.files.make("a", &bytes(1, 100));
    let selection = rig.files.select(&["a"]);
    let _undecided = rig.person.takes_their_time();
    let sending = rig.start(&selection, steady(), rig.id());
    until("the question", || rig.person.questions() == 1).await;

    sending.cancel();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Cancelled { told: true });
    until("the question to be taken back", || {
        rig.person.withdrawn() == 1
    })
    .await;
    assert_eq!(rig.person.endings(), Vec::new());
    assert!(tree(&rig.received).is_empty());
}

/// A transfer of one big file that waits, with some of it written, until the gate is opened.
async fn parked(rig: &Rig, hold: &Gate) -> super::engine::Sending {
    rig.files.make("big", &bytes(1, 6_000_000));
    let selection = rig.files.select(&["big"]);
    let held_up = hold.clone();
    rig.wire.on_put(move |put| {
        if put.on_conn >= 1_500_000 && !held_up.is_open() {
            Fault::Wait(held_up.clone())
        } else {
            Fault::Pass
        }
    });
    let sending = rig.start(&selection, steady(), rig.id());
    until("some of the file to be written", || {
        rig.wire.bytes() >= 1_500_000
    })
    .await;
    sending
}

/// Waits until the receiver has dropped the transfer, and holds nothing of it.
async fn until_dropped(rig: &Rig) {
    until("the receiver to drop the transfer", || {
        rig.person
            .endings()
            .iter()
            .any(|(_, ending)| *ending == Ending::Dropped)
    })
    .await;
    until("the staging to be gone", || held(&rig.received) == 0).await;
    assert!(tree(&rig.received).is_empty());
}

#[tokio::test]
async fn cancelling_in_the_middle_resets_the_lanes_and_the_receiver_keeps_nothing() {
    let rig = Rig::new();
    let hold = Gate::new();
    let sending = parked(&rig, &hold).await;

    sending.cancel();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Cancelled { told: true });
    let closed = rig.wire.log().closed();
    assert_ne!(closed, Vec::new());
    assert!(
        closed
            .iter()
            .all(|(_, _, how)| *how == Closed::Reset(close_code::CANCELLED)),
        "{closed:?}"
    );
    assert_eq!(rig.wire.log().requests(Kind::Finish), 0);
    until_dropped(&rig).await;
}

#[tokio::test]
async fn dropping_the_handle_in_the_middle_gives_the_transfer_up() {
    let rig = Rig::new();
    let hold = Gate::new();
    let sending = parked(&rig, &hold).await;

    drop(sending);

    until_dropped(&rig).await;
    assert_eq!(rig.wire.log().requests(Kind::Cancel), 1);
}

#[tokio::test]
async fn a_receiver_that_gives_the_transfer_up_stops_the_sender_for_good() {
    let rig = Rig::new();
    let hold = Gate::new();
    let id = rig.id();
    rig.files.make("big", &bytes(1, 6_000_000));
    let selection = rig.files.select(&["big"]);
    let held_up = hold.clone();
    rig.wire.on_put(move |put| {
        if put.on_conn >= 1_000_000 && !held_up.is_open() {
            Fault::Wait(held_up.clone())
        } else {
            Fault::Pass
        }
    });
    let sending = rig.start(&selection, steady(), id);
    until("some of the file to be written", || {
        rig.wire.bytes() >= 1_000_000
    })
    .await;

    // The person gives the transfer up on the receiving device.
    rig.incoming.cancel(&sender_device().id, id).await;
    hold.open();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Stopped(close_code::CANCELLED));
    let log = rig.wire.log();
    assert_eq!(log.requests(Kind::Finish), 0);
    assert_eq!(log.requests(Kind::Cancel), 0);
    assert_eq!(log.connections(), 1);
    until_dropped(&rig).await;
}

// Faults.

#[tokio::test]
async fn a_connection_that_goes_in_the_middle_is_followed_by_an_attempt_that_finishes_the_transfer()
{
    let rig = Rig::new();
    let content = bytes(1, 5_000_000);
    rig.files.make("big", &content);
    rig.files.make("small", b"small");
    let selection = rig.files.select(&["big", "small"]);
    rig.wire.on_put(|put| {
        if put.conn == 1 && put.on_conn >= 2_000_000 {
            Fault::Cut
        } else {
            Fault::Pass
        }
    });

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    let log = rig.wire.log();
    assert_eq!(log.connections(), 2);
    // The offer is made again, unchanged, and the person is not asked again.
    assert_eq!(log.requests(Kind::Offer), 2);
    assert_eq!(rig.person.questions(), 1);
    assert_eq!(
        tree(&rig.received),
        expect(&[("big", Some(&content)), ("small", Some(b"small"))])
    );
    until("the receiver to hear the end", || {
        rig.person.endings().len() == 1
    })
    .await;
}

#[tokio::test]
async fn an_offer_that_is_lost_with_the_connection_is_made_again_on_a_new_one() {
    let rig = Rig::new();
    let content = bytes(1, 1000);
    rig.files.make("a", &content);
    let selection = rig.files.select(&["a"]);
    rig.wire.on_call(|call| {
        if call.conn == 1 && call.kind == Kind::Offer {
            Fate::Lose
        } else {
            Fate::Pass
        }
    });

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert_eq!(rig.wire.log().connections(), 2);
    // The first offer never got to the receiver, so the person is asked once.
    assert_eq!(rig.person.questions(), 1);
    assert_eq!(tree(&rig.received), expect(&[("a", Some(&content))]));
}

#[tokio::test]
async fn a_list_that_stops_being_written_ends_the_attempt_before_any_other_lane_is_opened() {
    let rig = Rig::new();
    for n in 0..1100_u16 {
        rig.files.make(&format!("many/f{n:04}"), &bytes(7, 10));
    }
    let selection = rig.files.select(&["many"]);
    // The list is two frames, and lane 0 takes the first and then nothing.
    rig.wire.on_put(|put| {
        if put.conn == 1 && put.lane == 0 && put.on_lane >= 1 {
            Fault::Stall
        } else {
            Fault::Pass
        }
    });
    let policy = Policy {
        stall: Duration::from_millis(500),
        ..steady()
    };

    let outcome = ended(rig.start(&selection, policy, rig.id())).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    // The first connection never got past the list, and the second did everything.
    assert_eq!(lanes_on(&rig, 1), [0]);
    assert_eq!(lanes_on(&rig, 2), [0, 1, 2, 3]);
    assert_eq!(rig.person.questions(), 1);
    assert_eq!(tree(&rig.received).len(), 1101);
}

#[tokio::test]
async fn a_connection_that_cannot_be_made_is_tried_again_and_the_receiver_never_hears_of_it() {
    let rig = Rig::new();
    let content = bytes(1, 1000);
    rig.files.make("a", &content);
    let selection = rig.files.select(&["a"]);
    rig.wire
        .on_connect(|n| (n == 1).then_some(WireError::TimedOut));

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let log = rig.wire.log();
    assert_eq!((log.refused_connections(), log.connections()), (1, 1));
    assert_eq!(rig.person.questions(), 1);
    assert_eq!(tree(&rig.received), expect(&[("a", Some(&content))]));
}

#[tokio::test]
async fn lanes_that_take_nothing_end_the_attempt_and_the_next_one_finishes_the_transfer() {
    let rig = Rig::new();
    let content = bytes(1, 5_000_000);
    rig.files.make("big", &content);
    let selection = rig.files.select(&["big"]);
    rig.wire.on_put(|put| {
        if put.conn == 1 && put.on_conn >= 1_000_000 {
            Fault::Stall
        } else {
            Fault::Pass
        }
    });
    let policy = Policy {
        stall: Duration::from_secs(1),
        ..steady()
    };
    let started = Instant::now();

    let outcome = ended(rig.start(&selection, policy.clone(), rig.id())).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert!(started.elapsed() >= policy.stall);
    let log = rig.wire.log();
    assert_eq!(log.connections(), 2);
    // The lanes that were stuck were reset with 408, so the receiver knows it was a stall.
    assert!(log.resets().contains(&close_code::TIMEOUT));
    assert!(!log.resets().contains(&close_code::CANCELLED));
    assert_eq!(tree(&rig.received), expect(&[("big", Some(&content))]));
}

#[tokio::test]
async fn a_stop_of_408_from_the_receiver_is_followed_by_an_attempt_that_finishes_the_transfer() {
    let rig = Rig::new();
    let content = bytes(1, 5_000_000);
    rig.files.make("big", &content);
    let selection = rig.files.select(&["big"]);
    rig.wire.on_put(|put| {
        if put.conn == 1 && put.on_conn >= 1_000_000 {
            Fault::Stop(close_code::TIMEOUT)
        } else {
            Fault::Pass
        }
    });

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert_eq!(rig.wire.log().connections(), 2);
    assert_eq!(rig.person.questions(), 1);
    assert_eq!(tree(&rig.received), expect(&[("big", Some(&content))]));
}

#[tokio::test]
async fn an_outcome_that_was_lost_on_its_way_is_asked_for_and_nothing_is_sent_again() {
    let rig = Rig::new();
    let content = bytes(1, 2_500_000);
    rig.files.make("big", &content);
    let selection = rig.files.select(&["big"]);
    rig.wire.on_call(|call| {
        if call.conn == 1 && call.kind == Kind::Finish && call.nth == 0 {
            Fate::LoseReply
        } else {
            Fate::Pass
        }
    });

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    assert_eq!((sent.files, sent.bytes), (1, 2_500_000));
    let log = rig.wire.log();
    // The second connection asked for the outcome, and was told: it offered nothing and sent nothing.
    assert_eq!(log.connections(), 2);
    assert_eq!(log.requests(Kind::Offer), 1);
    assert_eq!(log.requests(Kind::Finish), 2);
    assert!(log.opened().iter().all(|(conn, _)| *conn == 1));
    assert_eq!(rig.person.questions(), 1);
    assert_eq!(tree(&rig.received), expect(&[("big", Some(&content))]));
    until("the receiver to hear the end", || {
        rig.person.endings().len() == 1
    })
    .await;
}

#[tokio::test]
async fn an_outcome_that_is_slow_to_come_is_waited_for_and_the_transfer_is_not_offered_again() {
    let rig = Rig::new();
    let content = bytes(1, 2_500_000);
    rig.files.make("big", &content);
    let selection = rig.files.select(&["big"]);
    rig.wire
        .on_call(|call| match (call.conn, call.kind, call.nth) {
            (1, Kind::Finish, 0) => Fate::LoseReply,
            // The question about the outcome is handled, and its answer does not arrive in time.
            (2, Kind::Finish, 0) => Fate::Swallow,
            _ => Fate::Pass,
        });

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    let Outcome::Done(sent) = outcome else {
        panic!("{outcome:?}");
    };
    assert!(sent.done.failed.is_empty(), "{:?}", sent.done);
    assert_eq!((sent.files, sent.bytes), (1, 2_500_000));
    let log = rig.wire.log();
    // The question was asked again on the second connection, and nothing was offered or sent.
    assert_eq!(log.connections(), 2);
    assert_eq!(log.requests(Kind::Offer), 1);
    assert_eq!(log.requests(Kind::Finish), 3);
    assert!(log.opened().iter().all(|(conn, _)| *conn == 1));
    assert_eq!(rig.person.questions(), 1);
    assert_eq!(tree(&rig.received), expect(&[("big", Some(&content))]));
}

#[tokio::test]
async fn an_outcome_that_is_asked_for_again_on_the_same_connection_is_the_same_outcome() {
    let rig = Rig::new();
    let content = bytes(1, 100_000);
    rig.files.make("a", &content);
    let selection = rig.files.select(&["a"]);
    rig.wire.on_call(|call| {
        if call.kind == Kind::Finish && call.nth == 0 {
            Fate::Swallow
        } else {
            Fate::Pass
        }
    });

    let outcome = ended(rig.start(&selection, steady(), rig.id())).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    let log = rig.wire.log();
    assert_eq!((log.connections(), log.requests(Kind::Finish)), (1, 2));
    assert_eq!(tree(&rig.received), expect(&[("a", Some(&content))]));
}

#[tokio::test]
async fn a_receiver_that_forgot_the_transfer_is_asked_again_and_sent_everything_again() {
    let rig = Rig::new();
    let content = bytes(1, 1_000_000);
    rig.files.make("a", &content);
    let selection = rig.files.select(&["a"]);
    let wait = Gate::new();
    let held_up = wait.clone();
    rig.wire.on_call(move |call| {
        if call.conn == 1 && call.kind == Kind::Finish && call.nth == 0 {
            Fate::After(held_up.clone())
        } else {
            Fate::Pass
        }
    });
    let sending = rig.start(&selection, steady(), rig.id());
    let log = rig.wire.log();
    until("the finish", || log.requests(Kind::Finish) == 1).await;

    // The receiver loses what it knew, as when it is restarted, while the sender waits for `Done`.
    rig.incoming.forget(&sender_device().id).await;
    wait.open();
    let outcome = ended(sending).await;

    assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    assert_eq!(log.connections(), 2);
    assert_eq!(log.requests(Kind::Offer), 2);
    // It knew nothing of the transfer, so the person is asked again.
    assert_eq!(rig.person.questions(), 2);
    assert_eq!(tree(&rig.received), expect(&[("a", Some(&content))]));
}

#[tokio::test]
async fn a_receiver_that_gives_the_transfer_up_after_the_lanes_ended_declines_the_repeat() {
    let rig = Rig::new();
    let id = rig.id();
    rig.files.make("a", &bytes(1, 1000));
    let selection = rig.files.select(&["a"]);
    let wait = Gate::new();
    let held_up = wait.clone();
    rig.wire.on_call(move |call| {
        if call.conn == 1 && call.kind == Kind::Finish && call.nth == 0 {
            Fate::After(held_up.clone())
        } else {
            Fate::Pass
        }
    });
    let sending = rig.start(&selection, steady(), id);
    let log = rig.wire.log();
    until("the finish", || log.requests(Kind::Finish) == 1).await;

    // The person on the receiving device gives the transfer up after every lane has ended. A stop
    // on a lane that has ended is not seen, so the sender learns of it from the answer to `Finish`,
    // and then offers the transfer again.
    rig.incoming.cancel(&sender_device().id, id).await;
    wait.open();
    let outcome = ended(sending).await;

    assert_eq!(outcome, Outcome::Declined(DeclineReason::Refused));
    assert_eq!(log.connections(), 2);
    assert_eq!(log.requests(Kind::Offer), 2);
    assert_eq!(rig.person.questions(), 1);
    assert!(tree(&rig.received).is_empty());
}

// More than one transfer.

#[tokio::test]
async fn a_device_may_send_two_transfers_at_once_and_a_third_waits_for_one_to_end() {
    let rig = Rig::new();
    let first = bytes(1, 3_000_000);
    let second = bytes(2, 3_000_000);
    rig.files.make("first", &first);
    rig.files.make("second", &second);
    rig.files.make("third", b"third");
    let hold = Gate::new();
    let held_up = hold.clone();
    rig.wire.on_put(move |put| {
        if put.on_conn >= 300_000 && !held_up.is_open() {
            Fault::Wait(held_up.clone())
        } else {
            Fault::Pass
        }
    });
    let one = rig.start(&rig.files.select(&["first"]), steady(), rig.id());
    let two = rig.start(&rig.files.select(&["second"]), steady(), rig.id());
    until("both questions", || rig.person.questions() == 2).await;

    // Two are going, and a third is turned away at once, as the receiver's limit says.
    let third = rig.start(&rig.files.select(&["third"]), steady(), rig.id());
    let outcome = ended(third).await;
    assert_eq!(outcome, Outcome::Declined(DeclineReason::RateLimited));

    hold.open();
    for sending in [one, two] {
        let outcome = ended(sending).await;
        assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
    }
    // The device has room for another now.
    let again = ended(rig.start(&rig.files.select(&["third"]), steady(), rig.id())).await;
    assert!(matches!(again, Outcome::Done(_)), "{again:?}");
    assert_eq!(
        tree(&rig.received),
        expect(&[
            ("first", Some(&first)),
            ("second", Some(&second)),
            ("third", Some(b"third"))
        ])
    );
}
