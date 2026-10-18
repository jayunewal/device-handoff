// SPDX-License-Identifier: Apache-2.0

//! The receiver of one transfer: the attempts it is received in, the lanes of each read side by
//! side, and the answers to `Finish` and `Cancel`.
//!
//! A [`Transfer`] is what the session keeps for an offer that it accepted. The session gives it
//! what the network gives it, and asks it what to say back:
//!
//! - [`Transfer::begin_attempt`] when the offer is accepted on a connection. It gives an
//!   [`Attempt`], and it ends the attempt before it, if there was one: the lanes of that one are
//!   stopped with 499.
//! - [`Attempt::lane`] for each lane that the sender opens on that connection, once the header of
//!   the lane is read. It reads the lane to its end under the rules of section 8, and says how the
//!   lane ended and what its stream is to be stopped with.
//! - [`Transfer::finish`] for `Finish`, which waits until every file is settled and then puts the
//!   items in place, and [`Transfer::cancel`] for `Cancel` and for a transfer that the session
//!   forgets.
//!
//! # What the lanes share
//!
//! The lanes of an attempt are tasks of their own, and what they have to agree on is in one place,
//! a [`watch`] channel that holds the `State`. Everything that moves the transfer on is a change
//! to it, and everything that waits (a lane for the list, `Finish` for the files) waits for a
//! change. The state says:
//!
//! - the **phase** of the transfer: receiving, being put in place, done, or dropped. It only moves
//!   forward, and a transfer that is dropped stays dropped.
//! - the number of the **current attempt**. A lane of an attempt that is not the current one is
//!   stopped with 499 and changes nothing, so a late lane of an attempt that has ended can neither
//!   drop the transfer nor settle a file.
//! - the **stop code** of the current attempt, once it has one: 408 for a stall, 507 for a full
//!   disk, and for a transfer that is dropped the code of the reason. Every lane of the attempt
//!   stops with it as soon as it is set.
//! - the [`Intake`] of the current attempt, from the moment its list is complete, and whether every
//!   file has an outcome.
//!
//! The list and the staging folder belong to the transfer and not to an attempt, and an attempt
//! that comes after another uses the ones that were made. The list is the one whose hash the offer
//! carried, so a later attempt sends the same one or breaks a rule, and it is checked again all the
//! same.
//!
//! # How an attempt ends
//!
//! A lane ends because its stream ends, or because the attempt is stopped. The timer of the
//! attempt stops it with 408 when no byte has been read from any of its lanes for
//! thirty seconds, and a lane stops it with 507 when the disk is full. Neither drops the transfer:
//! what was received stays. After a 408 the sender may try again on a new connection. After a 507
//! it does not. It sends `Finish`, and the files that were checked are put in place and the others
//! are `NoSpace` in `Done`. A disk with no room for the staging folder is the one case that gives
//! the transfer up, because nothing was received. A rule that the sender broke drops the transfer
//! and removes the staging folder.
//!
//! An attempt that is over does nothing more, even to a file. A lane that was stopped may be in
//! the middle of a write, or hold a frame that it has read and not applied yet, and the books of
//! two attempts share one staging folder while neither knows what the other was told of a file. So
//! the [`Intake`] of an attempt is retired before the next attempt makes its own: it waits for the
//! calls that are under way, and turns away every call after that. A late `Skip` of the old attempt
//! can then not remove a file that the new one has checked.
//!
//! A call that never returns, as a write to a share that stopped answering, holds up the lanes of
//! the attempts after it, and every one of them waits for it, not only the next. Those lanes are
//! stopped like any others when their attempt moves no byte, and the call goes on by itself.
//!
//! # How the transfer ends
//!
//! `Finish` is answered when the current attempt has every file settled, or when it was stopped and
//! will not get further. The answer is made once, whoever asks and however often, and the work
//! (putting the items in place) runs on a thread of its own that publishes the outcome itself, so
//! that a `Finish` whose request is dropped cannot leave the transfer half ended.

use std::{
    fmt,
    future::Future,
    io::ErrorKind,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use baton_proto::{
    close_code,
    transfer::{
        Done, FailReason, Finish, Offer,
        list::{ListChecker, Listing},
    },
};
use tokio::{io::AsyncRead, sync::watch, time::Instant};

use super::{Concluded, Counted, Fault, Frame, Halt, Intake, LaneReader, Lanes, Violation};
use crate::{
    transfer::{
        destination::Destination,
        staging::{StageError, Staging},
    },
    util::{blocking, lock},
};

/// How long a transfer may move no byte before its attempt is stopped (section 8, Time).
const STALL: Duration = Duration::from_secs(30);

/// How often the timer of an attempt looks at the count of bytes, as a part of the time it waits.
const TICKS_PER_STALL: u32 = 10;

/// Why a transfer takes no more attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Over {
    /// The transfer has ended: its items are in place, or are being put there. An offer that is
    /// repeated for it is answered as for any accepted transfer, and its lanes are stopped with 499.
    #[error("the transfer has ended")]
    Finished,
    /// The transfer was given up, by a rule that the sender broke, by the person, or by a failure
    /// of the receiver. Nothing of it is kept.
    #[error("the transfer was given up")]
    Dropped,
}

/// The transfer was given up, and has no outcome to give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the transfer was given up")]
pub struct Dropped;

/// How a lane ended, and what the session does with its stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneEnd {
    /// The sender finished the lane after its last frame. There is nothing to stop.
    Finished,
    /// The lane is not read any more, and the session stops its stream with this code. The code is
    /// that of the rule that was broken (400, or 413 for a list that is too long), 408 for a
    /// transfer that moved no byte, 507 for a full disk, or 499 for a lane that is not wanted any
    /// more: the person gave the transfer up, the transfer has ended, or the attempt of the lane is
    /// not the current one.
    Stopped(u32),
    /// The stream failed, as when the sender reset it or the connection was lost. There is nothing
    /// to stop, and the lane's frames are lost. A sender that comes back does so in a new attempt.
    Failed,
}

/// What a transfer is at, and what the lanes and `Finish` wait for. See the module documentation.
struct State {
    phase: Phase,
    /// The number of the current attempt, which is 0 before the first.
    attempt: u64,
    /// The code that the lanes of the current attempt are stopped with, once there is one.
    stop: Option<u32>,
    /// What the current attempt knows, from the moment its list is complete.
    intake: Option<Arc<Intake>>,
    /// Every file of the current attempt has an outcome.
    settled: bool,
}

#[derive(Debug, Clone)]
enum Phase {
    /// The lanes are being read.
    Receiving,
    /// `Finish` has what it waits for, and the items are being put in place.
    Concluding,
    /// The items are in place, and this is how it went.
    Done(Done),
    /// The transfer was given up.
    Dropped,
}

impl State {
    /// The code that a lane of attempt `attempt` is to be stopped with, or `None` while it may go
    /// on.
    fn stop_for(&self, attempt: u64) -> Option<u32> {
        if attempt != self.attempt {
            return Some(close_code::CANCELLED);
        }
        match self.phase {
            Phase::Receiving => self.stop,
            // The code of the reason, which is set for every way that a transfer is dropped.
            Phase::Dropped => Some(self.stop.unwrap_or(close_code::CANCELLED)),
            Phase::Concluding | Phase::Done(_) => Some(close_code::CANCELLED),
        }
    }

    /// Whether an attempt is going: it is the current one, nothing has stopped it, and it still has
    /// files to settle.
    fn moving(&self) -> bool {
        self.attempt > 0
            && matches!(self.phase, Phase::Receiving)
            && self.stop.is_none()
            && !self.settled
    }

    /// Whether the transfer is being received and no attempt is going: it has had none yet, or the
    /// last was stopped or has nothing more to settle, and the transfer waits for the sender to come
    /// back or to say it is done.
    fn waiting(&self) -> bool {
        matches!(self.phase, Phase::Receiving) && !self.moving()
    }

    /// Whether the transfer has ended: its items are in place, or it was given up.
    fn over(&self) -> bool {
        matches!(self.phase, Phase::Done(_) | Phase::Dropped)
    }

    /// `Finish` can be answered when every file has an outcome, or when the attempt was stopped
    /// and so will not get any further.
    fn ready(&self) -> bool {
        self.intake.is_some() && (self.settled || self.stop.is_some())
    }

    /// Whether `Finish` has something to do or to give: the outcome is known, the transfer is
    /// gone, or the transfer can be ended now.
    fn answerable(&self) -> bool {
        match self.phase {
            Phase::Done(_) | Phase::Dropped => true,
            Phase::Receiving => self.ready(),
            Phase::Concluding => false,
        }
    }
}

/// The list of a transfer, and the staging folder that its files are written to. They are made
/// when the first list is complete, and kept for the attempts that come after.
struct Prepared {
    listing: Arc<Listing>,
    staging: Arc<Staging>,
}

/// Why the list and the staging folder could not be made.
enum Prepare {
    /// The transfer was given up in the meantime, and nothing was made.
    Gone,
    /// The staging folder could not be made.
    Stage(StageError),
    /// The runtime is shutting down.
    Shutdown,
}

/// What the lanes of every attempt of a transfer, and the timers, have in common.
struct Shared {
    destination: Destination,
    offer: Offer,
    /// How long the attempts may move no byte.
    stall: Duration,
    /// The list and the staging folder, once the first list is complete. Whoever makes them holds
    /// the lock for as long as it takes, so they are made once.
    prepared: Mutex<Option<Arc<Prepared>>>,
    /// The intakes of attempts that a later attempt took the place of, until they are retired. An
    /// intake goes in when the attempt after it begins. It is retired, and comes out, when the list
    /// of an attempt is complete, which is before that attempt makes an intake of its own.
    earlier: Mutex<Vec<Arc<Intake>>>,
    state: watch::Sender<State>,
}

impl Shared {
    /// Stops the lanes of attempt `attempt` with `code`, unless it is not the current attempt, the
    /// transfer is not being received, or the attempt has a stop code already. Says whether it did.
    fn stop(&self, attempt: u64, code: u32) -> bool {
        self.state.send_if_modified(|state| {
            if state.attempt != attempt
                || !matches!(state.phase, Phase::Receiving)
                || state.stop.is_some()
            {
                return false;
            }
            state.stop = Some(code);
            true
        })
    }

    /// Whether attempt `attempt` is going: it is the current one, nothing has stopped it, and it
    /// still has files to settle.
    fn going(&self, attempt: u64) -> bool {
        let state = self.state.borrow();
        state.attempt == attempt
            && matches!(state.phase, Phase::Receiving)
            && state.stop.is_none()
            && !state.settled
    }

    /// Gives the transfer up because of `code`, a rule that attempt `attempt` broke, and removes
    /// what was received. Says whether it did: the attempt has to be the current one, and the
    /// transfer has to be received still.
    async fn drop_transfer(self: &Arc<Self>, attempt: u64, code: u32) -> bool {
        let dropped = self.state.send_if_modified(|state| {
            if state.attempt != attempt || !matches!(state.phase, Phase::Receiving) {
                return false;
            }
            state.phase = Phase::Dropped;
            state.stop = Some(code);
            true
        });
        if dropped {
            self.discard().await;
        }
        dropped
    }

    /// Removes the staging folder, if there is one. It is done whatever becomes of the caller.
    async fn discard(self: &Arc<Self>) {
        let shared = Arc::clone(self);
        // An error here is that the runtime is shutting down, and then nothing matters.
        let _ = blocking(move || shared.discard_now()).await;
    }

    fn discard_now(&self) {
        let slot = lock(&self.prepared);
        if let Some(prepared) = slot.as_ref()
            && let Err(error) = prepared.staging.discard()
            && error.kind() != ErrorKind::NotFound
        {
            tracing::warn!(kind = ?error.kind(), "a staging folder could not be removed");
        }
    }

    /// Makes the staging folder for `listing` if there is none yet, and gives the list and the
    /// folder that the transfer has. It is done whatever becomes of the caller, so a folder that was
    /// made is never lost track of.
    async fn prepare(self: &Arc<Self>, listing: Listing) -> Result<Arc<Prepared>, Prepare> {
        let shared = Arc::clone(self);
        match blocking(move || shared.prepare_now(listing)).await {
            Ok(result) => result,
            Err(_) => Err(Prepare::Shutdown),
        }
    }

    fn prepare_now(&self, listing: Listing) -> Result<Arc<Prepared>, Prepare> {
        self.retire_earlier();
        let mut slot = lock(&self.prepared);
        if let Some(prepared) = slot.as_ref() {
            return Ok(Arc::clone(prepared));
        }
        // A transfer that was given up makes no folder. One that is given up after this check finds
        // the folder in the slot, once this lock is let go, and removes it.
        if matches!(self.state.borrow().phase, Phase::Dropped) {
            return Err(Prepare::Gone);
        }
        let staging = self
            .destination
            .stage(self.offer.id, &listing)
            .map_err(Prepare::Stage)?;
        let prepared = Arc::new(Prepared {
            listing: Arc::new(listing),
            staging: Arc::new(staging),
        });
        *slot = Some(Arc::clone(&prepared));
        Ok(prepared)
    }

    /// Retires the intakes of the attempts that are over, which waits for the calls that they have
    /// under way. It is for a thread that may block, and it is done before the attempt that has its
    /// list complete makes an intake, so that a late call of an earlier attempt cannot undo what
    /// the new one does to the files.
    ///
    /// The intakes stay in the list until they are retired, so a second thread that gets here
    /// while the first still waits waits as well, and neither goes on before the gate is closed.
    fn retire_earlier(&self) {
        let earlier = lock(&self.earlier).clone();
        for intake in &earlier {
            intake.retire();
        }
        lock(&self.earlier).retain(|kept| !earlier.iter().any(|gone| Arc::ptr_eq(kept, gone)));
    }

    /// Takes the transfer from receiving to being put in place, if it can be, and does that on a
    /// thread of its own. The thread publishes the outcome, so nothing depends on the caller going
    /// on.
    fn start_conclusion(self: &Arc<Self>, finish: Finish) {
        let mut taken = None;
        self.state.send_if_modified(|state| {
            if !matches!(state.phase, Phase::Receiving) || !state.ready() {
                return false;
            }
            let Some(intake) = state.intake.clone() else {
                return false;
            };
            let unsettled = if state.stop == Some(close_code::NO_SPACE) {
                FailReason::NoSpace
            } else {
                FailReason::Incomplete
            };
            taken = Some((intake, unsettled));
            state.phase = Phase::Concluding;
            true
        });
        if let Some((intake, unsettled)) = taken {
            let shared = Arc::clone(self);
            drop(tokio::task::spawn_blocking(move || {
                shared.conclude(&intake, unsettled, finish);
            }));
        }
    }

    /// Puts the items in place and publishes how it went.
    fn conclude(&self, intake: &Intake, unsettled: FailReason, finish: Finish) {
        // If anything below panics, the transfer is not left in the middle of ending.
        let _unwind = GiveUpOnUnwind(self);
        let outcome = intake.conclude(unsettled);
        self.state.send_if_modified(|state| {
            // Given up while the items were being put in place: that stays.
            if !matches!(state.phase, Phase::Concluding) {
                return false;
            }
            match outcome {
                Ok(concluded) => {
                    note_difference(&finish, &concluded);
                    state.phase = Phase::Done(concluded.done);
                }
                Err(_) => state.phase = Phase::Dropped,
            }
            true
        });
    }
}

/// Gives the transfer up if the thread that is ending it unwinds.
struct GiveUpOnUnwind<'a>(&'a Shared);

impl Drop for GiveUpOnUnwind<'_> {
    fn drop(&mut self) {
        self.0.state.send_if_modified(|state| {
            if !matches!(state.phase, Phase::Concluding) {
                return false;
            }
            state.phase = Phase::Dropped;
            true
        });
    }
}

/// Logs that `Finish` and the frames of the lanes disagree about what was sent. The outcome of no
/// file depends on it: each file has its own frames.
fn note_difference(finish: &Finish, concluded: &Concluded) {
    if (finish.files, finish.bytes) != concluded.sent {
        tracing::warn!(
            said_files = finish.files,
            said_bytes = finish.bytes,
            counted_files = concluded.sent.0,
            counted_bytes = concluded.sent.1,
            "Finish and the frames of the lanes disagree about what was sent"
        );
    }
}

/// The receiver of one transfer. See the module documentation.
///
/// It is cheap to clone, and the clones are the same transfer. Nothing is done when the last one is
/// dropped: a transfer that the session forgets is [`cancel`](Self::cancel)led first, or its staging
/// folder is left for the next start of the app to sweep away.
#[derive(Clone)]
pub struct Transfer {
    shared: Arc<Shared>,
}

// The offer has the names of the person's files, and a log gets pasted into a bug report.
impl fmt::Debug for Transfer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transfer")
            .field("id", &self.shared.offer.id)
            .finish_non_exhaustive()
    }
}

impl Transfer {
    /// The receiver of the transfer that `offer` made, which goes into `destination`. The offer
    /// must be valid (section 8): the session has answered one that is not.
    pub fn new(destination: Destination, offer: Offer) -> Self {
        Self::with_stall(destination, offer, STALL)
    }

    fn with_stall(destination: Destination, offer: Offer, stall: Duration) -> Self {
        let (state, _) = watch::channel(State {
            phase: Phase::Receiving,
            attempt: 0,
            stop: None,
            intake: None,
            settled: false,
        });
        Self {
            shared: Arc::new(Shared {
                destination,
                offer,
                stall,
                prepared: Mutex::new(None),
                earlier: Mutex::new(Vec::new()),
                state,
            }),
        }
    }

    /// Begins an attempt, on a connection where `granted` lanes were accepted: the lanes that the
    /// sender may open on it are numbered from 0 below that. The attempt before it, if there was
    /// one, is over: its lanes are stopped with 499.
    ///
    /// Keep each attempt with its connection, because the header of a lane names no attempt and only
    /// the connection tells the lanes of two attempts apart. A lane goes to the attempt begun on the
    /// connection it came on, and not to the current attempt of the transfer: a late lane of the old
    /// connection would be read into the book of the new attempt, and its `End` or `Skip` for a file,
    /// followed by the new attempt's own, would be a second one, which is a violation. A connection
    /// begins at most one attempt of a transfer (section 8, Resuming): a second would mix the books
    /// in the same way, and its fresh lane numbers would let the old ones through.
    ///
    /// It starts the timer of the attempt, so it has to be called where a Tokio runtime runs.
    ///
    /// # Errors
    ///
    /// [`Over`] if the transfer has ended or was given up. Nothing is begun then.
    pub fn begin_attempt(&self, granted: u8) -> Result<Attempt, Over> {
        let mut number = 0;
        let mut over = None;
        let begun = self.shared.state.send_if_modified(|state| {
            match state.phase {
                Phase::Receiving => {}
                Phase::Concluding | Phase::Done(_) => {
                    over = Some(Over::Finished);
                    return false;
                }
                Phase::Dropped => {
                    over = Some(Over::Dropped);
                    return false;
                }
            }
            state.attempt += 1;
            number = state.attempt;
            state.stop = None;
            // The intake of the attempt before is retired by the next list that is complete.
            if let Some(earlier) = state.intake.take() {
                lock(&self.shared.earlier).push(earlier);
            }
            state.settled = false;
            true
        });
        if !begun {
            return Err(over.unwrap_or(Over::Dropped));
        }
        let bytes = Arc::new(AtomicU64::new(0));
        drop(tokio::spawn(stop_when_idle(
            Arc::downgrade(&self.shared),
            number,
            Arc::clone(&bytes),
            self.shared.stall,
        )));
        Ok(Attempt {
            shared: Arc::clone(&self.shared),
            number,
            lanes: Arc::new(Mutex::new(Lanes::new(granted))),
            bytes,
        })
    }

    /// Answers `Finish`: waits until every file of the transfer is settled, or the attempt was
    /// stopped, puts what was received in place, and gives the outcome. A transfer that has its
    /// outcome gives the same one again.
    ///
    /// The wait has no end of its own: a `Finish` that comes before the list does, as when a sender
    /// finishes without having sent it, waits for a new attempt. The session ends the wait with
    /// its own timer by dropping the future, which is safe at any point: the transfer ends once,
    /// whoever is waiting.
    ///
    /// # Errors
    ///
    /// [`Dropped`] if the transfer was given up, before or while it waited.
    pub async fn finish(&self, finish: &Finish) -> Result<Done, Dropped> {
        let mut updates = self.shared.state.subscribe();
        loop {
            {
                let state = updates
                    .wait_for(State::answerable)
                    .await
                    .map_err(|_| Dropped)?;
                match &state.phase {
                    Phase::Done(done) => return Ok(done.clone()),
                    Phase::Dropped => return Err(Dropped),
                    Phase::Receiving | Phase::Concluding => {}
                }
            }
            // The transfer can be ended. If another `Finish` got there first, or the attempt
            // changed meanwhile, this does nothing, and the next wait is for the outcome.
            self.shared.start_conclusion(*finish);
        }
    }

    /// Whether the transfer has ended: its items are in place, or it was given up.
    pub fn is_over(&self) -> bool {
        self.shared.state.borrow().over()
    }

    /// Waits until the transfer has ended, and gives how it ended: `Some` with the outcome when its
    /// items were put in place, `None` when it was given up. The wait has no end of its own.
    pub async fn closed(&self) -> Option<Done> {
        let mut updates = self.shared.state.subscribe();
        match updates.wait_for(State::over).await {
            Ok(state) => match &state.phase {
                Phase::Done(done) => Some(done.clone()),
                _ => None,
            },
            Err(_) => None,
        }
    }

    /// Waits until the transfer has had no attempt going for `limit` in one stretch, or has ended.
    /// A transfer that was accepted and never got an attempt counts from the start, and one whose
    /// attempt was stopped, as by a stall, from the stop. A new attempt starts the count again.
    ///
    /// It is for the owner of the transfer, who gives up on a sender that does not come back:
    /// [`cancel`](Self::cancel) removes what was received.
    pub async fn abandoned(&self, limit: Duration) {
        let mut updates = self.shared.state.subscribe();
        loop {
            // A time when no attempt is going, or the end.
            let over = match updates
                .wait_for(|state| state.over() || state.waiting())
                .await
            {
                Ok(state) => state.over(),
                Err(_) => return,
            };
            if over {
                return;
            }
            // For as long as `limit`, unless an attempt comes first.
            tokio::select! {
                () = tokio::time::sleep(limit) => return,
                changed = updates.wait_for(|state| !state.waiting()) => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// Gives the transfer up, as for `Cancel`, or for a transfer that the session forgets: the
    /// lanes of the attempt that is going are stopped with 499, and the staging folder is removed.
    /// What was put in place already stays. It can be called again, and after the transfer ended.
    pub async fn cancel(&self) {
        self.shared.state.send_if_modified(|state| {
            if matches!(state.phase, Phase::Dropped) {
                return false;
            }
            state.phase = Phase::Dropped;
            state.stop = Some(close_code::CANCELLED);
            true
        });
        self.shared.discard().await;
    }
}

/// One attempt at receiving a transfer: what one connection carries. See the module documentation.
///
/// It is cheap to clone, and the clones are the same attempt, so that each lane can be a task of
/// its own.
#[derive(Clone)]
pub struct Attempt {
    shared: Arc<Shared>,
    number: u64,
    /// The lane numbers that this attempt has taken.
    lanes: Arc<Mutex<Lanes>>,
    /// The bytes read from all the lanes of this attempt.
    bytes: Arc<AtomicU64>,
}

impl fmt::Debug for Attempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attempt")
            .field("number", &self.number)
            .finish_non_exhaustive()
    }
}

/// Where a lane hears of changes to the state of the transfer.
type Updates = watch::Receiver<State>;

/// Why a lane stopped reading.
enum Cause {
    /// The lane could not be read, or broke a rule.
    Fault(Fault),
    /// A frame could not be applied.
    Halt(Halt),
    /// The runtime is shutting down.
    Shutdown,
}

impl Attempt {
    /// Reads lane number `number` of this attempt from `stream`, which is positioned just after the
    /// header of the lane, until it ends. Lane 0 reads the list first, and the other lanes are not
    /// read until the list is complete (section 8, The list goes first). The frames are applied
    /// to the files as they are read, and the lane is read on while a write is under way on
    /// another.
    ///
    /// It says how the lane ended, and the session stops the stream with the code if it says to.
    /// The future can be dropped at any point, as when the connection is lost: what it has begun on
    /// the disk is finished by itself, and nothing is left in the middle.
    ///
    /// A violation by this lane stops every lane of the transfer with its code, and gives the
    /// transfer up. A lane of an attempt that is not the current one breaks nothing, as far as the
    /// transfer is concerned: it is stopped with 499.
    pub async fn lane<R>(&self, number: u8, stream: &mut R) -> LaneEnd
    where
        R: AsyncRead + Unpin + Send,
    {
        let mut updates = self.shared.state.subscribe();
        match self.read_lane(number, stream, &mut updates).await {
            Ok(()) => LaneEnd::Finished,
            Err(end) => end,
        }
    }

    async fn read_lane<R>(
        &self,
        number: u8,
        stream: &mut R,
        updates: &mut Updates,
    ) -> Result<(), LaneEnd>
    where
        R: AsyncRead + Unpin + Send,
    {
        if let Some(code) = self.stop_code() {
            return Err(LaneEnd::Stopped(code));
        }
        let taken = lock(&self.lanes).take(number);
        if let Err(violation) = taken {
            return Err(self.violation(violation).await);
        }
        let mut reader = LaneReader::new(Counted::new(stream, Arc::clone(&self.bytes)), number);
        let intake = if number == 0 {
            self.read_list(&mut reader, updates).await?
        } else {
            self.wait_for_list(updates).await?
        };
        self.read_frames(&mut reader, &intake, updates).await
    }

    /// The code that this attempt's lanes are stopped with, if they are.
    fn stop_code(&self) -> Option<u32> {
        self.shared.state.borrow().stop_for(self.number)
    }

    /// The code to stop a lane with when something else already ended the attempt or the transfer,
    /// as seen now.
    fn code_now(&self) -> u32 {
        self.stop_code().unwrap_or(close_code::CANCELLED)
    }

    /// Reads the list off lane 0, and makes the intake of the attempt from it.
    async fn read_list<S>(
        &self,
        reader: &mut LaneReader<S>,
        updates: &mut Updates,
    ) -> Result<Arc<Intake>, LaneEnd>
    where
        S: AsyncRead + Unpin + Send,
    {
        let offer = &self.shared.offer;
        let mut checker = match ListChecker::new(offer) {
            Ok(checker) => checker,
            Err(error) => return Err(self.violation(error.into()).await),
        };
        // The list is complete when it holds as many entries as the offer said. The checker stops
        // a list that has more of either kind.
        let wanted = u64::from(offer.summary.files) + u64::from(offer.summary.folders);
        let mut seen = 0_u64;
        while seen < wanted {
            let read = match self.unless_stopped(updates, reader.next(None)).await {
                Ok(read) => read,
                Err(code) => return Err(LaneEnd::Stopped(code)),
            };
            match read {
                Ok(Some(Frame::List(chunk))) => {
                    seen = seen.saturating_add(u64::try_from(chunk.entries.len()).unwrap_or(0));
                    if let Err(error) = checker.push(chunk) {
                        return Err(self.violation(error.into()).await);
                    }
                }
                // The reader gives nothing but pieces of the list before the list is complete.
                Ok(Some(_)) => return Err(self.violation(Violation::BeforeTheList).await),
                Ok(None) => return Err(self.violation(Violation::ShortList).await),
                Err(fault) => return Err(self.end_of(Cause::Fault(fault)).await),
            }
        }
        let listing = match checker.finish() {
            Ok(listing) => listing,
            Err(error) => return Err(self.violation(error.into()).await),
        };
        // The books of the attempts before this one are retired in there, and a call of theirs
        // that does not return (a write to a share that stopped answering) holds this lane up. The
        // stop of the attempt still gets through to the lane. What the thread has begun goes on by
        // itself, and its result is kept for the attempts after this one.
        let prepared = match self.unless_stopped(updates, self.prepare(listing)).await {
            Ok(prepared) => prepared?,
            Err(code) => return Err(LaneEnd::Stopped(code)),
        };
        self.publish(&prepared)
    }

    /// Gets the list and the staging folder of the transfer, making them if this is the first list.
    async fn prepare(&self, listing: Listing) -> Result<Arc<Prepared>, LaneEnd> {
        match self.shared.prepare(listing).await {
            Ok(prepared) => Ok(prepared),
            Err(Prepare::Gone) => Err(LaneEnd::Stopped(self.code_now())),
            Err(Prepare::Shutdown) => Err(LaneEnd::Failed),
            Err(Prepare::Stage(StageError::NoSpace)) => Err(self.out_of_room().await),
            Err(Prepare::Stage(error)) => {
                tracing::error!(%error, "the staging folder of a transfer could not be made");
                Err(self.give_up(close_code::INTERNAL).await)
            }
        }
    }

    /// Makes the intake of the attempt and publishes it, which lets the other lanes go on. It does
    /// not if the attempt was stopped or ended while the list was read.
    fn publish(&self, prepared: &Prepared) -> Result<Arc<Intake>, LaneEnd> {
        let intake = Arc::new(Intake::new(
            Arc::clone(&prepared.listing),
            Arc::clone(&prepared.staging),
        ));
        // A transfer of folders only has no file to wait for.
        let settled = intake.is_settled();
        let published = self.shared.state.send_if_modified(|state| {
            if state.attempt != self.number
                || !matches!(state.phase, Phase::Receiving)
                || state.stop.is_some()
                || state.intake.is_some()
            {
                return false;
            }
            state.intake = Some(Arc::clone(&intake));
            state.settled = settled;
            true
        });
        if published {
            Ok(intake)
        } else {
            Err(LaneEnd::Stopped(self.code_now()))
        }
    }

    /// Waits for the list, which another lane reads, and gives the intake of the attempt. The
    /// stream is not read meanwhile.
    async fn wait_for_list(&self, updates: &mut Updates) -> Result<Arc<Intake>, LaneEnd> {
        let attempt = self.number;
        let waited = updates
            .wait_for(|state| state.stop_for(attempt).is_some() || state.intake.is_some())
            .await;
        let Ok(state) = waited else {
            return Err(LaneEnd::Stopped(close_code::CANCELLED));
        };
        if let Some(code) = state.stop_for(attempt) {
            return Err(LaneEnd::Stopped(code));
        }
        state
            .intake
            .clone()
            .ok_or(LaneEnd::Stopped(close_code::CANCELLED))
    }

    /// Reads the frames of the lane to its end, and does what they say.
    async fn read_frames<S>(
        &self,
        reader: &mut LaneReader<S>,
        intake: &Arc<Intake>,
        updates: &mut Updates,
    ) -> Result<(), LaneEnd>
    where
        S: AsyncRead + Unpin + Send,
    {
        let mut told = false;
        loop {
            match self
                .unless_stopped(updates, apply_next(reader, intake))
                .await
            {
                Err(code) => return Err(LaneEnd::Stopped(code)),
                Ok(Ok(Some(settled))) => {
                    if settled && !told {
                        told = true;
                        self.tell_settled();
                    }
                }
                Ok(Ok(None)) => return Ok(()),
                Ok(Err(cause)) => return Err(self.end_of(cause).await),
            }
        }
    }

    /// Says that every file of this attempt has an outcome, which lets `Finish` go on.
    fn tell_settled(&self) {
        self.shared.state.send_if_modified(|state| {
            if state.attempt != self.number || state.settled {
                return false;
            }
            state.settled = true;
            true
        });
    }

    /// Runs `work`, unless the attempt is stopped first, and then gives the code. A lane that is
    /// stopped in the middle of a frame is lost, which is what a stop does to it.
    async fn unless_stopped<T>(
        &self,
        updates: &mut Updates,
        work: impl Future<Output = T>,
    ) -> Result<T, u32> {
        tokio::select! {
            biased;
            code = stopped(updates, self.number) => Err(code),
            value = work => Ok(value),
        }
    }

    /// What a lane that stopped for `cause` is told.
    async fn end_of(&self, cause: Cause) -> LaneEnd {
        match cause {
            Cause::Halt(Halt::Violation(violation)) | Cause::Fault(Fault::Violation(violation)) => {
                self.violation(violation).await
            }
            Cause::Fault(Fault::Cut) => self.violation(Violation::Cut).await,
            Cause::Fault(Fault::Io(error)) => {
                tracing::debug!(kind = ?error.kind(), "a lane of a transfer failed");
                LaneEnd::Failed
            }
            Cause::Halt(Halt::NoSpace) => self.out_of_room().await,
            // The files of the transfer are put in place, or removed, and some other part of this
            // module says why, in the state.
            Cause::Halt(Halt::Gone) => LaneEnd::Stopped(self.code_now()),
            Cause::Shutdown => LaneEnd::Failed,
        }
    }

    /// A rule of the transfer was broken on this lane.
    async fn violation(&self, violation: Violation) -> LaneEnd {
        tracing::warn!(%violation, "the sender of a transfer broke a rule of it");
        self.give_up(violation.code()).await
    }

    /// Gives the transfer up because of `code`, and says what this lane is stopped with: the code
    /// if it was this lane's attempt that gave the transfer up, and whatever stopped the others
    /// first if not.
    async fn give_up(&self, code: u32) -> LaneEnd {
        if self.shared.drop_transfer(self.number, code).await {
            LaneEnd::Stopped(code)
        } else {
            LaneEnd::Stopped(self.code_now())
        }
    }

    /// The disk is full: this attempt is stopped, and the transfer is kept, so that `Finish` puts
    /// what was checked in place and names the rest `NoSpace`. A transfer that has no list yet has
    /// received nothing, because the room for its staging folder is what ran out. It is given up:
    /// there is nothing to answer a `Finish` with.
    async fn out_of_room(&self) -> LaneEnd {
        let received = self.shared.state.borrow().intake.is_some();
        if received {
            self.shared.stop(self.number, close_code::NO_SPACE);
            LaneEnd::Stopped(self.code_now())
        } else {
            self.give_up(close_code::NO_SPACE).await
        }
    }
}

/// Gives the code that the lanes of attempt `attempt` are stopped with, once they are.
async fn stopped(updates: &mut Updates, attempt: u64) -> u32 {
    match updates
        .wait_for(|state| state.stop_for(attempt).is_some())
        .await
    {
        Ok(state) => state.stop_for(attempt).unwrap_or(close_code::CANCELLED),
        Err(_) => close_code::CANCELLED,
    }
}

/// Reads the next frame of a lane and does what it says. Says whether every file has an outcome
/// now, or `None` if the lane ended after its last frame.
async fn apply_next<S>(
    reader: &mut LaneReader<S>,
    intake: &Arc<Intake>,
) -> Result<Option<bool>, Cause>
where
    S: AsyncRead + Unpin + Send,
{
    let frame = match reader.next(Some(intake.listing())).await {
        Ok(Some(frame)) => frame,
        Ok(None) => return Ok(None),
        Err(fault) => return Err(Cause::Fault(fault)),
    };
    let settled = match frame {
        // The reader does not give one once the list is complete.
        Frame::List(_) => return Err(Cause::Fault(Violation::ListOutOfPlace.into())),
        Frame::Span { file, span, data } => {
            if !intake.wants(file) {
                // It was read to its end, as the protocol asks, and that is all.
                reader.recycle(data);
                return Ok(Some(intake.is_settled()));
            }
            let intake = Arc::clone(intake);
            let (data, settled) = blocking(move || {
                let settled = intake.span(file, span, &data);
                (data, settled)
            })
            .await
            .map_err(|_| Cause::Shutdown)?;
            reader.recycle(data);
            settled
        }
        Frame::End { file, hash } => {
            let intake = Arc::clone(intake);
            blocking(move || intake.end(file, hash))
                .await
                .map_err(|_| Cause::Shutdown)?
        }
        Frame::Skip { file, .. } => {
            let intake = Arc::clone(intake);
            blocking(move || intake.skip(file))
                .await
                .map_err(|_| Cause::Shutdown)?
        }
    };
    settled.map(Some).map_err(Cause::Halt)
}

/// Tells when a count has stopped moving for long enough.
#[derive(Debug)]
struct Idle {
    stall: Duration,
    /// The count when it was last seen to move.
    last: u64,
    /// When it was.
    since: Instant,
}

impl Idle {
    const fn new(stall: Duration, count: u64, now: Instant) -> Self {
        Self {
            stall,
            last: count,
            since: now,
        }
    }

    /// Whether the count, which is `count` at `now`, has been the same for `stall`.
    fn idle(&mut self, count: u64, now: Instant) -> bool {
        if count != self.last {
            self.last = count;
            self.since = now;
            return false;
        }
        now.saturating_duration_since(self.since) >= self.stall
    }
}

/// Stops attempt `attempt` with 408 once no byte has been read from its lanes for `stall`, and
/// ends when the attempt is over by itself. It holds the transfer only while it looks.
async fn stop_when_idle(weak: Weak<Shared>, attempt: u64, bytes: Arc<AtomicU64>, stall: Duration) {
    let tick = (stall / TICKS_PER_STALL).max(Duration::from_millis(1));
    let mut idle = Idle::new(stall, bytes.load(Ordering::Relaxed), Instant::now());
    loop {
        tokio::time::sleep(tick).await;
        let Some(shared) = weak.upgrade() else {
            return;
        };
        if !shared.going(attempt) {
            return;
        }
        if idle.idle(bytes.load(Ordering::Relaxed), Instant::now()) {
            if shared.stop(attempt, close_code::TIMEOUT) {
                tracing::info!("a transfer moved no byte, and its attempt is stopped");
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::Path,
        pin::Pin,
        sync::atomic::AtomicBool,
        task::{Context, Poll},
    };

    use baton_proto::{
        id::TransferId,
        transfer::{lane::SkipReason, list::ListEntry},
    };
    use tokio::io::{AsyncWriteExt, ReadBuf, duplex};

    use super::*;
    use crate::transfer::testing::{
        bytes, chunk, destination, end, end_with, file, folder, lane, list, offer_for, skip, span,
    };

    /// `a` (a folder), `a/x` (10 bytes), `a/y` (a mebibyte and 3 bytes, so two spans), `z` (5
    /// bytes) and `zero` (empty), in the order of the list.
    const A: u32 = 0;
    const X: u32 = 1;
    const Y: u32 = 2;
    const Z: u32 = 3;
    const ZERO: u32 = 4;
    const MIB: usize = 1 << 20;

    /// A stall that no test comes near.
    const SLOW: Duration = Duration::from_secs(30);
    /// The longest a test waits for something that has to happen.
    const WAIT: Duration = Duration::from_secs(20);

    fn entries() -> Vec<ListEntry> {
        vec![
            folder("a"),
            file("a/x", 10),
            file("a/y", MIB as u64 + 3),
            file("z", 5),
            file("zero", 0),
        ]
    }

    /// A transfer of the sample, with the bytes of its files and the frames that carry them.
    struct Rig {
        scratch: tempfile::TempDir,
        transfer: Transfer,
        x: Vec<u8>,
        y: Vec<u8>,
        z: Vec<u8>,
    }

    impl Rig {
        fn new() -> Self {
            Self::stalling_after(SLOW)
        }

        fn stalling_after(stall: Duration) -> Self {
            let (scratch, destination) = destination();
            Self {
                scratch,
                transfer: Transfer::with_stall(destination, offer_for(&entries()), stall),
                x: bytes(1, 10),
                y: bytes(2, MIB + 3),
                z: bytes(3, 5),
            }
        }

        /// The first attempt, on a connection that accepted four lanes.
        fn attempt(&self) -> Attempt {
            self.transfer.begin_attempt(4).unwrap()
        }

        fn root(&self) -> &Path {
            self.scratch.path()
        }

        /// What a file of the transfer holds in the destination, if it is there.
        fn received(&self, path: &str) -> Option<Vec<u8>> {
            fs::read(self.root().join(path)).ok()
        }

        /// The names of what the staging folder holds: a folder for each transfer that is going.
        fn staged(&self) -> Vec<String> {
            let mut names = Vec::new();
            for entry in fs::read_dir(self.root()).unwrap() {
                let path = entry.unwrap().path();
                if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".baton-incoming-")
                {
                    for inner in fs::read_dir(&path).unwrap() {
                        names.push(inner.unwrap().file_name().to_string_lossy().into_owned());
                    }
                }
            }
            names
        }

        fn finish() -> Finish {
            Finish {
                id: TransferId::from_bytes([9; TransferId::LEN]),
                files: 4,
                bytes: 10 + MIB as u64 + 3 + 5,
            }
        }

        /// The whole list in one piece.
        fn list() -> Vec<u8> {
            list(&chunk(entries()))
        }

        fn x(&self) -> Vec<u8> {
            lane(&[&span(X, 0, &self.x), &end_with(X, blake3::hash(&self.x))])
        }

        fn y_spans(&self) -> Vec<u8> {
            lane(&[&span(Y, 0, &self.y[..MIB]), &span(Y, 1, &self.y[MIB..])])
        }

        fn y_end(&self) -> Vec<u8> {
            end_with(Y, blake3::hash(&self.y))
        }

        fn z(&self) -> Vec<u8> {
            lane(&[&span(Z, 0, &self.z), &end_with(Z, blake3::hash(&self.z))])
        }

        fn zero() -> Vec<u8> {
            end_with(ZERO, blake3::hash(b""))
        }

        /// The list and then every file, as one lane carries them.
        fn everything(&self) -> Vec<u8> {
            lane(&[
                &Self::list(),
                &self.x(),
                &self.y_spans(),
                &self.y_end(),
                &self.z(),
                &Self::zero(),
            ])
        }

        /// Every file of the sample is in place, with its bytes.
        fn assert_all_in_place(&self, done: &Done) {
            assert_eq!(failed(done), [], "{done:?}");
            assert_eq!(done.more_failed, 0);
            assert_eq!(self.received("a/x"), Some(self.x.clone()));
            assert_eq!(self.received("a/y"), Some(self.y.clone()));
            assert_eq!(self.received("z"), Some(self.z.clone()));
            assert_eq!(self.received("zero"), Some(Vec::new()));
        }
    }

    /// Reads a lane that is all in `bytes`.
    async fn run(attempt: &Attempt, number: u8, bytes: &[u8]) -> LaneEnd {
        let mut stream = bytes;
        attempt.lane(number, &mut stream).await
    }

    /// The files that did not come out well, as `Done` names them.
    fn failed(done: &Done) -> Vec<(u32, FailReason)> {
        done.failed
            .iter()
            .map(|failure| (failure.file, failure.reason))
            .collect()
    }

    /// Waits for `work`, but not for ever.
    async fn within<T>(what: &str, work: impl Future<Output = T>) -> T {
        tokio::time::timeout(WAIT, work)
            .await
            .unwrap_or_else(|_| panic!("{what} took too long"))
    }

    /// Waits until `ready` is true, but not for ever.
    async fn until(what: &str, mut ready: impl FnMut() -> bool) {
        within(what, async {
            while !ready() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
    }

    /// Whether a task that is asked for something has not answered after a little while.
    async fn still_waiting<T>(asking: &mut tokio::task::JoinHandle<T>) -> bool {
        tokio::time::timeout(Duration::from_millis(100), asking)
            .await
            .is_err()
    }

    /// Waits until the list of the attempt that is going is in.
    async fn until_the_list_is_in(rig: &Rig) {
        until("the list", || {
            rig.transfer.shared.state.borrow().intake.is_some()
        })
        .await;
    }

    /// A stream that remembers whether anyone read it.
    struct Probe<'a> {
        bytes: &'a [u8],
        read: Arc<AtomicBool>,
    }

    impl AsyncRead for Probe<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.read.store(true, Ordering::SeqCst);
            let len = self.bytes.len().min(buf.remaining());
            let (now, later) = self.bytes.split_at(len);
            buf.put_slice(now);
            self.bytes = later;
            Poll::Ready(Ok(()))
        }
    }

    /// A stream that fails as one does that the other side reset.
    struct Reset;

    impl AsyncRead for Reset {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset")))
        }
    }

    // A transfer that arrives whole.

    #[tokio::test]
    async fn a_transfer_that_arrives_on_one_lane_is_put_in_place() {
        let rig = Rig::new();
        let attempt = rig.attempt();

        let end = run(&attempt, 0, &rig.everything()).await;
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert_eq!(end, LaneEnd::Finished);
        rig.assert_all_in_place(&done);
    }

    #[tokio::test]
    async fn lanes_side_by_side_carry_the_pieces_of_a_file_and_its_end_in_any_order() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let lane0 = lane(&[&Rig::list(), &rig.x(), &rig.z()]);
        let lane1 = rig.y_spans();
        let lane2 = lane(&[&rig.y_end(), &Rig::zero()]);

        // The lanes that wait for the list come first, and the list is read last.
        let (two, one, zero) = tokio::join!(
            run(&attempt, 2, &lane2),
            run(&attempt, 1, &lane1),
            run(&attempt, 0, &lane0),
        );
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert_eq!([zero, one, two], [LaneEnd::Finished; 3]);
        rig.assert_all_in_place(&done);
    }

    #[tokio::test]
    async fn the_other_lanes_are_not_read_until_the_list_is_complete() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let lane1 = rig.y_spans();
        let read = Arc::new(AtomicBool::new(false));
        let mut probe = Probe {
            bytes: &lane1,
            read: Arc::clone(&read),
        };
        let waiting = attempt.lane(1, &mut probe);
        tokio::pin!(waiting);
        for _ in 0..20 {
            tokio::select! {
                biased;
                end = &mut waiting => panic!("lane 1 ended before the list: {end:?}"),
                () = tokio::task::yield_now() => {}
            }
        }
        assert!(
            !read.load(Ordering::SeqCst),
            "lane 1 was read before the list was complete"
        );

        let lane0 = lane(&[&Rig::list(), &rig.x(), &rig.y_end(), &rig.z(), &Rig::zero()]);
        assert_eq!(run(&attempt, 0, &lane0).await, LaneEnd::Finished);
        assert_eq!(within("lane 1", waiting).await, LaneEnd::Finished);
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert!(read.load(Ordering::SeqCst));
        rig.assert_all_in_place(&done);
    }

    #[tokio::test]
    async fn a_lane_that_fails_leaves_the_transfer_to_the_others() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        assert_eq!(run(&attempt, 0, &rig.everything()).await, LaneEnd::Finished);

        let end = attempt.lane(1, &mut Reset).await;

        assert_eq!(end, LaneEnd::Failed);
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();
        rig.assert_all_in_place(&done);
    }

    #[tokio::test]
    async fn a_transfer_of_folders_only_ends_as_soon_as_its_list_is_in() {
        let (scratch, destination) = destination();
        let entries = [folder("d"), folder("d/e")];
        let transfer = Transfer::with_stall(destination, offer_for(&entries), SLOW);
        let attempt = transfer.begin_attempt(1).unwrap();
        let finish = Finish {
            id: TransferId::from_bytes([9; TransferId::LEN]),
            files: 0,
            bytes: 0,
        };
        // `Finish` is waiting before the list comes.
        let asking = tokio::spawn({
            let transfer = transfer.clone();
            async move { transfer.finish(&finish).await }
        });

        let end = run(&attempt, 0, &list(&chunk(entries.to_vec()))).await;
        let done = within("finish", asking).await.unwrap().unwrap();

        assert_eq!(end, LaneEnd::Finished);
        assert_eq!(failed(&done), []);
        assert!(scratch.path().join("d/e").is_dir());
    }

    // What a file does to the others.

    #[tokio::test]
    async fn a_file_with_a_wrong_hash_fails_alone() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let lane0 = lane(&[
            &Rig::list(),
            &span(X, 0, &rig.x),
            &end(X, 7),
            &rig.y_spans(),
            &rig.y_end(),
            &rig.z(),
            &Rig::zero(),
        ]);

        assert_eq!(run(&attempt, 0, &lane0).await, LaneEnd::Finished);
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert_eq!(failed(&done), [(X, FailReason::Mismatch)]);
        assert_eq!(rig.received("a/x"), None);
        assert_eq!(rig.received("a/y"), Some(rig.y.clone()));
        assert_eq!(rig.received("z"), Some(rig.z.clone()));
    }

    #[tokio::test]
    async fn a_file_that_the_sender_skips_is_named_as_skipped() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let lane0 = lane(&[
            &Rig::list(),
            &skip(X, SkipReason::Changed),
            &rig.y_spans(),
            &rig.y_end(),
            &rig.z(),
            &Rig::zero(),
        ]);

        assert_eq!(run(&attempt, 0, &lane0).await, LaneEnd::Finished);
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert_eq!(failed(&done), [(X, FailReason::Skipped)]);
        assert_eq!(rig.received("a/x"), None);
        assert_eq!(rig.received("zero"), Some(Vec::new()));
    }

    // Rules that the sender breaks.

    #[tokio::test]
    async fn a_rule_that_is_broken_stops_every_lane_and_gives_the_transfer_up() {
        let rig = Rig::new();
        let after_list = |parts: &[&[u8]]| lane(&[&[&Rig::list()[..]], parts].concat());
        let wrong_list = {
            let mut entries = entries();
            entries[1].modified_ms = Some(1_600_000_001_000);
            list(&chunk(entries))
        };
        let short_list = list(&chunk(entries()[..2].to_vec()));
        let cut = {
            let whole = span(X, 0, &rig.x);
            lane(&[&Rig::list(), &whole[..whole.len() - 3]])
        };
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("a span of a folder", after_list(&[&span(A, 0, b"x")])),
            (
                "a span of a file that is not in the list",
                after_list(&[&span(99, 0, b"x")]),
            ),
            ("an end for a folder", after_list(&[&end(A, 1)])),
            ("an end twice", after_list(&[&end(X, 1), &end(X, 2)])),
            (
                "a skip after an end",
                after_list(&[&end(X, 1), &skip(X, SkipReason::Missing)]),
            ),
            ("a tag that is not known", after_list(&[&[9]])),
            (
                "a piece of the list after the list",
                after_list(&[&Rig::list()]),
            ),
            ("a lane finished in the middle of a frame", cut),
            ("a list that is not the one offered", wrong_list),
            ("a lane finished before the list is complete", short_list),
            ("a frame before the list", lane(&[&end(X, 1)])),
        ];

        for (what, bytes) in cases {
            let rig = Rig::new();
            let attempt = rig.attempt();
            // A lane that is waiting for the list, and has nothing to say when it gets it.
            let (_open, mut quiet) = duplex(64);
            let waiting = tokio::spawn({
                let attempt = attempt.clone();
                async move { attempt.lane(1, &mut quiet).await }
            });

            let end = run(&attempt, 0, &bytes).await;

            assert_eq!(end, LaneEnd::Stopped(400), "{what}");
            assert_eq!(
                within(what, waiting).await.unwrap(),
                LaneEnd::Stopped(400),
                "{what}: the lane that waited"
            );
            assert_eq!(
                rig.transfer.finish(&Rig::finish()).await,
                Err(Dropped),
                "{what}"
            );
            assert_eq!(
                rig.transfer.begin_attempt(4).unwrap_err(),
                Over::Dropped,
                "{what}"
            );
            assert_eq!(rig.staged(), Vec::<String>::new(), "{what}");
            assert_eq!(rig.received("z"), None, "{what}");
        }
    }

    #[tokio::test]
    async fn a_lane_number_beyond_the_grant_or_used_twice_is_a_violation() {
        for number in [2, 3, 200] {
            let rig = Rig::new();
            let attempt = rig.transfer.begin_attempt(2).unwrap();

            let end = run(&attempt, number, &rig.everything()).await;

            assert_eq!(end, LaneEnd::Stopped(400), "lane {number}");
            assert_eq!(rig.transfer.finish(&Rig::finish()).await, Err(Dropped));
        }

        let rig = Rig::new();
        let attempt = rig.attempt();
        assert_eq!(run(&attempt, 0, &rig.everything()).await, LaneEnd::Finished);

        let again = run(&attempt, 0, &rig.everything()).await;

        assert_eq!(again, LaneEnd::Stopped(400));
        assert_eq!(rig.transfer.finish(&Rig::finish()).await, Err(Dropped));
    }

    #[tokio::test]
    async fn the_list_of_a_later_attempt_is_checked_as_the_first_was() {
        let rig = Rig::stalling_after(Duration::from_millis(100));
        let first = rig.attempt();
        let (mut tx, mut rx) = duplex(MIB);
        tx.write_all(&lane(&[&Rig::list(), &rig.x()]))
            .await
            .unwrap();
        assert_eq!(
            within("the stall", first.lane(0, &mut rx)).await,
            LaneEnd::Stopped(408)
        );
        assert_eq!(rig.staged().len(), 1);
        let second = rig.attempt();
        let mut changed = entries();
        changed[1].modified_ms = Some(1_600_000_001_000);

        let end = run(&second, 0, &list(&chunk(changed))).await;

        assert_eq!(end, LaneEnd::Stopped(400));
        assert_eq!(rig.transfer.finish(&Rig::finish()).await, Err(Dropped));
        assert_eq!(rig.staged(), Vec::<String>::new());
        assert_eq!(rig.received("a/x"), None);
    }

    // Attempts.

    #[tokio::test]
    async fn an_attempt_that_moves_no_byte_is_stopped_with_408_and_the_transfer_is_kept() {
        let rig = Rig::stalling_after(Duration::from_millis(100));
        let attempt = rig.attempt();
        let (mut tx, mut rx) = duplex(MIB);
        // The sender sends the list and one file, and goes quiet without finishing the lane.
        tx.write_all(&lane(&[&Rig::list(), &rig.x()]))
            .await
            .unwrap();

        let end = within("the stall", attempt.lane(0, &mut rx)).await;

        assert_eq!(end, LaneEnd::Stopped(408));
        // The transfer was not given up: `Finish` is answered for what there is.
        let done = within("finish", rig.transfer.finish(&Rig::finish()))
            .await
            .unwrap();
        assert_eq!(
            failed(&done),
            [
                (Y, FailReason::Incomplete),
                (Z, FailReason::Incomplete),
                (ZERO, FailReason::Incomplete)
            ]
        );
        assert_eq!(rig.received("a/x"), Some(rig.x.clone()));
        assert_eq!(rig.received("a/y"), None);
        drop(tx);
    }

    #[tokio::test]
    async fn a_transfer_goes_on_in_a_new_attempt_after_a_stall() {
        let rig = Rig::stalling_after(Duration::from_millis(100));
        let first = rig.attempt();
        let (mut tx, mut rx) = duplex(2 * MIB);
        tx.write_all(&lane(&[&Rig::list(), &rig.x(), &span(Y, 0, &rig.y[..MIB])]))
            .await
            .unwrap();
        assert_eq!(
            within("the stall", first.lane(0, &mut rx)).await,
            LaneEnd::Stopped(408)
        );

        // The sender connects again and sends everything, the part of the file that is held
        // included.
        let second = rig.attempt();
        let end = run(&second, 0, &rig.everything()).await;
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert_eq!(end, LaneEnd::Finished);
        rig.assert_all_in_place(&done);
        // A lane that arrives late on the first attempt is not wanted.
        assert_eq!(run(&first, 1, &[]).await, LaneEnd::Stopped(499));
        drop(tx);
    }

    #[tokio::test]
    async fn a_new_attempt_stops_the_lanes_of_the_one_before_with_499() {
        let rig = Rig::new();
        let first = rig.attempt();
        let (mut tx, mut rx) = duplex(MIB);
        tx.write_all(&Rig::list()).await.unwrap();
        let old = tokio::spawn({
            let first = first.clone();
            async move { first.lane(0, &mut rx).await }
        });
        until_the_list_is_in(&rig).await;

        let second = rig.attempt();

        assert_eq!(
            within("the old lane", old).await.unwrap(),
            LaneEnd::Stopped(499)
        );
        // What the old attempt does is no longer the transfer's business: it cannot give the
        // transfer up, or stop the new attempt.
        assert!(!rig.transfer.shared.drop_transfer(first.number, 400).await);
        assert!(!rig.transfer.shared.stop(first.number, 408));
        assert_eq!(run(&second, 0, &rig.everything()).await, LaneEnd::Finished);
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();
        rig.assert_all_in_place(&done);
        drop(tx);
    }

    #[tokio::test]
    async fn a_call_of_an_attempt_that_is_over_cannot_undo_what_the_next_attempt_checked() {
        let rig = Rig::new();
        let first = rig.attempt();
        // The first attempt got the list and the spans of `a/y`, and no `End` for it.
        let (mut old_tx, mut old_rx) = duplex(2 * MIB);
        old_tx
            .write_all(&lane(&[&Rig::list(), &rig.y_spans()]))
            .await
            .unwrap();
        let old_lane = tokio::spawn({
            let first = first.clone();
            async move { first.lane(0, &mut old_rx).await }
        });
        until_the_list_is_in(&rig).await;
        // What a lane of the first attempt calls, when it was stopped with a frame in hand and
        // the call comes late.
        let old = rig.transfer.shared.state.borrow().intake.clone().unwrap();

        let second = rig.attempt();
        assert_eq!(
            within("the old lane", old_lane).await.unwrap(),
            LaneEnd::Stopped(499)
        );
        let (mut tx, mut rx) = duplex(4 * MIB);
        tx.write_all(&rig.everything()).await.unwrap();
        let new_lane = tokio::spawn(async move { second.lane(0, &mut rx).await });
        until("every file of the second attempt is settled", || {
            rig.transfer.shared.state.borrow().settled
        })
        .await;

        assert!(matches!(old.skip(Y), Err(Halt::Gone)));
        assert!(matches!(old.span(Z, 0, &rig.z), Err(Halt::Gone)));
        drop(tx);
        assert_eq!(
            within("the new lane", new_lane).await.unwrap(),
            LaneEnd::Finished
        );
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();
        rig.assert_all_in_place(&done);
        drop(old_tx);
    }

    #[tokio::test]
    async fn the_intake_of_an_attempt_is_retired_before_the_next_attempt_makes_its_own() {
        let rig = Rig::new();
        let first = rig.attempt();
        assert_eq!(run(&first, 0, &Rig::list()).await, LaneEnd::Finished);
        let old = rig.transfer.shared.state.borrow().intake.clone().unwrap();
        let second = rig.attempt();
        // The old intake is still open: the new attempt has not got its list yet.
        assert!(old.skip(ZERO).is_ok());

        assert_eq!(run(&second, 0, &Rig::list()).await, LaneEnd::Finished);
        let new = rig.transfer.shared.state.borrow().intake.clone().unwrap();

        assert!(matches!(old.skip(Z), Err(Halt::Gone)));
        assert!(!Arc::ptr_eq(&old, &new));
        assert!(rig.transfer.shared.earlier.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_attempt_waits_for_every_earlier_book_to_be_retired_and_not_only_the_one_before() {
        let rig = Rig::new();
        let first = rig.attempt();
        assert_eq!(run(&first, 0, &Rig::list()).await, LaneEnd::Finished);
        let old = rig.transfer.shared.state.borrow().intake.clone().unwrap();
        // A call of the first book that does not return, as a write to a share that stopped
        // answering does not.
        let stuck = old.call_under_way();

        // The second attempt gets its list and waits for the first book, so it makes none of its
        // own. The third attempt ends it, and waits for the first book as well.
        let second = rig.attempt();
        let list = Rig::list();
        let mut lane2 = tokio::spawn(async move {
            let mut stream = &list[..];
            second.lane(0, &mut stream).await
        });
        assert!(still_waiting(&mut lane2).await);
        assert!(rig.transfer.shared.state.borrow().intake.is_none());
        let third = rig.attempt();
        assert_eq!(
            within("the second lane", lane2).await.unwrap(),
            LaneEnd::Stopped(499)
        );
        let everything = rig.everything();
        let mut lane3 = tokio::spawn(async move {
            let mut stream = &everything[..];
            third.lane(0, &mut stream).await
        });
        assert!(still_waiting(&mut lane3).await);
        assert!(rig.transfer.shared.state.borrow().intake.is_none());
        assert_eq!(rig.transfer.shared.earlier.lock().unwrap().len(), 1);

        drop(stuck);

        assert_eq!(
            within("the third lane", lane3).await.unwrap(),
            LaneEnd::Finished
        );
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();
        rig.assert_all_in_place(&done);
        assert!(matches!(old.skip(Z), Err(Halt::Gone)));
    }

    #[tokio::test]
    async fn a_lane_that_waits_for_an_earlier_book_is_stopped_when_its_attempt_moves_no_byte() {
        let rig = Rig::stalling_after(Duration::from_millis(100));
        let first = rig.attempt();
        assert_eq!(run(&first, 0, &Rig::list()).await, LaneEnd::Finished);
        let old = rig.transfer.shared.state.borrow().intake.clone().unwrap();
        let stuck = old.call_under_way();
        let second = rig.attempt();

        let end = within("the stall", run(&second, 0, &Rig::list())).await;

        assert_eq!(end, LaneEnd::Stopped(408));
        // The transfer is kept, and the next attempt goes on when the call has returned.
        drop(stuck);
        let third = rig.attempt();
        assert_eq!(run(&third, 0, &rig.everything()).await, LaneEnd::Finished);
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();
        rig.assert_all_in_place(&done);
    }

    #[tokio::test]
    async fn a_transfer_that_has_ended_takes_no_more_attempts_and_no_more_lanes() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        assert_eq!(run(&attempt, 0, &rig.everything()).await, LaneEnd::Finished);
        rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert_eq!(rig.transfer.begin_attempt(4).unwrap_err(), Over::Finished);
        assert_eq!(run(&attempt, 1, &[]).await, LaneEnd::Stopped(499));
    }

    // What the owner of a transfer asks.

    #[tokio::test]
    async fn a_transfer_says_whether_it_is_over_and_how_it_ended() {
        let rig = Rig::new();
        assert!(!rig.transfer.is_over());
        let attempt = rig.attempt();
        assert_eq!(run(&attempt, 0, &rig.everything()).await, LaneEnd::Finished);
        assert!(!rig.transfer.is_over());

        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();

        assert!(rig.transfer.is_over());
        assert_eq!(within("closed", rig.transfer.closed()).await, Some(done));
    }

    #[tokio::test]
    async fn closed_waits_for_the_end_and_gives_none_for_a_transfer_that_was_given_up() {
        let rig = Rig::new();
        let transfer = rig.transfer.clone();
        let mut closed = tokio::spawn(async move { transfer.closed().await });
        assert!(still_waiting(&mut closed).await);

        rig.transfer.cancel().await;

        assert_eq!(within("closed", closed).await.unwrap(), None);
        assert!(rig.transfer.is_over());
    }

    const HOUR: Duration = Duration::from_secs(3600);

    /// Lets the timers that are due fire and the tasks that wait for them go on. The clock is
    /// paused, so this moves it by a millisecond and no more.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }

    /// Spawns a task that waits for the transfer to be abandoned after an hour, and lets it start
    /// to wait: a timer counts from the first time its task is polled, and not from the spawn.
    async fn abandoned_after_an_hour(transfer: &Transfer) -> tokio::task::JoinHandle<()> {
        let transfer = transfer.clone();
        let waiting = tokio::spawn(async move { transfer.abandoned(HOUR).await });
        settle().await;
        waiting
    }

    #[tokio::test(start_paused = true)]
    async fn a_transfer_that_gets_no_attempt_is_abandoned_once_the_limit_has_passed() {
        let rig = Rig::new();
        let abandoned = abandoned_after_an_hour(&rig.transfer).await;

        tokio::time::advance(Duration::from_secs(3599)).await;
        settle().await;
        assert!(!abandoned.is_finished());

        tokio::time::advance(Duration::from_secs(2)).await;
        settle().await;
        assert!(abandoned.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn the_time_of_an_attempt_that_is_going_does_not_count_and_the_stop_starts_the_count() {
        let rig = Rig::new();
        let _attempt = rig.attempt();
        settle().await;
        let abandoned = abandoned_after_an_hour(&rig.transfer).await;

        // The attempt moves no byte, and is stopped once 30 seconds have passed. The hour counts
        // from the stop and not from the start.
        tokio::time::advance(Duration::from_secs(40)).await;
        settle().await;
        assert_eq!(rig.transfer.shared.state.borrow().stop, Some(408));
        assert!(!abandoned.is_finished());

        tokio::time::advance(Duration::from_secs(3580)).await;
        settle().await;
        assert!(!abandoned.is_finished());

        tokio::time::advance(Duration::from_secs(30)).await;
        settle().await;
        assert!(abandoned.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_attempt_starts_the_count_again() {
        let rig = Rig::new();
        let first = rig.attempt();
        settle().await;
        let abandoned = abandoned_after_an_hour(&rig.transfer).await;
        tokio::time::advance(Duration::from_secs(40)).await;
        settle().await;
        assert_eq!(run(&first, 0, &[]).await, LaneEnd::Stopped(408));

        // Half an hour on, the sender comes back, and its attempt stalls in turn, 40 seconds later.
        tokio::time::advance(HOUR / 2).await;
        settle().await;
        let _second = rig.transfer.begin_attempt(4).unwrap();
        settle().await;
        tokio::time::advance(Duration::from_secs(40)).await;
        settle().await;
        assert_eq!(rig.transfer.shared.state.borrow().stop, Some(408));

        // The count of the first stop would have ended by now, and the count of the second has not.
        tokio::time::advance(Duration::from_secs(3500)).await;
        settle().await;
        assert!(!abandoned.is_finished());

        tokio::time::advance(Duration::from_secs(200)).await;
        settle().await;
        assert!(abandoned.is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn a_transfer_that_is_over_is_not_waited_for() {
        let rig = Rig::new();
        rig.transfer.cancel().await;

        within("abandoned", rig.transfer.abandoned(HOUR)).await;
    }

    // Finish.

    #[tokio::test]
    async fn finish_waits_for_the_files_and_answers_once_they_are_settled() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let finish = Rig::finish();
        let mut asking = tokio::spawn({
            let transfer = rig.transfer.clone();
            async move { transfer.finish(&finish).await }
        });
        // There is no list yet, so there is nothing to answer with.
        assert!(still_waiting(&mut asking).await, "answered with no list");

        let (mut tx, mut rx) = duplex(64 * 1024);
        let lane0 = tokio::spawn({
            let attempt = attempt.clone();
            async move { attempt.lane(0, &mut rx).await }
        });
        tx.write_all(&lane(&[&Rig::list(), &rig.x()]))
            .await
            .unwrap();
        until_the_list_is_in(&rig).await;
        assert!(
            still_waiting(&mut asking).await,
            "answered with files missing"
        );

        tx.write_all(&lane(&[
            &rig.y_spans(),
            &rig.y_end(),
            &rig.z(),
            &Rig::zero(),
        ]))
        .await
        .unwrap();
        // The lane has not ended, but every file is settled.
        let done = within("finish", asking).await.unwrap().unwrap();

        rig.assert_all_in_place(&done);
        drop(tx);
        assert_eq!(
            within("the lane", lane0).await.unwrap(),
            LaneEnd::Stopped(499)
        );
    }

    #[tokio::test]
    async fn every_finish_gets_the_same_outcome() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let finish = Rig::finish();
        let asking: Vec<_> = (0..3)
            .map(|_| {
                let transfer = rig.transfer.clone();
                tokio::spawn(async move { transfer.finish(&finish).await })
            })
            .collect();
        assert_eq!(run(&attempt, 0, &rig.everything()).await, LaneEnd::Finished);

        let first = rig.transfer.finish(&finish).await.unwrap();

        for asked in asking {
            assert_eq!(within("finish", asked).await.unwrap().unwrap(), first);
        }
        assert_eq!(rig.transfer.finish(&finish).await.unwrap(), first);
        rig.assert_all_in_place(&first);
    }

    #[tokio::test]
    async fn a_finish_that_is_dropped_does_not_keep_the_transfer_from_ending() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let finish = Rig::finish();
        // Dropped while it waits for a list that is not there.
        assert!(
            tokio::time::timeout(Duration::from_millis(30), rig.transfer.finish(&finish))
                .await
                .is_err()
        );
        assert_eq!(run(&attempt, 0, &rig.everything()).await, LaneEnd::Finished);

        // Polled once by hand, which begins to put the items in place, and dropped. A timeout of
        // nothing does not do this: it drops the future only after a later poll, and by then the
        // thread that puts the items in place may have woken it.
        {
            let asked = std::pin::pin!(rig.transfer.finish(&finish));
            let _ = asked.poll(&mut Context::from_waker(std::task::Waker::noop()));
        }
        let done = within("finish", rig.transfer.finish(&finish))
            .await
            .unwrap();

        rig.assert_all_in_place(&done);
    }

    #[tokio::test]
    async fn a_full_disk_stops_the_attempt_with_507_and_finish_names_what_is_missing_as_no_space() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let (mut tx, mut rx) = duplex(MIB);
        tx.write_all(&lane(&[&Rig::list(), &rig.x()]))
            .await
            .unwrap();
        let lane0 = tokio::spawn({
            let attempt = attempt.clone();
            async move { attempt.lane(0, &mut rx).await }
        });
        until_the_list_is_in(&rig).await;

        assert!(
            rig.transfer
                .shared
                .stop(attempt.number, close_code::NO_SPACE)
        );

        assert_eq!(
            within("the lane", lane0).await.unwrap(),
            LaneEnd::Stopped(507)
        );
        let done = within("finish", rig.transfer.finish(&Rig::finish()))
            .await
            .unwrap();
        assert_eq!(
            failed(&done),
            [
                (Y, FailReason::NoSpace),
                (Z, FailReason::NoSpace),
                (ZERO, FailReason::NoSpace)
            ]
        );
        drop(tx);
    }

    #[tokio::test]
    async fn a_full_disk_after_the_list_keeps_the_transfer_for_finish() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let (mut tx, mut rx) = duplex(MIB);
        tx.write_all(&lane(&[&Rig::list(), &rig.x()]))
            .await
            .unwrap();
        let lane0 = tokio::spawn({
            let attempt = attempt.clone();
            async move { attempt.lane(0, &mut rx).await }
        });
        until_the_list_is_in(&rig).await;

        let end = attempt.out_of_room().await;

        assert_eq!(end, LaneEnd::Stopped(507));
        assert_eq!(
            within("the lane", lane0).await.unwrap(),
            LaneEnd::Stopped(507)
        );
        let done = within("finish", rig.transfer.finish(&Rig::finish()))
            .await
            .unwrap();
        assert_eq!(done.failed.len(), 3, "{done:?}");
        assert!(
            failed(&done)
                .iter()
                .all(|(_, reason)| *reason == FailReason::NoSpace)
        );
        drop(tx);
    }

    #[tokio::test]
    async fn a_full_disk_that_leaves_no_room_for_the_staging_folder_gives_the_transfer_up() {
        let rig = Rig::new();
        let attempt = rig.attempt();

        // No list has come, so there is no staging folder and nothing was received.
        let end = attempt.out_of_room().await;

        assert_eq!(end, LaneEnd::Stopped(507));
        assert_eq!(
            within("finish", rig.transfer.finish(&Rig::finish())).await,
            Err(Dropped)
        );
        assert_eq!(run(&attempt, 1, &[]).await, LaneEnd::Stopped(507));
        assert_eq!(rig.staged(), Vec::<String>::new());
    }

    // Cancel.

    #[tokio::test]
    async fn cancel_stops_the_lanes_and_removes_what_was_received() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        let (mut tx, mut rx) = duplex(MIB);
        tx.write_all(&lane(&[&Rig::list(), &rig.x()]))
            .await
            .unwrap();
        let lane0 = tokio::spawn({
            let attempt = attempt.clone();
            async move { attempt.lane(0, &mut rx).await }
        });
        until_the_list_is_in(&rig).await;
        assert_eq!(rig.staged().len(), 1);

        rig.transfer.cancel().await;

        assert_eq!(
            within("the lane", lane0).await.unwrap(),
            LaneEnd::Stopped(499)
        );
        assert_eq!(rig.staged(), Vec::<String>::new());
        assert_eq!(rig.transfer.finish(&Rig::finish()).await, Err(Dropped));
        assert_eq!(rig.transfer.begin_attempt(4).unwrap_err(), Over::Dropped);
        // It is the same when it is done again.
        rig.transfer.cancel().await;
        drop(tx);
    }

    #[tokio::test]
    async fn cancel_after_the_items_are_in_place_leaves_them_there() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        assert_eq!(run(&attempt, 0, &rig.everything()).await, LaneEnd::Finished);
        let done = rig.transfer.finish(&Rig::finish()).await.unwrap();
        rig.assert_all_in_place(&done);

        rig.transfer.cancel().await;

        rig.assert_all_in_place(&done);
        // The outcome is forgotten with the transfer.
        assert_eq!(rig.transfer.finish(&Rig::finish()).await, Err(Dropped));
    }

    #[tokio::test]
    async fn a_transfer_that_is_cancelled_before_its_list_makes_no_folder() {
        let rig = Rig::new();
        let attempt = rig.attempt();
        rig.transfer.cancel().await;

        let end = run(&attempt, 0, &rig.everything()).await;

        assert_eq!(end, LaneEnd::Stopped(499));
        assert_eq!(rig.staged(), Vec::<String>::new());
    }

    // The timer.

    #[test]
    fn a_count_that_does_not_move_is_idle_once_the_stall_has_passed() {
        let start = Instant::now();
        let mut idle = Idle::new(Duration::from_secs(30), 5, start);

        assert!(!idle.idle(5, start));
        assert!(!idle.idle(5, start + Duration::from_secs(29)));
        assert!(idle.idle(5, start + Duration::from_secs(30)));
        assert!(idle.idle(5, start + Duration::from_secs(31)));
    }

    #[test]
    fn a_count_that_moves_starts_the_time_again() {
        let start = Instant::now();
        let mut idle = Idle::new(Duration::from_secs(30), 5, start);

        assert!(!idle.idle(6, start + Duration::from_secs(29)));
        assert!(!idle.idle(6, start + Duration::from_secs(58)));
        assert!(idle.idle(6, start + Duration::from_secs(59)));
        assert!(!idle.idle(7, start + Duration::from_secs(60)));
        assert!(!idle.idle(7, start + Duration::from_secs(89)));
        assert!(idle.idle(7, start + Duration::from_secs(90)));
    }

    #[test]
    fn a_clock_that_seems_to_go_back_is_not_idle() {
        let start = Instant::now();
        let mut idle = Idle::new(Duration::from_secs(30), 5, start + Duration::from_secs(10));

        assert!(!idle.idle(5, start));
    }

    // The handles.

    #[tokio::test]
    async fn the_futures_and_the_handles_can_go_to_other_threads() {
        fn sendable<T: Send>(_: &T) {}
        fn shared<T: Send + Sync>() {}
        shared::<Transfer>();
        shared::<Attempt>();
        shared::<LaneEnd>();
        let rig = Rig::new();
        let attempt = rig.attempt();
        let mut stream = tokio::io::empty();
        let finish = Rig::finish();

        sendable(&attempt.lane(0, &mut stream));
        sendable(&rig.transfer.finish(&finish));
        sendable(&rig.transfer.cancel());
        sendable(&rig.transfer.closed());
        sendable(&rig.transfer.abandoned(SLOW));
    }

    #[tokio::test]
    async fn nothing_the_sender_chose_is_in_the_debug_output() {
        let rig = Rig::new();
        let attempt = rig.attempt();

        let transfer = format!("{:?}", rig.transfer);
        let attempt = format!("{attempt:?}");

        assert!(!transfer.contains("zero"), "{transfer}");
        assert!(transfer.starts_with("Transfer {"), "{transfer}");
        assert_eq!(attempt, "Attempt { number: 1, .. }");
    }

    // Many files on many lanes.

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn many_files_on_four_lanes_arrive_whole() {
        const FILES: usize = 24;
        let (scratch, destination) = destination();
        let sizes: Vec<usize> = (0..FILES)
            .map(|i| {
                if i % 7 == 0 {
                    0
                } else {
                    (i * 400_009 + 123) % 2_300_000
                }
            })
            .collect();
        let contents: Vec<Vec<u8>> = sizes
            .iter()
            .enumerate()
            .map(|(i, &size)| bytes(u8::try_from(i).unwrap(), size))
            .collect();
        let mut listed = vec![folder("d")];
        listed.extend(
            sizes
                .iter()
                .enumerate()
                .map(|(i, &size)| file(&format!("d/f{i:02}"), size as u64)),
        );
        let transfer = Transfer::with_stall(destination, offer_for(&listed), SLOW);
        let attempt = transfer.begin_attempt(4).unwrap();

        // The pieces of every file are dealt out to the lanes, each file's pieces and end to
        // different ones, and the list goes first on lane 0.
        let mut lanes: Vec<Vec<u8>> = vec![
            list(&chunk(listed.clone())),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ];
        for (i, content) in contents.iter().enumerate() {
            let position = u32::try_from(i + 1).unwrap();
            for (n, piece) in content.chunks(MIB).enumerate() {
                let to = (i + n) % 4;
                lanes[to].extend(span(position, u32::try_from(n).unwrap(), piece));
            }
            lanes[(i * 3) % 4].extend(end_with(position, blake3::hash(content)));
        }
        let finish = Finish {
            id: TransferId::from_bytes([9; TransferId::LEN]),
            files: u32::try_from(FILES).unwrap(),
            bytes: sizes.iter().map(|&size| size as u64).sum(),
        };

        let ends = futures_join(&attempt, &lanes).await;
        let done = within("finish", transfer.finish(&finish)).await.unwrap();

        assert_eq!(ends, [LaneEnd::Finished; 4]);
        assert_eq!(failed(&done), []);
        for (i, content) in contents.iter().enumerate() {
            let received = fs::read(scratch.path().join(format!("d/f{i:02}"))).unwrap();
            assert!(received == *content, "file {i} is not what was sent");
        }
    }

    /// Reads four lanes at once, each in a task of its own.
    async fn futures_join(attempt: &Attempt, lanes: &[Vec<u8>]) -> Vec<LaneEnd> {
        let tasks: Vec<_> = lanes
            .iter()
            .enumerate()
            .map(|(number, bytes)| {
                let attempt = attempt.clone();
                let bytes = bytes.clone();
                tokio::spawn(async move {
                    let mut stream = &bytes[..];
                    attempt
                        .lane(u8::try_from(number).unwrap(), &mut stream)
                        .await
                })
            })
            .collect();
        let mut ends = Vec::new();
        for task in tasks {
            ends.push(within("a lane", task).await.unwrap());
        }
        ends
    }
}
