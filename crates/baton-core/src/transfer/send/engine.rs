// SPDX-License-Identifier: Apache-2.0

//! Sending a transfer from its first attempt to its outcome.
//!
//! [`start`] takes what a person chose, a [`Wire`] to the receiver and the id of the transfer, and
//! runs it as a task of its own. It returns a [`Sending`], which shows how far the transfer has got,
//! can give it up, and says how it ended.
//!
//! A stop token ends every transfer that is made under it, as [`Sending::cancel`] ends one. A node
//! gives all its transfers the token that it stops by, so that a program that is closing does not
//! leave them running, and a transfer that is cancelled does not stop the others.
//!
//! A transfer that has a [`Ledger`] is written down before its first offer and when it ends, so that
//! a receiver that is left holding it can be told to let go, also by a run of the program that
//! starts later (see [`holds`](super::holds)).
//!
//! The task makes attempts ([`Job::attempt`]) one after another until one of them is the end of the
//! transfer. An attempt that fails in a way another may not is followed by a pause and another
//! attempt, and every attempt sends everything again, on a connection of its own (section 8,
//! Resuming). Three limits end the trying:
//!
//! - **Patience.** Attempts that keep failing without getting files moving give up after a while. An
//!   attempt that finished writing a chunk of file data starts the count again, and the pause
//!   between attempts with it, since the receiver is there and the link is the trouble.
//! - **Attempts.** A transfer that is cut off every time never finishes, and the attempts that sent
//!   something each time would go on for ever, so there is a most.
//! - **The person.** [`Sending::cancel`], or dropping the [`Sending`], gives the transfer up: the
//!   lanes are reset, the receiver is told, and nothing more is tried.

use std::sync::Arc;

use baton_proto::id::TransferId;
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{Instant, sleep},
};
use tokio_util::sync::CancellationToken;

use super::{
    attempt::{Job, Over},
    holds::Ledger,
    lane::ListFrames,
    outcome::{Error, Outcome, Why},
    policy::Policy,
    progress::{Progress, Stage, Tracker},
    selection::Selection,
    wire::Wire,
};

/// A transfer that is going. Dropping it gives the transfer up.
#[derive(Debug)]
#[must_use = "dropping a transfer that is going gives it up"]
pub struct Sending {
    id: TransferId,
    progress: watch::Receiver<Progress>,
    cancel: CancellationToken,
    task: JoinHandle<Outcome>,
}

impl Sending {
    /// The id the transfer has on the wire.
    pub fn id(&self) -> TransferId {
        self.id
    }

    /// How far the transfer has got. The latest value is always there, and a screen that looks at
    /// it now and then sees the latest.
    pub fn progress(&self) -> watch::Receiver<Progress> {
        self.progress.clone()
    }

    /// Gives the transfer up. The outcome says whether the receiver was told.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Waits for the end of the transfer.
    pub async fn outcome(mut self) -> Outcome {
        match (&mut self.task).await {
            Ok(outcome) => outcome,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            // The runtime is shutting down.
            Err(_) => Outcome::Failed(Error::Stopped),
        }
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Starts sending `selection` to the receiver at the other end of `wire`, as the transfer `id`. The
/// transfer is given up when `stop` is cancelled, as it is when [`Sending::cancel`] is called. A
/// `ledger` is told that the receiver may hold the transfer before the offer is made, and how the
/// transfer ended.
///
/// # Errors
///
/// The list cannot be made into the frames that lane 0 carries, which a list that the walk made does
/// not come to.
pub(crate) fn start(
    wire: Arc<dyn Wire>,
    selection: Arc<Selection>,
    policy: Policy,
    id: TransferId,
    stop: &CancellationToken,
    ledger: Option<Ledger>,
) -> Result<Sending, Error> {
    let list = ListFrames::new(selection.entries())
        .map(Arc::new)
        .map_err(|_| Error::Internal("the list cannot be made into frames"))?;
    let summary = selection.summary();
    let (tracker, progress) = Tracker::new(summary.files, summary.bytes);
    // A child, so that giving this transfer up leaves the others alone.
    let cancel = stop.child_token();
    let job = Job::new(
        wire,
        selection,
        list,
        id,
        policy,
        cancel.clone(),
        Arc::new(tracker),
    );
    let task = tokio::spawn(async move {
        if let Some(ledger) = &ledger {
            ledger.begin(id).await;
        }
        let outcome = run(&job).await;
        if let Some(ledger) = &ledger {
            ledger.ended(id, &outcome, job.offered()).await;
        }
        job.tracker.stage(Stage::Over);
        outcome
    });
    Ok(Sending {
        id,
        progress,
        cancel,
        task,
    })
}

/// Makes attempts until one of them is the end.
async fn run(job: &Job) -> Outcome {
    let policy = &job.policy;
    let attempts = policy.attempts.max(1);
    let mut pause = policy.backoff_start;
    let mut failing_since: Option<Instant> = None;
    for attempt in 1..=attempts {
        job.tracker.begin(attempt);
        let retry = match job.attempt().await {
            Over::Final(outcome) => return outcome,
            Over::Retry(retry) => retry,
        };
        if retry.moved {
            failing_since = None;
            pause = policy.backoff_start;
        }
        let since = *failing_since.get_or_insert_with(Instant::now);
        if attempt == attempts || since.elapsed() >= policy.patience {
            return Outcome::Failed(Error::GaveUp(retry.why));
        }
        job.tracker.stage(Stage::Waiting);
        tokio::select! {
            biased;
            () = job.cancel.cancelled() => return job.cancelled_without_link().await,
            () = sleep(pause) => {}
        }
        pause = pause.saturating_mul(2).min(policy.backoff_max);
    }
    // `attempts` is at least 1, and the last attempt returns.
    Outcome::Failed(Error::GaveUp(Why::Connection))
}
