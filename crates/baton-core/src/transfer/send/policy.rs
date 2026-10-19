// SPDX-License-Identifier: Apache-2.0

//! How long the sender waits, and how hard it tries.
//!
//! Some of these numbers are in the protocol, because the other device's numbers depend on them: how
//! long a person may take over an offer, and how long a lane may move no byte (section 8). The rest
//! are local choices, and nothing on the wire depends on them.

use std::time::Duration;

use baton_proto::limits::MAX_LANES;

/// The numbers an engine works to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Policy {
    /// How long to wait for the answer to an offer. A person has two minutes to give it, and this
    /// is a margin more (section 8, Offer).
    pub(crate) offer_wait: Duration,
    /// How many times to make an offer, each on a new stream, before the receiver is taken for gone.
    /// The receiver answers a repeat from the first copy's decision, so repeating does not ask the
    /// person again.
    pub(crate) offer_tries: u32,
    /// How long to wait for `Done`, which comes only once the receiver has dealt with every file
    /// (section 8, Finish).
    pub(crate) finish_wait: Duration,
    /// How many times to send `Finish` on a connection before the outcome is given up on. The
    /// receiver answers a repeat with the same outcome.
    pub(crate) finish_tries: u32,
    /// The longest to wait for the outcome of a `Finish` whose answer was never heard, which is
    /// asked for again on a new connection before anything is offered. A receiver that has the
    /// outcome, or does not know the transfer, answers at once. One that is putting the files in
    /// place answers when it is done, and one that waits for files that went with the old connection
    /// does not answer at all, so the wait is short, and the transfer is offered to a receiver that
    /// has not answered. If that receiver was only slow, it stops the lanes, and the outcome is asked
    /// for again with the wait and the tries of `Finish`.
    pub(crate) probe_wait: Duration,
    /// The longest `Cancel` is waited for.
    pub(crate) cancel_wait: Duration,
    /// How long a lane may take no byte, or fail to open, before the attempt is ended (section 8,
    /// Rules for the sender). The receiver ends an attempt that moves no byte for the same time.
    pub(crate) stall: Duration,
    /// The most lanes to use, if the receiver allows that many.
    pub(crate) lanes: u8,
    /// How long attempts may go on failing, one after another, before the transfer is given up. An
    /// attempt that gets files moving starts the count again.
    pub(crate) patience: Duration,
    /// The pause after the first failed attempt. It doubles for each one after that.
    pub(crate) backoff_start: Duration,
    /// The longest pause between attempts.
    pub(crate) backoff_max: Duration,
    /// The most attempts at all, whatever they did. Every attempt sends everything again, so a
    /// transfer that is cut off every time never finishes, and this ends it.
    pub(crate) attempts: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            offer_wait: Duration::from_secs(150),
            offer_tries: 3,
            finish_wait: Duration::from_mins(2),
            finish_tries: 10,
            probe_wait: Duration::from_secs(10),
            cancel_wait: Duration::from_secs(10),
            stall: Duration::from_secs(30),
            lanes: 4,
            patience: Duration::from_mins(10),
            backoff_start: Duration::from_secs(1),
            backoff_max: Duration::from_secs(30),
            attempts: 20,
        }
    }
}

impl Policy {
    /// The lanes to open when the receiver allows `allowed`, for a transfer of `pieces` pieces
    /// (spans, and the files that have none): no more than the receiver allows or the policy wants,
    /// and none that would have nothing to carry. There is always at least one, because lane 0
    /// carries the list.
    pub(crate) fn lanes_for(&self, allowed: u8, pieces: u64) -> u8 {
        let wanted = u64::from(self.lanes.min(allowed).min(MAX_LANES));
        u8::try_from(wanted.min(pieces).max(1)).unwrap_or(1)
    }
}

#[cfg(test)]
impl Policy {
    /// Waits that a test can sit through, with the clock running.
    pub(crate) fn quick() -> Self {
        Self {
            offer_wait: Duration::from_millis(400),
            offer_tries: 3,
            finish_wait: Duration::from_millis(400),
            finish_tries: 4,
            probe_wait: Duration::from_millis(200),
            cancel_wait: Duration::from_millis(200),
            stall: Duration::from_millis(300),
            lanes: 4,
            patience: Duration::from_secs(5),
            backoff_start: Duration::from_millis(5),
            backoff_max: Duration::from_millis(40),
            attempts: 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_numbers_of_the_protocol() {
        let policy = Policy::default();
        // Two minutes for the person and thirty seconds more (section 8, Offer).
        assert_eq!(policy.offer_wait, Duration::from_secs(150));
        // Two minutes for `Done` (section 8, Finish), and thirty seconds without a byte.
        assert_eq!(policy.finish_wait, Duration::from_secs(120));
        assert_eq!(policy.stall, Duration::from_secs(30));
        // The receiver grants four lanes.
        assert_eq!(policy.lanes, 4);
    }

    #[test]
    fn a_transfer_gets_the_lanes_it_can_use_and_the_receiver_allows() {
        let policy = Policy::default();
        // All the lanes that are wanted and allowed, when there is enough to carry.
        assert_eq!(policy.lanes_for(8, 1000), 4);
        assert_eq!(policy.lanes_for(4, 1000), 4);
        // The receiver allows fewer.
        assert_eq!(policy.lanes_for(2, 1000), 2);
        assert_eq!(policy.lanes_for(1, 1000), 1);
        // Not more lanes than there are pieces, and one for the list at least.
        assert_eq!(policy.lanes_for(8, 3), 3);
        assert_eq!(policy.lanes_for(8, 1), 1);
        assert_eq!(policy.lanes_for(8, 0), 1);
        // A policy that wants more than the protocol has lanes gets the lanes the protocol has.
        let greedy = Policy {
            lanes: 200,
            ..Policy::default()
        };
        assert_eq!(greedy.lanes_for(200, 1000), MAX_LANES);
        // A receiver that allows none is no reason to open none.
        assert_eq!(policy.lanes_for(0, 1000), 1);
    }
}
