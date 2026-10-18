// SPDX-License-Identifier: Apache-2.0

//! The transfers that other devices offer this device: how many a device may have going, what is
//! remembered of each decision, and which connection an attempt is on.
//!
//! [`receive`](super::receive) receives one transfer. This is the table that holds them all, keyed by
//! the device that made the offer and the id of the transfer, and it answers the requests of section 8
//! of `docs/protocol.md` about them: [`Incoming::offer`], [`Incoming::finish`] and
//! [`Incoming::cancel`]. The session gives it what the network gives it, and it asks the application,
//! through a [`TransferHandler`], for what only the application can say: where transfers go, how large
//! one may be, and whether the person takes this one.
//!
//! # An offer
//!
//! An offer is checked in the order of the protocol. A device that takes no files declines it as
//! `Unsupported`. A repeat of an offer that was decided is answered from memory. Then come the
//! checks that are made before anyone is asked, and that are not remembered, so that a sender that
//! has put the problem right can offer the same transfer again: `TooLarge`, `NoSpace` (with a reserve
//! of free space kept), and `RateLimited` when the device has [`GOING_PER_PEER`] transfers going
//! already, offers that wait for a person included, or has sent too many items. Last the
//! application is asked, once, and what it says is remembered: a refusal for ten minutes, an
//! accepted transfer for as long as it is kept.
//!
//! The question to the application is asked by a task of the table's own and not by the request that
//! made the offer. A sender that gives up and repeats the offer, perhaps on a new connection, finds the
//! question under way and waits for the same answer, and the person is asked once. A request that is
//! dropped, as when its connection goes, leaves the question where it is.
//!
//! # Attempts
//!
//! A transfer is received in attempts, one for each connection (section 8, Resuming). An accepted
//! offer begins an attempt on the connection it came on, unless that connection has the attempt
//! already: a repeat on the same connection is answered and changes nothing, because the lane
//! numbers of a connection are counted once. A repeat on a new connection ends the attempt before it.
//! When several copies of one offer wait for the same decision, the newest begins the attempt. It is
//! the one the sender is waiting for, and an older copy may be on a connection that is gone.
//!
//! The table knows which attempt is on which connection, so a lane is given the attempt of the
//! connection it arrived on ([`Incoming::lane`]) and nothing else: a lane for a transfer that another
//! device offered, that was not accepted, or that is on another connection than the attempt, gets
//! none and is stopped by the session.
//!
//! # Lanes are let go before the answer
//!
//! QUIC acknowledges the end of a lane before the receiver has let go of it, and the receiver counts
//! the lanes it holds. A sender that has its `Done` or `Cancelled` may start the next transfer at
//! once, and its lanes would be counted with the ones of the transfer that just ended. So the lane
//! tasks of a transfer hold a [`LaneGuard`] for as long as they hold their stream, and `Finish` and
//! `Cancel` wait until the guards are gone before they answer. The wait is bounded: a lane that is
//! stuck in a write must not hold an answer back for ever.
//!
//! # What is kept, and for how long
//!
//! A refusal and a transfer that ended are remembered for ten minutes, so that a repeat of the offer
//! or of `Finish` is answered the same way. A transfer that is accepted and has had no attempt going
//! for an hour is given up and what it received is removed (section 8, After a stop), and the table
//! forgets it with that: a decision is remembered for as long as what was received is kept (section
//! 8, A repeat), so a sender that comes back after the hour makes a new offer, and the person is
//! asked again. A transfer that the sender cancelled, or broke a rule of, is remembered for ten
//! minutes like the others. The table holds at most [`CAPACITY`] entries and forgets the oldest of
//! the ones that ended first. A device that is unpaired loses its entries ([`Incoming::forget`]), and
//! a table that is shut down loses all of them and takes no more offers.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use baton_proto::{
    id::{DeviceId, TransferId},
    message::DeclineReason,
    transfer::{Accepted, Declined, Done, Finish, Offer, TransferReply},
};
use tokio::{sync::watch, task::AbortHandle, time::Instant};

use super::{
    destination::{Destination, Room},
    receive::{Attempt, Over, Transfer},
};
use crate::{
    trust::PairedDevice,
    util::{BoxFuture, blocking, lock},
};

/// How many lanes the sender may open for a transfer.
pub const LANES_GRANTED: u8 = 4;

/// How many transfers one device may have going at once, offers that wait for a person included.
pub const GOING_PER_PEER: u8 = 2;

/// How many streams for lanes a connection allows: the lanes of the transfers that a device may have
/// going at once.
pub const LANE_STREAMS: u32 = GOING_PER_PEER as u32 * LANES_GRANTED as u32;

/// How many entries the table holds at most, those that ended and are only remembered included.
pub const CAPACITY: usize = 1024;

/// How much free space is kept beyond what a transfer needs. A transfer that would leave less is
/// declined as `NoSpace`, where the system can tell how much there is (see [`Room`]).
pub const KEEP_FREE: u64 = 512 << 20;

/// How long the person has to answer an offer. The sender waits a little longer (section 8, Offer).
const ASK_TIME: Duration = Duration::from_mins(2);

/// How long a refusal, and a transfer that ended, are remembered.
const MEMORY: Duration = Duration::from_mins(10);

/// How long an accepted transfer may have no attempt going before it is given up.
const KEEP_IDLE: Duration = Duration::from_hours(1);

/// The longest `Finish` waits for the outcome. The sender waits for two minutes and asks again.
const FINISH_WAIT: Duration = Duration::from_mins(3);

/// The longest an answer waits for the lanes of a transfer to be let go.
const RELEASE_WAIT: Duration = Duration::from_secs(5);

/// A connection, as the table tells one from another.
///
/// The session gives each connection a link when it is made (`Link::mint`), and uses it for the
/// requests that come on the connection and for its lanes. What the transport has to tell connections
/// apart will not do: the memory of a connection that ended is given to the next one, and an attempt
/// that was begun on the first would be taken for one begun on the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Link(pub(crate) usize);

impl Link {
    /// A link that no other connection has had in this run of the program.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "made by the session when it takes a connection, which comes next"
        )
    )]
    pub(crate) fn mint() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// What the application decides about the transfers that other devices offer.
///
/// By the time [`offered`](Self::offered) is called, the offer is well-formed, the device is paired,
/// the transfer fits in the room there is, and the device has no more transfers going than it may.
pub trait TransferHandler: Send + Sync + 'static {
    /// Where received transfers are put, or `None` when this device takes no files at the moment. The
    /// capability `transfer.files` is advertised only while there is a folder, and an offer made while
    /// there is none is declined as `Unsupported`.
    ///
    /// Open the folder once ([`Destination::open`]) and give a clone of it each time, because opening
    /// it removes what the transfers that are going hold.
    fn destination(&self) -> Option<Destination>;

    /// The most bytes a transfer may hold, as the person allows. A larger one is declined as
    /// `TooLarge` without asking anyone. `None` sets no limit.
    fn limit_bytes(&self) -> Option<u64> {
        None
    }

    /// Decides on an offer: the policy for the device that made it, and then the person, who sees the
    /// preview. The preview is text that another device wrote, so it must be shown under the rules for
    /// a device name (`sanitize_display_name`).
    ///
    /// The person has two minutes, and the offer is refused when they have run out. The future is
    /// dropped when the sender cancels the offer or the device is unpaired, and the question is to be
    /// taken back from the person then. It is also dropped if the answer comes after the time.
    fn offered<'a>(&'a self, from: &'a PairedDevice, offer: &'a Offer) -> BoxFuture<'a, Verdict>;

    /// A transfer that was accepted has ended. It is called once for each, with how it ended.
    fn ended(&self, from: &PairedDevice, id: TransferId, ending: Ending) {
        let _ = (from, id, ending);
    }
}

// An application that keeps a handle to its handler hands the node a clone of it.
impl<T: TransferHandler + ?Sized> TransferHandler for Arc<T> {
    fn destination(&self) -> Option<Destination> {
        (**self).destination()
    }

    fn limit_bytes(&self) -> Option<u64> {
        (**self).limit_bytes()
    }

    fn offered<'a>(&'a self, from: &'a PairedDevice, offer: &'a Offer) -> BoxFuture<'a, Verdict> {
        (**self).offered(from, offer)
    }

    fn ended(&self, from: &PairedDevice, id: TransferId, ending: Ending) {
        (**self).ended(from, id, ending);
    }
}

/// What the application says to an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Take the transfer.
    Accept,
    /// The person does not want it. The sender is told `Refused`, and the same offer is refused again
    /// for ten minutes without asking.
    Refuse,
    /// The policy for this device does not allow it. The sender is told `Blocked`.
    Block,
}

/// How an accepted transfer ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// The items are in place, except for the files named in the outcome.
    Done(Done),
    /// The transfer was given up and nothing of it is kept: by a rule the sender broke, by `Cancel`,
    /// by the person, because the device was unpaired, or because the sender did not come back.
    Dropped,
}

/// Why a `Finish` has no outcome to give.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FinishError {
    /// The transfer is not known: it was not accepted, it was given up, or it was forgotten. The
    /// session answers `Error { code: NotFound }`.
    #[error("the transfer is not known")]
    NotFound,
    /// The files did not settle in time. The session ends the request without an answer, and the
    /// sender asks again.
    #[error("the transfer did not settle in time")]
    TimedOut,
}

/// Where a transfer is. It only moves forward.
#[derive(Clone)]
enum Stage {
    /// The application is being asked.
    Deciding,
    /// Taken, and being received.
    Accepted(Transfer),
    /// Ended, and these are the files. It is what a repeated `Finish` is answered with.
    Finished(Done),
    /// Declined by the policy for the device or by the person.
    Refused(DeclineReason),
    /// Given up, or never taken: nothing of it is kept.
    Dropped,
}

/// The offers of one device are told apart by their ids.
type Key = (DeviceId, TransferId);

/// One transfer of the table.
struct Entry {
    /// The device that offered it. This copy is what the handler is told about, whatever happens to
    /// the trust store meanwhile.
    from: PairedDevice,
    offer: Offer,
    stage: watch::Sender<Stage>,
    /// How many lane tasks hold a [`LaneGuard`].
    lanes: watch::Sender<usize>,
    inner: Mutex<Inner>,
}

/// What of an entry changes on its own, apart from its stage.
struct Inner {
    /// The arrival number of the newest copy of the offer.
    newest: u64,
    /// The attempt that is going, and the connection it is on.
    current: Option<(Link, Attempt)>,
    /// The task that asks the application, while it does.
    asking: Option<AbortHandle>,
    /// When the table forgets the entry. It is set when the entry ends.
    forget_at: Option<Instant>,
}

impl Entry {
    fn new(from: PairedDevice, offer: Offer) -> Self {
        Self {
            from,
            offer,
            stage: watch::channel(Stage::Deciding).0,
            lanes: watch::channel(0).0,
            inner: Mutex::new(Inner {
                newest: 0,
                current: None,
                asking: None,
                forget_at: None,
            }),
        }
    }

    fn stage(&self) -> Stage {
        self.stage.borrow().clone()
    }

    /// Whether the entry counts against the transfers its device may have going.
    fn is_going(&self) -> bool {
        match &*self.stage.borrow() {
            Stage::Deciding => true,
            Stage::Accepted(transfer) => !transfer.is_over(),
            Stage::Finished(_) | Stage::Refused(_) | Stage::Dropped => false,
        }
    }

    /// Moves the entry on from `Deciding`. Says whether it did: a cancel can come first.
    fn decide(&self, stage: Stage) -> bool {
        self.stage.send_if_modified(|current| {
            if matches!(current, Stage::Deciding) {
                *current = stage;
                true
            } else {
                false
            }
        })
    }

    /// Moves an accepted transfer to the stage it ended in. Says whether it did.
    fn end(&self, stage: Stage) -> bool {
        self.stage.send_if_modified(|current| {
            if matches!(current, Stage::Accepted(_)) {
                *current = stage;
                true
            } else {
                false
            }
        })
    }

    /// Has the table forget the entry once `after` has passed.
    fn forget_in(&self, after: Duration) {
        lock(&self.inner)
            .forget_at
            .get_or_insert(Instant::now() + after);
    }

    /// Ends the question to the application, if it is under way.
    fn stop_asking(&self) {
        if let Some(asking) = lock(&self.inner).asking.take() {
            asking.abort();
        }
    }

    fn is_forgotten(&self, now: Instant) -> bool {
        lock(&self.inner)
            .forget_at
            .is_some_and(|forget_at| forget_at <= now)
    }
}

/// Records a refusal, which is remembered for a while.
fn refuse(entry: &Entry, reason: DeclineReason) {
    if entry.decide(Stage::Refused(reason)) {
        entry.forget_in(MEMORY);
    }
}

/// Waits until the application has answered, and gives the stage that came of it.
async fn decided(entry: &Entry) -> Stage {
    let mut updates = entry.stage.subscribe();
    match updates
        .wait_for(|stage| !matches!(stage, Stage::Deciding))
        .await
    {
        Ok(stage) => stage.clone(),
        Err(_) => Stage::Dropped,
    }
}

/// Waits until no lane task of the entry holds a guard any more, but not for long.
async fn released(entry: &Entry) {
    let mut updates = entry.lanes.subscribe();
    let _ = tokio::time::timeout(RELEASE_WAIT, updates.wait_for(|held| *held == 0)).await;
}

/// Refuses the offer if the task that asks the application ends without an answer, as when the
/// application panics.
struct RefuseUnlessAnswered(Arc<Entry>);

impl Drop for RefuseUnlessAnswered {
    fn drop(&mut self) {
        refuse(&self.0, DeclineReason::Refused);
    }
}

/// What a lane task holds for as long as it holds its stream. See the module documentation.
pub struct LaneGuard {
    entry: Arc<Entry>,
}

impl fmt::Debug for LaneGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaneGuard").finish_non_exhaustive()
    }
}

impl Drop for LaneGuard {
    fn drop(&mut self) {
        self.entry
            .lanes
            .send_modify(|held| *held = held.saturating_sub(1));
    }
}

/// The transfers that other devices offer this device. See the module documentation.
///
/// It is cheap to clone, and the clones are the same table.
#[derive(Clone)]
pub struct Incoming {
    shared: Arc<Shared>,
}

impl fmt::Debug for Incoming {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Incoming").finish_non_exhaustive()
    }
}

struct Shared {
    handler: Arc<dyn TransferHandler>,
    /// Charges one item to the device, and says whether it may send another: a whole transfer counts
    /// as one item against the limit on items (section 8, Offer).
    charge: Box<dyn Fn(&DeviceId) -> bool + Send + Sync>,
    entries: Mutex<HashMap<Key, Arc<Entry>>>,
    /// Numbers the offers in the order they arrive.
    arrivals: AtomicU64,
    /// Set by [`Incoming::shutdown`]. Whoever begins an entry looks at it with the entries locked,
    /// and `shutdown` sets it with them locked, so no entry is begun after the last one was given up.
    closed: AtomicBool,
}

/// What the table made of an offer that came.
enum Begun {
    /// The offer was made before.
    Known(Arc<Entry>),
    /// The table was shut down.
    Closed,
    /// The device has too many going, or too many items, or the table is full.
    Full,
    /// A new entry, whose question is to be asked.
    New(Arc<Entry>),
}

fn declined(id: TransferId, reason: DeclineReason) -> TransferReply {
    TransferReply::Declined(Declined { id, reason })
}

fn accepted(id: TransferId) -> TransferReply {
    TransferReply::Accepted(Accepted {
        id,
        lanes: LANES_GRANTED,
    })
}

impl Incoming {
    /// A table that asks `handler`, and charges each new transfer to the device with `charge`.
    pub fn new(
        handler: Arc<dyn TransferHandler>,
        charge: impl Fn(&DeviceId) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                handler,
                charge: Box::new(charge),
                entries: Mutex::new(HashMap::new()),
                arrivals: AtomicU64::new(0),
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// Whether this device takes files at the moment: it has a folder to receive into, and has not
    /// been shut down. The capability is advertised if it does.
    pub fn takes_files(&self) -> bool {
        self.destination().is_some()
    }

    /// The folder that transfers are received into, unless the table is shut down.
    fn destination(&self) -> Option<Destination> {
        if self.shared.closed.load(Ordering::Acquire) {
            return None;
        }
        self.shared.handler.destination()
    }

    /// Answers an offer that `from` made on the connection `link`.
    ///
    /// The offer must be valid on its own (`TransferRequest::validate`): the session has answered one
    /// that is not.
    pub async fn offer(&self, from: &PairedDevice, link: Link, offer: Offer) -> TransferReply {
        let id = offer.id;
        let Some(destination) = self.destination() else {
            return declined(id, DeclineReason::Unsupported);
        };
        if let Some(entry) = self.known(&(from.id, id)) {
            return self.repeated(&entry, link, &offer).await;
        }
        if let Some(reason) = self.too_much(&destination, &offer).await {
            return declined(id, reason);
        }
        match self.begin(from, &offer) {
            Begun::Known(entry) => self.repeated(&entry, link, &offer).await,
            Begun::Closed => declined(id, DeclineReason::Unsupported),
            Begun::Full => declined(id, DeclineReason::RateLimited),
            Begun::New(entry) => {
                self.ask(&entry, destination);
                self.answer(&entry, link).await
            }
        }
    }

    /// Answers `Finish`: waits until the transfer has its outcome, and gives it.
    ///
    /// # Errors
    ///
    /// [`FinishError::NotFound`] for a transfer that was not accepted, was given up or is forgotten,
    /// and [`FinishError::TimedOut`] when the files did not settle in time.
    pub async fn finish(&self, from: &DeviceId, finish: &Finish) -> Result<Done, FinishError> {
        let entry = self
            .known(&(*from, finish.id))
            .ok_or(FinishError::NotFound)?;
        let done = match entry.stage() {
            Stage::Deciding | Stage::Refused(_) | Stage::Dropped => {
                return Err(FinishError::NotFound);
            }
            Stage::Finished(done) => done,
            Stage::Accepted(transfer) => {
                match tokio::time::timeout(FINISH_WAIT, transfer.finish(finish)).await {
                    Ok(Ok(done)) => done,
                    Ok(Err(_)) => return Err(FinishError::NotFound),
                    Err(_) => return Err(FinishError::TimedOut),
                }
            }
        };
        released(&entry).await;
        Ok(done)
    }

    /// Answers `Cancel`: gives the transfer up, whatever stage it is at, and removes what was received.
    /// A transfer that is not known has nothing to cancel, and the answer is the same. It says
    /// whether the transfer was known, for a session that answers a device which takes no files.
    ///
    /// The lanes of the transfer are let go of before this returns, within a few seconds.
    pub async fn cancel(&self, from: &DeviceId, id: TransferId) -> bool {
        let Some(entry) = self.known(&(*from, id)) else {
            return false;
        };
        self.give_up(&entry).await;
        true
    }

    /// Gives the attempt of the connection `link` for a lane of the transfer `id` that `peer` offered,
    /// and a guard to hold for as long as the lane holds its stream. `None` for a lane that belongs
    /// to nothing: the transfer is not known, was not accepted, is over, or its attempt is on
    /// another connection. The session stops such a lane with 499.
    pub fn lane(
        &self,
        peer: &DeviceId,
        id: TransferId,
        link: Link,
    ) -> Option<(Attempt, LaneGuard)> {
        let entry = self.known(&(*peer, id))?;
        if !matches!(&*entry.stage.borrow(), Stage::Accepted(_)) {
            return None;
        }
        let attempt = match &lock(&entry.inner).current {
            Some((on, attempt)) if *on == link => attempt.clone(),
            _ => return None,
        };
        entry.lanes.send_modify(|held| *held += 1);
        Some((attempt, LaneGuard { entry }))
    }

    /// Gives up every transfer of `peer`, as when it is unpaired, and forgets them.
    pub async fn forget(&self, peer: &DeviceId) {
        self.forget_where(|owner| owner == peer).await;
    }

    /// Gives up the transfers of every device that `paired` does not say is paired, as when the
    /// list of paired devices changed and not every change was seen.
    pub async fn forget_unless(&self, paired: impl Fn(&DeviceId) -> bool) {
        self.forget_where(|owner| !paired(owner)).await;
    }

    async fn forget_where(&self, doomed: impl Fn(&DeviceId) -> bool) {
        let gone: Vec<Arc<Entry>> = {
            let mut entries = lock(&self.shared.entries);
            let keys: Vec<Key> = entries
                .keys()
                .filter(|(owner, _)| doomed(owner))
                .copied()
                .collect();
            keys.iter().filter_map(|key| entries.remove(key)).collect()
        };
        self.give_up_all(gone).await;
    }

    /// Gives up every transfer, as when the device stops. The table takes no offers after that.
    pub async fn shutdown(&self) {
        let gone: Vec<Arc<Entry>> = {
            let mut entries = lock(&self.shared.entries);
            self.shared.closed.store(true, Ordering::Release);
            entries.drain().map(|(_, entry)| entry).collect()
        };
        self.give_up_all(gone).await;
    }

    /// Gives up `gone` all together, so that lanes that do not let go delay the answer by
    /// `RELEASE_WAIT` once and not once for each entry. Each entry has a task of its own, which
    /// finishes the job also if the caller stops waiting.
    async fn give_up_all(&self, gone: Vec<Arc<Entry>>) {
        let tasks: Vec<_> = gone
            .into_iter()
            .map(|entry| {
                let table = self.clone();
                tokio::spawn(async move { table.give_up(&entry).await })
            })
            .collect();
        for task in tasks {
            // A task that panicked has nothing more to give up.
            let _ = task.await;
        }
    }

    // The entries.

    /// The entry of an offer that was made before, if the table still remembers it.
    fn known(&self, key: &Key) -> Option<Arc<Entry>> {
        let mut entries = lock(&self.shared.entries);
        forget_the_forgotten(&mut entries, Instant::now());
        entries.get(key).cloned()
    }

    /// Takes a place for a new offer, unless it was made before or cannot be taken.
    fn begin(&self, from: &PairedDevice, offer: &Offer) -> Begun {
        let key = (from.id, offer.id);
        let mut entries = lock(&self.shared.entries);
        if self.shared.closed.load(Ordering::Acquire) {
            return Begun::Closed;
        }
        forget_the_forgotten(&mut entries, Instant::now());
        if let Some(entry) = entries.get(&key) {
            return Begun::Known(Arc::clone(entry));
        }
        let going = entries
            .iter()
            .filter(|((owner, _), entry)| *owner == from.id && entry.is_going())
            .count();
        if going >= usize::from(GOING_PER_PEER) || !(self.shared.charge)(&from.id) {
            return Begun::Full;
        }
        if entries.len() >= CAPACITY && !forget_the_oldest_ended(&mut entries) {
            return Begun::Full;
        }
        let entry = Arc::new(Entry::new(from.clone(), offer.clone()));
        entries.insert(key, Arc::clone(&entry));
        Begun::New(entry)
    }

    /// The reason to decline an offer without asking anyone, if there is one: it is more than the
    /// person allows, or more than the volume has room for.
    async fn too_much(&self, destination: &Destination, offer: &Offer) -> Option<DeclineReason> {
        let bytes = offer.summary.bytes;
        if self
            .shared
            .handler
            .limit_bytes()
            .is_some_and(|limit| bytes > limit)
        {
            return Some(DeclineReason::TooLarge);
        }
        let destination = destination.clone();
        let needed = bytes.saturating_add(KEEP_FREE);
        match blocking(move || destination.room_for(needed)).await {
            Ok(Room::Short) => Some(DeclineReason::NoSpace),
            Ok(Room::Enough | Room::Unknown) | Err(_) => None,
        }
    }

    // The decision.

    /// Starts the task that asks the application about a new offer.
    fn ask(&self, entry: &Arc<Entry>, destination: Destination) {
        let table = self.clone();
        let asked = Arc::clone(entry);
        let task = tokio::spawn(async move {
            let _unless_answered = RefuseUnlessAnswered(Arc::clone(&asked));
            let handler = &table.shared.handler;
            let verdict =
                tokio::time::timeout(ASK_TIME, handler.offered(&asked.from, &asked.offer)).await;
            match verdict {
                Ok(Verdict::Accept) => {
                    let transfer = Transfer::new(destination, asked.offer.clone());
                    if asked.decide(Stage::Accepted(transfer.clone())) {
                        table.keep(&asked, transfer);
                    }
                }
                Ok(Verdict::Block) => refuse(&asked, DeclineReason::Blocked),
                Ok(Verdict::Refuse) | Err(_) => refuse(&asked, DeclineReason::Refused),
            }
        });
        lock(&entry.inner).asking = Some(task.abort_handle());
        // `begin` put the entry in the table before the task existed. A cancel, an unpairing or a
        // shutdown that came in that gap found no question to take back, so it is taken back here.
        if !matches!(*entry.stage.borrow(), Stage::Deciding) {
            entry.stop_asking();
        }
    }

    /// Watches an accepted transfer until it ends, and tells the application how it did. A transfer
    /// that no attempt has been going for an hour is given up.
    fn keep(&self, entry: &Arc<Entry>, transfer: Transfer) {
        let table = self.clone();
        let entry = Arc::clone(entry);
        drop(tokio::spawn(async move {
            // `abandoned` also returns when the transfer ends, so `closed` goes first: a transfer that
            // finished must not be given up as one that was left.
            let (outcome, left) = tokio::select! {
                biased;
                outcome = transfer.closed() => (outcome, false),
                () = transfer.abandoned(KEEP_IDLE) => {
                    let left = !transfer.is_over();
                    if left {
                        transfer.cancel().await;
                    }
                    (transfer.closed().await, left)
                }
            };
            let ending = if let Some(done) = outcome {
                entry.end(Stage::Finished(done.clone()));
                Ending::Done(done)
            } else {
                entry.end(Stage::Dropped);
                Ending::Dropped
            };
            // A transfer that was left for an hour has lost what the sender sent, and with it the
            // decision (section 8, A repeat): a sender that comes back is a new offer, and the
            // person is asked again. After a cancel or a broken rule no repeat is expected.
            let kept = if left && ending == Ending::Dropped {
                Duration::ZERO
            } else {
                MEMORY
            };
            entry.forget_in(kept);
            table
                .shared
                .handler
                .ended(&entry.from, entry.offer.id, ending);
        }));
    }

    /// Answers an offer that was made before: the same offer gets the answer it got, and any other
    /// under the same id is not valid.
    async fn repeated(&self, entry: &Arc<Entry>, link: Link, offer: &Offer) -> TransferReply {
        if entry.offer != *offer {
            return declined(offer.id, DeclineReason::Invalid);
        }
        self.answer(entry, link).await
    }

    /// The answer to a copy of an offer that arrived on `link`, once the application has decided.
    async fn answer(&self, entry: &Arc<Entry>, link: Link) -> TransferReply {
        let id = entry.offer.id;
        let seq = self.shared.arrivals.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut inner = lock(&entry.inner);
            inner.newest = inner.newest.max(seq);
        }
        match decided(entry).await {
            Stage::Deciding | Stage::Dropped => declined(id, DeclineReason::Refused),
            Stage::Refused(reason) => declined(id, reason),
            Stage::Finished(_) => accepted(id),
            Stage::Accepted(transfer) => match attempt_on(entry, &transfer, link, seq) {
                Ok(()) | Err(Over::Finished) => accepted(id),
                Err(Over::Dropped) => declined(id, DeclineReason::Refused),
            },
        }
    }

    /// Gives an entry up, whatever stage it is at, and waits for its lanes to be let go.
    async fn give_up(&self, entry: &Arc<Entry>) {
        match entry.stage() {
            Stage::Deciding => {
                if entry.decide(Stage::Dropped) {
                    entry.stop_asking();
                    entry.forget_in(MEMORY);
                }
            }
            Stage::Accepted(transfer) => {
                if !transfer.is_over() {
                    transfer.cancel().await;
                }
            }
            Stage::Finished(_) | Stage::Refused(_) | Stage::Dropped => {}
        }
        released(entry).await;
    }
}

/// Begins the attempt on `link` if it has none and this copy of the offer is the newest.
fn attempt_on(entry: &Entry, transfer: &Transfer, link: Link, seq: u64) -> Result<(), Over> {
    let mut inner = lock(&entry.inner);
    if inner.current.as_ref().is_some_and(|(on, _)| *on == link) || inner.newest != seq {
        return Ok(());
    }
    let attempt = transfer.begin_attempt(LANES_GRANTED)?;
    inner.current = Some((link, attempt));
    Ok(())
}

/// Removes the entries whose time has come.
fn forget_the_forgotten(entries: &mut HashMap<Key, Arc<Entry>>, now: Instant) {
    entries.retain(|_, entry| !entry.is_forgotten(now));
}

/// Removes the entry that ended first, to make room. Says whether there was one.
fn forget_the_oldest_ended(entries: &mut HashMap<Key, Arc<Entry>>) -> bool {
    let oldest = entries
        .iter()
        .filter_map(|(key, entry)| lock(&entry.inner).forget_at.map(|at| (at, *key)))
        .min_by_key(|(at, _)| *at);
    oldest.is_some_and(|(_, key)| entries.remove(&key).is_some())
}

#[cfg(test)]
mod tests {
    use std::{fs, pin::Pin, task::Poll};

    use baton_proto::{Platform, transfer::list::ListEntry};
    use tokio::{
        io::{AsyncWriteExt, duplex},
        sync::oneshot,
        task::JoinHandle,
    };

    use super::*;
    use crate::transfer::{
        receive::LaneEnd,
        testing::{bytes, chunk, destination, end_with, file, lane, list, offer_for, span},
    };

    /// The longest a test waits for something that has to happen.
    const WAIT: Duration = Duration::from_secs(20);

    /// The device with this number. No two numbers share an id.
    fn device_n(n: usize) -> PairedDevice {
        let mut id = [0; DeviceId::LEN];
        id[..8].copy_from_slice(&u64::try_from(n + 1).unwrap().to_le_bytes());
        PairedDevice {
            id: DeviceId::from_bytes(id),
            name: format!("device {n}"),
            local_name: None,
            name_changed: false,
            platform: Platform::Linux,
            added_at: 0,
            provenance: crate::Provenance::Paired,
            last_seen_at: None,
            relay: None,
            addrs: Vec::new(),
            policy: crate::DevicePolicy::default(),
        }
    }

    fn device(n: u8) -> PairedDevice {
        device_n(usize::from(n))
    }

    /// The id of the transfer `n`.
    fn id(n: u8) -> TransferId {
        TransferId::from_bytes([n; TransferId::LEN])
    }

    /// `x` (10 bytes) and `zero` (empty), in the order of the list.
    fn entries() -> Vec<ListEntry> {
        vec![file("x", 10), file("zero", 0)]
    }

    /// The offer of the sample under the id of the transfer `n`.
    fn offer(n: u8) -> Offer {
        Offer {
            id: id(n),
            ..offer_for(&entries())
        }
    }

    /// The offer of the sample under an id made of `n`, for the tests that need very many.
    fn numbered(n: usize) -> Offer {
        let mut id = [0; TransferId::LEN];
        id[..8].copy_from_slice(&u64::try_from(n).unwrap().to_le_bytes());
        Offer {
            id: TransferId::from_bytes(id),
            ..offer_for(&entries())
        }
    }

    /// The `Finish` of the sample under the id of the transfer `n`.
    fn finish(n: u8) -> Finish {
        Finish {
            id: id(n),
            files: 2,
            bytes: 10,
        }
    }

    /// One lane that carries all of the sample: the list, and then each file.
    fn everything() -> Vec<u8> {
        let x = bytes(1, 10);
        lane(&[
            &list(&chunk(entries())),
            &span(0, 0, &x),
            &end_with(0, blake3::hash(&x)),
            &end_with(1, blake3::hash(b"")),
        ])
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

    /// Lets the timers that are due fire and the tasks that wait for them go on. With the clock
    /// paused, this moves it by a millisecond and no more.
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }

    /// Whether a task that is asked for something has not answered after a little while.
    async fn still_waiting<T>(task: &mut JoinHandle<T>) -> bool {
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .is_err()
    }

    /// Polls `future` once, and gives what it gave if it was ready. A test that needs two requests
    /// to be answered in an order of its own polls them by hand.
    async fn poll_once<F: Future + Unpin>(future: &mut F) -> Option<F::Output> {
        std::future::poll_fn(|cx| match Pin::new(&mut *future).poll(cx) {
            Poll::Ready(value) => Poll::Ready(Some(value)),
            Poll::Pending => Poll::Ready(None),
        })
        .await
    }

    /// A question to the person that waits for the answer.
    type Question = (TransferId, oneshot::Sender<Verdict>);

    /// Counts a question that was taken back before it was answered.
    struct Withdrawn<'a> {
        count: &'a AtomicUsize,
        answered: bool,
    }

    impl Drop for Withdrawn<'_> {
        fn drop(&mut self) {
            if !self.answered {
                self.count.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// The application, played by the test.
    struct Fake {
        destination: Mutex<Option<Destination>>,
        limit: Mutex<Option<u64>>,
        /// What the person says at once. Without it, a question waits for the test to answer it.
        verdict: Mutex<Option<Verdict>>,
        waiting: Mutex<Vec<Question>>,
        asked: AtomicUsize,
        withdrawn: AtomicUsize,
        breaks: AtomicBool,
        /// A table to shut down when it asks for the limit on a transfer, which is after the offer
        /// has looked at the table and before it takes a place there.
        closing: Mutex<Option<Incoming>>,
        ended: Mutex<Vec<(DeviceId, TransferId, Ending)>>,
    }

    impl Fake {
        fn new(destination: Destination) -> Self {
            Self {
                destination: Mutex::new(Some(destination)),
                limit: Mutex::new(None),
                verdict: Mutex::new(None),
                waiting: Mutex::new(Vec::new()),
                asked: AtomicUsize::new(0),
                withdrawn: AtomicUsize::new(0),
                breaks: AtomicBool::new(false),
                closing: Mutex::new(None),
                ended: Mutex::new(Vec::new()),
            }
        }

        /// From now on the person answers at once.
        fn says(&self, verdict: Verdict) {
            *lock(&self.verdict) = Some(verdict);
        }

        /// How many times the person was asked.
        fn questions(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }

        /// How many questions were taken back before they were answered.
        fn withdrawn(&self) -> usize {
            self.withdrawn.load(Ordering::SeqCst)
        }

        /// How many questions wait for an answer.
        fn waiting(&self) -> usize {
            lock(&self.waiting)
                .iter()
                .filter(|(_, answer)| !answer.is_closed())
                .count()
        }

        async fn until_waiting(&self, count: usize) {
            until("the questions", || self.waiting() == count).await;
        }

        /// The person answers the question about the transfer `id`, which must be waiting.
        fn answer(&self, id: TransferId, verdict: Verdict) {
            let mut waiting = lock(&self.waiting);
            let at = waiting
                .iter()
                .position(|(asked, answer)| *asked == id && !answer.is_closed())
                .expect("a question about the transfer");
            let (_, answer) = waiting.remove(at);
            answer.send(verdict).expect("the question is still there");
        }

        /// What the application was told about the transfers that ended.
        fn endings(&self) -> Vec<(DeviceId, TransferId, Ending)> {
            lock(&self.ended).clone()
        }

        async fn until_ended(&self, count: usize) {
            until("the transfers to end", || lock(&self.ended).len() >= count).await;
        }
    }

    impl TransferHandler for Fake {
        fn destination(&self) -> Option<Destination> {
            lock(&self.destination).clone()
        }

        fn limit_bytes(&self) -> Option<u64> {
            if let Some(table) = lock(&self.closing).take() {
                table.shared.closed.store(true, Ordering::Release);
            }
            *lock(&self.limit)
        }

        fn offered<'a>(&'a self, _: &'a PairedDevice, offer: &'a Offer) -> BoxFuture<'a, Verdict> {
            Box::pin(async move {
                self.asked.fetch_add(1, Ordering::SeqCst);
                assert!(!self.breaks.load(Ordering::SeqCst), "the application broke");
                if let Some(verdict) = *lock(&self.verdict) {
                    return verdict;
                }
                let (answer, answered) = oneshot::channel();
                lock(&self.waiting).push((offer.id, answer));
                let mut question = Withdrawn {
                    count: &self.withdrawn,
                    answered: false,
                };
                let verdict = answered.await.unwrap_or(Verdict::Refuse);
                question.answered = true;
                verdict
            })
        }

        fn ended(&self, from: &PairedDevice, id: TransferId, ending: Ending) {
            lock(&self.ended).push((from.id, id, ending));
        }
    }

    /// A table over the application of a test, with a destination of its own.
    struct Rig {
        scratch: tempfile::TempDir,
        fake: Arc<Fake>,
        table: Incoming,
        charged: Arc<Mutex<Vec<DeviceId>>>,
    }

    impl Rig {
        fn new() -> Self {
            Self::allowing(|| true)
        }

        /// A table whose limit on items says `allow` for each new transfer.
        fn allowing(allow: impl Fn() -> bool + Send + Sync + 'static) -> Self {
            let (scratch, destination) = destination();
            let fake = Arc::new(Fake::new(destination));
            let charged = Arc::new(Mutex::new(Vec::new()));
            let table = Incoming::new(Arc::clone(&fake) as Arc<dyn TransferHandler>, {
                let charged = Arc::clone(&charged);
                move |from: &DeviceId| {
                    lock(&charged).push(*from);
                    allow()
                }
            });
            Self {
                scratch,
                fake,
                table,
                charged,
            }
        }

        /// The devices that were charged an item, in order.
        fn charged(&self) -> Vec<DeviceId> {
            lock(&self.charged).clone()
        }

        /// Has the offer made by a task of its own, as the session does for each request.
        fn offering(
            &self,
            from: &PairedDevice,
            link: usize,
            offer: Offer,
        ) -> JoinHandle<TransferReply> {
            let table = self.table.clone();
            let from = from.clone();
            tokio::spawn(async move { table.offer(&from, Link(link), offer).await })
        }

        /// Has the `Finish` of the sample answered by a task of its own.
        fn finishing(&self, from: &DeviceId, n: u8) -> JoinHandle<Result<Done, FinishError>> {
            let table = self.table.clone();
            let from = *from;
            tokio::spawn(async move { table.finish(&from, &finish(n)).await })
        }

        /// Has the `Cancel` answered by a task of its own.
        fn cancelling(&self, from: &DeviceId, n: u8) -> JoinHandle<bool> {
            let table = self.table.clone();
            let from = *from;
            tokio::spawn(async move { table.cancel(&from, id(n)).await })
        }

        /// Takes the transfer `n`, which the person has said yes to, and sends all of it on lane 0
        /// of the attempt on connection 1. The guard of the lane is kept.
        async fn received_whole(&self, from: &PairedDevice, n: u8) -> LaneGuard {
            let reply = self.table.offer(from, Link(1), offer(n)).await;
            assert_eq!(reply, accepted(id(n)));
            let (attempt, guard) = self.table.lane(&from.id, id(n), Link(1)).unwrap();
            let data = everything();
            let mut stream = data.as_slice();
            assert_eq!(attempt.lane(0, &mut stream).await, LaneEnd::Finished);
            guard
        }

        /// Waits until the transfer has its outcome, or was given up.
        async fn until_over(&self, from: &DeviceId, n: u8) {
            let entry = self.table.known(&(*from, id(n))).unwrap();
            until("the transfer to be over", || !entry.is_going()).await;
        }

        /// What a file of the transfers holds in the destination, if it is there.
        fn received(&self, path: &str) -> Option<Vec<u8>> {
            fs::read(self.scratch.path().join(path)).ok()
        }

        /// How many staging folders of transfers that are going the destination holds. They are
        /// in a folder of the destination's own.
        fn staging_folders(&self) -> usize {
            fs::read_dir(self.scratch.path())
                .unwrap()
                .map(|entry| entry.unwrap())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".baton-incoming-")
                })
                .map(|holder| fs::read_dir(holder.path()).unwrap().count())
                .sum()
        }

        /// How many entries the table holds.
        fn held(&self) -> usize {
            lock(&self.table.shared.entries).len()
        }
    }

    // An offer.

    #[tokio::test]
    async fn a_device_with_no_folder_takes_no_files_and_declines_as_unsupported() {
        let rig = Rig::new();
        *lock(&rig.fake.destination) = None;

        let reply = rig.table.offer(&device(2), Link(1), offer(1)).await;

        assert!(!rig.table.takes_files());
        assert_eq!(reply, declined(id(1), DeclineReason::Unsupported));
        assert_eq!(rig.fake.questions(), 0);
        assert_eq!(rig.charged(), []);
    }

    #[tokio::test]
    async fn an_offer_waits_for_the_person_and_is_taken_with_four_lanes() {
        let rig = Rig::new();
        let from = device(2);
        let mut asking = rig.offering(&from, 1, offer(1));
        rig.fake.until_waiting(1).await;
        assert!(still_waiting(&mut asking).await);

        rig.fake.answer(id(1), Verdict::Accept);

        let reply = within("the reply", asking).await.unwrap();
        assert_eq!(
            reply,
            TransferReply::Accepted(Accepted {
                id: id(1),
                lanes: 4
            })
        );
        assert!(rig.table.takes_files());
        assert_eq!(rig.fake.questions(), 1);
        assert_eq!(rig.charged(), [from.id]);
    }

    #[tokio::test]
    async fn a_repeat_of_an_offer_is_answered_from_memory_and_not_checked_again() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        let first = rig.table.offer(&from, Link(1), offer(1)).await;

        // The checks that come before the question are not made again for a repeat.
        *lock(&rig.fake.limit) = Some(1);
        let again = rig.table.offer(&from, Link(1), offer(1)).await;

        assert_eq!(first, accepted(id(1)));
        assert_eq!(again, first);
        assert_eq!(rig.fake.questions(), 1);
        assert_eq!(rig.charged(), [from.id]);
    }

    #[tokio::test]
    async fn another_offer_under_the_same_id_is_invalid_while_the_person_is_asked_and_after() {
        let rig = Rig::new();
        let from = device(2);
        let mut other = offer(1);
        other.summary.bytes += 1;
        let first = rig.offering(&from, 1, offer(1));
        rig.fake.until_waiting(1).await;

        let while_asking = rig.table.offer(&from, Link(1), other.clone()).await;
        rig.fake.answer(id(1), Verdict::Accept);
        within("the first offer", first).await.unwrap();
        let after = rig.table.offer(&from, Link(1), other).await;

        assert_eq!(while_asking, declined(id(1), DeclineReason::Invalid));
        assert_eq!(after, declined(id(1), DeclineReason::Invalid));
        assert_eq!(rig.fake.questions(), 1);
    }

    #[tokio::test]
    async fn copies_of_an_offer_wait_for_one_answer_and_the_newest_begins_the_attempt() {
        let rig = Rig::new();
        let from = device(2);
        // The copies are polled by hand, so that the older one is answered after the newer one.
        let mut older = Box::pin(rig.table.offer(&from, Link(1), offer(1)));
        within("the question", async {
            while rig.fake.waiting() == 0 {
                assert!(poll_once(&mut older).await.is_none());
                settle().await;
            }
        })
        .await;
        let mut newer = Box::pin(rig.table.offer(&from, Link(2), offer(1)));
        assert!(poll_once(&mut newer).await.is_none());

        rig.fake.answer(id(1), Verdict::Accept);
        settle().await;
        let newer_reply = poll_once(&mut newer).await;
        let older_reply = poll_once(&mut older).await;

        assert_eq!(newer_reply, Some(accepted(id(1))));
        assert_eq!(older_reply, Some(accepted(id(1))));
        assert_eq!(rig.fake.questions(), 1);
        assert!(rig.table.lane(&from.id, id(1), Link(2)).is_some());
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
    }

    #[tokio::test]
    async fn a_request_that_is_dropped_leaves_the_question_where_it_was() {
        let rig = Rig::new();
        let from = device(2);
        let asking = rig.offering(&from, 1, offer(1));
        rig.fake.until_waiting(1).await;

        // The connection goes, and with it the request.
        asking.abort();
        let _ = asking.await;
        settle().await;
        assert_eq!(rig.fake.withdrawn(), 0);
        assert_eq!(rig.fake.waiting(), 1);

        // The sender comes back on a new connection and repeats the offer: the same question.
        let again = rig.offering(&from, 2, offer(1));
        settle().await;
        rig.fake.answer(id(1), Verdict::Accept);
        let reply = within("the reply", again).await.unwrap();

        assert_eq!(reply, accepted(id(1)));
        assert_eq!(rig.fake.questions(), 1);
        assert!(rig.table.lane(&from.id, id(1), Link(2)).is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_is_remembered_for_ten_minutes() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Refuse);
        let from = device(2);
        let refused = declined(id(1), DeclineReason::Refused);

        assert_eq!(rig.table.offer(&from, Link(1), offer(1)).await, refused);
        tokio::time::advance(Duration::from_mins(9)).await;
        assert_eq!(rig.table.offer(&from, Link(1), offer(1)).await, refused);
        assert_eq!(rig.fake.questions(), 1);

        tokio::time::advance(Duration::from_mins(2)).await;
        rig.fake.says(Verdict::Accept);
        let reply = rig.table.offer(&from, Link(1), offer(1)).await;

        assert_eq!(reply, accepted(id(1)));
        assert_eq!(rig.fake.questions(), 2);
        // A transfer that was never taken has no end to tell the application of.
        assert_eq!(rig.fake.endings(), []);
    }

    #[tokio::test(start_paused = true)]
    async fn a_policy_that_blocks_is_told_as_blocked_and_is_remembered_like_a_refusal() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Block);
        let from = device(2);
        let blocked = declined(id(1), DeclineReason::Blocked);

        assert_eq!(rig.table.offer(&from, Link(1), offer(1)).await, blocked);
        assert_eq!(rig.table.offer(&from, Link(1), offer(1)).await, blocked);
        assert_eq!(rig.fake.questions(), 1);
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_person_who_does_not_answer_in_two_minutes_has_refused() {
        let rig = Rig::new();
        let from = device(2);
        let mut asking = rig.offering(&from, 1, offer(1));
        rig.fake.until_waiting(1).await;

        tokio::time::advance(Duration::from_secs(110)).await;
        assert!(still_waiting(&mut asking).await);
        tokio::time::advance(Duration::from_secs(20)).await;
        let reply = within("the reply", asking).await.unwrap();

        assert_eq!(reply, declined(id(1), DeclineReason::Refused));
        // The question is taken back, and the same offer is refused without asking again.
        until("the question to be taken back", || {
            rig.fake.withdrawn() == 1
        })
        .await;
        let again = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(again, declined(id(1), DeclineReason::Refused));
        assert_eq!(rig.fake.questions(), 1);
    }

    #[tokio::test]
    async fn an_application_that_breaks_leaves_the_offer_refused() {
        let rig = Rig::new();
        rig.fake.breaks.store(true, Ordering::SeqCst);
        let from = device(2);

        let reply = rig.table.offer(&from, Link(1), offer(1)).await;

        assert_eq!(reply, declined(id(1), DeclineReason::Refused));
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
    }

    // What is checked before the person is asked.

    #[tokio::test]
    async fn an_offer_over_the_limit_is_too_large_and_not_remembered() {
        let rig = Rig::new();
        *lock(&rig.fake.limit) = Some(9);
        let from = device(2);

        let reply = rig.table.offer(&from, Link(1), offer(1)).await;

        assert_eq!(reply, declined(id(1), DeclineReason::TooLarge));
        assert_eq!(rig.fake.questions(), 0);
        assert_eq!(rig.charged(), []);

        // The size of the transfer is allowed, and the offer is asked about now.
        *lock(&rig.fake.limit) = Some(10);
        rig.fake.says(Verdict::Accept);
        let again = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(again, accepted(id(1)));
    }

    #[cfg(not(target_vendor = "apple"))]
    #[tokio::test]
    async fn an_offer_that_leaves_no_room_is_no_space_and_not_remembered() {
        let rig = Rig::new();
        let from = device(2);
        let mut huge = offer(1);
        huge.summary.bytes = 1 << 62;

        let reply = rig.table.offer(&from, Link(1), huge).await;

        assert_eq!(reply, declined(id(1), DeclineReason::NoSpace));
        assert_eq!(rig.fake.questions(), 0);
        assert_eq!(rig.charged(), []);

        // The same id again with what fits: the first answer left nothing behind.
        rig.fake.says(Verdict::Accept);
        let again = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(again, accepted(id(1)));
    }

    #[tokio::test]
    async fn a_device_has_two_transfers_going_and_a_third_is_rate_limited() {
        let rig = Rig::new();
        let from = device(2);
        let first = rig.offering(&from, 1, offer(1));
        let second = rig.offering(&from, 1, offer(2));
        rig.fake.until_waiting(2).await;

        // Offers that wait for a person are going. Nobody is asked about the third, and it is not
        // remembered.
        let third = rig.table.offer(&from, Link(1), offer(3)).await;
        assert_eq!(third, declined(id(3), DeclineReason::RateLimited));
        assert_eq!(rig.fake.questions(), 2);
        assert_eq!(rig.charged(), [from.id, from.id]);

        // Another device has transfers of its own.
        let other = rig.offering(&device(3), 2, offer(3));
        rig.fake.until_waiting(3).await;

        // One that is over makes room for the next.
        rig.fake.answer(id(1), Verdict::Refuse);
        within("the first", first).await.unwrap();
        let third = rig.offering(&from, 1, offer(3));
        rig.fake.until_waiting(3).await;
        assert_eq!(rig.fake.questions(), 4);

        rig.fake.answer(id(2), Verdict::Refuse);
        rig.fake.answer(id(3), Verdict::Refuse);
        rig.fake.answer(id(3), Verdict::Refuse);
        for task in [second, other, third] {
            within("the reply", task).await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_transfer_that_was_taken_is_going_until_it_has_ended() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        rig.table.offer(&from, Link(1), offer(2)).await;
        let third = rig.table.offer(&from, Link(1), offer(3)).await;
        assert_eq!(third, declined(id(3), DeclineReason::RateLimited));

        let guard = rig.received_whole(&from, 1).await;
        drop(guard);
        let done = rig.table.finish(&from.id, &finish(1)).await.unwrap();
        let third = rig.table.offer(&from, Link(1), offer(3)).await;

        assert!(done.failed.is_empty() && done.more_failed == 0, "{done:?}");
        assert_eq!(third, accepted(id(3)));
    }

    #[tokio::test]
    async fn a_new_transfer_is_charged_once_and_a_decline_for_the_limit_is_not_remembered() {
        let allow = Arc::new(AtomicBool::new(false));
        let rig = Rig::allowing({
            let allow = Arc::clone(&allow);
            move || allow.load(Ordering::SeqCst)
        });
        rig.fake.says(Verdict::Accept);
        let from = device(2);

        let over = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(over, declined(id(1), DeclineReason::RateLimited));
        assert_eq!(rig.fake.questions(), 0);

        allow.store(true, Ordering::SeqCst);
        let taken = rig.table.offer(&from, Link(1), offer(1)).await;
        let again = rig.table.offer(&from, Link(1), offer(1)).await;

        assert_eq!(taken, accepted(id(1)));
        assert_eq!(again, taken);
        // The first was charged and turned away, and the second was charged and taken. The repeat
        // is the same transfer and costs nothing.
        assert_eq!(rig.charged(), [from.id, from.id]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_table_holds_no_more_than_its_capacity_and_forgets_the_oldest_refusal_first() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Refuse);
        let from = device(2);
        for n in 0..=CAPACITY {
            rig.table.offer(&from, Link(1), numbered(n)).await;
            tokio::time::advance(Duration::from_millis(1)).await;
        }
        assert_eq!(rig.held(), CAPACITY);
        let asked = rig.fake.questions();

        // The newest refusal is remembered, and the oldest was forgotten to make room.
        rig.table.offer(&from, Link(1), numbered(CAPACITY)).await;
        assert_eq!(rig.fake.questions(), asked);
        rig.table.offer(&from, Link(1), numbered(0)).await;
        assert_eq!(rig.fake.questions(), asked + 1);
        assert_eq!(rig.held(), CAPACITY);
    }

    #[tokio::test]
    async fn a_table_full_of_transfers_that_are_going_turns_a_new_one_away() {
        let rig = Rig::new();
        // Two for each of many devices, so that no device is over its own limit.
        for n in 0..CAPACITY {
            drop(rig.offering(&device_n(n / 2), 1, numbered(n)));
        }
        rig.fake.until_waiting(CAPACITY).await;

        let reply = rig
            .table
            .offer(&device_n(CAPACITY), Link(1), numbered(CAPACITY))
            .await;

        assert_eq!(rig.held(), CAPACITY);
        assert_eq!(
            reply,
            declined(numbered(CAPACITY).id, DeclineReason::RateLimited)
        );
    }

    // The lanes.

    #[tokio::test]
    async fn a_lane_is_given_only_for_the_transfer_the_peer_and_the_connection_of_the_attempt() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;

        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_some());
        assert!(
            rig.table.lane(&from.id, id(1), Link(2)).is_none(),
            "another connection"
        );
        assert!(
            rig.table.lane(&device(3).id, id(1), Link(1)).is_none(),
            "another device"
        );
        assert!(
            rig.table.lane(&from.id, id(2), Link(1)).is_none(),
            "another transfer"
        );
    }

    #[tokio::test]
    async fn no_lane_is_given_for_an_offer_that_waits_or_was_turned_down() {
        let rig = Rig::new();
        let from = device(2);
        let waiting = rig.offering(&from, 1, offer(1));
        rig.fake.until_waiting(1).await;
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());

        rig.fake.answer(id(1), Verdict::Refuse);
        within("the reply", waiting).await.unwrap();

        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
    }

    #[tokio::test]
    async fn an_offer_on_a_new_connection_ends_the_attempt_before_and_on_the_same_one_changes_nothing()
     {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let (old, guard) = rig.table.lane(&from.id, id(1), Link(1)).unwrap();
        let before = format!("{old:?}");
        let (_sender, mut stream) = duplex(1024);
        let reading = tokio::spawn(async move {
            let end = old.lane(0, &mut stream).await;
            drop(guard);
            end
        });

        // The same connection again: the attempt stays, and its lane is not touched.
        rig.table.offer(&from, Link(1), offer(1)).await;
        let (same, _guard) = rig.table.lane(&from.id, id(1), Link(1)).unwrap();
        settle().await;
        assert_eq!(format!("{same:?}"), before);
        assert!(!reading.is_finished());

        // A new connection: the lanes of the old attempt are stopped with 499.
        let reply = rig.table.offer(&from, Link(2), offer(1)).await;
        assert_eq!(reply, accepted(id(1)));
        let end = within("the old lane", reading).await.unwrap();
        assert_eq!(end, LaneEnd::Stopped(499));
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
        let (new, _guard) = rig.table.lane(&from.id, id(1), Link(2)).unwrap();
        assert_ne!(format!("{new:?}"), before);
    }

    // The end of a transfer.

    #[tokio::test]
    async fn a_transfer_that_arrives_is_finished_with_one_outcome_and_the_application_hears_once() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        let guard = rig.received_whole(&from, 1).await;
        drop(guard);

        let done = rig.table.finish(&from.id, &finish(1)).await.unwrap();
        let again = rig.table.finish(&from.id, &finish(1)).await.unwrap();

        assert_eq!(done.id, id(1));
        assert!(done.failed.is_empty() && done.more_failed == 0, "{done:?}");
        assert_eq!(again, done);
        assert_eq!(rig.received("x"), Some(bytes(1, 10)));
        assert_eq!(rig.received("zero"), Some(Vec::new()));
        rig.fake.until_ended(1).await;
        settle().await;
        assert_eq!(rig.fake.endings(), [(from.id, id(1), Ending::Done(done))]);

        // The offer repeated is told as before, and no lane is given any more.
        let repeat = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(repeat, accepted(id(1)));
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
        assert_eq!(rig.fake.questions(), 1);
    }

    #[tokio::test]
    async fn a_cancel_after_the_end_leaves_what_is_in_place() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        drop(rig.received_whole(&from, 1).await);
        let done = rig.table.finish(&from.id, &finish(1)).await.unwrap();

        rig.table.cancel(&from.id, id(1)).await;

        assert_eq!(rig.table.finish(&from.id, &finish(1)).await, Ok(done));
        assert_eq!(rig.received("x"), Some(bytes(1, 10)));
        rig.fake.until_ended(1).await;
        assert!(matches!(rig.fake.endings()[0].2, Ending::Done(_)));
    }

    #[tokio::test]
    async fn finish_for_a_transfer_that_is_not_there_is_not_found() {
        let rig = Rig::new();
        let from = device(2);
        let not_found = Err(FinishError::NotFound);

        // Never heard of.
        assert_eq!(rig.table.finish(&from.id, &finish(1)).await, not_found);

        // Waiting for the person.
        let asking = rig.offering(&from, 1, offer(2));
        rig.fake.until_waiting(1).await;
        assert_eq!(rig.table.finish(&from.id, &finish(2)).await, not_found);

        // Turned down.
        rig.fake.answer(id(2), Verdict::Refuse);
        within("the reply", asking).await.unwrap();
        assert_eq!(rig.table.finish(&from.id, &finish(2)).await, not_found);

        // Taken, but offered by another device.
        rig.fake.says(Verdict::Accept);
        rig.table.offer(&from, Link(1), offer(3)).await;
        assert_eq!(rig.table.finish(&device(3).id, &finish(3)).await, not_found);
    }

    #[tokio::test]
    async fn a_finish_that_waits_is_answered_not_found_when_the_transfer_is_given_up() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let mut finishing = rig.finishing(&from.id, 1);
        assert!(still_waiting(&mut finishing).await);

        rig.table.cancel(&from.id, id(1)).await;

        let reply = within("the finish", finishing).await.unwrap();
        assert_eq!(reply, Err(FinishError::NotFound));
    }

    #[tokio::test(start_paused = true)]
    async fn a_finish_that_the_files_do_not_settle_for_is_ended_after_three_minutes() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let mut finishing = rig.finishing(&from.id, 1);
        settle().await;

        tokio::time::advance(Duration::from_secs(170)).await;
        assert!(still_waiting(&mut finishing).await);
        tokio::time::advance(Duration::from_secs(20)).await;
        let reply = within("the finish", finishing).await.unwrap();

        assert_eq!(reply, Err(FinishError::TimedOut));
        // The transfer is not over: the sender asks again.
        assert_eq!(rig.fake.endings(), []);
    }

    // A rule that the sender breaks.

    #[tokio::test]
    async fn a_rule_the_sender_breaks_gives_the_transfer_up_and_nothing_of_it_is_kept() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let (attempt, _guard) = rig.table.lane(&from.id, id(1), Link(1)).unwrap();

        // The list, and then a span of a file that is not in it.
        let data = lane(&[&list(&chunk(entries())), &span(7, 0, b"?")]);
        let end = attempt.lane(0, &mut data.as_slice()).await;

        assert_eq!(end, LaneEnd::Stopped(400));
        rig.fake.until_ended(1).await;
        assert_eq!(rig.fake.endings(), [(from.id, id(1), Ending::Dropped)]);
        let finished = rig.table.finish(&from.id, &finish(1)).await;
        assert_eq!(finished, Err(FinishError::NotFound));
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
        let repeat = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(repeat, declined(id(1), DeclineReason::Refused));
        assert_eq!(rig.staging_folders(), 0);
    }

    // Cancel.

    #[tokio::test]
    async fn a_cancel_takes_the_question_back_and_the_same_offer_is_refused_for_a_while() {
        let rig = Rig::new();
        let from = device(2);
        let asking = rig.offering(&from, 1, offer(1));
        rig.fake.until_waiting(1).await;

        assert!(rig.table.cancel(&from.id, id(1)).await);

        let reply = within("the reply", asking).await.unwrap();
        assert_eq!(reply, declined(id(1), DeclineReason::Refused));
        until("the question to be taken back", || {
            rig.fake.withdrawn() == 1
        })
        .await;
        let again = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(again, declined(id(1), DeclineReason::Refused));
        assert_eq!(rig.fake.questions(), 1);
        assert_eq!(rig.fake.endings(), []);
    }

    #[tokio::test]
    async fn a_cancel_that_comes_before_the_question_is_recorded_still_takes_it_back() {
        // The entry is in the table before the task that asks exists. A cancel in between finds a
        // question that is not there yet, and the entry is already given up when the task is made.
        let rig = Rig::new();
        let from = device(2);
        let Begun::New(entry) = rig.table.begin(&from, &offer(1)) else {
            panic!("the offer is a new one");
        };
        assert!(rig.table.cancel(&from.id, id(1)).await);

        rig.table.ask(&entry, rig.table.destination().unwrap());
        settle().await;

        assert_eq!(rig.fake.waiting(), 0);
        assert_eq!(rig.fake.questions(), rig.fake.withdrawn());
        let again = rig.table.offer(&from, Link(1), offer(1)).await;
        assert_eq!(again, declined(id(1), DeclineReason::Refused));
        assert_eq!(rig.fake.questions(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_offer_is_asked_about_again_when_the_memory_of_it_has_gone() {
        let rig = Rig::new();
        let from = device(2);
        let asking = rig.offering(&from, 1, offer(1));
        rig.fake.until_waiting(1).await;
        rig.table.cancel(&from.id, id(1)).await;
        within("the reply", asking).await.unwrap();

        tokio::time::advance(Duration::from_mins(11)).await;
        rig.fake.says(Verdict::Accept);
        let again = rig.table.offer(&from, Link(1), offer(1)).await;

        assert_eq!(again, accepted(id(1)));
        assert_eq!(rig.fake.questions(), 2);
    }

    #[tokio::test]
    async fn a_cancel_for_a_transfer_that_is_not_known_leaves_nothing_behind() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);

        assert!(!rig.table.cancel(&from.id, id(1)).await);
        let reply = rig.table.offer(&from, Link(1), offer(1)).await;

        assert_eq!(reply, accepted(id(1)));
        assert_eq!(rig.fake.questions(), 1);
    }

    #[tokio::test]
    async fn a_cancel_gives_up_a_transfer_that_is_arriving_and_removes_what_it_received() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let (attempt, guard) = rig.table.lane(&from.id, id(1), Link(1)).unwrap();
        let (mut sender, mut stream) = duplex(64 * 1024);
        let reading = tokio::spawn(async move {
            let end = attempt.lane(0, &mut stream).await;
            drop(guard);
            end
        });
        let started = lane(&[&list(&chunk(entries())), &span(0, 0, &bytes(1, 10))]);
        sender.write_all(&started).await.unwrap();
        until("the staging folder", || rig.staging_folders() == 1).await;

        rig.table.cancel(&from.id, id(1)).await;

        let end = within("the lane", reading).await.unwrap();
        assert_eq!(end, LaneEnd::Stopped(499));
        rig.fake.until_ended(1).await;
        assert_eq!(rig.fake.endings(), [(from.id, id(1), Ending::Dropped)]);
        assert_eq!(rig.staging_folders(), 0);
        assert_eq!(rig.received("x"), None);
        let finished = rig.table.finish(&from.id, &finish(1)).await;
        assert_eq!(finished, Err(FinishError::NotFound));
    }

    // Letting go of the lanes.

    #[tokio::test(start_paused = true)]
    async fn finish_answers_after_the_lanes_are_let_go_and_not_before() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        let guard = rig.received_whole(&from, 1).await;
        let mut finishing = rig.finishing(&from.id, 1);
        rig.until_over(&from.id, 1).await;

        // The outcome is known, and a lane still holds its stream.
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(still_waiting(&mut finishing).await);
        let letting_go = Instant::now();
        drop(guard);

        let done = within("the finish", finishing).await.unwrap().unwrap();
        assert!(done.failed.is_empty() && done.more_failed == 0, "{done:?}");
        assert!(
            letting_go.elapsed() < Duration::from_secs(1),
            "the answer waited for the time to run out"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_answers_after_the_lanes_are_let_go_and_not_before() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let (_attempt, guard) = rig.table.lane(&from.id, id(1), Link(1)).unwrap();
        let mut cancelling = rig.cancelling(&from.id, 1);
        rig.until_over(&from.id, 1).await;

        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(still_waiting(&mut cancelling).await);
        let letting_go = Instant::now();
        drop(guard);

        within("the cancel", cancelling).await.unwrap();
        assert!(
            letting_go.elapsed() < Duration::from_secs(1),
            "the answer waited for the time to run out"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_lane_that_is_never_let_go_holds_an_answer_back_for_a_few_seconds_only() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        let _stuck = rig.received_whole(&from, 1).await;
        rig.table.offer(&from, Link(1), offer(2)).await;
        let (_attempt, _other) = rig.table.lane(&from.id, id(2), Link(1)).unwrap();

        let done = within("the finish", rig.table.finish(&from.id, &finish(1))).await;
        let done = done.unwrap();
        assert!(done.failed.is_empty() && done.more_failed == 0, "{done:?}");
        within("the cancel", rig.table.cancel(&from.id, id(2))).await;
    }

    /// Takes the transfers `ns` that `from` offers, and holds a lane of each that is never let go.
    async fn with_lanes_held(rig: &Rig, from: &PairedDevice, ns: &[u8]) -> Vec<LaneGuard> {
        let mut held = Vec::new();
        for &n in ns {
            let link = Link(usize::from(n));
            assert_eq!(rig.table.offer(from, link, offer(n)).await, accepted(id(n)));
            held.push(rig.table.lane(&from.id, id(n), link).unwrap().1);
        }
        held
    }

    /// Checks that `give_up`, run on a table where two devices have two transfers each, with a lane
    /// that is never let go in every one, waits for the lanes once for all the transfers it gives up.
    async fn waits_once_for_lanes_that_are_never_let_go<Fut: Future<Output = ()>>(
        give_up: impl FnOnce(Incoming) -> Fut,
    ) {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let _held = (
            with_lanes_held(&rig, &device(2), &[1, 2]).await,
            with_lanes_held(&rig, &device(3), &[1, 2]).await,
        );
        let started = Instant::now();

        give_up(rig.table.clone()).await;

        let took = started.elapsed();
        assert!(
            took >= RELEASE_WAIT,
            "it did not wait for the lanes: {took:?}"
        );
        assert!(
            took < RELEASE_WAIT + Duration::from_secs(1),
            "it waited for the lanes of one transfer after the other: {took:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_waits_once_for_lanes_that_are_never_let_go() {
        waits_once_for_lanes_that_are_never_let_go(|table| async move { table.shutdown().await })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn unpairing_waits_once_for_lanes_that_are_never_let_go() {
        let gone = device(2).id;
        waits_once_for_lanes_that_are_never_let_go(
            |table| async move { table.forget(&gone).await },
        )
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn forgetting_the_devices_that_are_not_paired_waits_once_for_lanes_that_are_never_let_go()
    {
        waits_once_for_lanes_that_are_never_let_go(|table| async move {
            table.forget_unless(|_| false).await;
        })
        .await;
    }

    // Unpairing and stopping.

    #[tokio::test]
    async fn unpairing_a_device_gives_up_its_transfers_and_no_one_elses() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let (gone, kept) = (device(2), device(3));
        rig.table.offer(&gone, Link(1), offer(1)).await;
        rig.table.offer(&kept, Link(2), offer(1)).await;
        lock(&rig.fake.verdict).take();
        let asking = rig.offering(&gone, 1, offer(2));
        rig.fake.until_waiting(1).await;

        rig.table.forget(&gone.id).await;

        // The question is taken back, and the offer is refused.
        let reply = within("the reply", asking).await.unwrap();
        assert_eq!(reply, declined(id(2), DeclineReason::Refused));
        until("the question to be taken back", || {
            rig.fake.withdrawn() == 1
        })
        .await;
        // The transfer that was arriving is given up, with the application told.
        rig.fake.until_ended(1).await;
        assert_eq!(rig.fake.endings(), [(gone.id, id(1), Ending::Dropped)]);
        assert!(rig.table.lane(&gone.id, id(1), Link(1)).is_none());
        // The other device's transfer, which has the same id, is as it was.
        assert!(rig.table.lane(&kept.id, id(1), Link(2)).is_some());
        // And the table has forgotten the offers of the device that was unpaired: it is asked again.
        rig.fake.says(Verdict::Accept);
        let again = rig.table.offer(&gone, Link(3), offer(1)).await;
        assert_eq!(again, accepted(id(1)));
        assert_eq!(rig.fake.questions(), 4);
    }

    #[tokio::test]
    async fn stopping_gives_up_every_transfer() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let (one, two) = (device(2), device(3));
        rig.table.offer(&one, Link(1), offer(1)).await;
        rig.table.offer(&two, Link(2), offer(2)).await;

        rig.table.shutdown().await;

        rig.fake.until_ended(2).await;
        let mut endings = rig.fake.endings();
        endings.sort_by_key(|(_, id, _)| id.to_string());
        assert_eq!(
            endings,
            [
                (one.id, id(1), Ending::Dropped),
                (two.id, id(2), Ending::Dropped)
            ]
        );
        assert!(rig.table.lane(&one.id, id(1), Link(1)).is_none());
        assert_eq!(rig.held(), 0);
    }

    #[tokio::test]
    async fn the_transfers_of_devices_that_are_no_longer_paired_are_given_up() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let (gone, kept) = (device(2), device(3));
        rig.table.offer(&gone, Link(1), offer(1)).await;
        rig.table.offer(&kept, Link(2), offer(2)).await;

        rig.table.forget_unless(|peer| *peer == kept.id).await;

        rig.fake.until_ended(1).await;
        assert_eq!(rig.fake.endings(), [(gone.id, id(1), Ending::Dropped)]);
        assert!(rig.table.lane(&gone.id, id(1), Link(1)).is_none());
        assert!(rig.table.lane(&kept.id, id(2), Link(2)).is_some());
    }

    #[tokio::test]
    async fn a_table_that_was_shut_down_takes_no_more_offers() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        assert_eq!(
            rig.table.offer(&from, Link(1), offer(1)).await,
            accepted(id(1))
        );
        assert!(rig.table.takes_files());

        rig.table.shutdown().await;

        assert!(!rig.table.takes_files());
        let reply = rig.table.offer(&from, Link(1), offer(2)).await;
        assert_eq!(reply, declined(id(2), DeclineReason::Unsupported));
        assert_eq!(rig.fake.questions(), 1);
        // An offer that got past the first check before the shutdown is turned away where the
        // entries are made, and leaves nothing.
        assert!(matches!(rig.table.begin(&from, &offer(3)), Begun::Closed));
        assert_eq!(rig.held(), 0);
    }

    #[tokio::test]
    async fn an_offer_that_the_shutdown_overtakes_is_declined_as_unsupported_and_leaves_nothing() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        *lock(&rig.fake.closing) = Some(rig.table.clone());

        let reply = rig.table.offer(&device(2), Link(1), offer(1)).await;

        assert_eq!(reply, declined(id(1), DeclineReason::Unsupported));
        assert_eq!(rig.fake.questions(), 0);
        assert_eq!(rig.charged(), Vec::<DeviceId>::new());
        assert_eq!(rig.held(), 0);
    }

    #[test]
    fn minted_links_are_all_different() {
        let links: std::collections::HashSet<Link> = std::thread::scope(|scope| {
            let makers: Vec<_> = (0..4)
                .map(|_| scope.spawn(|| (0..1000).map(|_| Link::mint()).collect::<Vec<_>>()))
                .collect();
            makers
                .into_iter()
                .flat_map(|maker| maker.join().unwrap())
                .collect()
        });
        assert_eq!(links.len(), 4000);
    }

    #[tokio::test]
    async fn a_handle_to_a_handler_is_one_that_passes_everything_on() {
        let (_scratch, destination) = destination();
        let fake = Arc::new(Fake::new(destination));
        *lock(&fake.limit) = Some(7);
        fake.says(Verdict::Block);
        let handle: Arc<dyn TransferHandler> = Arc::new(Arc::clone(&fake));

        assert!(handle.destination().is_some());
        assert_eq!(handle.limit_bytes(), Some(7));
        assert_eq!(handle.offered(&device(2), &offer(1)).await, Verdict::Block);
        handle.ended(&device(2), id(1), Ending::Dropped);
        assert_eq!(fake.endings(), [(device(2).id, id(1), Ending::Dropped)]);
    }

    #[tokio::test]
    async fn an_accepted_transfer_that_is_over_is_not_going_before_its_entry_is_moved_on() {
        // The task that moves an entry on from `Accepted` is one of the table's own, and a new offer
        // can come before it has run.
        let (_scratch, destination) = destination();
        let entry = Entry::new(device(2), offer(1));
        let transfer = Transfer::new(destination, offer(1));
        assert!(entry.decide(Stage::Accepted(transfer.clone())));
        assert!(entry.is_going());

        transfer.cancel().await;

        assert!(!entry.is_going());
    }

    #[tokio::test]
    async fn an_offer_repeated_for_a_transfer_that_was_given_up_before_its_entry_moved_on_is_refused()
     {
        // The entry still says `Accepted` for the moment between the transfer being given up and the
        // task that watches it moving the entry on. A copy of the offer that comes then finds no
        // attempt to begin, and is not told that the transfer is going.
        let rig = Rig::new();
        let from = device(2);
        let entry = Arc::new(Entry::new(from, offer(1)));
        let transfer = Transfer::new(rig.table.destination().unwrap(), offer(1));
        assert!(entry.decide(Stage::Accepted(transfer.clone())));
        transfer.cancel().await;

        let reply = rig.table.answer(&entry, Link(1)).await;

        assert_eq!(reply, declined(id(1), DeclineReason::Refused));
        assert!(lock(&entry.inner).current.is_none());
    }

    // Time.

    #[tokio::test(start_paused = true)]
    async fn a_sender_that_does_not_come_back_for_an_hour_is_given_up_and_what_it_sent_removed() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let (attempt, guard) = rig.table.lane(&from.id, id(1), Link(1)).unwrap();
        let (mut sender, mut stream) = duplex(64 * 1024);
        let reading = tokio::spawn(async move {
            let end = attempt.lane(0, &mut stream).await;
            drop(guard);
            end
        });
        sender.write_all(&list(&chunk(entries()))).await.unwrap();
        until("the staging folder", || rig.staging_folders() == 1).await;

        // Thirty seconds with no byte stop the attempt, and the hour starts there. The first jump of
        // the clock only shows the timer that bytes came, and the second finds that none did since.
        tokio::time::advance(Duration::from_secs(40)).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(40)).await;
        let end = within("the lane", reading).await.unwrap();
        assert_eq!(end, LaneEnd::Stopped(408));
        tokio::time::advance(Duration::from_secs(3500)).await;
        settle().await;
        assert_eq!(rig.fake.endings(), []);
        assert_eq!(rig.staging_folders(), 1);

        tokio::time::advance(Duration::from_secs(200)).await;

        rig.fake.until_ended(1).await;
        assert_eq!(rig.fake.endings(), [(from.id, id(1), Ending::Dropped)]);
        assert_eq!(rig.staging_folders(), 0);
        assert!(rig.table.lane(&from.id, id(1), Link(1)).is_none());
        drop(sender);
    }

    #[tokio::test(start_paused = true)]
    async fn a_sender_that_comes_back_after_the_hour_is_asked_about_again() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;

        // No byte comes, and the attempt is stopped. The hour passes, and the transfer is given up.
        tokio::time::advance(Duration::from_secs(40)).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(40)).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(3500)).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(200)).await;
        rig.fake.until_ended(1).await;
        assert_eq!(rig.fake.endings(), [(from.id, id(1), Ending::Dropped)]);

        // What the decision was for is gone with it (section 8, A repeat): the same offer is a new
        // one, and the person is not told "no" for ten minutes because the sender was away.
        let again = rig.table.offer(&from, Link(2), offer(1)).await;

        assert_eq!(again, accepted(id(1)));
        assert_eq!(rig.fake.questions(), 2);
        assert!(rig.table.lane(&from.id, id(1), Link(2)).is_some());
    }

    // What the types promise.

    #[tokio::test]
    async fn nothing_the_sender_chose_is_in_a_debug_print() {
        let rig = Rig::new();
        rig.fake.says(Verdict::Accept);
        let from = device(2);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let (_attempt, guard) = rig.table.lane(&from.id, id(1), Link(1)).unwrap();

        assert_eq!(format!("{:?}", rig.table), "Incoming { .. }");
        assert_eq!(format!("{guard:?}"), "LaneGuard { .. }");
    }

    fn is_send<T: Send>(_: &T) {}

    fn is_send_and_sync<T: Send + Sync>() {}

    #[tokio::test]
    async fn the_table_and_its_futures_can_be_used_from_tasks() {
        is_send_and_sync::<Incoming>();
        is_send_and_sync::<Link>();
        let rig = Rig::new();
        let from = device(2);
        rig.fake.says(Verdict::Accept);
        rig.table.offer(&from, Link(1), offer(1)).await;
        let lane = rig.table.lane(&from.id, id(1), Link(1)).unwrap();

        is_send(&lane);
        is_send(&rig.table.offer(&from, Link(1), offer(1)));
        is_send(&rig.table.finish(&from.id, &finish(1)));
        is_send(&rig.table.cancel(&from.id, id(1)));
        is_send(&rig.table.forget(&from.id));
        is_send(&rig.table.shutdown());
    }
}
