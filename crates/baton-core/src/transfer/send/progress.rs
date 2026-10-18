// SPDX-License-Identifier: Apache-2.0

//! How far a transfer has got, for whoever shows it to a person.
//!
//! The engine keeps one [`Progress`] and changes it as it goes. It is a `watch`, so that a screen
//! that looks at it now and then sees the latest and a transfer that moves a megabyte at a time
//! does not queue a message for each.

use tokio::sync::watch;

/// What a transfer is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    /// Making a connection to the other device.
    Connecting,
    /// The offer is made, and the answer is awaited. A person may be deciding.
    Asking,
    /// The files are going out.
    Sending,
    /// Everything is sent, and the other device is putting it in place.
    Finishing,
    /// An attempt has ended, and the next waits for a while.
    Waiting,
    /// The transfer is over, whichever way it ended.
    Over,
}

/// How far a transfer has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Progress {
    /// What the transfer is doing now.
    pub(crate) stage: Stage,
    /// The attempt that is going, from 1.
    pub(crate) attempt: u32,
    /// How many files the transfer holds, folders not counted.
    pub(crate) files: u32,
    /// How many bytes those files add up to.
    pub(crate) bytes: u64,
    /// How many bytes of files have been written to the connection in this attempt, counting the
    /// chunks that were written whole. It starts again at 0 with each attempt, because an attempt
    /// sends everything.
    pub(crate) sent: u64,
}

/// The engine's end of a [`Progress`].
#[derive(Debug)]
pub(crate) struct Tracker(watch::Sender<Progress>);

impl Tracker {
    /// A tracker for a transfer of `files` files that hold `bytes` bytes, and the end to look at it
    /// from.
    pub(crate) fn new(files: u32, bytes: u64) -> (Self, watch::Receiver<Progress>) {
        let (tx, rx) = watch::channel(Progress {
            stage: Stage::Connecting,
            attempt: 0,
            files,
            bytes,
            sent: 0,
        });
        (Self(tx), rx)
    }

    /// An attempt begins: the count of what it sent starts again.
    pub(crate) fn begin(&self, attempt: u32) {
        self.0.send_modify(|progress| {
            progress.stage = Stage::Connecting;
            progress.attempt = attempt;
            progress.sent = 0;
        });
    }

    pub(crate) fn stage(&self, stage: Stage) {
        self.0.send_modify(|progress| progress.stage = stage);
    }

    /// A chunk with `bytes` bytes of files was written to the connection, whole.
    pub(crate) fn sent(&self, bytes: u64) {
        self.0
            .send_modify(|progress| progress.sent = progress.sent.saturating_add(bytes));
    }

    /// How far the transfer has got now.
    pub(crate) fn now(&self) -> Progress {
        *self.0.borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_of_what_was_sent_starts_again_with_each_attempt() {
        let (tracker, watching) = Tracker::new(3, 300);
        assert_eq!(
            *watching.borrow(),
            Progress {
                stage: Stage::Connecting,
                attempt: 0,
                files: 3,
                bytes: 300,
                sent: 0
            }
        );

        tracker.begin(1);
        tracker.stage(Stage::Sending);
        tracker.sent(100);
        tracker.sent(50);
        let now = *watching.borrow();
        assert_eq!((now.stage, now.attempt, now.sent), (Stage::Sending, 1, 150));
        assert_eq!(tracker.now(), now);

        tracker.stage(Stage::Waiting);
        tracker.begin(2);
        let now = *watching.borrow();
        assert_eq!(
            (now.stage, now.attempt, now.sent),
            (Stage::Connecting, 2, 0)
        );
        // What the transfer holds does not change.
        assert_eq!((now.files, now.bytes), (3, 300));
    }

    #[test]
    fn a_tracker_with_nobody_looking_still_keeps_count() {
        let (tracker, watching) = Tracker::new(1, 10);
        drop(watching);
        tracker.sent(7);
        assert_eq!(tracker.now().sent, 7);
    }
}
