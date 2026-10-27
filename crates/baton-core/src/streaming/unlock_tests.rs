// SPDX-License-Identifier: Apache-2.0

//! The unlock of a locked screen. The first part holds the rules of the source to a clock that the
//! test sets, with the record of the viewer in memory. The second does the same for the rules of
//! the viewer. The third has a real source in front of a viewer that a test writes by hand, the
//! fourth a real viewer in front of a source that a test writes by hand, and the fifth has the two
//! real sessions face to face.

use std::{
    collections::VecDeque,
    fmt,
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{
    DeviceId,
    id::SessionId,
    limits::MAX_SEALED_KEY_BYTES,
    message::Message,
    stream::Codec,
    unlock::{BoxKey, Challenge, SealedKey, Sealing, SealingKey},
};
use baton_stream::{
    KIND_CONTROL,
    control::{
        Config as Format, InputBatch, Lift, Pause, PauseReason, Report, SourceMessage, Thermal,
        ViewerMessage, Want,
    },
    header::{FrameHeader, encode_prefix},
    sender::LtrToken,
    unlock::{Ask, Cancelled, Done, Opened, Sealed},
};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;

use super::{
    CancelReason, DecodeError, Decoder, DecoderState, End, Granted, KeepError, Question, Settled,
    SoundPlan, SourceEvents, SourceHandle, SourceParams, SourceSetup, Streaming, Takes,
    UnlockAttempt, UnlockOutcome, Unlocker, ViewHandle, ViewParams, Viewing,
    answer_tests::quick,
    unlock::{
        Answer, Asking, Concluded, Finish, Handoff, Patience, Record, Replacement, Step,
        UnlockSetup, UnlockStore, Unlocking, Waiting,
    },
};
use crate::{
    Error,
    testing::{Device, PATIENCE, QUIET, connect, next},
    uni::{Arrival, Kind, Sinks},
    util::lock,
    wire::{read_next, write_message},
};

const MAC: DeviceId = DeviceId::from_bytes([0x4d; DeviceId::LEN]);
const PHONE: DeviceId = DeviceId::from_bytes([0x50; DeviceId::LEN]);

fn session_id() -> SessionId {
    SessionId::from_bytes([7; SessionId::LEN])
}

fn secs(count: u64) -> Duration {
    Duration::from_secs(count)
}

fn millis(count: u64) -> Duration {
    Duration::from_millis(count)
}

/// Waits until `condition` holds.
async fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// A sealed key of the shape that the first version makes. Its byte tells it from another.
fn sealed_key(byte: u8) -> SealedKey {
    SealedKey::new(vec![byte; 113])
}

/// The key of a box. Its byte tells it from another.
fn box_key(byte: u8) -> BoxKey {
    BoxKey::from_bytes([byte; BoxKey::LEN])
}

fn challenge(byte: u8) -> Challenge {
    Challenge::from_bytes([byte; Challenge::LEN])
}

/// What a device keeps for the viewer once the person has set the unlock up. The box and the sealed
/// key carry `version`, so that a test can tell them from the ones that replace them.
fn record_between(target: DeviceId, viewer: DeviceId, version: u8) -> Record {
    let mut point = vec![0x04];
    point.extend((0..64_u8).map(|byte| 0xc0 ^ byte));
    Record {
        boxed: vec![0xb0 + version; 40],
        sealed: sealed_key(version),
        sealing: Sealing::EciesP256,
        public: SealingKey::new(point),
        target,
        viewer,
    }
}

fn record(version: u8) -> Record {
    record_between(MAC, PHONE, version)
}

/// The question of the source for attempt `number`.
fn question_for(number: u64, byte: u8) -> Sealed {
    Sealed {
        number,
        challenge: challenge(byte),
        sealed: sealed_key(byte),
    }
}

#[allow(clippy::unnecessary_wraps)] // it is compared with what a step says, which is an `Option`
fn told(number: u64, outcome: UnlockOutcome) -> Option<SourceMessage> {
    Some(SourceMessage::UnlockDone(Done { number, outcome }))
}

fn unlock_message(number: u64) -> ViewerMessage {
    ViewerMessage::Unlock(Ask { number })
}

fn cancelled(reason: CancelReason) -> ViewerMessage {
    ViewerMessage::UnlockCancelled(Cancelled { reason })
}

// The record of a device in memory, where a node keeps it in a file.

/// A disk that is slow: a write waits until the test lets it through.
#[derive(Clone, Default)]
struct Slow(Arc<(Mutex<Gate>, Condvar)>);

#[derive(Default)]
struct Gate {
    /// The writes that may go through.
    allowed: usize,
    /// The writes that have come to wait, in all.
    arrived: usize,
}

impl Slow {
    /// Lets `count` more writes through.
    fn allow(&self, count: usize) {
        lock(&self.0.0).allowed += count;
        self.0.1.notify_all();
    }

    /// How many writes have come to wait, in all.
    fn arrived(&self) -> usize {
        lock(&self.0.0).arrived
    }

    /// A write comes, and waits to be let through. It goes on after the patience of a test, so that
    /// a test that failed does not hold the runtime.
    fn pass(&self) {
        let (gate, signal) = &*self.0;
        let mut gate = lock(gate);
        gate.arrived += 1;
        let mut waited = Duration::ZERO;
        while gate.allowed == 0 && waited < PATIENCE {
            gate = signal
                .wait_timeout(gate, millis(20))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            waited += millis(20);
        }
        gate.allowed = gate.allowed.saturating_sub(1);
    }
}

#[derive(Default)]
struct Kept {
    /// The device that keeps the record, and the viewer it is for.
    devices: Option<(DeviceId, DeviceId)>,
    record: Option<Record>,
    /// An attempt marked the setup as ended.
    ended: bool,
    history: Vec<Concluded>,
    restored: Vec<SealedKey>,
    /// Writing the record fails.
    unwritable: bool,
    /// A setup that ends is not marked, as when the disk is full.
    forgetful: bool,
    /// Writes of the history and of the state of the setup wait for the test.
    slow: Option<Slow>,
}

#[derive(Clone, Default)]
struct MemoryStore(Arc<Mutex<Kept>>);

impl MemoryStore {
    fn with(record: Record) -> Self {
        let store = Self::default();
        {
            let mut kept = lock(&store.0);
            kept.devices = Some((record.target, record.viewer));
            kept.record = Some(record);
        }
        store
    }

    /// How the attempts ended, oldest first.
    fn history(&self) -> Vec<Concluded> {
        lock(&self.0).history.clone()
    }

    fn finishes(&self) -> Vec<Finish> {
        self.history().iter().map(|each| each.finish).collect()
    }

    /// The sealed key that the record has now.
    fn sealed(&self) -> Option<SealedKey> {
        lock(&self.0)
            .record
            .as_ref()
            .map(|record| record.sealed.clone())
    }

    fn boxed(&self) -> Option<Vec<u8>> {
        lock(&self.0)
            .record
            .as_ref()
            .map(|record| record.boxed.clone())
    }

    /// There is a record and nothing ended the setup.
    fn usable(&self) -> bool {
        let kept = lock(&self.0);
        kept.record.is_some() && !kept.ended
    }

    fn restored(&self) -> Vec<SealedKey> {
        lock(&self.0).restored.clone()
    }

    /// The person sets the unlock up again at the device: another box, another sealed key.
    fn set_up_again(&self, version: u8) {
        let mut kept = lock(&self.0);
        let (target, viewer) = kept.devices.expect("nothing was ever set up");
        kept.record = Some(record_between(target, viewer, version));
        kept.ended = false;
    }

    /// The person turns the unlock off.
    fn turn_off(&self) {
        lock(&self.0).record = None;
    }

    fn make_unwritable(&self) {
        lock(&self.0).unwritable = true;
    }

    fn make_forgetful(&self) {
        lock(&self.0).forgetful = true;
    }

    /// The writes of what came of an attempt, and of the state of the setup, wait for the test.
    fn make_slow(&self) -> Slow {
        let slow = Slow::default();
        lock(&self.0).slow = Some(slow.clone());
        slow
    }

    /// A write waits for the disk, without the record locked.
    fn wait_for_disk(&self) {
        let slow = lock(&self.0).slow.clone();
        if let Some(slow) = slow {
            slow.pass();
        }
    }
}

impl UnlockStore for MemoryStore {
    fn record(&self) -> Option<Record> {
        let kept = lock(&self.0);
        kept.record.clone().filter(|_| !kept.ended)
    }

    fn replace(&self, old: &SealedKey, new: Replacement) -> Result<(), KeepError> {
        let mut kept = lock(&self.0);
        if kept.unwritable {
            return Err(KeepError::Unwritable);
        }
        match kept.record.as_mut() {
            Some(record) if &record.sealed == old => {
                record.boxed = new.boxed;
                record.sealed = new.sealed;
                Ok(())
            }
            _ => Err(KeepError::Changed),
        }
    }

    fn concluded(&self, concluded: Concluded) {
        self.wait_for_disk();
        let mut kept = lock(&self.0);
        let same = kept.record.as_ref().map(|record| &record.sealed) == concluded.sealed.as_ref();
        if concluded.finish.ends_setup() && !kept.forgetful && same {
            kept.ended = true;
        }
        kept.history.push(concluded);
    }

    fn restore(&self, sealed: &SealedKey) {
        self.wait_for_disk();
        let mut kept = lock(&self.0);
        if kept.record.as_ref().map(|record| &record.sealed) == Some(sealed) {
            kept.ended = false;
        }
        kept.restored.push(sealed.clone());
    }
}

// The rules of the source, with a clock that the test sets.

/// The rules for a session whose screen is locked, with the unlock set up, and the store they read.
fn ready() -> (Unlocking, MemoryStore) {
    let store = MemoryStore::with(record(1));
    let mut unlocking = Unlocking::new(Arc::new(store.clone()), PHONE, secs(60), secs(20));
    let _ = unlocking.locked(true, secs(0));
    (unlocking, store)
}

/// Writes down what a step asked to have written down, as the session does on a thread that may
/// wait.
fn write(step: &mut Step) {
    if let Some(job) = step.job.take() {
        job.run();
    }
}

/// What the step said to the viewer, if it said anything.
fn said(step: &Step) -> Option<SourceMessage> {
    step.say.clone()
}

/// The question that the step sent.
fn asked_question(step: &Step) -> Sealed {
    match &step.say {
        Some(SourceMessage::UnlockSealed(sealed)) => sealed.clone(),
        other => panic!("the source did not ask the question: {other:?}"),
    }
}

/// The answer to a question, with this key.
fn answer_with(question: &Sealed, key: BoxKey) -> Opened {
    Opened {
        challenge: question.challenge,
        key,
    }
}

/// Whether the step asked the question.
fn asks(step: &Step) -> bool {
    matches!(step.say, Some(SourceMessage::UnlockSealed(_)))
}

/// An attempt that the platform has been given, and the wait for what it reports.
struct Given {
    attempt: UnlockAttempt,
    waiting: Waiting,
}

/// Asks, and answers with a key: the rules hand the platform an attempt.
fn given(unlocking: &mut Unlocking, number: u64, at: Duration) -> Given {
    let asked = unlocking.asked(number, at, true);
    let question = asked_question(&asked);
    let step = unlocking.opened(answer_with(&question, box_key(0x77)), at);
    assert!(step.say.is_none() && step.job.is_none());
    let Handoff { attempt, waiting } = step.hand.expect("the platform was not given the attempt");
    Given { attempt, waiting }
}

/// The platform puts the password in a new box, which it does before it checks or types anything.
fn keeps(attempt: &mut UnlockAttempt) {
    attempt.keep(vec![0xee; 40], sealed_key(2)).unwrap();
}

/// The platform says `outcome` as it is, with the box it has kept or none, and the session waits
/// for it and writes it down. The viewer is not told yet.
async fn report_as_it_is(unlocking: &mut Unlocking, given: Given, outcome: UnlockOutcome) {
    let Given { attempt, waiting } = given;
    attempt.done(outcome);
    let (number, report) = waiting.report(secs(5)).await;
    unlocking
        .reported(number, report)
        .expect("nothing was to be written")
        .run();
}

/// The platform says `outcome`, and the session waits for it and writes it down. The viewer is not
/// told yet. The platform has kept a new box by then, unless it says `Stale` or `Failed`, which it
/// says without.
async fn report(unlocking: &mut Unlocking, mut given: Given, outcome: UnlockOutcome) {
    if !matches!(outcome, UnlockOutcome::Stale | UnlockOutcome::Failed) {
        keeps(&mut given.attempt);
    }
    report_as_it_is(unlocking, given, outcome).await;
}

/// The record of what the platform reported is written, and the viewer is told. A job that comes
/// with it, which takes the setup back, is done first.
fn closed(unlocking: &mut Unlocking, number: u64, now: Duration) -> Option<SourceMessage> {
    let mut step = unlocking.closed(number, now);
    write(&mut step);
    step.say
}

#[test]
fn a_setup_ends_with_a_rejected_password_a_box_that_was_not_opened_a_lost_key_and_a_box_not_kept() {
    for finish in [
        Finish::Done(UnlockOutcome::Rejected),
        Finish::Done(UnlockOutcome::Stale),
        Finish::Cancelled(CancelReason::KeyLost),
        Finish::Unkept(UnlockOutcome::Unlocked),
        Finish::Unkept(UnlockOutcome::Rejected),
        Finish::Unkept(UnlockOutcome::NotLocked),
        Finish::Unkept(UnlockOutcome::NotPrivate),
    ] {
        assert!(finish.ends_setup(), "{finish:?}");
    }
    for finish in [
        Finish::Done(UnlockOutcome::Unlocked),
        Finish::Done(UnlockOutcome::NotSetUp),
        Finish::Done(UnlockOutcome::NotLocked),
        Finish::Done(UnlockOutcome::NotPrivate),
        Finish::Done(UnlockOutcome::Busy),
        Finish::Done(UnlockOutcome::TimedOut),
        Finish::Done(UnlockOutcome::Failed),
        Finish::Done(UnlockOutcome::Other),
        Finish::Cancelled(CancelReason::Declined),
        Finish::Cancelled(CancelReason::Failed),
        Finish::Cancelled(CancelReason::Other),
        Finish::Ended,
    ] {
        assert!(!finish.ends_setup(), "{finish:?}");
    }
}

#[test]
fn the_debug_form_of_a_record_says_how_big_the_box_is_and_not_what_is_in_it() {
    let text = format!("{:?}", record(1));
    assert!(text.contains("40 bytes"), "{text}");
    // The bytes of the box are 177.
    assert!(!text.contains("177"), "{text}");
}

#[test]
fn a_viewer_that_asks_when_all_is_ready_is_sent_the_sealed_key_and_a_challenge() {
    let (mut unlocking, store) = ready();
    let step = unlocking.asked(7, secs(1), true);
    let question = asked_question(&step);
    assert_eq!(question.number, 7);
    assert_eq!(question.sealed, sealed_key(1));
    assert!(step.job.is_none() && step.hand.is_none());
    assert_eq!(store.history(), []);
}

#[test]
fn each_question_has_a_challenge_of_its_own() {
    let (mut unlocking, _store) = ready();
    let mut challenges: Vec<Challenge> = Vec::new();
    for number in 1..=20 {
        let step = unlocking.asked(number, secs(number), true);
        challenges.push(asked_question(&step).challenge);
        let _ = unlocking.cancelled(CancelReason::Declined, secs(number));
    }
    for (index, each) in challenges.iter().enumerate() {
        assert!(
            !challenges[index + 1..].contains(each),
            "a challenge came twice"
        );
    }
}

#[test]
fn an_unlock_is_refused_in_the_order_that_the_protocol_gives() {
    let lifted = Lift {
        screen: true,
        input: true,
    };

    // Busy comes first: an attempt goes, the setup is gone, the privacy is lifted, and the screen
    // is not locked.
    let (mut unlocking, store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1));
    store.turn_off();
    let _ = unlocking.lifted(lifted, secs(2));
    let _ = unlocking.locked(false, secs(2));
    assert_eq!(
        said(&unlocking.asked(2, secs(3), false)),
        told(2, UnlockOutcome::Busy)
    );
    drop(attempt);

    // Then NotSetUp, with the privacy lifted and the screen not locked.
    let (mut unlocking, store) = ready();
    store.turn_off();
    let _ = unlocking.lifted(lifted, secs(1));
    let _ = unlocking.locked(false, secs(1));
    assert_eq!(
        said(&unlocking.asked(3, secs(2), false)),
        told(3, UnlockOutcome::NotSetUp)
    );

    // Then NotPrivate, with the screen not locked.
    let (mut unlocking, _store) = ready();
    let _ = unlocking.locked(false, secs(1));
    assert_eq!(
        said(&unlocking.asked(4, secs(2), false)),
        told(4, UnlockOutcome::NotPrivate)
    );

    // And NotLocked last.
    let (mut unlocking, _store) = ready();
    let _ = unlocking.locked(false, secs(1));
    assert_eq!(
        said(&unlocking.asked(5, secs(2), true)),
        told(5, UnlockOutcome::NotLocked)
    );
}

#[test]
fn the_privacy_is_in_place_only_if_both_switches_are_off_and_the_platform_vouches_for_it() {
    let lift = |screen, input| Lift { screen, input };
    for (lifted, in_place) in [
        (lift(true, false), true),
        (lift(false, true), true),
        (lift(true, true), true),
        (lift(false, false), false),
    ] {
        let (mut unlocking, _store) = ready();
        let _ = unlocking.lifted(lifted, secs(0));
        assert_eq!(
            said(&unlocking.asked(1, secs(1), in_place)),
            told(1, UnlockOutcome::NotPrivate),
            "{lifted:?} {in_place}"
        );
    }
    // Both put back, and the platform vouches for it: the question comes.
    let (mut unlocking, _store) = ready();
    let _ = unlocking.lifted(lift(true, true), secs(0));
    let _ = unlocking.lifted(lift(false, false), secs(1));
    assert!(asks(&unlocking.asked(1, secs(2), true)));
}

#[test]
fn a_refusal_is_written_down_with_the_record_it_was_about() {
    let (mut unlocking, store) = ready();
    let _ = unlocking.locked(false, secs(1));
    let mut step = unlocking.asked(1, secs(2), true);
    write(&mut step);
    store.turn_off();
    let mut step = unlocking.asked(2, secs(4), true);
    write(&mut step);
    assert_eq!(
        store.history(),
        [
            Concluded {
                finish: Finish::Done(UnlockOutcome::NotLocked),
                sealed: Some(sealed_key(1)),
            },
            Concluded {
                finish: Finish::Done(UnlockOutcome::NotSetUp),
                sealed: None,
            },
        ]
    );
}

#[test]
fn a_viewer_that_asks_again_and_again_fills_the_history_once_a_second() {
    let (mut unlocking, _store) = ready();
    let _ = unlocking.locked(false, secs(0));
    let mut written = Vec::new();
    for (number, at) in [(1, 0), (2, 400), (3, 900), (4, 1000), (5, 1500), (6, 2100)] {
        let step = unlocking.asked(number, millis(at), true);
        // The viewer is told every time.
        assert_eq!(said(&step), told(number, UnlockOutcome::NotLocked));
        written.push(step.job.is_some());
    }
    assert_eq!(written, [true, false, false, true, false, true]);
}

#[test]
fn a_wrong_challenge_is_dropped_and_the_right_one_hands_the_platform_the_attempt() {
    let (mut unlocking, _store) = ready();
    let question = asked_question(&unlocking.asked(3, secs(1), true));
    let wrong = Opened {
        challenge: challenge(0),
        key: box_key(0x55),
    };
    let step = unlocking.opened(wrong, secs(2));
    assert!(step.say.is_none() && step.job.is_none() && step.hand.is_none());
    // The attempt waits still.
    assert_eq!(
        said(&unlocking.asked(4, secs(3), true)),
        told(4, UnlockOutcome::Busy)
    );

    let step = unlocking.opened(answer_with(&question, box_key(0x66)), secs(3));
    let handoff = step.hand.expect("the platform was not given the attempt");
    let attempt = handoff.attempt;
    assert_eq!(handoff.waiting.number, 3);
    assert_eq!(attempt.key(), &box_key(0x66));
    assert_eq!(attempt.boxed(), &[0xb1; 40][..]);
    assert_eq!(attempt.sealing(), Sealing::EciesP256);
    assert_eq!(attempt.public(), &record(1).public);
    assert_eq!(attempt.target(), MAC);
    assert_eq!(attempt.viewer(), PHONE);
    assert!(attempt.is_live());
}

#[test]
fn an_answer_that_has_no_question_or_comes_twice_does_nothing() {
    let (mut unlocking, _store) = ready();
    let stray = Opened {
        challenge: challenge(9),
        key: box_key(1),
    };
    // Nothing was asked.
    assert!(unlocking.opened(stray, secs(1)).hand.is_none());
    // The answer comes twice: the second finds an attempt that goes.
    let question = asked_question(&unlocking.asked(1, secs(2), true));
    let first = unlocking.opened(answer_with(&question, box_key(2)), secs(3));
    assert!(first.hand.is_some());
    let second = unlocking.opened(answer_with(&question, box_key(3)), secs(3));
    assert!(second.hand.is_none() && second.say.is_none() && second.job.is_none());
}

#[test]
fn a_setup_that_changed_while_the_face_was_asked_for_ends_the_attempt_and_not_the_setup() {
    // Set up again: the key that comes back opens the old box.
    let (mut unlocking, store) = ready();
    let question = asked_question(&unlocking.asked(1, secs(1), true));
    store.set_up_again(2);
    let mut step = unlocking.opened(answer_with(&question, box_key(4)), secs(2));
    assert_eq!(said(&step), told(1, UnlockOutcome::Failed));
    assert!(step.hand.is_none());
    write(&mut step);
    assert!(store.usable());
    assert_eq!(store.sealed(), Some(sealed_key(2)));
    // The next question is for the new box.
    let step = unlocking.asked(2, secs(3), true);
    assert_eq!(asked_question(&step).sealed, sealed_key(2));

    // Turned off: there is nothing to unlock.
    let (mut unlocking, store) = ready();
    let question = asked_question(&unlocking.asked(1, secs(1), true));
    store.turn_off();
    let step = unlocking.opened(answer_with(&question, box_key(4)), secs(2));
    assert_eq!(said(&step), told(1, UnlockOutcome::NotSetUp));
    assert!(step.hand.is_none());
}

#[test]
fn a_record_for_another_viewer_is_no_record() {
    let other = DeviceId::from_bytes([0x51; DeviceId::LEN]);
    let store = MemoryStore::with(record_between(MAC, other, 1));
    let mut unlocking = Unlocking::new(Arc::new(store), PHONE, secs(60), secs(20));
    let _ = unlocking.locked(true, secs(0));
    assert_eq!(
        said(&unlocking.asked(1, secs(1), true)),
        told(1, UnlockOutcome::NotSetUp)
    );
}

#[test]
fn a_viewer_that_gives_up_ends_the_attempt_and_is_told_nothing() {
    let (mut unlocking, store) = ready();
    let _ = unlocking.asked(1, secs(1), true);
    let mut step = unlocking.cancelled(CancelReason::Declined, secs(2));
    assert!(step.say.is_none());
    write(&mut step);
    assert_eq!(
        store.finishes(),
        [Finish::Cancelled(CancelReason::Declined)]
    );
    assert!(store.usable());
    // The next attempt is not busy.
    assert!(asks(&unlocking.asked(2, secs(3), true)));

    // A viewer that gives up when nothing goes changes nothing.
    let (mut unlocking, store) = ready();
    let mut step = unlocking.cancelled(CancelReason::Failed, secs(1));
    assert!(step.say.is_none() && step.job.is_none());
    write(&mut step);
    assert_eq!(store.history(), []);
}

#[test]
fn a_viewer_that_lost_its_key_ends_the_setup_whatever_the_store_does() {
    let (mut unlocking, store) = ready();
    store.make_forgetful();
    let _ = unlocking.asked(1, secs(1), true);
    let mut step = unlocking.cancelled(CancelReason::KeyLost, secs(2));
    write(&mut step);
    assert_eq!(store.finishes(), [Finish::Cancelled(CancelReason::KeyLost)]);
    // The store did not mark the record, and this session does not go on with it all the same.
    assert!(store.usable());
    assert_eq!(
        said(&unlocking.asked(2, secs(3), true)),
        told(2, UnlockOutcome::NotSetUp)
    );
}

#[test]
fn a_question_that_is_not_answered_in_time_ends_with_timed_out() {
    let (mut unlocking, store) = ready();
    let _ = unlocking.asked(1, secs(10), true);
    let step = unlocking.tick(millis(69_900));
    assert!(step.say.is_none() && step.job.is_none());
    let mut step = unlocking.tick(secs(70));
    assert_eq!(said(&step), told(1, UnlockOutcome::TimedOut));
    write(&mut step);
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::TimedOut)]);
    // It is over once, and the next attempt is not busy.
    assert!(unlocking.tick(secs(80)).say.is_none());
    assert!(asks(&unlocking.asked(2, secs(81), true)));
}

#[test]
fn a_switch_turned_on_while_the_face_is_asked_for_ends_the_attempt() {
    let (mut unlocking, store) = ready();
    let _ = unlocking.asked(1, secs(1), true);
    let mut step = unlocking.lifted(
        Lift {
            screen: false,
            input: true,
        },
        secs(2),
    );
    assert_eq!(said(&step), told(1, UnlockOutcome::NotPrivate));
    write(&mut step);
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::NotPrivate)]);
    // A switch put back says nothing.
    let step = unlocking.lifted(Lift::default(), secs(3));
    assert!(step.say.is_none() && step.job.is_none());
}

#[test]
fn a_switch_turned_on_while_the_password_is_typed_waits_for_the_platform() {
    let (mut unlocking, _store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1)).attempt;
    let step = unlocking.lifted(
        Lift {
            screen: true,
            input: true,
        },
        secs(2),
    );
    assert!(step.say.is_none() && step.job.is_none());
    assert!(attempt.is_live());
}

#[test]
fn a_screen_that_is_unlocked_by_someone_else_ends_the_attempt_that_waits_for_the_face() {
    let (mut unlocking, store) = ready();
    let _ = unlocking.asked(1, secs(1), true);
    let mut step = unlocking.locked(false, secs(2));
    assert_eq!(said(&step), told(1, UnlockOutcome::NotLocked));
    write(&mut step);
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::NotLocked)]);
    // Locked again: the next question comes.
    let _ = unlocking.locked(true, secs(3));
    assert!(asks(&unlocking.asked(2, secs(4), true)));
}

#[tokio::test]
async fn what_the_platform_reports_is_written_down_before_the_viewer_is_told() {
    for (outcome, ends_setup) in [
        (UnlockOutcome::Unlocked, false),
        (UnlockOutcome::Rejected, true),
        (UnlockOutcome::Stale, true),
        (UnlockOutcome::Failed, false),
        (UnlockOutcome::NotLocked, false),
        (UnlockOutcome::NotPrivate, false),
    ] {
        let (mut unlocking, store) = ready();
        let attempt = given(&mut unlocking, 4, secs(1));
        report(&mut unlocking, attempt, outcome).await;
        // It is on disk, and the viewer has not been told yet.
        assert_eq!(store.finishes(), [Finish::Done(outcome)], "{outcome:?}");
        assert_eq!(store.usable(), !ends_setup, "{outcome:?}");
        assert_eq!(
            closed(&mut unlocking, 4, secs(3)),
            told(4, outcome),
            "{outcome:?}"
        );
        // And the next attempt finds the setup as the record says.
        let next = said(&unlocking.asked(5, secs(4), true));
        if ends_setup {
            assert_eq!(next, told(5, UnlockOutcome::NotSetUp), "{outcome:?}");
        } else {
            assert!(
                matches!(next, Some(SourceMessage::UnlockSealed(_))),
                "{outcome:?}"
            );
        }
    }
}

#[tokio::test]
async fn an_outcome_that_the_platform_may_not_say_is_failed() {
    for outcome in [
        UnlockOutcome::Busy,
        UnlockOutcome::NotSetUp,
        UnlockOutcome::TimedOut,
        UnlockOutcome::Other,
    ] {
        let (mut unlocking, store) = ready();
        let attempt = given(&mut unlocking, 1, secs(1));
        report(&mut unlocking, attempt, outcome).await;
        assert_eq!(
            store.finishes(),
            [Finish::Done(UnlockOutcome::Failed)],
            "{outcome:?}"
        );
        assert!(store.usable(), "{outcome:?}");
        assert_eq!(
            closed(&mut unlocking, 1, secs(3)),
            told(1, UnlockOutcome::Failed),
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn a_platform_that_says_what_follows_a_new_box_without_having_kept_one_ends_the_setup() {
    for outcome in [
        UnlockOutcome::Unlocked,
        UnlockOutcome::Rejected,
        UnlockOutcome::NotLocked,
        UnlockOutcome::NotPrivate,
    ] {
        let (mut unlocking, store) = ready();
        let attempt = given(&mut unlocking, 4, secs(1));
        report_as_it_is(&mut unlocking, attempt, outcome).await;
        // The record says so, about the box that the source still holds, before the viewer is told.
        assert_eq!(
            store.history(),
            [Concluded {
                finish: Finish::Unkept(outcome),
                sealed: Some(sealed_key(1)),
            }],
            "{outcome:?}"
        );
        assert!(!store.usable(), "{outcome:?}");
        // The viewer is told what the platform said, and finds no setup after it.
        assert_eq!(
            closed(&mut unlocking, 4, secs(3)),
            told(4, outcome),
            "{outcome:?}"
        );
        assert_eq!(
            said(&unlocking.asked(5, secs(4), true)),
            told(5, UnlockOutcome::NotSetUp),
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn a_platform_that_types_after_a_box_that_could_not_be_kept_ends_the_setup() {
    let (mut unlocking, store) = ready();
    let Given {
        mut attempt,
        waiting,
    } = given(&mut unlocking, 1, secs(1));
    store.make_unwritable();
    assert_eq!(
        attempt.keep(vec![0xee; 40], sealed_key(2)),
        Err(KeepError::Unwritable)
    );
    attempt.done(UnlockOutcome::Unlocked);
    let (number, report) = waiting.report(secs(5)).await;
    unlocking.reported(number, report).unwrap().run();
    assert_eq!(store.finishes(), [Finish::Unkept(UnlockOutcome::Unlocked)]);
    assert!(!store.usable());
    assert_eq!(
        closed(&mut unlocking, 1, secs(3)),
        told(1, UnlockOutcome::Unlocked)
    );
}

#[tokio::test]
async fn a_platform_that_keeps_no_box_and_says_failed_or_stale_has_kept_the_order() {
    // It could not keep one, and typed nothing: the setup is as it was.
    let (mut unlocking, store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1));
    report_as_it_is(&mut unlocking, attempt, UnlockOutcome::Failed).await;
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::Failed)]);
    assert!(store.usable());
    assert_eq!(
        closed(&mut unlocking, 1, secs(3)),
        told(1, UnlockOutcome::Failed)
    );

    // The key did not open the box, which the platform finds before it has a password to put in a
    // new one. The setup ends for that.
    let (mut unlocking, store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1));
    report_as_it_is(&mut unlocking, attempt, UnlockOutcome::Stale).await;
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::Stale)]);
    assert!(!store.usable());
}

#[tokio::test]
async fn a_rejection_without_a_new_box_is_not_given_back_when_the_screen_unlocks() {
    // While the record is on its way to the disk.
    let (mut unlocking, store) = ready();
    let Given { attempt, waiting } = given(&mut unlocking, 1, secs(1));
    attempt.done(UnlockOutcome::Rejected);
    let (number, report) = waiting.report(secs(5)).await;
    let job = unlocking.reported(number, report).unwrap();
    let step = unlocking.locked(false, secs(11));
    assert!(step.say.is_none() && step.job.is_none());
    job.run();
    let mut step = unlocking.closed(1, secs(12));
    assert_eq!(said(&step), told(1, UnlockOutcome::Rejected));
    assert!(step.job.is_none());
    write(&mut step);
    assert_eq!(store.restored(), []);
    assert!(!store.usable());

    // And within the grace after the viewer was told.
    let (mut unlocking, store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1));
    report_as_it_is(&mut unlocking, attempt, UnlockOutcome::Rejected).await;
    assert_eq!(
        closed(&mut unlocking, 1, secs(12)),
        told(1, UnlockOutcome::Rejected)
    );
    let mut step = unlocking.locked(false, secs(15));
    assert!(step.say.is_none());
    write(&mut step);
    assert_eq!(store.restored(), []);
    assert!(!store.usable());
}

#[tokio::test]
async fn only_a_rejection_is_told_as_unlocked_when_the_screen_unlocked_while_it_was_written() {
    for outcome in [UnlockOutcome::Stale, UnlockOutcome::Failed] {
        let (mut unlocking, store) = ready();
        let Given { attempt, waiting } = given(&mut unlocking, 1, secs(1));
        attempt.done(outcome);
        let (number, report) = waiting.report(secs(5)).await;
        let job = unlocking.reported(number, report).unwrap();
        let step = unlocking.locked(false, secs(11));
        assert!(step.say.is_none() && step.job.is_none());
        job.run();
        let mut step = unlocking.closed(1, secs(12));
        assert_eq!(said(&step), told(1, outcome), "{outcome:?}");
        assert!(step.job.is_none(), "{outcome:?}");
        write(&mut step);
        assert_eq!(store.restored(), [], "{outcome:?}");
    }
}

#[tokio::test]
async fn an_attempt_that_is_let_go_of_without_a_word_has_failed() {
    let (mut unlocking, store) = ready();
    let Given { attempt, waiting } = given(&mut unlocking, 1, secs(1));
    drop(attempt);
    let (number, report) = waiting.report(secs(5)).await;
    assert!(report.is_some());
    unlocking.reported(number, report).unwrap().run();
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::Failed)]);
    assert_eq!(
        closed(&mut unlocking, 1, secs(2)),
        told(1, UnlockOutcome::Failed)
    );
}

#[tokio::test]
async fn a_platform_that_does_not_report_in_time_has_failed_and_the_attempt_is_no_longer_live() {
    let (mut unlocking, store) = ready();
    let Given { attempt, waiting } = given(&mut unlocking, 1, secs(1));
    let (number, report) = waiting.report(millis(30)).await;
    assert!(report.is_none());
    assert!(attempt.is_live());
    unlocking.reported(number, report).unwrap().run();
    assert!(!attempt.is_live());
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::Failed)]);
    assert_eq!(
        closed(&mut unlocking, 1, secs(2)),
        told(1, UnlockOutcome::Failed)
    );
    // What the platform says later is heard by no one.
    attempt.done(UnlockOutcome::Unlocked);
    assert_eq!(store.finishes().len(), 1);
}

#[tokio::test]
async fn a_report_for_another_attempt_or_a_second_report_is_dropped() {
    let (mut unlocking, store) = ready();
    let Given {
        mut attempt,
        waiting,
    } = given(&mut unlocking, 1, secs(1));
    keeps(&mut attempt);
    attempt.done(UnlockOutcome::Unlocked);
    let (number, report) = waiting.report(secs(5)).await;
    assert!(unlocking.reported(number + 1, None).is_none());
    unlocking.reported(number, report).unwrap().run();
    assert!(unlocking.reported(number, None).is_none());
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::Unlocked)]);
    // The viewer is told of no other attempt, and of this one when its record is written.
    assert_eq!(closed(&mut unlocking, number + 1, secs(2)), None);
    assert_eq!(
        closed(&mut unlocking, number, secs(2)),
        told(number, UnlockOutcome::Unlocked)
    );
}

#[tokio::test]
async fn the_source_is_busy_until_the_record_is_written_and_the_viewer_told() {
    let (mut unlocking, _store) = ready();
    let Given {
        mut attempt,
        waiting,
    } = given(&mut unlocking, 1, secs(1));
    keeps(&mut attempt);
    attempt.done(UnlockOutcome::Unlocked);
    // Busy while the platform works.
    assert_eq!(
        said(&unlocking.asked(2, secs(2), true)),
        told(2, UnlockOutcome::Busy)
    );
    let (number, report) = waiting.report(secs(5)).await;
    let job = unlocking.reported(number, report).unwrap();
    // And while the record is on its way to the disk.
    assert_eq!(
        said(&unlocking.asked(3, secs(3), true)),
        told(3, UnlockOutcome::Busy)
    );
    job.run();
    let _ = closed(&mut unlocking, number, secs(4));
    assert!(asks(&unlocking.asked(4, secs(5), true)));
}

#[tokio::test]
async fn a_viewer_that_gives_up_while_the_password_is_typed_stops_the_attempt_and_hears_nothing() {
    let (mut unlocking, store) = ready();
    let Given {
        mut attempt,
        waiting,
    } = given(&mut unlocking, 1, secs(1));
    keeps(&mut attempt);
    let step = unlocking.cancelled(CancelReason::Failed, secs(2));
    assert!(step.say.is_none() && step.job.is_none());
    assert!(!attempt.is_live());
    // The platform stops, and says what it knows. That is written down, and no one is told.
    attempt.done(UnlockOutcome::NotLocked);
    let (number, report) = waiting.report(secs(5)).await;
    unlocking.reported(number, report).unwrap().run();
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::NotLocked)]);
    assert_eq!(closed(&mut unlocking, number, secs(3)), None);
}

#[tokio::test]
async fn the_box_is_replaced_before_anything_is_typed_and_a_rejected_password_ends_the_new_setup() {
    let (mut unlocking, store) = ready();
    let Given {
        mut attempt,
        waiting,
    } = given(&mut unlocking, 1, secs(1));
    attempt.keep(vec![0xee; 40], sealed_key(2)).unwrap();
    assert_eq!(store.sealed(), Some(sealed_key(2)));
    assert_eq!(store.boxed(), Some(vec![0xee; 40]));
    attempt.done(UnlockOutcome::Rejected);
    let (number, report) = waiting.report(secs(5)).await;
    unlocking.reported(number, report).unwrap().run();
    // The setup that ended is the one with the new box.
    assert_eq!(
        store.history(),
        [Concluded {
            finish: Finish::Done(UnlockOutcome::Rejected),
            sealed: Some(sealed_key(2)),
        }]
    );
    assert!(!store.usable());
}

#[tokio::test]
async fn keeping_a_box_has_its_limits() {
    let (mut unlocking, store) = ready();
    let Given { mut attempt, .. } = given(&mut unlocking, 1, secs(1));
    assert_eq!(
        attempt.keep(Vec::new(), sealed_key(2)),
        Err(KeepError::Invalid)
    );
    assert_eq!(
        attempt.keep(vec![1; 1025], sealed_key(2)),
        Err(KeepError::Invalid)
    );
    assert_eq!(
        attempt.keep(vec![1; 40], SealedKey::new(Vec::new())),
        Err(KeepError::Invalid)
    );
    assert_eq!(
        attempt.keep(
            vec![1; 40],
            SealedKey::new(vec![1; MAX_SEALED_KEY_BYTES + 1])
        ),
        Err(KeepError::Invalid)
    );
    assert_eq!(store.sealed(), Some(sealed_key(1)));
    // The largest box that is allowed, 1024 bytes, is kept, once.
    attempt.keep(vec![1; 1024], sealed_key(2)).unwrap();
    assert_eq!(
        attempt.keep(vec![2; 40], sealed_key(3)),
        Err(KeepError::Twice)
    );
    assert_eq!(store.sealed(), Some(sealed_key(2)));
}

#[tokio::test]
async fn a_box_that_cannot_be_written_leaves_the_old_one() {
    let (mut unlocking, store) = ready();
    let Given { mut attempt, .. } = given(&mut unlocking, 1, secs(1));
    store.make_unwritable();
    assert_eq!(
        attempt.keep(vec![0xee; 40], sealed_key(2)),
        Err(KeepError::Unwritable)
    );
    assert_eq!(store.sealed(), Some(sealed_key(1)));
    assert_eq!(store.boxed(), Some(vec![0xb1; 40]));
}

#[tokio::test]
async fn a_box_is_not_kept_over_a_setup_that_changed_or_for_an_attempt_that_is_over() {
    // The person set the unlock up again while the platform worked.
    let (mut unlocking, store) = ready();
    let Given { mut attempt, .. } = given(&mut unlocking, 1, secs(1));
    store.set_up_again(5);
    assert_eq!(
        attempt.keep(vec![0xee; 40], sealed_key(2)),
        Err(KeepError::Changed)
    );
    assert_eq!(store.sealed(), Some(sealed_key(5)));

    // The viewer gave up.
    let (mut unlocking, store) = ready();
    let Given { mut attempt, .. } = given(&mut unlocking, 1, secs(1));
    let _ = unlocking.cancelled(CancelReason::Failed, secs(2));
    assert_eq!(
        attempt.keep(vec![0xee; 40], sealed_key(2)),
        Err(KeepError::Over)
    );
    assert_eq!(store.sealed(), Some(sealed_key(1)));

    // The session ended.
    let (mut unlocking, store) = ready();
    let Given { mut attempt, .. } = given(&mut unlocking, 1, secs(1));
    let _ = unlocking.ended(secs(2));
    assert_eq!(
        attempt.keep(vec![0xee; 40], sealed_key(2)),
        Err(KeepError::Over)
    );
    assert_eq!(store.sealed(), Some(sealed_key(1)));
}

#[tokio::test]
async fn a_rejected_password_ends_the_setup_of_the_session_whatever_the_store_does() {
    for outcome in [UnlockOutcome::Rejected, UnlockOutcome::Stale] {
        let (mut unlocking, store) = ready();
        store.make_forgetful();
        let attempt = given(&mut unlocking, 1, secs(1));
        report(&mut unlocking, attempt, outcome).await;
        let _ = closed(&mut unlocking, 1, secs(2));
        assert!(store.usable(), "{outcome:?}");
        assert_eq!(
            said(&unlocking.asked(2, secs(3), true)),
            told(2, UnlockOutcome::NotSetUp),
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn a_screen_that_unlocked_while_the_rejection_was_written_gives_the_setup_back_and_is_told() {
    let (mut unlocking, store) = ready();
    let Given {
        mut attempt,
        waiting,
    } = given(&mut unlocking, 1, secs(1));
    keeps(&mut attempt);
    attempt.done(UnlockOutcome::Rejected);
    let (number, report) = waiting.report(secs(5)).await;
    let job = unlocking.reported(number, report).unwrap();
    // The system unlocks the screen while the record is on its way to the disk.
    let step = unlocking.locked(false, secs(11));
    assert!(step.say.is_none() && step.job.is_none());
    job.run();
    // The record says what the platform said.
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::Rejected)]);
    assert!(!store.usable());
    // The setup is back before the viewer is told, so that its next tap finds it. The viewer is
    // told that the screen is unlocked, which is what it is, whatever the system said at first.
    let mut step = unlocking.closed(1, secs(12));
    assert_eq!(said(&step), told(1, UnlockOutcome::Unlocked));
    assert!(step.job.is_some());
    assert!(!store.usable());
    write(&mut step);
    assert_eq!(store.restored(), [sealed_key(2)]);
    assert!(store.usable());
    let _ = unlocking.locked(true, secs(40));
    assert!(asks(&unlocking.asked(2, secs(41), true)));
}

#[tokio::test]
async fn a_screen_that_unlocks_soon_after_a_rejected_password_gives_the_setup_back() {
    let (mut unlocking, store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1));
    report(&mut unlocking, attempt, UnlockOutcome::Rejected).await;
    assert_eq!(
        closed(&mut unlocking, 1, secs(12)),
        told(1, UnlockOutcome::Rejected)
    );
    assert!(!store.usable());
    // The system took the password after all: the screen unlocks 20 seconds after the viewer was
    // told that it was rejected.
    let mut step = unlocking.locked(false, secs(32));
    assert!(step.say.is_none());
    write(&mut step);
    assert_eq!(store.restored(), [sealed_key(2)]);
    assert!(store.usable());
    // And the next session of the screen can be unlocked again.
    let _ = unlocking.locked(true, secs(40));
    assert!(asks(&unlocking.asked(2, secs(41), true)));
}

#[tokio::test]
async fn a_screen_that_unlocks_later_after_a_rejected_password_does_not() {
    let (mut unlocking, store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1));
    report(&mut unlocking, attempt, UnlockOutcome::Rejected).await;
    let _ = closed(&mut unlocking, 1, secs(12));
    let mut step = unlocking.locked(false, millis(32_001));
    write(&mut step);
    assert_eq!(store.restored(), []);
    assert!(!store.usable());
    let _ = unlocking.locked(true, secs(40));
    assert_eq!(
        said(&unlocking.asked(2, secs(41), true)),
        told(2, UnlockOutcome::NotSetUp)
    );
}

#[tokio::test]
async fn a_box_that_the_key_did_not_open_is_not_given_back_when_the_screen_unlocks() {
    let (mut unlocking, store) = ready();
    let attempt = given(&mut unlocking, 1, secs(1));
    report(&mut unlocking, attempt, UnlockOutcome::Stale).await;
    let _ = closed(&mut unlocking, 1, secs(3));
    let mut step = unlocking.locked(false, secs(4));
    write(&mut step);
    assert_eq!(store.restored(), []);
    assert!(!store.usable());
}

#[tokio::test]
async fn a_session_that_ends_writes_down_what_it_waited_for() {
    // A question that waits for the face.
    let (mut unlocking, store) = ready();
    let _ = unlocking.asked(1, secs(1), true);
    let mut step = unlocking.ended(secs(2));
    assert!(step.say.is_none());
    write(&mut step);
    assert_eq!(store.finishes(), [Finish::Ended]);

    // An attempt that the platform works on stops being live, and what the platform reports is
    // written down when it does.
    let (mut unlocking, store) = ready();
    let Given {
        mut attempt,
        waiting,
    } = given(&mut unlocking, 1, secs(1));
    keeps(&mut attempt);
    let step = unlocking.ended(secs(2));
    assert!(step.say.is_none() && step.job.is_none());
    assert!(!attempt.is_live());
    attempt.done(UnlockOutcome::Rejected);
    let (number, report) = waiting.report(secs(5)).await;
    unlocking.reported(number, report).unwrap().run();
    assert_eq!(store.finishes(), [Finish::Done(UnlockOutcome::Rejected)]);

    // Nothing goes: nothing is written.
    let (mut unlocking, store) = ready();
    let step = unlocking.ended(secs(1));
    assert!(step.say.is_none() && step.job.is_none());
    assert_eq!(store.history(), []);
}

// The rules of the viewer, with a clock that the test sets.

/// What the platform of a viewer was asked.
#[derive(Debug, Clone, PartialEq)]
enum Seen {
    /// The question of the source: the sealed key, and the source it came from.
    Open {
        sealed: SealedKey,
        source: DeviceId,
    },
    Cancel,
    Settled(Settled),
    Ended,
}

#[derive(Default)]
struct Faces {
    seen: Vec<Seen>,
    /// The questions that the face has not answered, oldest first.
    held: VecDeque<Question>,
    /// Drops the question at once, as a platform does that finds it cannot ask.
    refuse: bool,
}

/// The platform of a viewer that asks for the face of its person, and the person.
#[derive(Clone, Default)]
struct Face(Arc<Mutex<Faces>>);

impl Face {
    fn refusing() -> Self {
        let face = Self::default();
        lock(&face.0).refuse = true;
        face
    }

    fn seen(&self) -> Vec<Seen> {
        lock(&self.0).seen.clone()
    }

    /// The oldest question that the face has not answered.
    fn question(&self) -> Question {
        lock(&self.0)
            .held
            .pop_front()
            .expect("the platform was asked no question")
    }

    /// Waits until the platform has been asked this many things.
    async fn until_seen(&self, count: usize) {
        until("the platform to be asked", || self.seen().len() >= count).await;
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
        self.note(Seen::Open {
            sealed: question.sealed().clone(),
            source: question.source(),
        });
        let mut faces = lock(&(self.0).0);
        if !faces.refuse {
            faces.held.push_back(question);
        }
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

/// The rules of a viewer, the answers that its platform gives, and the platform.
fn fresh(patience: Patience) -> (Asking, mpsc::UnboundedReceiver<Answer>, Face) {
    let face = Face::default();
    let (asking, answers) = Asking::new(Box::new(Looking(face.clone())), patience);
    (asking, answers, face)
}

fn answer_of(answers: &mut mpsc::UnboundedReceiver<Answer>) -> Answer {
    answers.try_recv().expect("the platform gave no answer")
}

fn finished(number: u64, outcome: UnlockOutcome) -> Done {
    Done { number, outcome }
}

#[test]
fn a_viewer_that_is_not_told_otherwise_waits_ten_seconds_for_the_question_and_thirty_for_the_end() {
    let (mut asking, mut answers, face) = fresh(Patience::default());
    assert_eq!(asking.ask(secs(100)), Some(unlock_message(1)));
    assert_eq!(asking.tick(millis(109_999)), None);
    assert_eq!(
        asking.tick(secs(110)),
        Some(cancelled(CancelReason::Failed))
    );
    assert_eq!(
        face.seen().last(),
        Some(&Seen::Settled(Settled::NoQuestion))
    );

    // The key is sent, and the end does not come.
    assert_eq!(asking.ask(secs(200)), Some(unlock_message(2)));
    asking.sealed(question_for(2, 3), MAC);
    face.question().opened(box_key(1));
    assert!(
        asking
            .answered(answer_of(&mut answers), secs(201))
            .is_some()
    );
    assert_eq!(asking.tick(millis(230_999)), None);
    assert!(!face.seen().contains(&Seen::Settled(Settled::NoResult)));
    assert_eq!(asking.tick(secs(231)), None);
    assert_eq!(face.seen().last(), Some(&Seen::Settled(Settled::NoResult)));
}

#[test]
fn a_source_waits_a_minute_for_the_face_and_gives_the_platform_less_time_than_the_viewer_waits() {
    let params = SourceParams::default();
    assert_eq!(params.unlock_wait, secs(60));
    assert_eq!(params.unlock_work, secs(25));
    assert_eq!(params.unlock_grace, secs(20));
    assert!(params.unlock_work < Patience::default().end);
}

#[test]
fn a_viewer_numbers_its_attempts_from_one_and_starts_none_while_one_goes() {
    let (mut asking, _answers, _face) = fresh(Patience::default());
    assert_eq!(asking.ask(secs(1)), Some(unlock_message(1)));
    assert_eq!(asking.ask(secs(2)), None);
    asking.done(finished(1, UnlockOutcome::NotLocked));
    assert_eq!(asking.ask(secs(3)), Some(unlock_message(2)));
}

#[test]
fn a_request_that_could_not_be_queued_frees_the_viewer_and_spends_the_number() {
    let (mut asking, _answers, _face) = fresh(Patience::default());
    assert_eq!(asking.ask(secs(1)), Some(unlock_message(1)));
    asking.unasked();
    assert_eq!(asking.ask(secs(2)), Some(unlock_message(2)));
}

#[test]
fn a_question_is_taken_only_for_the_attempt_that_waits_for_it() {
    let (mut asking, _answers, face) = fresh(Patience::default());
    // None was asked.
    asking.sealed(question_for(1, 1), MAC);
    assert_eq!(face.seen(), []);
    let _ = asking.ask(secs(1));
    // The question of another attempt.
    asking.sealed(question_for(2, 2), MAC);
    assert_eq!(face.seen(), []);
    // The question of this one comes to the platform, with the source it came from.
    asking.sealed(question_for(1, 3), MAC);
    assert_eq!(
        face.seen(),
        [Seen::Open {
            sealed: sealed_key(3),
            source: MAC
        }]
    );
    // And a second question for it is dropped.
    asking.sealed(question_for(1, 4), MAC);
    assert_eq!(face.seen().len(), 1);
}

#[test]
fn the_answer_of_the_platform_goes_to_the_source() {
    let (mut asking, mut answers, face) = fresh(Patience::default());
    let _ = asking.ask(secs(1));
    asking.sealed(question_for(1, 3), MAC);
    face.question().opened(box_key(0x42));
    let message = asking.answered(answer_of(&mut answers), secs(2));
    assert_eq!(
        message,
        Some(ViewerMessage::UnlockOpened(Opened {
            challenge: challenge(3),
            key: box_key(0x42),
        }))
    );
    // The viewer waits for the end now, and starts nothing.
    assert_eq!(asking.ask(secs(3)), None);
}

#[test]
fn a_question_that_the_person_declines_tells_the_source_why_and_frees_the_viewer() {
    for reason in [
        CancelReason::Declined,
        CancelReason::KeyLost,
        CancelReason::Failed,
        CancelReason::Other,
    ] {
        let (mut asking, mut answers, face) = fresh(Patience::default());
        let _ = asking.ask(secs(1));
        asking.sealed(question_for(1, 3), MAC);
        face.question().declined(reason);
        let message = asking.answered(answer_of(&mut answers), secs(2));
        assert_eq!(message, Some(cancelled(reason)), "{reason:?}");
        // The platform ended the attempt itself, and is not told how it ended.
        assert_eq!(face.seen().len(), 1, "{reason:?}");
        assert_eq!(asking.ask(secs(3)), Some(unlock_message(2)), "{reason:?}");
    }
}

#[test]
fn a_question_that_is_dropped_is_declined_with_failed() {
    let (mut asking, mut answers, face) = fresh(Patience::default());
    let _ = asking.ask(secs(1));
    asking.sealed(question_for(1, 3), MAC);
    drop(face.question());
    let message = asking.answered(answer_of(&mut answers), secs(2));
    assert_eq!(message, Some(cancelled(CancelReason::Failed)));
}

#[test]
fn an_answer_to_a_question_that_is_not_open_any_more_is_dropped() {
    let (mut asking, mut answers, face) = fresh(Patience::default());
    let _ = asking.ask(secs(1));
    asking.sealed(question_for(1, 3), MAC);
    let old = face.question();
    // The source gives up on the attempt, and the viewer starts another.
    asking.done(finished(1, UnlockOutcome::TimedOut));
    let _ = asking.ask(secs(2));
    asking.sealed(question_for(2, 4), MAC);
    // The old question is answered at last.
    old.opened(box_key(1));
    assert_eq!(asking.answered(answer_of(&mut answers), secs(3)), None);
    // The new one is still open.
    face.question().opened(box_key(2));
    assert!(asking.answered(answer_of(&mut answers), secs(4)).is_some());
    // So is the answer to an earlier question when the source repeats its challenge.
    let (mut asking, mut answers, face) = fresh(Patience::default());
    let _ = asking.ask(secs(1));
    asking.sealed(question_for(1, 3), MAC);
    let old = face.question();
    asking.done(finished(1, UnlockOutcome::TimedOut));
    let _ = asking.ask(secs(2));
    asking.sealed(question_for(2, 3), MAC);
    old.opened(box_key(1));
    assert_eq!(asking.answered(answer_of(&mut answers), secs(3)), None);
    // An answer when none is open is dropped too.
    let (mut idle, mut answers, face) = asking_with_question();
    idle.ended();
    face.question().opened(box_key(3));
    assert_eq!(idle.answered(answer_of(&mut answers), secs(5)), None);
}

/// A viewer whose question is open.
fn asking_with_question() -> (Asking, mpsc::UnboundedReceiver<Answer>, Face) {
    let (mut asking, answers, face) = fresh(Patience::default());
    let _ = asking.ask(secs(1));
    asking.sealed(question_for(1, 3), MAC);
    (asking, answers, face)
}

#[test]
fn what_the_source_says_of_the_end_reaches_the_platform_once() {
    // The key was sent, and the source says what came of it.
    let (mut asking, mut answers, face) = asking_with_question();
    face.question().opened(box_key(1));
    let _ = asking.answered(answer_of(&mut answers), secs(2));
    asking.done(finished(1, UnlockOutcome::Unlocked));
    assert_eq!(
        face.seen().last(),
        Some(&Seen::Settled(Settled::Said(UnlockOutcome::Unlocked)))
    );
    // Said again, it is dropped.
    asking.done(finished(1, UnlockOutcome::Unlocked));
    assert_eq!(face.seen().len(), 2);

    // The source refuses at once: the viewer waited for the question.
    let (mut asking, _answers, face) = fresh(Patience::default());
    let _ = asking.ask(secs(1));
    asking.done(finished(1, UnlockOutcome::NotLocked));
    assert_eq!(
        face.seen(),
        [Seen::Settled(Settled::Said(UnlockOutcome::NotLocked))]
    );
    assert_eq!(asking.ask(secs(2)), Some(unlock_message(2)));
}

#[test]
fn an_end_for_another_attempt_or_for_none_is_dropped() {
    let (mut asking, _answers, face) = fresh(Patience::default());
    asking.done(finished(1, UnlockOutcome::Unlocked));
    let _ = asking.ask(secs(1));
    asking.done(finished(2, UnlockOutcome::Unlocked));
    assert_eq!(face.seen(), []);
    // The attempt goes on: another cannot start.
    assert_eq!(asking.ask(secs(2)), None);
}

#[test]
fn an_end_while_the_question_is_open_closes_the_prompt() {
    let (mut asking, mut answers, face) = asking_with_question();
    asking.done(finished(1, UnlockOutcome::TimedOut));
    assert_eq!(
        face.seen()[1..],
        [
            Seen::Cancel,
            Seen::Settled(Settled::Said(UnlockOutcome::TimedOut))
        ]
    );
    // The person answers all the same: it is dropped.
    face.question().opened(box_key(1));
    assert_eq!(asking.answered(answer_of(&mut answers), secs(2)), None);
}

#[test]
fn a_viewer_gives_up_on_a_question_that_does_not_come_and_on_an_end_that_does_not_come() {
    let patience = Patience {
        question: secs(10),
        end: secs(30),
    };
    // No question in ten seconds.
    let (mut asking, _answers, face) = fresh(patience);
    let _ = asking.ask(secs(5));
    assert_eq!(asking.tick(millis(14_900)), None);
    assert_eq!(asking.tick(secs(15)), Some(cancelled(CancelReason::Failed)));
    assert_eq!(face.seen(), [Seen::Settled(Settled::NoQuestion)]);
    // A question that comes late is dropped, and the viewer can start another attempt.
    asking.sealed(question_for(1, 1), MAC);
    assert_eq!(face.seen().len(), 1);
    assert_eq!(asking.ask(secs(16)), Some(unlock_message(2)));

    // No end in thirty seconds from the key.
    let (mut asking, mut answers, face) = asking_with_question();
    face.question().opened(box_key(1));
    let _ = asking.answered(answer_of(&mut answers), secs(5));
    assert_eq!(asking.tick(millis(34_900)), None);
    assert_eq!(asking.tick(secs(35)), None);
    assert_eq!(face.seen()[1..], [Seen::Settled(Settled::NoResult)]);
    // An end that comes late is dropped.
    asking.done(finished(1, UnlockOutcome::Unlocked));
    assert_eq!(face.seen().len(), 2);
    assert_eq!(asking.ask(secs(36)), Some(unlock_message(2)));
}

#[test]
fn a_question_that_the_person_has_open_is_not_timed() {
    let (mut asking, _answers, face) = asking_with_question();
    // The person takes their time. The source decides when it has waited long enough.
    assert_eq!(asking.tick(secs(3_600)), None);
    assert_eq!(face.seen().len(), 1);
}

#[test]
fn the_end_of_a_session_closes_the_prompt_and_tells_the_platform_once() {
    let (mut asking, _answers, face) = asking_with_question();
    asking.ended();
    assert_eq!(face.seen()[1..], [Seen::Cancel, Seen::Ended]);

    let (mut asking, _answers, face) = fresh(Patience::default());
    let _ = asking.ask(secs(1));
    asking.ended();
    assert_eq!(face.seen(), [Seen::Ended]);
}

// A real source, in front of a viewer that the test writes.

/// The messages of a control stream, read on a task of its own, so that waiting to see that nothing
/// comes does not cut a message in two.
struct Wire<M> {
    messages: mpsc::UnboundedReceiver<M>,
}

impl<M: Message + fmt::Debug + Send + 'static> Wire<M> {
    /// Reads `recv`, and passes on the messages for which `keep` is true.
    fn read(mut recv: RecvStream, keep: fn(&M) -> bool) -> Self {
        let (sender, messages) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok(Some(message)) = read_next::<M>(&mut recv, PATIENCE).await {
                if keep(&message) && sender.send(message).is_err() {
                    return;
                }
            }
        });
        Self { messages }
    }

    /// The next message.
    async fn next(&mut self) -> M {
        tokio::time::timeout(PATIENCE, self.messages.recv())
            .await
            .expect("no message came")
            .expect("the control stream ended")
    }

    /// Waits to see that nothing comes.
    async fn nothing(&mut self) {
        if let Ok(message) = tokio::time::timeout(QUIET, self.messages.recv()).await {
            panic!("a message came: {message:?}");
        }
    }
}

/// What the viewer says, without its reports.
fn not_a_report(message: &ViewerMessage) -> bool {
    !matches!(message, ViewerMessage::Report(_))
}

/// What the platform of a source does about an unlock: it vouches for the privacy when the test
/// says so, and gives the test the attempts it is given.
struct Typing {
    in_place: Arc<AtomicBool>,
    attempts: mpsc::UnboundedSender<UnlockAttempt>,
}

impl SourceEvents for Typing {
    fn report(&mut self, _: Report) {}

    fn want(&mut self, _: Want) {}

    fn input(&mut self, _: InputBatch, _: Duration) {}

    fn held(&mut self, _: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn private_in_place(&mut self) -> bool {
        self.in_place.load(Ordering::SeqCst)
    }

    fn unlock(&mut self, attempt: UnlockAttempt) {
        let _ = self.attempts.send(attempt);
    }

    fn ended(&mut self) {}
}

/// A platform that has what the trait gives it and nothing else, which is no privacy to vouch for
/// and no password to type.
struct Reticent;

impl SourceEvents for Reticent {
    fn report(&mut self, _: Report) {}

    fn want(&mut self, _: Want) {}

    fn input(&mut self, _: InputBatch, _: Duration) {}

    fn held(&mut self, _: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn ended(&mut self) {}
}

/// A platform that vouches for the privacy, and has the rest from the trait: it cannot type.
struct Vouching;

impl SourceEvents for Vouching {
    fn report(&mut self, _: Report) {}

    fn want(&mut self, _: Want) {}

    fn input(&mut self, _: InputBatch, _: Duration) {}

    fn held(&mut self, _: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn private_in_place(&mut self) -> bool {
        true
    }

    fn ended(&mut self) {}
}

/// The picture format of the tests.
fn format() -> Format {
    Format {
        epoch: 0,
        codec: Codec::H264,
        width: 1440,
        height: 900,
        fps: 60,
    }
}

/// What a viewer that can take a great deal says.
fn takes() -> Takes {
    Takes {
        max_width: 3840,
        max_height: 2160,
        max_fps: 120,
        codecs: vec![Codec::H264],
    }
}

fn quick_source() -> SourceParams {
    SourceParams {
        tick_every: millis(20),
        ..SourceParams::default()
    }
}

/// The setup of a session that controls a window and keeps the screen private, with `unlock`.
fn source_setup(params: SourceParams, private: bool, unlock: UnlockSetup) -> SourceSetup {
    SourceSetup {
        config: format(),
        control: true,
        private,
        takes: takes(),
        sound: SoundPlan::None,
        unlock,
        leave: false,
        params,
    }
}

/// A source session whose device keeps the unlock for the viewer, and the viewer's device in front
/// of it.
struct SourceRig {
    source: Device,
    viewer: Device,
    accepted: Connection,
    control: mpsc::UnboundedReceiver<Arrival>,
    handle: SourceHandle,
    store: MemoryStore,
    /// What the platform says of the cover, the hold and the guard.
    in_place: Arc<AtomicBool>,
    attempts: mpsc::UnboundedReceiver<UnlockAttempt>,
}

impl SourceRig {
    /// What the session is set up with, given the store that is made for it.
    async fn with(params: SourceParams, unlock: impl FnOnce(&MemoryStore) -> UnlockSetup) -> Self {
        Self::on(params, unlock, |in_place, attempts| {
            Box::new(Typing { in_place, attempts })
        })
        .await
    }

    /// A source session that takes an unlock, on the platform that `platform` makes of the
    /// switch for the privacy and the way to the attempts that the rig reads.
    async fn on(
        params: SourceParams,
        unlock: impl FnOnce(&MemoryStore) -> UnlockSetup,
        platform: impl FnOnce(
            Arc<AtomicBool>,
            mpsc::UnboundedSender<UnlockAttempt>,
        ) -> Box<dyn SourceEvents>,
    ) -> Self {
        let streaming = Streaming::new();
        let sinks = Sinks::default().install(Kind::Control, 1, streaming.clone());
        let source = Device::new(sinks, PATIENCE).await;
        let (viewer, mut receivers) = Device::taking(&[(Kind::Control, 1)]).await;
        let control = receivers.remove(0);
        let (dialed, accepted) = connect(&source, &viewer).await;
        let paired = source.trust.get(&viewer.id).unwrap();
        let store = MemoryStore::with(record_between(source.id, viewer.id, 1));
        let (sender, attempts) = mpsc::unbounded_channel();
        let in_place = Arc::new(AtomicBool::new(true));
        // The session is given the platform in a box, as a node gives it.
        let platform = platform(Arc::clone(&in_place), sender);
        let setup = source_setup(params, true, unlock(&store));
        let handle = streaming
            .source(&paired, dialed, None, session_id(), setup, platform)
            .unwrap();
        Self {
            source,
            viewer,
            accepted,
            control,
            handle,
            store,
            in_place,
            attempts,
        }
    }

    /// A source session that takes an unlock.
    async fn offering(params: SourceParams) -> Self {
        Self::with(params, |store| {
            UnlockSetup::Offered(Arc::new(store.clone()))
        })
        .await
    }

    /// The rig, the viewer's control stream, and the source's after the picture format.
    async fn connected(params: SourceParams) -> (Self, SendStream, Wire<SourceMessage>) {
        let mut rig = Self::offering(params).await;
        let (viewer, source) = rig.streams().await;
        (rig, viewer, source)
    }

    /// The viewer's control stream, and the source's after the picture format.
    async fn streams(&mut self) -> (SendStream, Wire<SourceMessage>) {
        let arrival = next(&mut self.control).await;
        assert!(matches!(arrival.kind, Kind::Control));
        let mut recv = arrival.recv;
        let first = read_next::<SourceMessage>(&mut recv, PATIENCE)
            .await
            .expect("the source wrote a message that cannot be read");
        assert!(matches!(first, Some(SourceMessage::Config(_))));
        let source = Wire::read(recv, |_| true);
        let mut send = self.accepted.open_uni().await.unwrap();
        send.write_all(&encode_prefix(KIND_CONTROL, session_id()))
            .await
            .unwrap();
        (send, source)
    }

    /// The attempt that the platform is given.
    async fn attempt(&mut self) -> UnlockAttempt {
        tokio::time::timeout(PATIENCE, self.attempts.recv())
            .await
            .expect("the platform was given no attempt")
            .expect("the platform was dropped")
    }

    /// Waits to see that the platform is given no attempt.
    async fn no_attempt(&mut self) {
        let given = tokio::time::timeout(QUIET, self.attempts.recv()).await;
        assert!(given.is_err(), "the platform was given an attempt");
    }

    async fn ended(&self) -> End {
        tokio::time::timeout(PATIENCE, self.handle.ended())
            .await
            .expect("the session did not end")
    }

    /// Waits until the history has this many entries.
    async fn until_written(&self, count: usize) {
        until("the history to be written", || {
            self.store.history().len() >= count
        })
        .await;
    }
}

/// The viewer says `message`.
async fn say(viewer: &mut SendStream, message: ViewerMessage) {
    write_message(viewer, &message).await.unwrap();
}

/// Asks, and gives the question that comes.
async fn ask(viewer: &mut SendStream, source: &mut Wire<SourceMessage>, number: u64) -> Sealed {
    say(viewer, unlock_message(number)).await;
    match source.next().await {
        SourceMessage::UnlockSealed(sealed) => sealed,
        other => panic!("the source did not ask the question: {other:?}"),
    }
}

/// What the source says to a viewer that is refused or told how it went.
fn done_message(number: u64, outcome: UnlockOutcome) -> SourceMessage {
    SourceMessage::UnlockDone(Done { number, outcome })
}

#[tokio::test]
async fn a_viewer_whose_face_opens_the_key_gets_the_screen_unlocked_and_is_told_after_the_record() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    rig.handle.pause(PauseReason::Locked);
    assert_eq!(
        source.next().await,
        SourceMessage::Pause(Pause {
            reason: PauseReason::Locked
        })
    );
    let question = ask(&mut viewer, &mut source, 1).await;
    assert_eq!(question.number, 1);
    assert_eq!(question.sealed, sealed_key(1));

    // An answer to another question is dropped.
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(Opened {
            challenge: challenge(0),
            key: box_key(0x11),
        }),
    )
    .await;
    rig.no_attempt().await;

    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(0x11))),
    )
    .await;
    let mut attempt = rig.attempt().await;
    assert_eq!(attempt.key(), &box_key(0x11));
    assert_eq!(attempt.boxed(), &[0xb1; 40][..]);
    assert_eq!(attempt.target(), rig.source.id);
    assert_eq!(attempt.viewer(), rig.viewer.id);
    // The platform puts the password in a new box, and types it.
    attempt.keep(vec![0xee; 40], sealed_key(2)).unwrap();
    attempt.done(UnlockOutcome::Unlocked);
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::Unlocked)
    );
    // What the viewer is told is on disk already.
    assert_eq!(
        rig.store.finishes(),
        [Finish::Done(UnlockOutcome::Unlocked)]
    );
    assert_eq!(rig.store.sealed(), Some(sealed_key(2)));
    assert_eq!(rig.store.boxed(), Some(vec![0xee; 40]));
}

#[tokio::test]
async fn a_refusal_is_answered_at_once_with_the_number_that_came() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    // The screen is not locked.
    say(&mut viewer, unlock_message(5)).await;
    assert_eq!(
        source.next().await,
        done_message(5, UnlockOutcome::NotLocked)
    );

    // The screen is locked, and the platform cannot vouch for the privacy.
    rig.in_place.store(false, Ordering::SeqCst);
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    say(&mut viewer, unlock_message(6)).await;
    assert_eq!(
        source.next().await,
        done_message(6, UnlockOutcome::NotPrivate)
    );

    // The person turned the unlock off at the device.
    rig.in_place.store(true, Ordering::SeqCst);
    rig.store.turn_off();
    say(&mut viewer, unlock_message(7)).await;
    assert_eq!(
        source.next().await,
        done_message(7, UnlockOutcome::NotSetUp)
    );

    // And set it up again: the question comes, and a second request while it waits is busy.
    rig.store.set_up_again(2);
    let question = ask(&mut viewer, &mut source, 8).await;
    assert_eq!((question.number, question.sealed), (8, sealed_key(2)));
    say(&mut viewer, unlock_message(9)).await;
    assert_eq!(source.next().await, done_message(9, UnlockOutcome::Busy));
    rig.no_attempt().await;
}

#[tokio::test]
async fn a_platform_that_does_not_answer_for_the_privacy_is_not_private() {
    let mut rig = SourceRig::on(
        quick_source(),
        |store| UnlockSetup::Offered(Arc::new(store.clone())),
        |_, _| Box::new(Reticent),
    )
    .await;
    let (mut viewer, mut source) = rig.streams().await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    say(&mut viewer, unlock_message(1)).await;
    // No question comes for the face: the answer is a refusal, and the setup is as it was.
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::NotPrivate)
    );
    assert_eq!(rig.store.sealed(), Some(sealed_key(1)));
    assert!(rig.store.usable());
}

#[tokio::test]
async fn a_platform_that_cannot_type_lets_go_of_the_attempt_and_the_viewer_hears_failed() {
    let mut rig = SourceRig::on(
        quick_source(),
        |store| UnlockSetup::Offered(Arc::new(store.clone())),
        |_, _| Box::new(Vouching),
    )
    .await;
    let (mut viewer, mut source) = rig.streams().await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    assert_eq!(source.next().await, done_message(1, UnlockOutcome::Failed));
    assert_eq!(rig.store.finishes(), [Finish::Done(UnlockOutcome::Failed)]);
    // Nothing was kept, nothing ended the setup, and the viewer can ask again.
    assert_eq!(rig.store.sealed(), Some(sealed_key(1)));
    assert!(rig.store.usable());
    let again = ask(&mut viewer, &mut source, 2).await;
    assert_eq!(again.number, 2);
}

#[tokio::test]
async fn a_session_whose_offer_did_not_say_unlock_drops_the_messages_of_one() {
    let mut rig = SourceRig::with(quick_source(), |_| UnlockSetup::NotOffered).await;
    let (mut viewer, mut source) = rig.streams().await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    for message in [
        unlock_message(1),
        ViewerMessage::UnlockOpened(Opened {
            challenge: challenge(1),
            key: box_key(1),
        }),
        cancelled(CancelReason::Declined),
    ] {
        say(&mut viewer, message).await;
    }
    source.nothing().await;
    rig.no_attempt().await;
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );
}

#[tokio::test]
async fn an_unlock_is_offered_only_in_a_private_session() {
    let streaming = Streaming::new();
    let sinks = Sinks::default().install(Kind::Control, 1, streaming.clone());
    let source = Device::new(sinks, PATIENCE).await;
    let (viewer, _receivers) = Device::taking(&[(Kind::Control, 1)]).await;
    let (dialed, _accepted) = connect(&source, &viewer).await;
    let paired = source.trust.get(&viewer.id).unwrap();
    let store = MemoryStore::with(record_between(source.id, viewer.id, 1));
    let (sender, _attempts) = mpsc::unbounded_channel();
    let platform = Typing {
        in_place: Arc::new(AtomicBool::new(true)),
        attempts: sender,
    };
    let setup = source_setup(quick_source(), false, UnlockSetup::Offered(Arc::new(store)));
    let started = streaming.source(&paired, dialed, None, session_id(), setup, platform);
    assert!(matches!(started, Err(Error::Config(_))));
}

#[tokio::test]
async fn a_viewer_that_gives_up_while_the_password_is_typed_stops_the_attempt() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    let mut attempt = rig.attempt().await;
    assert!(attempt.is_live());
    say(&mut viewer, cancelled(CancelReason::Failed)).await;
    until("the attempt to stop being live", || !attempt.is_live()).await;
    assert_eq!(
        attempt.keep(vec![0xee; 40], sealed_key(2)),
        Err(KeepError::Over)
    );
    // The platform says what it knows. It is written down, and the viewer, who gave up, is not told.
    attempt.done(UnlockOutcome::Failed);
    rig.until_written(1).await;
    assert_eq!(rig.store.finishes(), [Finish::Done(UnlockOutcome::Failed)]);
    source.nothing().await;
    // And the viewer can try again.
    let again = ask(&mut viewer, &mut source, 2).await;
    assert_eq!(again.number, 2);
}

#[tokio::test]
async fn a_question_that_nobody_answers_ends_with_timed_out() {
    let params = SourceParams {
        unlock_wait: millis(100),
        ..quick_source()
    };
    let (mut rig, mut viewer, mut source) = SourceRig::connected(params).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::TimedOut)
    );
    rig.until_written(1).await;
    assert_eq!(
        rig.store.finishes(),
        [Finish::Done(UnlockOutcome::TimedOut)]
    );
    // The answer that comes late is dropped.
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    rig.no_attempt().await;
    // And the next attempt is not busy.
    let again = ask(&mut viewer, &mut source, 2).await;
    assert_ne!(again.challenge, question.challenge);
}

#[tokio::test]
async fn a_platform_that_takes_too_long_has_failed() {
    let params = SourceParams {
        unlock_work: millis(100),
        ..quick_source()
    };
    let (mut rig, mut viewer, mut source) = SourceRig::connected(params).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    let mut attempt = rig.attempt().await;
    // The platform says nothing.
    assert_eq!(source.next().await, done_message(1, UnlockOutcome::Failed));
    assert!(!attempt.is_live());
    assert_eq!(
        attempt.keep(vec![0xee; 40], sealed_key(2)),
        Err(KeepError::Over)
    );
    assert_eq!(rig.store.finishes(), [Finish::Done(UnlockOutcome::Failed)]);
    // What it says later is heard by no one.
    attempt.done(UnlockOutcome::Unlocked);
    source.nothing().await;
    assert_eq!(rig.store.finishes().len(), 1);
}

#[tokio::test]
async fn a_session_that_ends_while_the_face_is_asked_for_writes_that_down() {
    let (rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let _ = ask(&mut viewer, &mut source, 1).await;
    rig.handle.end();
    assert_eq!(rig.ended().await, End::Local);
    rig.until_written(1).await;
    assert_eq!(rig.store.finishes(), [Finish::Ended]);
}

#[tokio::test]
async fn a_session_that_ends_while_the_password_is_typed_stops_the_attempt_and_keeps_its_report() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    let mut attempt = rig.attempt().await;
    keeps(&mut attempt);
    rig.handle.end();
    assert_eq!(rig.ended().await, End::Local);
    until("the attempt to stop being live", || !attempt.is_live()).await;
    // The platform typed before it saw that the session was over, and the password was rejected.
    attempt.done(UnlockOutcome::Rejected);
    rig.until_written(1).await;
    assert_eq!(
        rig.store.finishes(),
        [Finish::Done(UnlockOutcome::Rejected)]
    );
    assert!(!rig.store.usable());
}

#[tokio::test]
async fn a_switch_that_the_viewer_turns_on_while_the_face_is_asked_for_ends_the_attempt() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let _ = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::Lift(Lift {
            screen: true,
            input: false,
        }),
    )
    .await;
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::NotPrivate)
    );
    // Put back, the viewer can ask again.
    say(&mut viewer, ViewerMessage::Lift(Lift::default())).await;
    let again = ask(&mut viewer, &mut source, 2).await;
    assert_eq!(again.number, 2);
    rig.no_attempt().await;
}

#[tokio::test]
async fn a_screen_that_is_unlocked_by_other_means_ends_the_question() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let _ = ask(&mut viewer, &mut source, 1).await;
    rig.handle.resume();
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::NotLocked)
    );
    assert_eq!(source.next().await, SourceMessage::Resume);
    rig.no_attempt().await;
}

#[tokio::test]
async fn a_pause_for_another_reason_than_the_lock_leaves_the_screen_unlocked() {
    let (rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    for (number, reason) in [(1, PauseReason::Hidden), (2, PauseReason::Other)] {
        rig.handle.pause(reason);
        assert_eq!(source.next().await, SourceMessage::Pause(Pause { reason }));
        say(&mut viewer, unlock_message(number)).await;
        assert_eq!(
            source.next().await,
            done_message(number, UnlockOutcome::NotLocked),
            "{reason:?}"
        );
        rig.handle.resume();
        assert_eq!(source.next().await, SourceMessage::Resume);
    }
}

#[tokio::test]
async fn a_screen_that_unlocks_after_the_session_has_ended_gives_nothing_back() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    let mut attempt = rig.attempt().await;
    keeps(&mut attempt);
    attempt.done(UnlockOutcome::Rejected);
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::Rejected)
    );
    assert!(!rig.store.usable());
    rig.handle.end();
    assert_eq!(rig.ended().await, End::Local);
    // The platform tells a session that is over that the screen is unlocked, within the grace.
    rig.handle.resume();
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.store.restored(), []);
    assert!(!rig.store.usable());
}

#[tokio::test]
async fn the_viewer_is_told_only_when_the_record_of_what_came_of_it_is_written() {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    let disk = rig.store.make_slow();
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    let mut attempt = rig.attempt().await;
    keeps(&mut attempt);
    attempt.done(UnlockOutcome::Unlocked);
    until("the record to come to the disk", || disk.arrived() == 1).await;
    // The disk is slow, and the viewer waits.
    source.nothing().await;
    assert_eq!(rig.store.finishes(), []);
    disk.allow(1);
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::Unlocked)
    );
    assert_eq!(
        rig.store.finishes(),
        [Finish::Done(UnlockOutcome::Unlocked)]
    );
}

#[tokio::test]
async fn a_screen_that_unlocks_while_the_rejection_is_written_gives_the_setup_back_before_the_viewer_is_told()
 {
    let (mut rig, mut viewer, mut source) = SourceRig::connected(quick_source()).await;
    let disk = rig.store.make_slow();
    rig.handle.pause(PauseReason::Locked);
    assert!(matches!(source.next().await, SourceMessage::Pause(_)));
    let question = ask(&mut viewer, &mut source, 1).await;
    say(
        &mut viewer,
        ViewerMessage::UnlockOpened(answer_with(&question, box_key(1))),
    )
    .await;
    let mut attempt = rig.attempt().await;
    keeps(&mut attempt);
    attempt.done(UnlockOutcome::Rejected);
    until("the record to come to the disk", || disk.arrived() == 1).await;
    // The system unlocks the screen while the rejection is on its way to the disk.
    rig.handle.resume();
    assert_eq!(source.next().await, SourceMessage::Resume);
    disk.allow(1);
    // The rejection is written, and the setup is given back, which the disk takes its time over.
    until("the setup to come to the disk", || disk.arrived() == 2).await;
    source.nothing().await;
    assert_eq!(rig.store.restored(), []);
    disk.allow(1);
    // The screen is unlocked, so that is what the viewer is told, and what the record says is what
    // the platform said.
    assert_eq!(
        source.next().await,
        done_message(1, UnlockOutcome::Unlocked)
    );
    assert_eq!(
        rig.store.finishes(),
        [Finish::Done(UnlockOutcome::Rejected)]
    );
    assert_eq!(rig.store.restored(), [sealed_key(2)]);
    assert!(rig.store.usable());
}

// A real viewer, in front of a source that the test writes.

/// A decoder that takes everything and does nothing.
struct Mute;

impl Decoder for Mute {
    fn decode(&mut self, _frame: &FrameHeader, _picture: Vec<u8>) -> Result<(), DecodeError> {
        Ok(())
    }

    fn show(&mut self, _number: u64) {}

    fn source(&mut self, _message: SourceMessage) {}

    fn state(&self) -> DecoderState {
        DecoderState {
            decode_us: 1000,
            thermal: Thermal::Nominal,
        }
    }

    fn ended(&mut self) {}
}

/// A viewer session, and the source's device in front of it.
struct ViewRig {
    _viewer: Device,
    source: Device,
    dialed: Connection,
    control: mpsc::UnboundedReceiver<Arrival>,
    handle: ViewHandle,
    face: Face,
}

impl ViewRig {
    async fn new() -> Self {
        Self::with(quick(), true, Some(Face::default())).await
    }

    /// `unlock` is whether the offer said that the viewer may unlock, `face` the platform that the
    /// application gave the session, if it gave one.
    async fn with(params: ViewParams, unlock: bool, face: Option<Face>) -> Self {
        let streaming = Streaming::new();
        let sinks = Sinks::default()
            .install(Kind::Video, 12, streaming.clone())
            .install(Kind::Control, 2, streaming.clone());
        let viewer = Device::new(sinks, PATIENCE).await;
        let (source, mut receivers) = Device::taking(&[(Kind::Control, 1)]).await;
        let control = receivers.remove(0);
        let (dialed, accepted) = connect(&source, &viewer).await;
        let paired = viewer.trust.get(&source.id).unwrap();
        let mut viewing = Viewing::raw(Mute, takes(), params);
        let face = match face {
            Some(face) => {
                viewing = viewing.with_unlocker(Looking(face.clone()));
                face
            }
            None => Face::default(),
        };
        let granted = Granted {
            control: true,
            private: true,
            sound: false,
            unlock,
            leave: false,
        };
        let handle = streaming
            .view_granted(&paired, accepted, session_id(), viewing, granted)
            .unwrap();
        Self {
            _viewer: viewer,
            source,
            dialed,
            control,
            handle,
            face,
        }
    }

    /// Opens the source's control stream, as a source does once the viewer has accepted.
    async fn open_source_control(&self) -> SendStream {
        let mut send = self.dialed.open_uni().await.unwrap();
        send.write_all(&encode_prefix(KIND_CONTROL, session_id()))
            .await
            .unwrap();
        send
    }

    /// The source's control stream, and the viewer's as the source reads it.
    async fn streams(&mut self) -> (SendStream, Wire<ViewerMessage>) {
        let send = self.open_source_control().await;
        let arrival = next(&mut self.control).await;
        assert!(matches!(arrival.kind, Kind::Control));
        (send, Wire::read(arrival.recv, not_a_report))
    }

    async fn ended(&self) -> End {
        tokio::time::timeout(PATIENCE, self.handle.ended())
            .await
            .expect("the session did not end")
    }
}

/// The source says `message`.
async fn say_source(send: &mut SendStream, message: SourceMessage) {
    write_message(send, &message).await.unwrap();
}

/// Params with short patience for the question and the end of an unlock.
fn impatient() -> ViewParams {
    ViewParams {
        unlock: Patience {
            question: millis(150),
            end: millis(300),
        },
        ..quick()
    }
}

#[tokio::test]
async fn the_person_asks_the_source_asks_a_question_and_the_face_opens_it() {
    let mut rig = ViewRig::new().await;
    let (mut source, mut viewer) = rig.streams().await;
    assert!(rig.handle.can_unlock());
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    rig.face.until_seen(1).await;
    assert_eq!(
        rig.face.seen(),
        [Seen::Open {
            sealed: sealed_key(3),
            source: rig.source.id
        }]
    );
    rig.face.question().opened(box_key(0x42));
    assert_eq!(
        viewer.next().await,
        ViewerMessage::UnlockOpened(Opened {
            challenge: challenge(3),
            key: box_key(0x42),
        })
    );
    say_source(
        &mut source,
        SourceMessage::UnlockDone(finished(1, UnlockOutcome::Unlocked)),
    )
    .await;
    rig.face.until_seen(2).await;
    assert_eq!(
        rig.face.seen()[1..],
        [Seen::Settled(Settled::Said(UnlockOutcome::Unlocked))]
    );
    // And the person can ask again.
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(2));
}

#[tokio::test]
async fn the_person_cannot_ask_while_an_attempt_goes() {
    let mut rig = ViewRig::new().await;
    let (_source, mut viewer) = rig.streams().await;
    assert!(rig.handle.unlock());
    assert!(!rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    viewer.nothing().await;
}

#[tokio::test]
async fn a_question_that_is_not_for_the_attempt_is_dropped() {
    let mut rig = ViewRig::new().await;
    let (mut source, mut viewer) = rig.streams().await;
    // None was asked. The view has dealt with the question when the quiet is over.
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 1))).await;
    viewer.nothing().await;
    assert_eq!(rig.face.seen(), []);
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    // The question of another attempt.
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(2, 2))).await;
    // The right one, and then the same again.
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 4))).await;
    rig.face.until_seen(1).await;
    viewer.nothing().await;
    assert_eq!(
        rig.face.seen(),
        [Seen::Open {
            sealed: sealed_key(3),
            source: rig.source.id
        }]
    );
}

#[tokio::test]
async fn a_question_that_is_declined_or_dropped_tells_the_source() {
    for (reason, drop_it) in [
        (CancelReason::Declined, false),
        (CancelReason::KeyLost, false),
        (CancelReason::Failed, true),
    ] {
        let mut rig = ViewRig::new().await;
        let (mut source, mut viewer) = rig.streams().await;
        assert!(rig.handle.unlock());
        assert_eq!(viewer.next().await, unlock_message(1));
        say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
        rig.face.until_seen(1).await;
        if drop_it {
            drop(rig.face.question());
        } else {
            rig.face.question().declined(reason);
        }
        assert_eq!(viewer.next().await, cancelled(reason), "{reason:?}");
        // The platform ended it, so it is not told how it ended, and the person can ask again.
        assert!(rig.handle.unlock(), "{reason:?}");
        assert_eq!(viewer.next().await, unlock_message(2), "{reason:?}");
        assert_eq!(rig.face.seen().len(), 1, "{reason:?}");
    }
}

#[tokio::test]
async fn a_platform_that_drops_the_question_where_it_is_given_does_not_stall_the_session() {
    let mut rig = ViewRig::with(quick(), true, Some(Face::refusing())).await;
    let (mut source, mut viewer) = rig.streams().await;
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    assert_eq!(viewer.next().await, cancelled(CancelReason::Failed));
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(2));
}

#[tokio::test]
async fn a_source_that_sends_no_question_in_time_is_given_up_on() {
    let mut rig = ViewRig::with(impatient(), true, Some(Face::default())).await;
    let (mut source, mut viewer) = rig.streams().await;
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    assert_eq!(viewer.next().await, cancelled(CancelReason::Failed));
    rig.face.until_seen(1).await;
    assert_eq!(rig.face.seen(), [Seen::Settled(Settled::NoQuestion)]);
    // The question that comes late is dropped.
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(2));
    assert_eq!(rig.face.seen().len(), 1);
}

#[tokio::test]
async fn a_source_that_does_not_say_how_it_went_in_time_leaves_the_viewer_not_knowing() {
    let mut rig = ViewRig::with(impatient(), true, Some(Face::default())).await;
    let (mut source, mut viewer) = rig.streams().await;
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    rig.face.until_seen(1).await;
    rig.face.question().opened(box_key(1));
    assert!(matches!(
        viewer.next().await,
        ViewerMessage::UnlockOpened(_)
    ));
    rig.face.until_seen(2).await;
    assert_eq!(rig.face.seen()[1..], [Seen::Settled(Settled::NoResult)]);
    // The end that comes late is dropped, and the person can ask again.
    say_source(
        &mut source,
        SourceMessage::UnlockDone(finished(1, UnlockOutcome::Unlocked)),
    )
    .await;
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(2));
    assert_eq!(rig.face.seen().len(), 2);
}

#[tokio::test]
async fn an_end_while_the_question_is_open_closes_the_prompt_and_drops_the_answer() {
    let mut rig = ViewRig::new().await;
    let (mut source, mut viewer) = rig.streams().await;
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    rig.face.until_seen(1).await;
    say_source(
        &mut source,
        SourceMessage::UnlockDone(finished(1, UnlockOutcome::TimedOut)),
    )
    .await;
    rig.face.until_seen(3).await;
    assert_eq!(
        rig.face.seen()[1..],
        [
            Seen::Cancel,
            Seen::Settled(Settled::Said(UnlockOutcome::TimedOut))
        ]
    );
    // The person answers all the same, and the key is not sent.
    rig.face.question().opened(box_key(1));
    viewer.nothing().await;
}

#[tokio::test]
async fn an_end_that_is_for_no_attempt_is_dropped_and_is_no_violation() {
    let mut rig = ViewRig::new().await;
    let (mut source, mut viewer) = rig.streams().await;
    say_source(
        &mut source,
        SourceMessage::UnlockDone(finished(1, UnlockOutcome::Unlocked)),
    )
    .await;
    viewer.nothing().await;
    assert_eq!(rig.face.seen(), []);
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );
}

#[tokio::test]
async fn a_session_without_an_unlocker_cannot_unlock_and_drops_what_the_source_says() {
    let mut rig = ViewRig::with(quick(), true, None).await;
    let (mut source, mut viewer) = rig.streams().await;
    assert!(!rig.handle.can_unlock());
    assert!(!rig.handle.unlock());
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    say_source(
        &mut source,
        SourceMessage::UnlockDone(finished(1, UnlockOutcome::Unlocked)),
    )
    .await;
    viewer.nothing().await;
    assert!(
        tokio::time::timeout(QUIET, rig.handle.ended())
            .await
            .is_err(),
        "the session ended"
    );
}

#[tokio::test]
async fn a_session_whose_offer_did_not_say_unlock_cannot_unlock_and_ignores_its_unlocker() {
    let rig = ViewRig::with(quick(), false, Some(Face::default())).await;
    assert!(!rig.handle.can_unlock());
    assert!(!rig.handle.unlock());
    // The platform is told nothing, not even that the session ended.
    rig.handle.end();
    assert_eq!(rig.ended().await, End::Local);
    assert_eq!(rig.face.seen(), []);
}

#[tokio::test]
async fn a_source_that_sends_a_message_of_an_unlock_that_the_offer_did_not_say_breaks_the_rule() {
    for message in [
        SourceMessage::UnlockSealed(question_for(1, 3)),
        SourceMessage::UnlockDone(finished(1, UnlockOutcome::Unlocked)),
    ] {
        let mut rig = ViewRig::with(quick(), false, Some(Face::default())).await;
        let (mut source, _viewer) = rig.streams().await;
        say_source(&mut source, message.clone()).await;
        assert_eq!(rig.ended().await, End::Violation, "{message:?}");
        assert_eq!(rig.face.seen(), [], "{message:?}");
    }
}

#[tokio::test]
async fn a_person_who_asks_when_the_queue_to_the_source_is_full_is_not_left_waiting_for_an_answer()
{
    let mut rig = ViewRig::new().await;
    // The source has not opened its control stream, so nothing leaves the queue.
    let mut queued = 0;
    while rig.handle.lift(queued % 2 == 0, false) {
        queued += 1;
        assert!(queued <= 64, "the queue has no end");
    }
    assert!(queued > 0, "the queue took nothing");
    assert!(!rig.handle.unlock(), "a full queue took a message");
    // The source opens its stream, and what was queued goes out.
    let (_source, mut viewer) = rig.streams().await;
    let mut lifts = 0;
    while lifts < queued {
        if let ViewerMessage::Lift(_) = viewer.next().await {
            lifts += 1;
        }
    }
    // The attempt that was not made is no attempt that goes: the person can ask, with a new number.
    until("the queue to take an unlock", || rig.handle.unlock()).await;
    assert_eq!(viewer.next().await, unlock_message(2));
}

#[tokio::test]
async fn the_end_of_a_session_tells_the_unlocker_and_the_person_cannot_ask_any_more() {
    let mut rig = ViewRig::new().await;
    let (mut source, mut viewer) = rig.streams().await;
    assert!(rig.handle.unlock());
    assert_eq!(viewer.next().await, unlock_message(1));
    say_source(&mut source, SourceMessage::UnlockSealed(question_for(1, 3))).await;
    rig.face.until_seen(1).await;
    rig.handle.end();
    assert_eq!(rig.ended().await, End::Local);
    assert_eq!(rig.face.seen()[1..], [Seen::Cancel, Seen::Ended]);
    rig.handle.end();
    assert_eq!(rig.face.seen().len(), 3);
    assert!(!rig.handle.unlock());
}

// The two sessions face to face.

/// A real source and a real viewer on one connection, with the unlock set up at the source.
struct PairRig {
    source_device: Device,
    _viewer_device: Device,
    source: SourceHandle,
    view: ViewHandle,
    store: MemoryStore,
    attempts: mpsc::UnboundedReceiver<UnlockAttempt>,
    face: Face,
}

impl PairRig {
    async fn new() -> Self {
        let source_side = Streaming::new();
        let viewer_side = Streaming::new();
        let source_sinks = Sinks::default().install(Kind::Control, 1, source_side.clone());
        let viewer_sinks = Sinks::default()
            .install(Kind::Video, 12, viewer_side.clone())
            .install(Kind::Control, 2, viewer_side.clone());
        let source_device = Device::new(source_sinks, PATIENCE).await;
        let viewer_device = Device::new(viewer_sinks, PATIENCE).await;
        let (dialed, accepted) = connect(&source_device, &viewer_device).await;
        let store = MemoryStore::with(record_between(source_device.id, viewer_device.id, 1));
        let (sender, attempts) = mpsc::unbounded_channel();
        let platform = Typing {
            in_place: Arc::new(AtomicBool::new(true)),
            attempts: sender,
        };
        let setup = source_setup(
            quick_source(),
            true,
            UnlockSetup::Offered(Arc::new(store.clone())),
        );
        let source = source_side
            .source(
                &source_device.trust.get(&viewer_device.id).unwrap(),
                dialed,
                None,
                session_id(),
                setup,
                platform,
            )
            .unwrap();
        let face = Face::default();
        let viewing = Viewing::raw(Mute, takes(), quick()).with_unlocker(Looking(face.clone()));
        let granted = Granted {
            control: true,
            private: true,
            sound: false,
            unlock: true,
            leave: false,
        };
        let view = viewer_side
            .view_granted(
                &viewer_device.trust.get(&source_device.id).unwrap(),
                accepted,
                session_id(),
                viewing,
                granted,
            )
            .unwrap();
        // The screen is locked.
        source.pause(PauseReason::Locked);
        Self {
            source_device,
            _viewer_device: viewer_device,
            source,
            view,
            store,
            attempts,
            face,
        }
    }

    /// The person asks, the source asks its question, and the face is asked.
    async fn asked(&self) -> Question {
        assert!(self.view.unlock());
        until("the face to be asked", || {
            !lock(&self.face.0).held.is_empty()
        })
        .await;
        self.face.question()
    }

    async fn attempt(&mut self) -> UnlockAttempt {
        tokio::time::timeout(PATIENCE, self.attempts.recv())
            .await
            .expect("the platform was given no attempt")
            .expect("the platform was dropped")
    }

    /// Waits until the platform of the viewer has been told how many things.
    async fn until_seen(&self, count: usize) {
        self.face.until_seen(count).await;
    }
}

#[tokio::test]
async fn a_person_whose_face_opens_the_key_unlocks_the_screen() {
    let mut rig = PairRig::new().await;
    assert!(rig.view.can_unlock());
    let question = rig.asked().await;
    assert_eq!(question.sealed(), &sealed_key(1));
    assert_eq!(question.source(), rig.source_device.id);
    question.opened(box_key(0x42));
    let mut attempt = rig.attempt().await;
    assert_eq!(attempt.key(), &box_key(0x42));
    attempt.keep(vec![0xee; 40], sealed_key(2)).unwrap();
    attempt.done(UnlockOutcome::Unlocked);
    rig.until_seen(2).await;
    assert_eq!(
        rig.face.seen()[1..],
        [Seen::Settled(Settled::Said(UnlockOutcome::Unlocked))]
    );
    assert_eq!(
        rig.store.finishes(),
        [Finish::Done(UnlockOutcome::Unlocked)]
    );
    // The key that was used opens nothing now: the record has another sealed key.
    assert_eq!(rig.store.sealed(), Some(sealed_key(2)));
}

#[tokio::test]
async fn a_password_that_was_rejected_ends_the_setup_for_the_next_try() {
    let mut rig = PairRig::new().await;
    let question = rig.asked().await;
    question.opened(box_key(1));
    let mut attempt = rig.attempt().await;
    keeps(&mut attempt);
    attempt.done(UnlockOutcome::Rejected);
    rig.until_seen(2).await;
    assert_eq!(
        rig.face.seen()[1..],
        [Seen::Settled(Settled::Said(UnlockOutcome::Rejected))]
    );
    assert!(!rig.store.usable());
    // The next try is refused at once.
    assert!(rig.view.unlock());
    rig.until_seen(3).await;
    assert_eq!(
        rig.face.seen()[2..],
        [Seen::Settled(Settled::Said(UnlockOutcome::NotSetUp))]
    );
}

#[tokio::test]
async fn a_person_who_declines_can_try_again_and_one_who_lost_the_key_cannot() {
    let rig = PairRig::new().await;
    rig.asked().await.declined(CancelReason::Declined);
    until("the cancel to be written", || {
        rig.store.finishes() == [Finish::Cancelled(CancelReason::Declined)]
    })
    .await;
    assert!(rig.store.usable());
    // Asked again, the face is asked again.
    rig.asked().await.declined(CancelReason::KeyLost);
    until("the setup to end", || !rig.store.usable()).await;
    assert!(rig.view.unlock());
    rig.until_seen(3).await;
    assert_eq!(
        rig.face.seen()[2..],
        [Seen::Settled(Settled::Said(UnlockOutcome::NotSetUp))]
    );
}

#[tokio::test]
async fn a_session_that_ends_while_the_face_is_asked_for_ends_the_question_at_both_ends() {
    let rig = PairRig::new().await;
    let _question = rig.asked().await;
    rig.view.end();
    tokio::time::timeout(PATIENCE, rig.source.ended())
        .await
        .expect("the source did not end");
    assert_eq!(rig.face.seen()[1..], [Seen::Cancel, Seen::Ended]);
    until("the end to be written", || {
        rig.store.finishes() == [Finish::Ended]
    })
    .await;
    assert!(rig.store.usable());
}
