// SPDX-License-Identifier: Apache-2.0

//! Unlocking a locked screen from the device that views it (`docs/adr/0013-unlock-from-a-phone.md`,
//! and section 9 of `docs/protocol.md`).
//!
//! The messages are in `baton_stream::unlock`. This module holds the rules that the two sides
//! follow, and what each side is given and provides.
//!
//! The **source** keeps, for a viewer whose person set the unlock up, a box with the password in
//! it, and the key of the box sealed to a key of the viewer. That record belongs to the node,
//! which gives the session an [`UnlockStore`] to read and to change it. [`Unlocking`] holds the
//! rules for one session. It takes what the viewer says and what the session knows (the screen is
//! locked, the viewer lifted the privacy, time has passed) and answers with a [`Step`]: what to
//! say to the viewer, what to write down, and when to hand the platform an [`UnlockAttempt`]. It
//! waits for nothing and touches no stream: the session carries out the step. The platform does
//! what only it can. It opens the box with the key, puts the password in a new box, types it, and
//! says what came of it.
//!
//! The **viewer** asks with [`ViewHandle::unlock`](super::ViewHandle::unlock), and answers the
//! question of the source with the face of its person, which the platform provides behind
//! [`Unlocker`]. [`Asking`] holds the rules for one session.
//!
//! The key of the box, the challenge and the sealed key are secrets. None of the types here prints
//! them.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use baton_proto::{
    DeviceId,
    unlock::{BoxKey, Challenge, SealedKey, Sealing, SealingKey},
};
pub use baton_stream::unlock::{CancelReason, Outcome as UnlockOutcome};
use baton_stream::{
    control::{Lift, SourceMessage, ViewerMessage},
    unlock::{Ask, Cancelled, Done, Opened, Sealed},
};
use tokio::sync::{mpsc, oneshot};

use crate::util::random_bytes;

/// The most bytes that the box of an unlock can have. The platform makes the box from a password
/// of up to 127 bytes, padded, with a nonce and a tag, which is far less.
pub(crate) const MAX_BOX_BYTES: usize = 1024;

/// A refusal, a cancellation or a time-out is written to the history at most this often, so that a
/// viewer that asks again and again cannot make the source rewrite its file at that rate. What the
/// platform reports, and whatever ends the setup, is always written.
const NOTE_EVERY: Duration = Duration::from_secs(1);

// What the source keeps, and what the node provides.

/// What a source keeps for one viewer once the person has set the unlock up for it.
#[derive(Clone)]
pub(crate) struct Record {
    /// The password, padded and encrypted by the platform. The core keeps it and does not read it.
    pub(crate) boxed: Vec<u8>,
    /// The key of the box, sealed to `public`. This is what the viewer opens.
    pub(crate) sealed: SealedKey,
    /// How `sealed` is sealed.
    pub(crate) sealing: Sealing,
    /// The key of the viewer that a new key of the box is sealed to at every unlock.
    pub(crate) public: SealingKey,
    /// This device. With `viewer` it is what the platform binds the box to.
    pub(crate) target: DeviceId,
    /// The viewer the record is for.
    pub(crate) viewer: DeviceId,
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("boxed", &format_args!("{} bytes", self.boxed.len()))
            .field("sealed", &self.sealed)
            .field("sealing", &self.sealing)
            .field("public", &self.public)
            .field("viewer", &self.viewer.short())
            .finish_non_exhaustive()
    }
}

/// The box and the sealed key that replace the ones of a record, after an unlock.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the store that writes it comes with the node, which is next"
    )
)]
pub(crate) struct Replacement {
    /// The password in a new box, under a new key.
    pub(crate) boxed: Vec<u8>,
    /// The new key of the box, sealed to the key of the viewer.
    pub(crate) sealed: SealedKey,
}

/// How an attempt ended, for the history that the source keeps of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Finish {
    /// The source ended it with this outcome. A refusal is one.
    Done(UnlockOutcome),
    /// The viewer gave up.
    Cancelled(CancelReason),
    /// The session ended while the source waited for the viewer.
    Ended,
}

impl Finish {
    /// Whether the setup of the viewer ends with this: the source marks it as unusable, answers
    /// `NotSetUp` to every later `Unlock`, and the person has to set it up again at the source
    /// (ADR 0013). A password that the system did not accept ends it, so does a box that the key
    /// did not open, and so does a viewer that says it has lost its key.
    pub(crate) fn ends_setup(self) -> bool {
        matches!(
            self,
            Self::Done(UnlockOutcome::Stale | UnlockOutcome::Rejected)
                | Self::Cancelled(CancelReason::KeyLost)
        )
    }
}

/// What the node is told about an attempt that is over, to write down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Concluded {
    /// How it ended.
    pub(crate) finish: Finish,
    /// The sealed key of the record that the attempt was about, if there was one. A setup that
    /// ends ends only if the record still has this sealed key: a person who set the unlock up
    /// again meanwhile keeps the new setup.
    pub(crate) sealed: Option<SealedKey>,
}

/// What the node provides to a session that can be unlocked: the record of the viewer, and the
/// history.
///
/// The record is read from memory and changed on disk. [`record`](Self::record) does not wait, and
/// the other methods do: the session calls them from a thread that may, never with its own lock
/// held, and the platform calls [`replace`](Self::replace) from one of its own.
pub(crate) trait UnlockStore: Send + Sync + 'static {
    /// The record of the viewer, if an unlock is set up for it and it is still usable.
    fn record(&self) -> Option<Record>;

    /// Replaces the box and the sealed key of the record, in one step that either happens whole or
    /// not at all, if the record still has the sealed key `old`.
    fn replace(&self, old: &SealedKey, new: Replacement) -> Result<(), KeepError>;

    /// Writes down how an attempt ended, and, when [`Finish::ends_setup`] says so and the record
    /// still has the sealed key, marks the record as unusable.
    fn concluded(&self, concluded: Concluded);

    /// Marks the record as usable again, if it has this sealed key. A password that was rejected
    /// ended the setup, and the screen unlocked soon after: the password was right.
    fn restore(&self, sealed: &SealedKey);
}

/// What a session does about an unlock.
#[derive(Clone)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the node offers an unlock when it keeps a record, which is next"
    )
)]
pub(crate) enum UnlockSetup {
    /// This device did not advertise `unlock.target`, or the viewer did not advertise
    /// `unlock.approve`. The viewer was not told that it may unlock, and the messages of an
    /// unlock are a violation.
    Unsupported,
    /// The device can be unlocked, and the offer of this session did not say `unlock`. A viewer
    /// has no reason to ask, and a message that comes anyway is dropped, as the protocol says.
    NotOffered,
    /// The offer said `unlock`: the session takes the messages of an unlock, with this record.
    Offered(Arc<dyn UnlockStore>),
}

impl fmt::Debug for UnlockSetup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "Unsupported",
            Self::NotOffered => "NotOffered",
            Self::Offered(_) => "Offered",
        })
    }
}

// What the platform of the source is given.

/// Why the source cannot keep the new box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeepError {
    /// The attempt is over: the viewer gave up, the session ended, or the time ran out. Nothing
    /// is typed.
    #[error("the attempt is over")]
    Over,
    /// The attempt has kept a box already.
    #[error("the attempt has kept a box already")]
    Twice,
    /// The box or the sealed key is empty or beyond its limit.
    #[error("the box or the sealed key is empty or too large")]
    Invalid,
    /// The unlock was set up again, turned off or ended since the question was asked, so the box
    /// that was open is not the one that the source keeps.
    #[error("the unlock was set up again, turned off or ended")]
    Changed,
    /// The new box could not be written.
    #[error("the new box could not be written")]
    Unwritable,
}

/// What the platform reports of an attempt, once.
pub(super) struct Reported {
    outcome: UnlockOutcome,
    /// The sealed key that the record of the attempt had when it ended: the new one if the box was
    /// replaced.
    sealed: SealedKey,
}

/// An unlock that the platform of the source is to carry out.
///
/// The viewer's person was recognised and the key of the box is open. The platform opens the box
/// with the key, and takes the password out of it. Before it types anything it puts the password in
/// a new box, under a new key of its own that it seals to [`public`](Self::public), and keeps both
/// with [`keep`](Self::keep), so that the key that was just used opens nothing that the source
/// holds afterwards. If it cannot keep them, it types nothing. Then it checks that the screen is
/// locked and the privacy is in place, wakes the display if it is off, types the password and
/// Return, one key at a time, asks [`is_live`](Self::is_live) before each key, waits for the system
/// to say that the screen is unlocked, wipes everything it holds, and says what came of it with
/// [`done`](Self::done). It types the password at most once.
///
/// What the platform may report is `Unlocked`, `Rejected` (typed, and the screen stayed locked),
/// `Stale` (the key did not open the box), `Failed` (anything else that is the source's own),
/// `NotLocked` and `NotPrivate` (nothing was typed). The session takes any other outcome as
/// `Failed`. An attempt that is dropped without a word reports `Failed`.
///
/// The methods do not wait for the session, so they may be called from any thread, also from
/// within [`SourceEvents::unlock`](super::SourceEvents::unlock). [`keep`](Self::keep) waits for
/// the disk, so the platform calls it from a thread of its own.
pub struct UnlockAttempt {
    key: BoxKey,
    record: Record,
    /// The sealed key that the record has now: the new one once the box is kept.
    sealed: SealedKey,
    kept: bool,
    live: Arc<AtomicBool>,
    store: Arc<dyn UnlockStore>,
    done: Option<oneshot::Sender<Reported>>,
}

impl UnlockAttempt {
    /// The key of the box, which the viewer sent. It is wiped when the attempt is dropped.
    #[must_use]
    pub fn key(&self) -> &BoxKey {
        &self.key
    }

    /// The box that the key opens: the password, padded and encrypted by the platform that made
    /// it. The source keeps it and does not read it.
    #[must_use]
    pub fn boxed(&self) -> &[u8] {
        &self.record.boxed
    }

    /// How a new key of the box is sealed to [`public`](Self::public).
    #[must_use]
    pub fn sealing(&self) -> Sealing {
        self.record.sealing
    }

    /// The key of the viewer that the new key of the box is sealed to.
    #[must_use]
    pub fn public(&self) -> &SealingKey {
        &self.record.public
    }

    /// This device, which the box is bound to.
    #[must_use]
    pub fn target(&self) -> DeviceId {
        self.record.target
    }

    /// The viewer that the box is bound to.
    #[must_use]
    pub fn viewer(&self) -> DeviceId {
        self.record.viewer
    }

    /// Whether the attempt goes on. It is not once the viewer has given up, the session has ended,
    /// the time for the platform has run out, or it has been reported. A platform that finds it
    /// false types nothing more, and says what it knows.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    /// Replaces the box and the sealed key that the source keeps by a new box, with the password
    /// in it, and the new key of the box, sealed. One step that either happens whole or not at
    /// all. The platform calls it once, before it types.
    ///
    /// # Errors
    ///
    /// The new box could not be kept. The source still holds the box it had, and the platform
    /// types nothing and reports `Failed`.
    pub fn keep(&mut self, boxed: Vec<u8>, sealed: SealedKey) -> Result<(), KeepError> {
        if !self.is_live() {
            return Err(KeepError::Over);
        }
        if self.kept {
            return Err(KeepError::Twice);
        }
        if boxed.is_empty() || boxed.len() > MAX_BOX_BYTES || sealed.validate().is_err() {
            return Err(KeepError::Invalid);
        }
        let new = Replacement {
            boxed,
            sealed: sealed.clone(),
        };
        self.store.replace(&self.sealed, new)?;
        self.sealed = sealed;
        self.kept = true;
        Ok(())
    }

    /// Says what came of the attempt. It ends the attempt, and the viewer is told.
    pub fn done(mut self, outcome: UnlockOutcome) {
        self.finish(outcome);
    }

    fn finish(&mut self, outcome: UnlockOutcome) {
        if let Some(done) = self.done.take() {
            // Nobody is waiting if the session gave up on the attempt.
            let _ = done.send(Reported {
                outcome,
                sealed: self.sealed.clone(),
            });
        }
    }
}

impl Drop for UnlockAttempt {
    fn drop(&mut self) {
        self.finish(UnlockOutcome::Failed);
    }
}

impl fmt::Debug for UnlockAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnlockAttempt")
            .field("live", &self.is_live())
            .field("kept", &self.kept)
            .finish_non_exhaustive()
    }
}

// The rules of the source.

/// What a session is to do after an event of an unlock. The session carries it out, in this order.
#[derive(Default)]
pub(super) struct Step {
    /// Said to the viewer.
    pub(super) say: Option<SourceMessage>,
    /// Done on a thread that may wait.
    pub(super) job: Option<Job>,
    /// Handed to the platform.
    pub(super) hand: Option<Handoff>,
}

impl Step {
    fn say(message: SourceMessage) -> Self {
        Self {
            say: Some(message),
            ..Self::default()
        }
    }
}

/// A call to the store, which waits for the disk.
pub(super) struct Job {
    store: Arc<dyn UnlockStore>,
    work: Work,
}

enum Work {
    Conclude(Concluded),
    Restore(SealedKey),
}

impl Job {
    /// Does it. Called from a thread that may wait, with no lock held.
    pub(super) fn run(self) {
        match self.work {
            Work::Conclude(concluded) => self.store.concluded(concluded),
            Work::Restore(sealed) => self.store.restore(&sealed),
        }
    }
}

/// An attempt for the platform, and the wait for what it reports.
pub(super) struct Handoff {
    pub(super) attempt: UnlockAttempt,
    pub(super) waiting: Waiting,
}

/// The wait for the report of the platform.
pub(super) struct Waiting {
    pub(super) number: u64,
    reported: oneshot::Receiver<Reported>,
}

impl Waiting {
    /// Waits as long as `within`. A platform that lets go of the attempt without a word, or does
    /// not say in time, has failed, and the report is `None`.
    pub(super) async fn report(self, within: Duration) -> (u64, Option<Reported>) {
        let report = tokio::time::timeout(within, self.reported).await;
        (self.number, report.ok().and_then(Result::ok))
    }
}

/// An attempt that waits for the face of the viewer's person.
struct Asked {
    number: u64,
    challenge: Challenge,
    /// The sealed key that was sent. The key that comes back opens the box that goes with it.
    sealed: SealedKey,
    since: Duration,
}

/// An attempt that the platform carries out.
struct Working {
    number: u64,
    /// Cleared when the attempt is over for the platform.
    live: Arc<AtomicBool>,
    /// The viewer gave up. It is told nothing more.
    cancelled: bool,
    /// What the platform reported, and the sealed key of the record it was about, once it has.
    /// There is no key when the platform did not report.
    reported: Option<(UnlockOutcome, Option<SealedKey>)>,
}

enum Attempt {
    Idle,
    Asked(Asked),
    Working(Working),
}

/// A password that was rejected, which the screen may yet prove right.
struct Grace {
    at: Duration,
    sealed: SealedKey,
}

/// The rules of the source for the unlock of one session.
pub(super) struct Unlocking {
    store: Arc<dyn UnlockStore>,
    /// The viewer of the session, which the record has to be for.
    viewer: DeviceId,
    /// How long the source waits for the face of the person at the viewer.
    wait: Duration,
    /// How long after a rejected password a screen that unlocks proves it right.
    grace: Duration,
    attempt: Attempt,
    /// The setup ended in this session. It holds whatever the store says: the store may have
    /// failed to write it.
    shut: bool,
    /// What the viewer last said about the privacy.
    lifted: Lift,
    /// The platform paused the session because the screen is locked.
    locked: bool,
    rejected: Option<Grace>,
    /// When the last record of a refusal, a cancellation or a time-out was written.
    noted: Option<Duration>,
}

impl Unlocking {
    pub(super) fn new(
        store: Arc<dyn UnlockStore>,
        viewer: DeviceId,
        wait: Duration,
        grace: Duration,
    ) -> Self {
        Self {
            store,
            viewer,
            wait,
            grace,
            attempt: Attempt::Idle,
            shut: false,
            lifted: Lift::default(),
            locked: false,
            rejected: None,
            noted: None,
        }
    }

    /// The record of the viewer, if the setup is usable.
    fn usable(&self) -> Option<Record> {
        if self.shut {
            return None;
        }
        self.store
            .record()
            .filter(|record| record.viewer == self.viewer)
    }

    fn job(&self, work: Work) -> Job {
        Job {
            store: Arc::clone(&self.store),
            work,
        }
    }

    /// Takes the attempt that waits for the face. Any other attempt stays as it is.
    fn take_asked(&mut self) -> Option<Asked> {
        match std::mem::replace(&mut self.attempt, Attempt::Idle) {
            Attempt::Asked(asked) => Some(asked),
            other => {
                self.attempt = other;
                None
            }
        }
    }

    /// A record of how an attempt ended that nothing waits for. At most one a second, unless the
    /// setup ends with it.
    fn note(&mut self, now: Duration, concluded: Concluded) -> Option<Job> {
        if !concluded.finish.ends_setup() {
            if self
                .noted
                .is_some_and(|at| now.saturating_sub(at) < NOTE_EVERY)
            {
                return None;
            }
            self.noted = Some(now);
        }
        Some(self.job(Work::Conclude(concluded)))
    }

    /// Ends an attempt that the source will not carry out, and says why.
    fn refuse(
        &mut self,
        number: u64,
        outcome: UnlockOutcome,
        sealed: Option<SealedKey>,
        now: Duration,
    ) -> Step {
        let concluded = Concluded {
            finish: Finish::Done(outcome),
            sealed,
        };
        Step {
            say: Some(SourceMessage::UnlockDone(Done { number, outcome })),
            job: self.note(now, concluded),
            hand: None,
        }
    }

    /// The viewer asks to unlock. The checks come in the order that the protocol gives.
    /// `in_place` is what the platform says of the cover, the hold and the guard.
    pub(super) fn asked(&mut self, number: u64, now: Duration, in_place: bool) -> Step {
        if !matches!(self.attempt, Attempt::Idle) {
            return self.refuse(number, UnlockOutcome::Busy, None, now);
        }
        let Some(record) = self.usable() else {
            return self.refuse(number, UnlockOutcome::NotSetUp, None, now);
        };
        let sealed = record.sealed;
        if self.lifted.screen || self.lifted.input || !in_place {
            return self.refuse(number, UnlockOutcome::NotPrivate, Some(sealed), now);
        }
        if !self.locked {
            return self.refuse(number, UnlockOutcome::NotLocked, Some(sealed), now);
        }
        let Ok(bytes) = random_bytes::<{ Challenge::LEN }>() else {
            return self.refuse(number, UnlockOutcome::Failed, Some(sealed), now);
        };
        let challenge = Challenge::from_bytes(bytes);
        self.attempt = Attempt::Asked(Asked {
            number,
            challenge,
            sealed: sealed.clone(),
            since: now,
        });
        Step::say(SourceMessage::UnlockSealed(Sealed {
            number,
            challenge,
            sealed,
        }))
    }

    /// The viewer answers the question. An answer to any other question, or to none, is dropped.
    pub(super) fn opened(&mut self, opened: Opened, now: Duration) -> Step {
        if !matches!(&self.attempt, Attempt::Asked(asked) if asked.challenge == opened.challenge) {
            return Step::default();
        }
        let Some(asked) = self.take_asked() else {
            return Step::default();
        };
        let number = asked.number;
        let Some(record) = self.usable() else {
            return self.refuse(number, UnlockOutcome::NotSetUp, None, now);
        };
        // The key opens the box that goes with the sealed key that was sent. A person who set the
        // unlock up again meanwhile has another box, and this key does not open it.
        if record.sealed != asked.sealed {
            return self.refuse(number, UnlockOutcome::Failed, None, now);
        }
        let live = Arc::new(AtomicBool::new(true));
        let (done, reported) = oneshot::channel();
        self.attempt = Attempt::Working(Working {
            number,
            live: Arc::clone(&live),
            cancelled: false,
            reported: None,
        });
        Step {
            hand: Some(Handoff {
                attempt: UnlockAttempt {
                    key: opened.key,
                    sealed: record.sealed.clone(),
                    record,
                    kept: false,
                    live,
                    store: Arc::clone(&self.store),
                    done: Some(done),
                },
                waiting: Waiting { number, reported },
            }),
            ..Step::default()
        }
    }

    /// The viewer gives up. The source says nothing more, and an attempt that the platform is
    /// carrying out stops being live.
    pub(super) fn cancelled(&mut self, reason: CancelReason, now: Duration) -> Step {
        if let Attempt::Working(working) = &mut self.attempt {
            working.cancelled = true;
            working.live.store(false, Ordering::SeqCst);
            return Step::default();
        }
        let Some(asked) = self.take_asked() else {
            return Step::default();
        };
        if reason == CancelReason::KeyLost {
            self.shut = true;
        }
        let concluded = Concluded {
            finish: Finish::Cancelled(reason),
            sealed: Some(asked.sealed),
        };
        Step {
            job: self.note(now, concluded),
            ..Step::default()
        }
    }

    /// Ends the attempt that waits for the face, because of `outcome`.
    fn give_up(&mut self, outcome: UnlockOutcome, now: Duration) -> Step {
        let Some(asked) = self.take_asked() else {
            return Step::default();
        };
        self.refuse(asked.number, outcome, Some(asked.sealed), now)
    }

    /// The viewer says what the people at the source may do. A switch that is turned on ends an
    /// attempt that waits for the face. One turned on while the password is typed takes effect when
    /// the typing is over: the platform does both on one thread.
    pub(super) fn lifted(&mut self, lift: Lift, now: Duration) -> Step {
        self.lifted = lift;
        if lift.screen || lift.input {
            self.give_up(UnlockOutcome::NotPrivate, now)
        } else {
            Step::default()
        }
    }

    /// The platform paused the session because the screen is locked, or resumed it.
    pub(super) fn locked(&mut self, locked: bool, now: Duration) -> Step {
        self.locked = locked;
        if locked {
            return Step::default();
        }
        // The lock is gone. Nobody can unlock a screen that is not locked, and a password that
        // was rejected a moment ago was right after all.
        if matches!(self.attempt, Attempt::Asked(_)) {
            return self.give_up(UnlockOutcome::NotLocked, now);
        }
        match self.rejected.take() {
            Some(rejected) if now.saturating_sub(rejected.at) <= self.grace => {
                self.take_back(rejected.sealed)
            }
            _ => Step::default(),
        }
    }

    /// A password that was rejected proved right: the setup is usable again.
    fn take_back(&mut self, sealed: SealedKey) -> Step {
        self.shut = false;
        Step {
            job: Some(self.job(Work::Restore(sealed))),
            ..Step::default()
        }
    }

    /// Looks at the clock.
    pub(super) fn tick(&mut self, now: Duration) -> Step {
        let waited = matches!(
            &self.attempt,
            Attempt::Asked(asked) if now.saturating_sub(asked.since) >= self.wait
        );
        if waited {
            self.give_up(UnlockOutcome::TimedOut, now)
        } else {
            Step::default()
        }
    }

    /// The session is over.
    pub(super) fn ended(&mut self, now: Duration) -> Step {
        // The platform reports on an attempt that it works on, and the report is written then.
        if let Attempt::Working(working) = &self.attempt {
            working.live.store(false, Ordering::SeqCst);
            return Step::default();
        }
        let Some(asked) = self.take_asked() else {
            return Step::default();
        };
        let concluded = Concluded {
            finish: Finish::Ended,
            sealed: Some(asked.sealed),
        };
        Step {
            job: self.note(now, concluded),
            ..Step::default()
        }
    }

    /// The platform has reported, or has not in time (`None`). Returns the record to write: the
    /// viewer is told only once it is written.
    pub(super) fn reported(&mut self, number: u64, report: Option<Reported>) -> Option<Job> {
        let Attempt::Working(working) = &mut self.attempt else {
            return None;
        };
        if working.number != number || working.reported.is_some() {
            return None;
        }
        working.live.store(false, Ordering::SeqCst);
        let (outcome, sealed) = match report {
            Some(Reported { outcome, sealed }) => (outcome, Some(sealed)),
            None => (UnlockOutcome::Failed, None),
        };
        // What the platform may say. It does not decide what the source answers to a viewer.
        let outcome = match outcome {
            UnlockOutcome::Unlocked
            | UnlockOutcome::Rejected
            | UnlockOutcome::Stale
            | UnlockOutcome::NotLocked
            | UnlockOutcome::NotPrivate => outcome,
            _ => UnlockOutcome::Failed,
        };
        let finish = Finish::Done(outcome);
        if finish.ends_setup() {
            self.shut = true;
        }
        working.reported = Some((outcome, sealed.clone()));
        Some(self.job(Work::Conclude(Concluded { finish, sealed })))
    }

    /// The record of what the platform reported is written. Returns what the viewer is told,
    /// unless it gave up. A password that was rejected is right after all if the screen is no
    /// longer locked: the job takes the setup back, and the viewer is told when it is done.
    pub(super) fn closed(&mut self, number: u64, now: Duration) -> Step {
        if !matches!(&self.attempt, Attempt::Working(working) if working.number == number) {
            return Step::default();
        }
        let Attempt::Working(working) = std::mem::replace(&mut self.attempt, Attempt::Idle) else {
            return Step::default();
        };
        let Some((outcome, sealed)) = working.reported else {
            return Step::default();
        };
        let mut step = Step::default();
        if let (UnlockOutcome::Rejected, Some(sealed)) = (outcome, sealed) {
            if self.locked {
                self.rejected = Some(Grace { at: now, sealed });
            } else {
                step = self.take_back(sealed);
            }
        }
        step.say =
            (!working.cancelled).then_some(SourceMessage::UnlockDone(Done { number, outcome }));
        step
    }
}

// What the platform of the viewer provides.

/// What the viewer's person did with the question of the source.
enum Reply {
    Opened(BoxKey),
    Declined(CancelReason),
}

/// An answer to a question, on its way from the platform to the session.
pub(super) struct Answer {
    number: u64,
    reply: Reply,
}

/// The question of the source, which the viewer's person answers with their face.
///
/// It is the key of the source's box, sealed to a key that the viewer made for this source. The
/// platform opens it with the private half of that key, which works only after the face of its
/// person is recognised, and answers with [`opened`](Self::opened). If the person does not agree,
/// their face is not recognised, or the key is gone, it answers with
/// [`declined`](Self::declined). A question that is dropped is declined with `Failed`.
///
/// The platform opens the sealed key with the key it made for [`source`](Self::source), and tries
/// no other: a sealed key does not name its key, and a platform that tried its keys in turn would
/// open one that was copied from another source. A platform that has no key for the source
/// declines with `KeyLost`.
///
/// An answer that comes after the attempt is over, because the source gave up on it or the session
/// ended, is dropped, and the key in it is wiped.
#[derive(Debug)]
pub struct Question {
    answers: mpsc::UnboundedSender<Answer>,
    number: u64,
    sealed: SealedKey,
    source: DeviceId,
    answered: bool,
}

impl Question {
    /// The key of the source's box, sealed to the key that this device made for the source.
    #[must_use]
    pub fn sealed(&self) -> &SealedKey {
        &self.sealed
    }

    /// The source of the session: the device whose key opens the sealed key.
    #[must_use]
    pub fn source(&self) -> DeviceId {
        self.source
    }

    /// The face was recognised and the sealed key is open: sends the key to the source. The
    /// platform wipes its own copy.
    pub fn opened(mut self, key: BoxKey) {
        self.answer(Reply::Opened(key));
    }

    /// The person does not agree, their face was not recognised, or the key is gone.
    pub fn declined(mut self, reason: CancelReason) {
        self.answer(Reply::Declined(reason));
    }

    fn answer(&mut self, reply: Reply) {
        self.answered = true;
        // The session is over if nobody receives it.
        let _ = self.answers.send(Answer {
            number: self.number,
            reply,
        });
    }
}

impl Drop for Question {
    fn drop(&mut self) {
        if !self.answered {
            self.answer(Reply::Declined(CancelReason::Failed));
        }
    }
}

/// How an attempt that the person made came to an end, when the platform did not end it itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Settled {
    /// The source said what came of it.
    Said(UnlockOutcome),
    /// The source sent no question in time, and the viewer gave up (ten seconds).
    NoQuestion,
    /// The viewer sent the key and the source did not say what came of it in time (thirty seconds).
    /// The screen may be unlocked, and the viewer does not know.
    NoResult,
}

/// What asks for the face of the viewer's person, which the platform provides.
///
/// A viewer whose offer allows an unlock, and that was given an unlocker
/// ([`Viewing::with_unlocker`](super::Viewing::with_unlocker)), can start an attempt with
/// [`ViewHandle::unlock`](super::ViewHandle::unlock). Every method is called with the session's
/// lock held, so none of them waits and none answers a [`Question`] before it returns: a platform
/// that asks for a face hands the work to a thread of its own.
pub trait Unlocker: Send + 'static {
    /// The source sent its question. The platform asks for the face of the person, in words that
    /// name the source, and answers the question when it can.
    fn open(&mut self, question: Question);

    /// The attempt ended while the question was open: the source gave up on it, or the session is
    /// ending. The platform stops asking for the face. An answer that comes later is dropped.
    fn cancel(&mut self);

    /// The attempt ended, and this is what became of it. It is not called for an attempt that the
    /// platform ended itself by declining the question.
    fn settled(&mut self, settled: Settled);

    /// The session is over. The platform lets go of everything it holds for it, and is called no
    /// more.
    fn ended(&mut self);
}

impl Unlocker for Box<dyn Unlocker> {
    fn open(&mut self, question: Question) {
        (**self).open(question);
    }

    fn cancel(&mut self) {
        (**self).cancel();
    }

    fn settled(&mut self, settled: Settled) {
        (**self).settled(settled);
    }

    fn ended(&mut self) {
        (**self).ended();
    }
}

// The rules of the viewer.

/// Where the attempt of the viewer stands.
#[derive(Debug, Clone, Copy)]
enum Phase {
    Idle,
    /// The `Unlock` is sent, and the question has not come.
    Waiting {
        number: u64,
        since: Duration,
    },
    /// The question is with the platform.
    Opening {
        number: u64,
        challenge: Challenge,
    },
    /// The key is sent, and the end has not come.
    Sent {
        number: u64,
        since: Duration,
    },
}

/// How long the viewer waits, and for what. The defaults are where the protocol puts them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Patience {
    /// For the question, after the `Unlock`.
    pub(crate) question: Duration,
    /// For the end, after the key.
    pub(crate) end: Duration,
}

impl Default for Patience {
    fn default() -> Self {
        Self {
            question: Duration::from_secs(10),
            end: Duration::from_secs(30),
        }
    }
}

/// The rules of the viewer for the unlock of one session. The viewer starts no attempt while its
/// own is going, and numbers them from 1.
pub(super) struct Asking {
    unlocker: Box<dyn Unlocker>,
    answers: mpsc::UnboundedSender<Answer>,
    patience: Patience,
    next: u64,
    phase: Phase,
}

impl Asking {
    /// The rules, and the stream of answers that the session applies with [`answered`](Self::answered).
    pub(super) fn new(
        unlocker: Box<dyn Unlocker>,
        patience: Patience,
    ) -> (Self, mpsc::UnboundedReceiver<Answer>) {
        let (answers, receiver) = mpsc::unbounded_channel();
        let asking = Self {
            unlocker,
            answers,
            patience,
            next: 1,
            phase: Phase::Idle,
        };
        (asking, receiver)
    }

    /// The person asks to unlock. Returns the message to send, unless an attempt is going.
    pub(super) fn ask(&mut self, now: Duration) -> Option<ViewerMessage> {
        if !matches!(self.phase, Phase::Idle) {
            return None;
        }
        let number = self.next;
        self.next = self.next.saturating_add(1);
        self.phase = Phase::Waiting { number, since: now };
        Some(ViewerMessage::Unlock(Ask { number }))
    }

    /// The message of `ask` could not be queued.
    pub(super) fn unasked(&mut self) {
        self.phase = Phase::Idle;
    }

    /// The source sends its question. Dropped unless the viewer waits for the question of that
    /// attempt.
    pub(super) fn sealed(&mut self, sealed: Sealed, source: DeviceId) {
        let Phase::Waiting { number, .. } = self.phase else {
            return;
        };
        if sealed.number != number {
            return;
        }
        self.phase = Phase::Opening {
            number,
            challenge: sealed.challenge,
        };
        self.unlocker.open(Question {
            answers: self.answers.clone(),
            number,
            sealed: sealed.sealed,
            source,
            answered: false,
        });
    }

    /// The platform answered the question. Returns the message to send. An answer to a question
    /// that is not open any more is dropped, and the key in it is wiped.
    pub(super) fn answered(&mut self, answer: Answer, now: Duration) -> Option<ViewerMessage> {
        let Phase::Opening { number, challenge } = self.phase else {
            return None;
        };
        if answer.number != number {
            return None;
        }
        match answer.reply {
            Reply::Opened(key) => {
                self.phase = Phase::Sent { number, since: now };
                Some(ViewerMessage::UnlockOpened(Opened { challenge, key }))
            }
            Reply::Declined(reason) => {
                self.phase = Phase::Idle;
                Some(ViewerMessage::UnlockCancelled(Cancelled { reason }))
            }
        }
    }

    /// The source says what came of the attempt. Dropped unless it is the end of the attempt that
    /// the viewer waits for.
    pub(super) fn done(&mut self, done: Done) {
        let (Phase::Waiting { number, .. }
        | Phase::Opening { number, .. }
        | Phase::Sent { number, .. }) = self.phase
        else {
            return;
        };
        if done.number != number {
            return;
        }
        if matches!(self.phase, Phase::Opening { .. }) {
            self.unlocker.cancel();
        }
        self.phase = Phase::Idle;
        self.unlocker.settled(Settled::Said(done.outcome));
    }

    /// Looks at the clock. Returns the message to send.
    pub(super) fn tick(&mut self, now: Duration) -> Option<ViewerMessage> {
        match self.phase {
            Phase::Waiting { since, .. } if now.saturating_sub(since) >= self.patience.question => {
                self.phase = Phase::Idle;
                self.unlocker.settled(Settled::NoQuestion);
                Some(ViewerMessage::UnlockCancelled(Cancelled {
                    reason: CancelReason::Failed,
                }))
            }
            Phase::Sent { since, .. } if now.saturating_sub(since) >= self.patience.end => {
                self.phase = Phase::Idle;
                self.unlocker.settled(Settled::NoResult);
                None
            }
            _ => None,
        }
    }

    /// The session is over.
    pub(super) fn ended(&mut self) {
        if matches!(self.phase, Phase::Opening { .. }) {
            self.unlocker.cancel();
        }
        self.phase = Phase::Idle;
        self.unlocker.ended();
    }
}
