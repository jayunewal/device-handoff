// SPDX-License-Identifier: Apache-2.0

//! The record of the unlock of one paired device, as the sessions with it read and change it
//! (`docs/adr/0013-unlock-from-a-phone.md`).
//!
//! [`NodeUnlockStore`] is the [`UnlockStore`] of a node: it keeps the record in the list of
//! paired devices ([`PairedDevice::unlock`](crate::PairedDevice::unlock)) and writes the history
//! there. Every change is a compare-and-swap on the sealed key that the session read, so that a
//! person who sets the unlock up again, or turns it off, while an attempt goes on is never
//! overwritten by it, and it is saved before it takes effect, as every change to the list is.
//!
//! A setup that ends is not tried again, also when the file cannot be written: the node
//! remembers the end for as long as it runs ([`UnwrittenEnds`]).

use std::sync::{Arc, Mutex};

use baton_proto::{DeviceId, unlock::SealedKey};

use crate::{
    clock::Clock,
    error::Result,
    streaming::unlock::{Concluded, KeepError, Record, Replacement, UnlockStore},
    trust::TrustStore,
    unlock_record::UnlockEntry,
    util::lock,
};

/// How many setups that ended but could not be written down are remembered while the program
/// runs. The person has to set the unlock up again for each, so this is far more than there are.
const MAX_UNWRITTEN_ENDS: usize = 64;

/// The setups that ended and that the disk did not take the word of: the paired device and the
/// sealed key of the record. A setup that ends must not be tried again, so the node remembers
/// the end for as long as it runs when it could not write it down.
#[derive(Debug, Default)]
pub(crate) struct UnwrittenEnds(Mutex<Vec<(DeviceId, Vec<u8>)>>);

impl UnwrittenEnds {
    fn add(&self, viewer: DeviceId, sealed: &SealedKey) {
        let mut ends = lock(&self.0);
        let end = (viewer, sealed.as_bytes().to_vec());
        if !ends.contains(&end) {
            ends.push(end);
            let surplus = ends.len().saturating_sub(MAX_UNWRITTEN_ENDS);
            ends.drain(..surplus);
        }
    }

    fn remove(&self, viewer: DeviceId, sealed: &SealedKey) {
        lock(&self.0).retain(|(other, bytes)| *other != viewer || bytes != sealed.as_bytes());
    }

    fn holds(&self, viewer: DeviceId, sealed: &SealedKey) -> bool {
        lock(&self.0)
            .iter()
            .any(|(other, bytes)| *other == viewer && bytes == sealed.as_bytes())
    }
}

/// What came of a change to the record.
enum Change {
    /// It was made and saved.
    Made,
    /// It was not: there is no such device, or the record is not the one that was meant.
    Declined,
    /// It was meant, and the file could not be written. Nothing changed.
    Unwritable,
}

/// What a device needs to be unlocked from a paired device: the list that keeps the records and
/// the history, the clock of the history, and what could not be written down. A session with a
/// paired device is given the store for it.
pub(crate) struct Unlocks {
    trust: TrustStore,
    clock: Arc<dyn Clock>,
    /// This device.
    target: DeviceId,
    unwritten: Arc<UnwrittenEnds>,
}

impl Unlocks {
    pub(crate) fn new(trust: TrustStore, clock: Arc<dyn Clock>, target: DeviceId) -> Self {
        Self {
            trust,
            clock,
            target,
            unwritten: Arc::default(),
        }
    }

    /// The store for the unlock of `viewer`, a paired device.
    pub(crate) fn store_for(&self, viewer: DeviceId) -> NodeUnlockStore {
        NodeUnlockStore::new(
            self.trust.clone(),
            Arc::clone(&self.clock),
            self.target,
            viewer,
            Arc::clone(&self.unwritten),
        )
    }
}

/// The record of the unlock of one paired device, and the history, kept in the list of paired
/// devices. It is what the sessions with that device are given.
pub(crate) struct NodeUnlockStore {
    trust: TrustStore,
    clock: Arc<dyn Clock>,
    /// This device.
    target: DeviceId,
    /// The paired device that unlocks this one.
    viewer: DeviceId,
    unwritten: Arc<UnwrittenEnds>,
}

impl NodeUnlockStore {
    fn new(
        trust: TrustStore,
        clock: Arc<dyn Clock>,
        target: DeviceId,
        viewer: DeviceId,
        unwritten: Arc<UnwrittenEnds>,
    ) -> Self {
        Self {
            trust,
            clock,
            target,
            viewer,
            unwritten,
        }
    }

    /// What came of `written`, saying so in the log if the file could not be written.
    fn changed(&self, what: &str, written: Result<Option<()>>) -> Change {
        match written {
            Ok(Some(())) => Change::Made,
            Ok(None) => Change::Declined,
            Err(error) => {
                tracing::warn!(peer = %self.viewer.short(), %error, "could not write down {what}");
                Change::Unwritable
            }
        }
    }
}

impl UnlockStore for NodeUnlockStore {
    fn record(&self) -> Option<Record> {
        let device = self.trust.get(&self.viewer)?;
        let record = device.unlock?.record(self.target, self.viewer)?;
        (!self.unwritten.holds(self.viewer, &record.sealed)).then_some(record)
    }

    fn replace(&self, old: &SealedKey, new: Replacement) -> Result<(), KeepError> {
        if self.unwritten.holds(self.viewer, old) {
            return Err(KeepError::Changed);
        }
        let swapped = self.trust.amend(&self.viewer, |device, _| {
            device
                .unlock
                .as_mut()?
                .replace(self.target, old, new)
                .then_some(())
        });
        match self.changed("the new box of an unlock", swapped) {
            Change::Made => Ok(()),
            Change::Declined => Err(KeepError::Changed),
            Change::Unwritable => Err(KeepError::Unwritable),
        }
    }

    fn concluded(&self, concluded: Concluded) {
        let Concluded { finish, sealed } = concluded;
        let ends = finish.ends_setup().then_some(sealed).flatten();
        let at = self.clock.now_unix();
        // The mark that the setup ended and the line in the history are one write, so that a
        // crash leaves both or neither.
        let written = self.trust.amend(&self.viewer, |device, history| {
            history.push(UnlockEntry {
                at,
                device: device.id,
                name: device.display_name().to_owned(),
                finish,
            });
            if let (Some(sealed), Some(unlock)) = (&ends, device.unlock.as_mut()) {
                unlock.end(sealed);
            }
            Some(())
        });
        if matches!(
            self.changed("how an unlock ended", written),
            Change::Unwritable
        ) && let Some(sealed) = &ends
        {
            self.unwritten.add(self.viewer, sealed);
        }
    }

    fn restore(&self, sealed: &SealedKey) {
        let restored = self.trust.amend(&self.viewer, |device, _| {
            device.unlock.as_mut()?.restore(sealed).then_some(())
        });
        // A change that cannot be saved is not made, so an end that was written stays if the
        // file cannot be written now: the setup is not tried again. An end that was never
        // written is forgotten whatever the disk does.
        let _ = self.changed("an unlock that can be tried again", restored);
        self.unwritten.remove(self.viewer, sealed);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use baton_proto::{
        DeviceInfo, Platform,
        unlock::{Sealing, SealingKey},
    };
    use tempfile::TempDir;

    use super::*;
    use crate::{
        clock::ManualClock,
        streaming::{CancelReason, UnlockFinish, UnlockOutcome},
        testing::unlocks::{boxed, new_unlock, point, replacement, sealed},
        trust::TrustStore,
        unlock_record::MAX_UNLOCK_HISTORY,
    };

    fn id(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; DeviceId::LEN])
    }

    /// This device, which the person set the unlock up on.
    fn target() -> DeviceId {
        id(1)
    }

    /// The paired device that unlocks it.
    fn phone() -> DeviceId {
        id(2)
    }

    /// A paired device that has no unlock.
    fn tablet() -> DeviceId {
        id(3)
    }

    fn info(name: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.to_owned(),
            platform: Platform::Linux,
            app_version: "0.1.0".to_owned(),
        }
    }

    /// The store of a node that has the phone and the tablet paired, and an unlock set up for the
    /// phone (`sealed(1)`), on a disk that the test can take away.
    struct Rig {
        dir: TempDir,
        trust: TrustStore,
        clock: Arc<ManualClock>,
        unwritten: Arc<UnwrittenEnds>,
        store: NodeUnlockStore,
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join("state")).unwrap();
            let trust = TrustStore::load(dir.path().join("state").join("devices.json")).unwrap();
            trust.pair(phone(), &info("Phone"), 100).unwrap();
            trust.pair(tablet(), &info("Tablet"), 100).unwrap();
            trust
                .set_up_unlock(&phone(), target(), new_unlock(1), 1_000)
                .unwrap();
            let clock = Arc::new(ManualClock::new(5_000));
            let unwritten = Arc::new(UnwrittenEnds::default());
            let store = Self::store_of(&trust, &clock, &unwritten, phone());
            Self {
                dir,
                trust,
                clock,
                unwritten,
                store,
            }
        }

        fn store_of(
            trust: &TrustStore,
            clock: &Arc<ManualClock>,
            unwritten: &Arc<UnwrittenEnds>,
            viewer: DeviceId,
        ) -> NodeUnlockStore {
            NodeUnlockStore::new(
                trust.clone(),
                Arc::clone(clock) as Arc<dyn Clock>,
                target(),
                viewer,
                Arc::clone(unwritten),
            )
        }

        /// The store for another paired device, on the same list.
        fn store_for(&self, viewer: DeviceId) -> NodeUnlockStore {
            Self::store_of(&self.trust, &self.clock, &self.unwritten, viewer)
        }

        /// Takes the disk away: the list cannot be saved.
        fn disk_fails(&self) {
            fs::remove_dir_all(self.dir.path().join("state")).unwrap();
        }

        fn disk_works(&self) {
            fs::create_dir(self.dir.path().join("state")).unwrap();
        }

        /// What the file holds, read by a list of its own.
        fn on_disk(&self) -> TrustStore {
            TrustStore::load(self.trust.path()).unwrap()
        }

        fn phone_unlock(&self) -> crate::PairedUnlock {
            self.trust.get(&phone()).unwrap().unlock.unwrap()
        }

        fn conclude(&self, finish: UnlockFinish, sealed: Option<SealedKey>) {
            self.store.concluded(Concluded { finish, sealed });
        }
    }

    fn rejected() -> UnlockFinish {
        UnlockFinish::Done(UnlockOutcome::Rejected)
    }

    #[test]
    fn the_record_is_what_the_person_set_up_for_this_device_and_that_one() {
        let rig = Rig::new();
        let record = rig.store.record().unwrap();
        assert_eq!(record.boxed, boxed(1));
        assert_eq!(record.sealed, sealed(1));
        assert_eq!(record.sealing, Sealing::EciesP256);
        assert_eq!(record.public, SealingKey::new(point(0xc0)));
        assert_eq!((record.target, record.viewer), (target(), phone()));
    }

    #[test]
    fn there_is_no_record_without_a_setup_for_this_device_and_that_one() {
        let rig = Rig::new();
        assert!(
            rig.store_for(tablet()).record().is_none(),
            "paired, but nothing set up"
        );
        assert!(rig.store_for(id(9)).record().is_none(), "not paired");

        let elsewhere = NodeUnlockStore::new(
            rig.trust.clone(),
            Arc::clone(&rig.clock) as Arc<dyn Clock>,
            id(7),
            phone(),
            Arc::clone(&rig.unwritten),
        );
        assert!(elsewhere.record().is_none(), "set up for another device");

        rig.trust.turn_off_unlock(&phone()).unwrap();
        assert!(rig.store.record().is_none(), "turned off");
    }

    #[test]
    fn a_setup_that_ended_has_no_record() {
        let rig = Rig::new();
        rig.conclude(rejected(), Some(sealed(1)));
        assert!(rig.store.record().is_none());
    }

    #[test]
    fn replacing_puts_the_new_box_and_the_new_key_in_the_file() {
        let rig = Rig::new();
        rig.store.replace(&sealed(1), replacement(2)).unwrap();

        let record = rig.store.record().unwrap();
        assert_eq!((record.boxed, record.sealed), (boxed(2), sealed(2)));
        let saved = rig.on_disk().get(&phone()).unwrap().unlock.unwrap();
        let record = saved.record(target(), phone()).unwrap();
        assert_eq!((record.boxed, record.sealed), (boxed(2), sealed(2)));
        assert_eq!(
            saved.set_up_at(),
            1_000,
            "it is still the setup of the person"
        );
    }

    #[test]
    fn replacing_with_a_key_that_is_not_the_records_changes_nothing() {
        let rig = Rig::new();
        let before = fs::read(rig.trust.path()).unwrap();

        assert_eq!(
            rig.store.replace(&sealed(9), replacement(2)),
            Err(KeepError::Changed)
        );
        assert_eq!(fs::read(rig.trust.path()).unwrap(), before);

        rig.store.replace(&sealed(1), replacement(2)).unwrap();
        assert_eq!(
            rig.store.replace(&sealed(1), replacement(3)),
            Err(KeepError::Changed),
            "the key that was read is not the record's any more"
        );
        assert_eq!(rig.store.record().unwrap().sealed, sealed(2));
    }

    #[test]
    fn a_person_who_changes_the_unlock_while_an_attempt_goes_on_is_not_overwritten() {
        let rig = Rig::new();
        // Set up again: the box of the attempt is not the one that is kept.
        rig.trust
            .set_up_unlock(&phone(), target(), new_unlock(3), 2_000)
            .unwrap();
        assert_eq!(
            rig.store.replace(&sealed(1), replacement(2)),
            Err(KeepError::Changed)
        );
        assert_eq!(rig.store.record().unwrap().sealed, sealed(3));

        // Turned off.
        rig.trust.turn_off_unlock(&phone()).unwrap();
        assert_eq!(
            rig.store.replace(&sealed(3), replacement(4)),
            Err(KeepError::Changed)
        );
        assert!(rig.trust.get(&phone()).unwrap().unlock.is_none());

        // Unpaired.
        rig.trust
            .set_up_unlock(&phone(), target(), new_unlock(5), 3_000)
            .unwrap();
        rig.trust.remove(&phone()).unwrap();
        assert_eq!(
            rig.store.replace(&sealed(5), replacement(6)),
            Err(KeepError::Changed)
        );
        assert!(rig.store.record().is_none());
    }

    #[test]
    fn a_box_that_cannot_be_written_is_not_kept_and_the_old_one_stays() {
        let rig = Rig::new();
        rig.disk_fails();
        assert_eq!(
            rig.store.replace(&sealed(1), replacement(2)),
            Err(KeepError::Unwritable)
        );
        assert_eq!(rig.store.record().unwrap().sealed, sealed(1));

        rig.disk_works();
        rig.store.replace(&sealed(1), replacement(2)).unwrap();
        assert_eq!(
            rig.on_disk().get(&phone()).unwrap().unlock.unwrap(),
            rig.phone_unlock()
        );
    }

    #[test]
    fn an_end_that_ends_the_setup_is_written_with_its_line_in_one_save() {
        let rig = Rig::new();
        rig.conclude(rejected(), Some(sealed(1)));

        let saved = rig.on_disk();
        assert!(!saved.get(&phone()).unwrap().can_unlock(&target()));
        assert_eq!(
            saved.unlock_history(),
            [UnlockEntry {
                at: 5_000,
                device: phone(),
                name: "Phone".to_owned(),
                finish: rejected(),
            }]
        );

        // Neither is written if the disk does not take the save.
        let rig = Rig::new();
        rig.disk_fails();
        rig.conclude(rejected(), Some(sealed(1)));
        assert!(rig.trust.get(&phone()).unwrap().can_unlock(&target()));
        assert_eq!(rig.trust.unlock_history(), Vec::new());
    }

    #[test]
    fn only_the_finishes_that_the_rules_name_end_the_setup() {
        let outcomes = [
            (UnlockOutcome::Unlocked, false),
            (UnlockOutcome::NotSetUp, false),
            (UnlockOutcome::NotLocked, false),
            (UnlockOutcome::NotPrivate, false),
            (UnlockOutcome::Busy, false),
            (UnlockOutcome::Stale, true),
            (UnlockOutcome::Rejected, true),
            (UnlockOutcome::TimedOut, false),
            (UnlockOutcome::Failed, false),
            (UnlockOutcome::Other, false),
        ];
        let reasons = [
            (CancelReason::Declined, false),
            (CancelReason::KeyLost, true),
            (CancelReason::Failed, false),
            (CancelReason::Other, false),
        ];
        let mut finishes: Vec<_> = outcomes
            .into_iter()
            .map(|(outcome, ends)| (UnlockFinish::Done(outcome), ends))
            .chain(outcomes.into_iter().map(|(outcome, _)| {
                // A platform that said it without having kept a new box, whatever it said.
                (UnlockFinish::Unkept(outcome), true)
            }))
            .chain(
                reasons
                    .into_iter()
                    .map(|(reason, ends)| (UnlockFinish::Cancelled(reason), ends)),
            )
            .collect();
        finishes.push((UnlockFinish::Ended, false));

        for (finish, ends) in finishes {
            let rig = Rig::new();
            rig.conclude(finish, Some(sealed(1)));
            assert_eq!(rig.phone_unlock().usable(), !ends, "{finish:?}");
            assert_eq!(rig.store.record().is_some(), !ends, "{finish:?}");
            assert_eq!(rig.trust.unlock_history().len(), 1, "{finish:?}");
        }
    }

    #[test]
    fn an_end_for_an_unlock_that_was_set_up_again_meanwhile_does_not_end_the_new_setup() {
        let rig = Rig::new();
        rig.trust
            .set_up_unlock(&phone(), target(), new_unlock(3), 2_000)
            .unwrap();
        rig.conclude(rejected(), Some(sealed(1)));

        assert!(rig.phone_unlock().usable());
        assert_eq!(rig.store.record().unwrap().sealed, sealed(3));
        assert_eq!(
            rig.trust.unlock_history().len(),
            1,
            "the attempt is still written down"
        );
    }

    #[test]
    fn an_end_without_a_record_writes_only_its_line() {
        let rig = Rig::new();
        let tablet_store = rig.store_for(tablet());
        tablet_store.concluded(Concluded {
            finish: UnlockFinish::Done(UnlockOutcome::NotSetUp),
            sealed: None,
        });

        let history = rig.trust.unlock_history();
        assert_eq!(history.len(), 1);
        assert_eq!(
            (history[0].device, history[0].name.as_str()),
            (tablet(), "Tablet")
        );
        assert!(rig.trust.get(&tablet()).unwrap().unlock.is_none());
        assert!(rig.store.record().is_some(), "the phone is not touched");
    }

    #[test]
    fn an_end_that_names_a_key_but_does_not_end_the_setup_leaves_the_record_alone() {
        let rig = Rig::new();
        rig.conclude(UnlockFinish::Done(UnlockOutcome::Unlocked), Some(sealed(1)));
        assert_eq!(rig.store.record().unwrap().sealed, sealed(1));
    }

    #[test]
    fn the_history_has_the_time_and_the_name_of_the_moment() {
        let rig = Rig::new();
        rig.conclude(UnlockFinish::Done(UnlockOutcome::Unlocked), None);
        rig.clock.advance(60);
        rig.trust
            .update(&phone(), |device| {
                device.local_name = Some("Jay's iPhone".to_owned());
            })
            .unwrap();
        rig.conclude(UnlockFinish::Ended, None);

        let shown: Vec<_> = rig
            .on_disk()
            .unlock_history()
            .into_iter()
            .map(|line| (line.at, line.name, line.finish))
            .collect();
        assert_eq!(
            shown,
            [
                (5_060, "Jay's iPhone".to_owned(), UnlockFinish::Ended),
                (
                    5_000,
                    "Phone".to_owned(),
                    UnlockFinish::Done(UnlockOutcome::Unlocked)
                ),
            ]
        );
    }

    #[test]
    fn the_history_keeps_the_last_hundred() {
        let rig = Rig::new();
        for _ in 0..MAX_UNLOCK_HISTORY + 5 {
            rig.clock.advance(1);
            rig.conclude(UnlockFinish::Done(UnlockOutcome::Busy), None);
        }
        let history = rig.on_disk().unlock_history();
        assert_eq!(history.len(), MAX_UNLOCK_HISTORY);
        assert_eq!(history[0].at, 5_000 + 105);
        assert_eq!(history[99].at, 5_000 + 6);
    }

    #[test]
    fn an_end_of_a_device_that_is_not_paired_writes_nothing() {
        let rig = Rig::new();
        let stranger = rig.store_for(id(9));
        stranger.concluded(Concluded {
            finish: rejected(),
            sealed: Some(sealed(1)),
        });
        assert_eq!(rig.trust.unlock_history(), Vec::new());
        assert!(rig.store.record().is_some());
    }

    #[test]
    fn a_setup_that_ended_and_could_not_be_written_is_not_tried_again_while_the_node_runs() {
        let rig = Rig::new();
        rig.disk_fails();
        rig.conclude(rejected(), Some(sealed(1)));

        // The list still says that it is usable, because the disk did not take the word of the end.
        assert!(rig.phone_unlock().usable());
        // The node does not offer it, and does not change it either.
        assert!(rig.store.record().is_none());
        assert_eq!(
            rig.store.replace(&sealed(1), replacement(2)),
            Err(KeepError::Changed)
        );

        // A restart forgets what the disk never heard of.
        let restarted = Rig::store_of(
            &rig.trust,
            &rig.clock,
            &Arc::new(UnwrittenEnds::default()),
            phone(),
        );
        assert!(restarted.record().is_some());

        // The person sets it up again: a new setup is a different record.
        rig.disk_works();
        rig.trust
            .set_up_unlock(&phone(), target(), new_unlock(3), 6_000)
            .unwrap();
        assert_eq!(rig.store.record().unwrap().sealed, sealed(3));
    }

    #[test]
    fn the_stores_that_a_node_gives_its_sessions_share_what_it_could_not_write_down() {
        let rig = Rig::new();
        let unlocks = Unlocks::new(
            rig.trust.clone(),
            Arc::clone(&rig.clock) as Arc<dyn Clock>,
            target(),
        );
        // A store is for the device it is asked for, on the device that the node is.
        let first = unlocks.store_for(phone());
        assert_eq!(first.record().unwrap().sealed, sealed(1));
        assert!(unlocks.store_for(tablet()).record().is_none());

        // A session ends the setup, and the disk does not take the word of it.
        rig.disk_fails();
        first.concluded(Concluded {
            finish: rejected(),
            sealed: Some(sealed(1)),
        });
        assert!(first.record().is_none());

        // The session that comes after is given a store of the same node, and the setup is over
        // for it as well.
        assert!(unlocks.store_for(phone()).record().is_none());
    }

    #[test]
    fn the_ledger_holds_an_end_once_however_often_it_is_told() {
        let ends = UnwrittenEnds::default();
        ends.add(phone(), &sealed(1));
        for _ in 0..70 {
            ends.add(tablet(), &sealed(2));
        }
        assert!(ends.holds(tablet(), &sealed(2)));
        assert!(
            ends.holds(phone(), &sealed(1)),
            "an end that is told again takes no further place and pushes no other out"
        );
    }

    #[test]
    fn the_ledger_remembers_the_last_sixty_four_ends() {
        let ends = UnwrittenEnds::default();
        for version in 0..65 {
            ends.add(phone(), &sealed(version));
        }
        assert!(!ends.holds(phone(), &sealed(0)), "the oldest is forgotten");
        for version in 1..65 {
            assert!(ends.holds(phone(), &sealed(version)), "{version}");
        }
    }

    #[test]
    fn forgetting_an_end_leaves_every_other() {
        let ends = UnwrittenEnds::default();
        ends.add(phone(), &sealed(1));
        // The unlock of the same device was set up again, and ended again.
        ends.add(phone(), &sealed(2));
        // Another device with a key of the same bytes.
        ends.add(tablet(), &sealed(2));

        ends.remove(phone(), &sealed(2));

        assert!(!ends.holds(phone(), &sealed(2)));
        assert!(
            ends.holds(phone(), &sealed(1)),
            "another key of that device"
        );
        assert!(
            ends.holds(tablet(), &sealed(2)),
            "that key of another device"
        );
    }

    #[test]
    fn a_finish_that_does_not_end_the_setup_and_could_not_be_written_does_not_end_it_for_the_run() {
        let rig = Rig::new();
        rig.disk_fails();
        rig.conclude(UnlockFinish::Done(UnlockOutcome::Busy), Some(sealed(1)));
        assert_eq!(rig.store.record().unwrap().sealed, sealed(1));
    }

    #[test]
    fn restoring_gives_back_a_setup_that_ended_and_keeps_its_line() {
        let rig = Rig::new();
        rig.conclude(rejected(), Some(sealed(1)));
        rig.store.restore(&sealed(9));
        assert!(rig.store.record().is_none(), "another key restores nothing");

        rig.store.restore(&sealed(1));
        assert_eq!(rig.store.record().unwrap().sealed, sealed(1));
        assert!(rig.on_disk().get(&phone()).unwrap().can_unlock(&target()));
        assert_eq!(rig.on_disk().unlock_history().len(), 1);
    }

    #[test]
    fn restoring_a_setup_whose_end_was_never_written_forgets_the_end() {
        let rig = Rig::new();
        rig.disk_fails();
        rig.conclude(rejected(), Some(sealed(1)));
        assert!(rig.store.record().is_none());

        rig.store.restore(&sealed(1));
        assert_eq!(rig.store.record().unwrap().sealed, sealed(1));
    }

    #[test]
    fn restoring_a_setup_whose_end_was_written_leaves_it_ended_when_the_disk_does_not_take_it() {
        let rig = Rig::new();
        rig.conclude(rejected(), Some(sealed(1)));
        rig.disk_fails();

        rig.store.restore(&sealed(1));
        assert!(rig.store.record().is_none());
        assert!(!rig.trust.get(&phone()).unwrap().can_unlock(&target()));
    }

    #[test]
    fn what_could_not_be_written_down_is_remembered_for_a_viewer_and_a_key_but_not_for_ever() {
        let ends = UnwrittenEnds::default();
        assert!(!ends.holds(phone(), &sealed(1)));
        ends.add(phone(), &sealed(1));
        ends.add(phone(), &sealed(1));
        assert!(ends.holds(phone(), &sealed(1)));
        assert!(!ends.holds(phone(), &sealed(2)));
        assert!(!ends.holds(tablet(), &sealed(1)), "it is for that viewer");
        ends.remove(phone(), &sealed(1));
        assert!(
            !ends.holds(phone(), &sealed(1)),
            "one entry, whatever the times it was added"
        );

        let many = u8::try_from(MAX_UNWRITTEN_ENDS).unwrap() + 10;
        for version in 0..many {
            ends.add(phone(), &sealed(version));
        }
        assert!(!ends.holds(phone(), &sealed(0)), "the oldest is forgotten");
        assert!(ends.holds(phone(), &sealed(many - 1)));
        assert_eq!(lock(&ends.0).len(), MAX_UNWRITTEN_ENDS);
    }

    #[test]
    fn two_devices_are_each_given_their_own_record() {
        let rig = Rig::new();
        rig.trust
            .set_up_unlock(&tablet(), target(), new_unlock(4), 1_500)
            .unwrap();
        let tablet_store = rig.store_for(tablet());
        assert_eq!(tablet_store.record().unwrap().sealed, sealed(4));
        assert_eq!(rig.store.record().unwrap().sealed, sealed(1));

        rig.conclude(rejected(), Some(sealed(1)));
        assert!(rig.store.record().is_none());
        assert_eq!(
            tablet_store.record().unwrap().sealed,
            sealed(4),
            "the end of one is not the end of the other"
        );
    }
}
