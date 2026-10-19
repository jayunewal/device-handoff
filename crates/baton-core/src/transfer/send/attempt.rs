// SPDX-License-Identifier: Apache-2.0

//! One attempt to send a transfer, on one connection.
//!
//! An attempt follows section 8 from the offer to the outcome:
//!
//! 1. **Connect and offer.** It asks the wire for a connection that is new, makes the offer, and
//!    waits for the answer for as long as a person may take. An answer that does not come is asked
//!    for again on a new stream of the same connection, and the receiver gives the same answer, so
//!    the person is not asked twice.
//! 2. **Lanes.** It opens lane 0 and writes the list, and only then opens the other lanes. A lane
//!    that fails ends the attempt, and what that means depends on how it failed (see below).
//! 3. **Finish.** When every lane has written everything and ended, it sends `Finish` and waits for
//!    `Done`, asking again if the answer is slow. The connection is held until then, because the
//!    bytes of the lanes are still on their way.
//!
//! An attempt that is not the end of the transfer says so, and [`engine`](super::engine) decides
//! whether to try again. Everything is sent again in a new attempt, on a new connection, with the
//! offer unchanged (section 8, Resuming).
//!
//! # How a stop is read
//!
//! A lane that the receiver stops while the attempt is going tells the sender what the receiver
//! wants. A stop of 408 is a stall: the attempt is over and another may follow. A stop of 507 is a
//! full disk: no more is written, but `Finish` is still sent, because the receiver puts in place the
//! files it has checked and names the others in `Done`, and no other attempt follows. Any other code
//! ends the transfer, and no `Finish` is sent. Once `Finish` is sent the answer to it decides, and a
//! stop on a lane that has ended changes nothing, whatever its code: QUIC tells the sender of a stop
//! for as long as the end of the lane has not been acknowledged.
//!
//! # When the answer to `Finish` is not heard
//!
//! A connection that goes after `Finish` was sent may have gone after the receiver had the outcome.
//! The attempt after it asks for the outcome before it offers, on its new connection, by making the
//! `Finish` again. A receiver that has the outcome says so at once, and nothing is sent again. One
//! that does not know the transfer says so at once too, and is offered it and sent everything, as
//! after any other failure.
//!
//! A receiver that says nothing within the `probe_wait` of the policy is in one of two states, and
//! the sender cannot tell which. It is putting the files in place, and answers when it is done. Or it
//! is waiting for files that went with the old connection, and never answers, because nothing that
//! it waits for is on its way. So the attempt does not wait any longer. It offers the transfer. A
//! receiver that waits for files takes the offer as it takes any repeat, and the attempt goes on and
//! ends with its own `Finish`, as usual. A receiver that is done with the transfer accepts the offer
//! all the same and stops the lanes with 499, which is how it stops a lane of a transfer that has
//! ended, and the attempt then asks for the outcome again and waits for it as it waits for any
//! `Finish`, instead of taking the stop for the receiver giving the transfer up.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use baton_proto::{
    close_code,
    id::TransferId,
    message::ErrorCode,
    transfer::{
        Cancel, Finish, TransferReply, TransferRequest, hash::span_count, lane::SkipReason,
    },
};
use tokio::{sync::oneshot, task::JoinSet, time::timeout};
use tokio_util::sync::CancellationToken;

use super::{
    lane::{self, Halt, ListFrames, Listing, Plan, ResetCode},
    outcome::{Error, Outcome, Sent, Why},
    policy::Policy,
    produce::Producer,
    progress::{Stage, Tracker},
    selection::Selection,
    wire::{Link, Reply, Wire, WireError},
};
use crate::util::lock;

/// How an attempt ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Over {
    /// The transfer is over.
    Final(Outcome),
    /// Another attempt may do better.
    Retry(Retry),
}

/// An attempt that failed in a way that another may not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Retry {
    pub(crate) why: Why,
    /// Whether the attempt got files moving: it finished writing at least one chunk of file data to
    /// the connection.
    pub(crate) moved: bool,
}

/// What the lanes of an attempt came to, when the transfer is not over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    /// Every lane wrote everything and ended.
    Written,
    /// The receiver ran out of room, and `Finish` is still to be sent.
    NoRoom,
}

/// What came of asking.
enum Answer {
    Reply(Reply),
    /// Every try went unanswered.
    Silent,
    Lost,
    Refused(u32),
    Cancelled,
}

/// What a `Finish` said, and what the outcome is made of when the receiver answers it.
#[derive(Debug, Clone)]
struct Unanswered {
    files: u32,
    bytes: u64,
    skipped: Vec<(u32, SkipReason)>,
}

impl Unanswered {
    /// The `Finish` for the transfer `id` that says this.
    fn request(&self, id: TransferId) -> TransferRequest {
        TransferRequest::Finish(Finish {
            id,
            files: self.files,
            bytes: self.bytes,
        })
    }
}

/// What an attempt leaves for the next.
#[derive(Debug, Default)]
struct Memory {
    /// Whether the receiver may know the transfer: an offer was made.
    offered: bool,
    unanswered: Option<Unanswered>,
}

/// A transfer that is being sent: what every attempt at it shares. See the module documentation.
pub(crate) struct Job {
    wire: Arc<dyn Wire>,
    selection: Arc<Selection>,
    list: Arc<ListFrames>,
    id: TransferId,
    pub(crate) policy: Policy,
    pub(crate) cancel: CancellationToken,
    pub(crate) tracker: Arc<Tracker>,
    memory: Mutex<Memory>,
}

impl Job {
    pub(crate) fn new(
        wire: Arc<dyn Wire>,
        selection: Arc<Selection>,
        list: Arc<ListFrames>,
        id: TransferId,
        policy: Policy,
        cancel: CancellationToken,
        tracker: Arc<Tracker>,
    ) -> Self {
        Self {
            wire,
            selection,
            list,
            id,
            policy,
            cancel,
            tracker,
            memory: Mutex::new(Memory::default()),
        }
    }

    /// Whether the receiver may know the transfer by now.
    pub(crate) fn offered(&self) -> bool {
        lock(&self.memory).offered
    }

    /// Carries the transfer as far as it goes on one connection.
    pub(crate) async fn attempt(&self) -> Over {
        self.tracker.stage(Stage::Connecting);
        let link = match self.connect().await {
            Ok(link) => link,
            Err(over) => return over,
        };
        let earlier = match self.probe(&*link).await {
            Ok(earlier) => earlier,
            Err(over) => return over,
        };
        self.tracker.stage(Stage::Asking);
        let allowed = match self.offer(&*link).await {
            Ok(allowed) => allowed,
            Err(over) => return over,
        };
        self.tracker.stage(Stage::Sending);
        let producer = Arc::new(Producer::new(Arc::clone(&self.selection)));
        let after = match (self.send(&link, allowed, &producer).await, &earlier) {
            (Ok(after), _) => after,
            // The receiver says that the transfer is over for new lanes, and it may be over because
            // it is done: the `Finish` that was never answered says which.
            (Err(Over::Final(Outcome::Stopped(close_code::CANCELLED))), Some(told)) => {
                return self.outcome_after_stop(&*link, told).await;
            }
            (Err(over), _) => return over,
        };
        self.tracker.stage(Stage::Finishing);
        self.finish(&*link, after, &producer).await
    }

    async fn connect(&self) -> Result<Arc<dyn Link>, Over> {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(Over::Final(self.cancelled_without_link().await)),
            connected = self.wire.connect() => match connected {
                Ok(link) => Ok(Arc::from(link)),
                Err(error) => Err(self.over(error)),
            },
        }
    }

    /// Asks for the outcome of a `Finish` that was never answered, before anything is offered.
    ///
    /// `Err` is for an attempt that is over: the receiver gave the outcome, or something that ends
    /// the attempt. `Ok(None)` is for a receiver that has no outcome to give, and the attempt goes
    /// on. `Ok(Some(_))` is for one that said nothing in time, which is the `Finish` it was asked
    /// about, for the attempt to ask about again if it has to.
    ///
    /// It is the same `Finish`, but it is not waited for as one is. See the module documentation.
    async fn probe(&self, link: &dyn Link) -> Result<Option<Unanswered>, Over> {
        let Some(told) = lock(&self.memory).unanswered.clone() else {
            return Ok(None);
        };
        let answer = self
            .ask(link, &told.request(self.id), self.policy.probe_wait, 1)
            .await;
        if matches!(answer, Answer::Silent) {
            return Ok(Some(told));
        }
        if let Some(over) = self.ended(link, answer, &told).await {
            return Err(over);
        }
        lock(&self.memory).unanswered = None;
        Ok(None)
    }

    /// The receiver stopped the lanes of an attempt with 499 after it said nothing to the question
    /// about the outcome of the `Finish` that `told` describes. Asks again, and waits as for any
    /// `Finish`. A receiver that has no such transfer gave it up, and that is what 499 meant.
    async fn outcome_after_stop(&self, link: &dyn Link, told: &Unanswered) -> Over {
        self.outcome(link, told)
            .await
            .unwrap_or(Over::Final(Outcome::Stopped(close_code::CANCELLED)))
    }

    /// Makes the offer, and gives the number of lanes the receiver allows.
    async fn offer(&self, link: &dyn Link) -> Result<u8, Over> {
        let request = TransferRequest::Offer(self.selection.offer(self.id));
        lock(&self.memory).offered = true;
        let answer = self
            .ask(
                link,
                &request,
                self.policy.offer_wait,
                self.policy.offer_tries,
            )
            .await;
        match answer {
            Answer::Reply(Reply::Transfer(TransferReply::Accepted(accepted)))
                if accepted.id == self.id =>
            {
                Ok(accepted.lanes)
            }
            Answer::Reply(Reply::Transfer(TransferReply::Declined(declined)))
                if declined.id == self.id =>
            {
                Err(Over::Final(Outcome::Declined(declined.reason)))
            }
            Answer::Reply(Reply::Error(ErrorCode::Internal | ErrorCode::Other)) => {
                Err(self.retry(Why::Receiver))
            }
            Answer::Reply(Reply::Error(_)) => Err(Over::Final(Outcome::Failed(Error::Protocol(
                "the offer was not understood",
            )))),
            Answer::Reply(Reply::Transfer(_)) => Err(Over::Final(Outcome::Failed(
                Error::Protocol("the answer to the offer was not about the offer"),
            ))),
            Answer::Silent => Err(self.retry(Why::Silent)),
            Answer::Lost => Err(self.retry(Why::Connection)),
            Answer::Refused(code) => Err(Over::Final(Outcome::Failed(Error::Refused(code)))),
            Answer::Cancelled => Err(Over::Final(self.cancelled(link).await)),
        }
    }

    /// Opens the lanes, writes everything, and waits for every lane to end.
    async fn send(
        &self,
        link: &Arc<dyn Link>,
        allowed: u8,
        producer: &Arc<Producer>,
    ) -> Result<After, Over> {
        let lanes = self.policy.lanes_for(allowed, pieces(&self.selection));
        let reset = ResetCode::new();
        let mut tasks: JoinSet<Result<(), Halt>> = JoinSet::new();
        let (written, mut listed) = oneshot::channel();
        tasks.spawn(lane::run(
            Arc::clone(link),
            self.id,
            self.plan(0, producer, &reset),
            Some(Listing {
                frames: Arc::clone(&self.list),
                written,
            }),
        ));
        let mut listing = true;
        loop {
            tokio::select! {
                biased;
                () = self.cancel.cancelled() => {
                    end_lanes(&mut tasks, &reset, close_code::CANCELLED).await;
                    return Err(Over::Final(self.cancelled(&**link).await));
                }
                // Lane 0 has written the list, and the others may be opened. A lane 0 that failed
                // first drops the signal, and its end is the next thing to come.
                list = &mut listed, if listing => {
                    listing = false;
                    if list.is_ok() {
                        for number in 1..lanes {
                            tasks.spawn(lane::run(
                                Arc::clone(link),
                                self.id,
                                self.plan(number, producer, &reset),
                                None,
                            ));
                        }
                    }
                }
                joined = tasks.join_next() => match joined {
                    None => return Ok(After::Written),
                    Some(Ok(Ok(()))) => {}
                    Some(Ok(Err(halt))) => {
                        end_lanes(&mut tasks, &reset, close_code::TIMEOUT).await;
                        return self.halted(halt);
                    }
                    Some(Err(error)) => match error.try_into_panic() {
                        Ok(panic) => std::panic::resume_unwind(panic),
                        // A lane is only aborted by the attempt, and not before it is over.
                        Err(_) => return Err(Over::Final(Outcome::Failed(Error::Stopped))),
                    },
                },
            }
        }
    }

    /// What a lane that did not end well makes of the attempt.
    fn halted(&self, halt: Halt) -> Result<After, Over> {
        match halt {
            Halt::Stopped(close_code::NO_SPACE) => Ok(After::NoRoom),
            Halt::Stopped(close_code::TIMEOUT) => Err(self.retry(Why::TimedOut)),
            Halt::Stopped(code) => Err(Over::Final(Outcome::Stopped(code))),
            Halt::Lost => Err(self.retry(Why::Connection)),
            Halt::Stalled => Err(self.retry(Why::Stalled)),
        }
    }

    /// What one lane is told to do.
    fn plan(&self, number: u8, producer: &Arc<Producer>, reset: &ResetCode) -> Plan {
        Plan {
            number,
            stall: self.policy.stall,
            reset: reset.clone(),
            producer: Arc::clone(producer),
            tracker: Arc::clone(&self.tracker),
        }
    }

    /// Sends `Finish`, and gives the outcome.
    ///
    /// The counts it carries are those of the files whose last span was read. After a stop with 507
    /// that can be more than were written, and the receiver, which only logs a difference, does not
    /// mind.
    async fn finish(&self, link: &dyn Link, after: After, producer: &Producer) -> Over {
        let totals = producer.totals();
        let told = Unanswered {
            files: totals.ended_files,
            bytes: totals.ended_bytes,
            skipped: producer.skipped(),
        };
        lock(&self.memory).unanswered = Some(told.clone());
        if let Some(over) = self.outcome(link, &told).await {
            return over;
        }
        lock(&self.memory).unanswered = None;
        match after {
            After::NoRoom => Over::Final(Outcome::Stopped(close_code::NO_SPACE)),
            After::Written => self.retry(Why::Forgotten),
        }
    }

    /// Makes the `Finish` that `told` describes, waits for the answer as long as a `Finish` is
    /// waited for, and gives what it makes of the attempt (see [`ended`](Self::ended)).
    async fn outcome(&self, link: &dyn Link, told: &Unanswered) -> Option<Over> {
        let answer = self
            .ask(
                link,
                &told.request(self.id),
                self.policy.finish_wait,
                self.policy.finish_tries,
            )
            .await;
        self.ended(link, answer, told).await
    }

    /// What the answer to the `Finish` that `told` describes makes of the attempt. It is the same
    /// whether the `Finish` was sent at the end of the attempt or asked about at the start of the
    /// next. `None` is for a receiver that has no transfer of this id: it gave it up, or never had
    /// room for it, and the caller decides what that means.
    async fn ended(&self, link: &dyn Link, answer: Answer, told: &Unanswered) -> Option<Over> {
        Some(match answer {
            Answer::Reply(Reply::Transfer(TransferReply::Done(done))) if done.id == self.id => {
                Over::Final(Outcome::Done(Sent {
                    done,
                    files: told.files,
                    bytes: told.bytes,
                    skipped: told.skipped.clone(),
                }))
            }
            Answer::Reply(
                Reply::Error(ErrorCode::NotFound) | Reply::Transfer(TransferReply::Declined(_)),
            ) => return None,
            Answer::Reply(Reply::Error(ErrorCode::Internal | ErrorCode::Other)) => {
                self.retry(Why::Receiver)
            }
            Answer::Reply(Reply::Error(ErrorCode::Malformed)) => Over::Final(Outcome::Failed(
                Error::Protocol("the end of the transfer was not understood"),
            )),
            Answer::Reply(Reply::Transfer(_)) => Over::Final(Outcome::Failed(Error::Protocol(
                "the answer to the end of the transfer was not about it",
            ))),
            Answer::Silent => Over::Final(Outcome::Failed(Error::NoOutcome)),
            Answer::Lost => self.retry(Why::Connection),
            Answer::Refused(code) => Over::Final(Outcome::Failed(Error::Refused(code))),
            Answer::Cancelled => Over::Final(self.cancelled(link).await),
        })
    }

    /// Makes `request`, and makes it again on a new stream each time `wait` runs out, up to `tries`
    /// times.
    async fn ask(
        &self,
        link: &dyn Link,
        request: &TransferRequest,
        wait: Duration,
        tries: u32,
    ) -> Answer {
        for _ in 0..tries {
            let reply = tokio::select! {
                biased;
                () = self.cancel.cancelled() => return Answer::Cancelled,
                reply = link.request(request.clone(), wait) => reply,
            };
            match reply {
                Ok(reply) => return Answer::Reply(reply),
                Err(WireError::TimedOut) => {}
                Err(WireError::Lost) => return Answer::Lost,
                Err(WireError::Refused(code)) => return Answer::Refused(code),
            }
        }
        Answer::Silent
    }

    /// Tells the receiver that the transfer is given up, and waits a moment for it to say so.
    async fn tell(&self, link: &dyn Link) -> bool {
        let request = TransferRequest::Cancel(Cancel { id: self.id });
        match link.request(request, self.policy.cancel_wait).await {
            Ok(Reply::Transfer(TransferReply::Cancelled(cancelled))) => cancelled.id == self.id,
            // A receiver that takes no part in transfers has nothing of this one.
            Ok(Reply::Transfer(TransferReply::Declined(declined))) => declined.id == self.id,
            _ => false,
        }
    }

    /// The transfer is given up on a connection that is up.
    async fn cancelled(&self, link: &dyn Link) -> Outcome {
        Outcome::Cancelled {
            told: self.tell(link).await,
        }
    }

    /// The transfer is given up when no connection is up: a connection is made for the one request,
    /// if the receiver may know the transfer at all.
    pub(crate) async fn cancelled_without_link(&self) -> Outcome {
        if !self.offered() {
            return Outcome::Cancelled { told: true };
        }
        let connected = timeout(self.policy.cancel_wait, self.wire.connect()).await;
        let told = match connected {
            Ok(Ok(link)) => self.tell(&*link).await,
            _ => false,
        };
        Outcome::Cancelled { told }
    }

    fn retry(&self, why: Why) -> Over {
        Over::Retry(Retry {
            why,
            moved: self.tracker.now().sent > 0,
        })
    }

    /// What an error of the wire makes of an attempt that has no connection yet.
    fn over(&self, error: WireError) -> Over {
        match error {
            WireError::Lost | WireError::TimedOut => self.retry(Why::Connection),
            WireError::Refused(code) => Over::Final(Outcome::Failed(Error::Refused(code))),
        }
    }
}

/// Resets the lanes that are still going with `code`, and waits until they are gone.
async fn end_lanes(tasks: &mut JoinSet<Result<(), Halt>>, reset: &ResetCode, code: u32) {
    reset.set(code);
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

/// How many pieces a transfer has: a span for each megabyte of a file, and one for a file that has
/// none, which carries only its `End`.
fn pieces(selection: &Selection) -> u64 {
    selection
        .entries()
        .iter()
        .filter(|entry| !entry.folder)
        .map(|entry| span_count(entry.size).max(1))
        .sum()
}
