// SPDX-License-Identifier: Apache-2.0

//! The pairing connections a device is serving.
//!
//! Anyone who can reach a device may open a `baton-pair/1` connection, so only a few are
//! served at once. A table that turned newcomers away when it was full would let a handful
//! of idle connections keep every real joiner out for as long as the attacker held them.
//! Here a newcomer takes the place of the oldest connection that has not shown a valid
//! proof instead, so keeping the table full of strangers takes a steady stream of new
//! connections, each with a completed handshake, and not a few sockets left open.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

/// Pairing connections served at the same time. A real pairing needs one.
pub(crate) const MAX_PAIRING_CONNECTIONS: usize = 8;

struct Entry {
    /// The connection is being judged or has shown a valid proof, so it keeps its place.
    protected: bool,
    /// Cancelled when a newer connection takes this one's place.
    pushed_out: CancellationToken,
}

#[derive(Default)]
struct Table {
    next: u64,
    /// The connections being served, by order of arrival.
    entries: BTreeMap<u64, Entry>,
}

/// Which pairing connections are being served, and who gives way when there are too many.
pub(crate) struct PairSlots {
    capacity: usize,
    table: Mutex<Table>,
}

impl PairSlots {
    pub(crate) fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity,
            table: Mutex::new(Table::default()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Table> {
        // Every change leaves the table consistent, so a poisoned lock is safe to reuse.
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Gives a new connection a place. When the table is full, the oldest connection that
    /// is not protected is told to stop and its place is taken. `None` means every place
    /// is held by a connection that has shown a valid proof.
    pub(crate) fn admit(self: &Arc<Self>) -> Option<PairSlot> {
        let mut table = self.lock();
        if table.entries.len() >= self.capacity {
            let oldest = table
                .entries
                .iter()
                .find_map(|(id, entry)| (!entry.protected).then_some(*id))?;
            if let Some(entry) = table.entries.remove(&oldest) {
                entry.pushed_out.cancel();
            }
        }
        let id = table.next;
        table.next += 1;
        let pushed_out = CancellationToken::new();
        table.entries.insert(
            id,
            Entry {
                protected: false,
                pushed_out: pushed_out.clone(),
            },
        );
        drop(table);
        Some(PairSlot {
            slots: Arc::clone(self),
            id,
            pushed_out,
        })
    }

    /// How many connections hold a place.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().entries.len()
    }
}

/// A connection's place in the table, which it gives up when dropped.
pub(crate) struct PairSlot {
    slots: Arc<PairSlots>,
    id: u64,
    pushed_out: CancellationToken,
}

impl PairSlot {
    /// Resolves when a newer connection has taken this one's place.
    pub(crate) fn pushed_out(&self) -> WaitForCancellationFuture<'_> {
        self.pushed_out.cancelled()
    }

    /// Keeps this connection from being pushed out until [`unprotect`](Self::unprotect) is
    /// called or it ends. Returns `false` if it was pushed out already, in which case it
    /// must not go on to do anything that has an effect.
    ///
    /// A connection is protected before its request is judged, because a request that
    /// turns out to be valid starts something that cannot be taken back, and the connection
    /// must not be stopped halfway through.
    #[must_use]
    pub(crate) fn protect(&self) -> bool {
        match self.slots.lock().entries.get_mut(&self.id) {
            Some(entry) => {
                entry.protected = true;
                true
            }
            None => false,
        }
    }

    /// Makes the connection open to being pushed out again: its request was not valid,
    /// so it has shown nothing.
    pub(crate) fn unprotect(&self) {
        if let Some(entry) = self.slots.lock().entries.get_mut(&self.id) {
            entry.protected = false;
        }
    }
}

impl Drop for PairSlot {
    fn drop(&mut self) {
        // Does nothing if this connection was pushed out and its place is already gone.
        self.slots.lock().entries.remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_pushed_out(slot: &PairSlot) -> bool {
        slot.pushed_out.is_cancelled()
    }

    #[test]
    fn places_are_handed_out_until_the_table_is_full() {
        let slots = PairSlots::new(3);
        let held: Vec<_> = (0..3).map(|_| slots.admit().unwrap()).collect();
        assert_eq!(slots.len(), 3);
        assert!(held.iter().all(|slot| !is_pushed_out(slot)));
    }

    #[test]
    fn a_newcomer_to_a_full_table_pushes_out_the_oldest() {
        let slots = PairSlots::new(3);
        let first = slots.admit().unwrap();
        let second = slots.admit().unwrap();
        let third = slots.admit().unwrap();

        let fourth = slots.admit().unwrap();
        assert!(is_pushed_out(&first));
        assert!(!is_pushed_out(&second) && !is_pushed_out(&third) && !is_pushed_out(&fourth));
        assert_eq!(
            slots.len(),
            3,
            "the table never holds more than its capacity"
        );

        let fifth = slots.admit().unwrap();
        assert!(is_pushed_out(&second));
        assert!(!is_pushed_out(&third) && !is_pushed_out(&fourth) && !is_pushed_out(&fifth));
        assert_eq!(slots.len(), 3);
    }

    #[test]
    fn a_protected_connection_is_passed_over() {
        let slots = PairSlots::new(2);
        let proven = slots.admit().unwrap();
        let other = slots.admit().unwrap();
        assert!(proven.protect());

        // `proven` is the oldest, but `other` is the oldest that can be pushed out.
        let newcomer = slots.admit().unwrap();
        assert!(!is_pushed_out(&proven));
        assert!(is_pushed_out(&other));
        assert!(!is_pushed_out(&newcomer));
    }

    #[test]
    fn a_table_of_protected_connections_turns_newcomers_away() {
        let slots = PairSlots::new(2);
        let first = slots.admit().unwrap();
        let second = slots.admit().unwrap();
        assert!(first.protect() && second.protect());

        assert!(slots.admit().is_none());
        assert_eq!(slots.len(), 2);
        assert!(!is_pushed_out(&first) && !is_pushed_out(&second));
    }

    #[test]
    fn a_request_that_proves_invalid_leaves_its_connection_open_to_being_pushed_out() {
        let slots = PairSlots::new(2);
        let first = slots.admit().unwrap();
        let second = slots.admit().unwrap();
        assert!(first.protect());
        first.unprotect();

        let _third = slots.admit().unwrap();
        // It kept its age, so it is still the oldest.
        assert!(is_pushed_out(&first));
        assert!(!is_pushed_out(&second));
    }

    #[test]
    fn a_connection_that_was_pushed_out_cannot_be_protected() {
        let slots = PairSlots::new(1);
        let first = slots.admit().unwrap();
        let _second = slots.admit().unwrap();
        assert!(is_pushed_out(&first));
        assert!(!first.protect(), "a connection that is gone was protected");
    }

    #[test]
    fn a_finished_connection_gives_its_place_back() {
        let slots = PairSlots::new(2);
        let first = slots.admit().unwrap();
        let second = slots.admit().unwrap();
        assert!(first.protect() && second.protect());
        assert!(slots.admit().is_none());

        drop(first);
        assert_eq!(slots.len(), 1);
        let third = slots.admit().unwrap();
        assert!(!is_pushed_out(&second) && !is_pushed_out(&third));
    }

    #[test]
    fn dropping_a_connection_that_was_pushed_out_does_not_free_the_newcomers_place() {
        let slots = PairSlots::new(1);
        let first = slots.admit().unwrap();
        let second = slots.admit().unwrap();
        assert!(is_pushed_out(&first));

        // The old connection winds down after the newcomer took its place.
        drop(first);
        assert_eq!(slots.len(), 1);
        assert!(!is_pushed_out(&second));
        // The table is still full, so the next newcomer pushes the second one out.
        let _third = slots.admit().unwrap();
        assert!(is_pushed_out(&second));
    }

    #[tokio::test]
    async fn the_connection_that_is_pushed_out_is_woken() {
        let slots = PairSlots::new(1);
        let first = slots.admit().unwrap();
        let waiting = tokio::spawn(async move {
            first.pushed_out().await;
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());

        let _second = slots.admit().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .expect("the connection was never told")
            .unwrap();
    }
}
