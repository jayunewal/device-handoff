// SPDX-License-Identifier: Apache-2.0

//! What a device keeps so that a paired device can unlock it, and the history of the attempts
//! (`docs/adr/0013-unlock-from-a-phone.md`).
//!
//! The person sets the unlock up at this device, for one paired device at a time. The platform
//! checks the password and hands over a finished record: the password in a box that only a key
//! can open, that key sealed to a key of the paired device, and the public key it was sealed to.
//! This module keeps it. The record is part of the paired device ([`PairedDevice::unlock`]), so
//! that unpairing the device deletes it, and it is saved in the same file and in the same step as
//! everything else about the device. The core never reads the password and never holds a key of
//! the box: it keeps bytes that it cannot open.
//!
//! A box, a sealed key and a public key are not printed by anything here.

use std::fmt;

use baton_proto::{
    DeviceId,
    device::sanitize_display_name,
    unlock::{SealedKey, Sealing, SealingKey},
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, Result},
    streaming::{
        UnlockFinish,
        unlock::{MAX_BOX_BYTES, Record, Replacement},
    },
    trust::{PairedDevice, TrustStore, hex_id},
};

/// How many attempts the history keeps. When there are more, the oldest are forgotten.
pub const MAX_UNLOCK_HISTORY: usize = 100;

/// The size of a public key of P-256 in the uncompressed form of ANSI X9.63, which is the only
/// way of sealing that a target sets up with, and its first byte.
const P256_POINT_BYTES: usize = 65;
const P256_UNCOMPRESSED: u8 = 0x04;

/// What the platform hands over when the person has set the unlock up for a paired device.
///
/// The platform has checked the password against the account of the person, put it in a box under
/// a random key, sealed that key to `public`, and wiped the password and the key. The key of the
/// box is in none of this.
#[derive(Clone)]
pub struct NewUnlock {
    /// The password, padded and encrypted. The core keeps it and does not read it.
    pub boxed: Vec<u8>,
    /// The key of the box, sealed to `public`. This is what the paired device opens.
    pub sealed: SealedKey,
    /// How `sealed` is sealed.
    pub sealing: Sealing,
    /// The key of the paired device that the key of the box was sealed to, and that a new key is
    /// sealed to at every unlock.
    pub public: SealingKey,
}

impl fmt::Debug for NewUnlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewUnlock")
            .field("boxed", &format_args!("{} bytes", self.boxed.len()))
            .field("sealed", &self.sealed)
            .field("sealing", &self.sealing)
            .field("public", &self.public)
            .finish()
    }
}

impl NewUnlock {
    /// Whether the record is within the limits of the protocol, and made the way this version
    /// seals.
    fn check(&self) -> std::result::Result<(), &'static str> {
        if self.boxed.is_empty() || self.boxed.len() > MAX_BOX_BYTES {
            return Err("the box of an unlock is empty or too large");
        }
        if self.sealed.validate().is_err() {
            return Err("the sealed key of an unlock is empty or too large");
        }
        check_sealing_key(self.sealing, &self.public)
    }
}

/// Whether `public` is a key that a secret can be sealed to the way `sealing` says, and one that
/// this version seals with. A viewer holds the key it gives to this, and so does a target that is
/// given one. The size of the key is the size that the way of sealing has: one that is added
/// later checks its own.
pub(crate) fn check_sealing_key(
    sealing: Sealing,
    public: &SealingKey,
) -> std::result::Result<(), &'static str> {
    match sealing {
        Sealing::EciesP256 => {
            let point = public.as_bytes();
            if point.len() != P256_POINT_BYTES || point.first() != Some(&P256_UNCOMPRESSED) {
                return Err("the public key of an unlock is not a point of P-256");
            }
        }
        Sealing::Other => {
            return Err("an unlock cannot be set up with a way of sealing that is not known");
        }
    }
    Ok(())
}

/// The unlock that the person set up at this device for a paired device, as it is kept.
///
/// It is useless without the paired device: the box opens with a key that only the paired device
/// can open, after the face of its person. It is deleted when the person turns the unlock off,
/// sets it up again (which replaces it), or unpairs the device.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedUnlock {
    /// This device when the person set the unlock up. The box is bound to it, so a record that
    /// comes from another device (a copied file) is of no use here.
    #[serde(with = "hex_id")]
    target: DeviceId,
    /// The password, padded and encrypted by the platform. It is replaced at every unlock.
    #[serde(rename = "box", with = "base64_bytes")]
    boxed: Vec<u8>,
    /// The key of the box, sealed to `public`. It is replaced at every unlock.
    #[serde(with = "base64_bytes")]
    sealed: Vec<u8>,
    /// How `sealed` is sealed.
    sealing: Sealing,
    /// The key of the paired device that every new key of the box is sealed to.
    #[serde(with = "base64_bytes")]
    public: Vec<u8>,
    /// When the person set it up, in seconds since the Unix epoch.
    set_up_at: u64,
    /// Whether the unlock can be tried. It cannot after a password that the system did not
    /// accept, a box that the key did not open, or a paired device that lost its key: only the
    /// person at this device can set it up again.
    usable: bool,
}

impl fmt::Debug for PairedUnlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairedUnlock")
            .field("target", &self.target.short())
            .field("boxed", &format_args!("{} bytes", self.boxed.len()))
            .field("sealed", &format_args!("{} bytes", self.sealed.len()))
            .field("sealing", &self.sealing)
            .field("public", &format_args!("{} bytes", self.public.len()))
            .field("set_up_at", &self.set_up_at)
            .field("usable", &self.usable)
            .finish()
    }
}

impl PairedUnlock {
    /// The record of an unlock that was just set up, for the device `target`.
    fn set_up(
        target: DeviceId,
        new: NewUnlock,
        now: u64,
    ) -> std::result::Result<Self, &'static str> {
        new.check()?;
        Ok(Self {
            target,
            boxed: new.boxed,
            sealed: new.sealed.as_bytes().to_vec(),
            sealing: new.sealing,
            public: new.public.as_bytes().to_vec(),
            set_up_at: now,
            usable: true,
        })
    }

    /// When the person set the unlock up, in seconds since the Unix epoch.
    #[must_use]
    pub fn set_up_at(&self) -> u64 {
        self.set_up_at
    }

    /// Whether the unlock can be tried. If not, it ended (the password was not accepted, the box
    /// was not opened by its key, or the paired device lost its key), and only the person at
    /// this device can set it up again.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.usable
    }

    /// Whether what was read from a file is within the limits, and something this version can
    /// use. A record that is not is dropped when the file is read, as if it had not been set up.
    pub(crate) fn is_valid(&self) -> bool {
        NewUnlock {
            boxed: self.boxed.clone(),
            sealed: SealedKey::new(self.sealed.clone()),
            sealing: self.sealing,
            public: SealingKey::new(self.public.clone()),
        }
        .check()
        .is_ok()
    }

    /// What a session needs to unlock for `viewer` on `target`, if the record is usable and was
    /// made for `target`.
    pub(crate) fn record(&self, target: DeviceId, viewer: DeviceId) -> Option<Record> {
        (self.usable && self.target == target).then(|| Record {
            boxed: self.boxed.clone(),
            sealed: SealedKey::new(self.sealed.clone()),
            sealing: self.sealing,
            public: SealingKey::new(self.public.clone()),
            target,
            viewer,
        })
    }

    /// Whether the record is usable on `target` and holds exactly the sealed key `sealed`.
    pub(crate) fn is_usable_with(&self, target: DeviceId, sealed: &SealedKey) -> bool {
        self.usable && self.target == target && self.sealed == sealed.as_bytes()
    }

    /// Puts a new box and a new sealed key in place of the ones of the record, if it is usable on
    /// `target` and still holds the sealed key `old`. Returns whether it did.
    pub(crate) fn replace(&mut self, target: DeviceId, old: &SealedKey, new: Replacement) -> bool {
        if !self.is_usable_with(target, old) {
            return false;
        }
        self.boxed = new.boxed;
        self.sealed = new.sealed.as_bytes().to_vec();
        true
    }

    /// Ends the setup, if the record holds the sealed key `sealed`: the person has to set it up
    /// again. Returns whether it did.
    pub(crate) fn end(&mut self, sealed: &SealedKey) -> bool {
        let holds = self.sealed == sealed.as_bytes();
        if holds {
            self.usable = false;
        }
        holds
    }

    /// Takes back an ended setup, if the record holds the sealed key `sealed`. Returns whether
    /// it did.
    pub(crate) fn restore(&mut self, sealed: &SealedKey) -> bool {
        let ended = !self.usable && self.sealed == sealed.as_bytes();
        if ended {
            self.usable = true;
        }
        ended
    }
}

impl PairedDevice {
    /// Whether the person set up an unlock for this device that can be tried on `target`, which
    /// is this device: it was made for it, and it has not ended.
    #[must_use]
    pub fn can_unlock(&self, target: &DeviceId) -> bool {
        self.unlock
            .as_ref()
            .is_some_and(|unlock| unlock.usable && unlock.target == *target)
    }
}

/// An attempt to unlock this device, in the history that it keeps.
///
/// It says nothing of the password, the keys or the challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlockEntry {
    /// When it ended, in seconds since the Unix epoch.
    pub at: u64,
    /// The paired device that asked.
    #[serde(with = "hex_id")]
    pub device: DeviceId,
    /// The name of that device as the person gave it, at that time.
    pub name: String,
    /// How it ended.
    pub finish: UnlockFinish,
}

/// The last attempts to unlock this device, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UnlockHistory(Vec<UnlockEntry>);

impl UnlockHistory {
    /// The history that a file holds: the newest [`MAX_UNLOCK_HISTORY`] entries, with names made
    /// fit to show.
    pub(crate) fn from_stored(mut entries: Vec<UnlockEntry>) -> Self {
        let surplus = entries.len().saturating_sub(MAX_UNLOCK_HISTORY);
        entries.drain(..surplus);
        for entry in &mut entries {
            entry.name = sanitize_display_name(&entry.name);
        }
        Self(entries)
    }

    /// Writes down an attempt, and forgets the oldest if there are too many.
    pub(crate) fn push(&mut self, entry: UnlockEntry) {
        self.0.push(entry);
        let surplus = self.0.len().saturating_sub(MAX_UNLOCK_HISTORY);
        self.0.drain(..surplus);
    }

    /// The attempts in the order they were made, for saving.
    pub(crate) fn entries(&self) -> &[UnlockEntry] {
        &self.0
    }

    /// The attempts, the latest first, for showing.
    pub(crate) fn newest_first(&self) -> Vec<UnlockEntry> {
        self.0.iter().rev().cloned().collect()
    }
}

impl TrustStore {
    /// Keeps the unlock that the person set up at this device (`target`) for the paired device
    /// `id`, and replaces what was kept for it before. Returns `false` if there is no such
    /// device.
    ///
    /// `now` is the time in seconds since the Unix epoch. The record is saved before it takes
    /// effect, as every change to the list is, and this blocks the calling thread for that long.
    ///
    /// # Errors
    ///
    /// [`Error::BadItem`] if the record is beyond a limit or made with a way of sealing that this
    /// version does not know; otherwise the list could not be saved, and nothing changed.
    pub fn set_up_unlock(
        &self,
        id: &DeviceId,
        target: DeviceId,
        new: NewUnlock,
        now: u64,
    ) -> Result<bool> {
        let unlock =
            PairedUnlock::set_up(target, new, now).map_err(|why| Error::BadItem(why.to_owned()))?;
        let kept = self.amend(id, |device, _| {
            device.unlock = Some(unlock);
            Some(())
        })?;
        Ok(kept.is_some())
    }

    /// Deletes the unlock that was set up for the paired device `id`: the box, the sealed key
    /// and the public key. Returns `false` if there was none.
    ///
    /// # Errors
    ///
    /// The list could not be saved, and nothing changed.
    pub fn turn_off_unlock(&self, id: &DeviceId) -> Result<bool> {
        let off = self.amend(id, |device, _| device.unlock.take().map(|_| ()))?;
        Ok(off.is_some())
    }

    /// The last attempts to unlock this device, the latest first. There are at most
    /// [`MAX_UNLOCK_HISTORY`].
    #[must_use]
    pub fn unlock_history(&self) -> Vec<UnlockEntry> {
        self.history().newest_first()
    }
}

/// Bytes are written as base64 text so that the file stays readable.
mod base64_bytes {
    use data_encoding::BASE64;
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&BASE64.encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        BASE64
            .decode(String::deserialize(deserializer)?.as_bytes())
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use baton_proto::{DeviceInfo, Platform, limits::MAX_SEALED_KEY_BYTES};
    use data_encoding::BASE64;
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;
    use crate::{
        fsutil::write_atomic,
        streaming::{CancelReason, UnlockOutcome},
        testing::unlocks::{boxed, new_unlock as unlock, point, replacement, sealed},
        trust::PairOutcome,
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

    fn info(name: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.to_owned(),
            platform: Platform::Linux,
            app_version: "0.1.0".to_owned(),
        }
    }

    fn set_up(version: u8) -> PairedUnlock {
        PairedUnlock::set_up(target(), unlock(version), 1_234).unwrap()
    }

    /// A list with the phone in it, in a directory of its own.
    fn store_with_phone() -> (TempDir, TrustStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::load(dir.path().join("devices.json")).unwrap();
        store.pair(phone(), &info("Phone"), 100).unwrap();
        (dir, store)
    }

    /// The file of the list as JSON.
    fn file(store: &TrustStore) -> Value {
        serde_json::from_str(&fs::read_to_string(store.path()).unwrap()).unwrap()
    }

    /// Writes `value` as the file of the list and loads the list from it.
    fn load(dir: &TempDir, value: &Value) -> TrustStore {
        let path = dir.path().join("devices.json");
        write_atomic(&path, value.to_string().as_bytes()).unwrap();
        TrustStore::load(path).unwrap()
    }

    /// A list of the phone and a tablet, the phone with an unlock, and its file.
    fn list_with_an_unlock() -> (TempDir, Value) {
        let (dir, store) = store_with_phone();
        store.pair(id(3), &info("Tablet"), 200).unwrap();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_234)
            .unwrap();
        let value = file(&store);
        assert_eq!(
            value["devices"][0]["id"],
            "02".repeat(32),
            "the phone is first"
        );
        (dir, value)
    }

    /// A way to damage a `T`.
    type Damage<T> = fn(&mut T);

    fn entry(at: u64, name: &str) -> UnlockEntry {
        UnlockEntry {
            at,
            device: phone(),
            name: name.to_owned(),
            finish: UnlockFinish::Done(UnlockOutcome::Unlocked),
        }
    }

    #[test]
    fn an_unlock_made_the_way_this_version_seals_is_accepted() {
        let kept = set_up(1);
        assert!(kept.is_valid());
        assert!(kept.usable());
        assert_eq!(kept.set_up_at(), 1_234);
        let record = kept.record(target(), phone()).unwrap();
        assert_eq!(record.boxed, boxed(1));
        assert_eq!(record.sealed, sealed(1));
        assert_eq!(record.sealing, Sealing::EciesP256);
        assert_eq!(record.public, SealingKey::new(point(0xc0)));
        assert_eq!((record.target, record.viewer), (target(), phone()));
    }

    #[test]
    fn an_unlock_at_the_limits_is_accepted() {
        let mut new = unlock(1);
        new.boxed = vec![1; MAX_BOX_BYTES];
        new.sealed = SealedKey::new(vec![1; MAX_SEALED_KEY_BYTES]);
        assert!(PairedUnlock::set_up(target(), new, 1).is_ok());
    }

    #[test]
    fn an_unlock_beyond_a_limit_is_refused() {
        let cases: [(&str, Damage<NewUnlock>); 9] = [
            ("an empty box", |new| new.boxed.clear()),
            ("a box over the limit", |new| {
                new.boxed = vec![1; MAX_BOX_BYTES + 1];
            }),
            ("an empty sealed key", |new| {
                new.sealed = SealedKey::new(Vec::new());
            }),
            ("a sealed key over the limit", |new| {
                new.sealed = SealedKey::new(vec![1; MAX_SEALED_KEY_BYTES + 1]);
            }),
            ("an empty public key", |new| {
                new.public = SealingKey::new(Vec::new());
            }),
            ("a public key a byte short", |new| {
                new.public = SealingKey::new(point(0xc0)[1..].to_vec());
            }),
            ("a public key a byte long", |new| {
                new.public = SealingKey::new([point(0xc0), vec![0]].concat());
            }),
            ("a compressed point", |new| {
                new.public = SealingKey::new([vec![0x02], point(0xc0)[1..].to_vec()].concat());
            }),
            ("a way of sealing that is not known", |new| {
                new.sealing = Sealing::Other;
            }),
        ];
        for (what, damage) in cases {
            let mut new = unlock(1);
            damage(&mut new);
            assert!(PairedUnlock::set_up(target(), new, 1).is_err(), "{what}");
        }
    }

    #[test]
    fn a_device_without_an_unlock_is_written_as_before() {
        let (_dir, store) = store_with_phone();
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("unlock"), "{text}");
        assert!(!store.get(&phone()).unwrap().can_unlock(&target()));
    }

    #[test]
    fn an_unlock_is_kept_for_the_device_and_survives_a_reload() {
        let (_dir, store) = store_with_phone();
        assert!(
            store
                .set_up_unlock(&phone(), target(), unlock(1), 1_234)
                .unwrap()
        );
        assert!(store.get(&phone()).unwrap().can_unlock(&target()));

        let again = TrustStore::load(store.path()).unwrap();
        assert_eq!(again.list(), store.list());
        let kept = again.get(&phone()).unwrap().unlock.unwrap();
        assert_eq!((kept.set_up_at(), kept.usable()), (1_234, true));
        assert_eq!(kept.record(target(), phone()).unwrap().boxed, boxed(1));
    }

    #[test]
    fn the_file_holds_the_unlock_as_readable_text() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_234)
            .unwrap();
        assert_eq!(
            file(&store)["devices"][0]["unlock"],
            json!({
                "target": "01".repeat(32),
                "box": BASE64.encode(&boxed(1)),
                "sealed": BASE64.encode(sealed(1).as_bytes()),
                "sealing": "EciesP256",
                "public": BASE64.encode(&point(0xc0)),
                "set_up_at": 1_234,
                "usable": true,
            })
        );
    }

    #[test]
    fn an_unlock_for_a_device_that_is_not_paired_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::load(dir.path().join("devices.json")).unwrap();
        assert!(
            !store
                .set_up_unlock(&phone(), target(), unlock(1), 1)
                .unwrap()
        );
        assert!(!store.path().exists(), "there was nothing to save");
    }

    #[test]
    fn an_unlock_that_is_refused_changes_nothing() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_000)
            .unwrap();
        let before = fs::read(store.path()).unwrap();

        let mut bad = unlock(2);
        bad.sealing = Sealing::Other;
        let error = store
            .set_up_unlock(&phone(), target(), bad, 2_000)
            .unwrap_err();
        assert!(matches!(error, Error::BadItem(_)), "{error}");
        assert_eq!(fs::read(store.path()).unwrap(), before);
        let kept = store.get(&phone()).unwrap().unlock.unwrap();
        assert_eq!(kept.set_up_at(), 1_000);
    }

    #[test]
    fn setting_the_unlock_up_again_replaces_it_and_makes_it_usable() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_000)
            .unwrap();
        // The setup ends: the system did not accept the password.
        store
            .amend(&phone(), |device, _| {
                device.unlock.as_mut()?.end(&sealed(1)).then_some(())
            })
            .unwrap()
            .unwrap();
        let device = store.get(&phone()).unwrap();
        assert!(!device.can_unlock(&target()));
        assert!(!device.unlock.unwrap().usable());

        store
            .set_up_unlock(&phone(), target(), unlock(2), 2_000)
            .unwrap();
        let device = store.get(&phone()).unwrap();
        assert!(device.can_unlock(&target()));
        let kept = device.unlock.unwrap();
        assert_eq!((kept.set_up_at(), kept.usable()), (2_000, true));
        let record = kept.record(target(), phone()).unwrap();
        assert_eq!((record.boxed, record.sealed), (boxed(2), sealed(2)));
    }

    #[test]
    fn an_unlock_that_ended_is_kept_but_cannot_be_used() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_000)
            .unwrap();
        store
            .amend(&phone(), |device, _| {
                device.unlock.as_mut()?.end(&sealed(1)).then_some(())
            })
            .unwrap();

        let again = TrustStore::load(store.path()).unwrap();
        let device = again.get(&phone()).unwrap();
        assert!(!device.can_unlock(&target()));
        let kept = device.unlock.unwrap();
        assert!(!kept.usable());
        assert!(kept.record(target(), phone()).is_none());
    }

    #[test]
    fn turning_the_unlock_off_deletes_it_from_the_file() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_234)
            .unwrap();
        assert!(store.turn_off_unlock(&phone()).unwrap());

        assert!(store.get(&phone()).unwrap().unlock.is_none());
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("unlock"), "{text}");
        assert!(!text.contains(&BASE64.encode(&boxed(1))), "{text}");
        assert!(
            !store.turn_off_unlock(&phone()).unwrap(),
            "there is nothing left to turn off"
        );
        assert!(!store.turn_off_unlock(&id(9)).unwrap(), "no such device");
    }

    #[test]
    fn unpairing_deletes_the_unlock_and_pairing_again_does_not_bring_it_back() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_234)
            .unwrap();
        store.remove(&phone()).unwrap().unwrap();
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains(&BASE64.encode(&boxed(1))), "{text}");

        assert_eq!(
            store.pair(phone(), &info("Phone"), 3_000).unwrap(),
            PairOutcome::Added
        );
        assert!(store.get(&phone()).unwrap().unlock.is_none());
    }

    #[test]
    fn pairing_a_paired_device_again_keeps_its_unlock() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_234)
            .unwrap();
        assert_eq!(
            store.pair(phone(), &info("Phone"), 3_000).unwrap(),
            PairOutcome::Refreshed
        );
        assert!(store.get(&phone()).unwrap().can_unlock(&target()));
    }

    #[test]
    fn an_unlock_made_for_another_device_cannot_be_used_on_this_one() {
        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_234)
            .unwrap();
        let device = store.get(&phone()).unwrap();
        let elsewhere = id(7);
        assert!(device.can_unlock(&target()));
        assert!(!device.can_unlock(&elsewhere));

        let mut kept = device.unlock.unwrap();
        assert!(kept.record(elsewhere, phone()).is_none());
        assert!(!kept.is_usable_with(elsewhere, &sealed(1)));
        assert!(!kept.replace(elsewhere, &sealed(1), replacement(2)));
        assert_eq!(kept, set_up(1));
    }

    #[test]
    fn a_change_to_the_record_applies_only_to_the_sealed_key_that_was_read() {
        let mut kept = set_up(1);

        assert!(!kept.replace(target(), &sealed(9), replacement(2)));
        assert_eq!(
            kept,
            set_up(1),
            "a key that is not the record's changes nothing"
        );
        assert!(kept.replace(target(), &sealed(1), replacement(2)));
        let record = kept.record(target(), phone()).unwrap();
        assert_eq!((record.boxed, record.sealed), (boxed(2), sealed(2)));
        assert_eq!(kept.set_up_at(), 1_234, "the rest of the record stays");
        assert!(
            !kept.replace(target(), &sealed(1), replacement(3)),
            "the old key no longer applies"
        );

        assert!(!kept.end(&sealed(1)));
        assert!(kept.usable());
        assert!(kept.end(&sealed(2)));
        assert!(!kept.usable());
        assert!(
            !kept.replace(target(), &sealed(2), replacement(4)),
            "an ended setup keeps what it has"
        );

        assert!(!kept.restore(&sealed(1)));
        assert!(!kept.usable());
        assert!(kept.restore(&sealed(2)));
        assert!(kept.usable());
        assert!(!kept.restore(&sealed(2)), "it was not ended");
    }

    #[test]
    fn an_unlock_that_cannot_be_read_is_left_out_and_the_rest_of_the_list_is_kept() {
        let (dir, good) = list_with_an_unlock();
        let damages: [(&str, Damage<Value>); 10] = [
            ("null", |unlock| *unlock = Value::Null),
            ("text", |unlock| *unlock = json!("secret")),
            ("a number", |unlock| *unlock = json!(7)),
            ("a list", |unlock| *unlock = json!([])),
            ("nothing in it", |unlock| *unlock = json!({})),
            ("a field missing", |unlock| {
                unlock.as_object_mut().unwrap().remove("sealed");
            }),
            ("a box that is not base64", |unlock| {
                unlock["box"] = json!("not base64!");
            }),
            ("a box that is a number", |unlock| unlock["box"] = json!(3)),
            ("a flag that is text", |unlock| {
                unlock["usable"] = json!("yes");
            }),
            ("a target that is not an id", |unlock| {
                unlock["target"] = json!("zz");
            }),
        ];
        for (what, damage) in damages {
            let mut value = good.clone();
            damage(&mut value["devices"][0]["unlock"]);
            let store = load(&dir, &value);
            assert_eq!(store.len(), 2, "{what}");
            assert!(store.get(&phone()).unwrap().unlock.is_none(), "{what}");
            assert!(store.get(&id(3)).is_some(), "{what}");
        }
        assert!(load(&dir, &good).get(&phone()).unwrap().unlock.is_some());
    }

    #[test]
    fn an_unlock_beyond_a_limit_in_the_file_is_left_out() {
        let (dir, good) = list_with_an_unlock();
        let damages: [(&str, Damage<Value>); 4] = [
            ("an empty box", |unlock| unlock["box"] = json!("")),
            ("a box over the limit", |unlock| {
                unlock["box"] = json!(BASE64.encode(&vec![1; MAX_BOX_BYTES + 1]));
            }),
            ("a way of sealing nobody knows", |unlock| {
                unlock["sealing"] = json!("SomethingNew");
            }),
            ("a key that is not a point", |unlock| {
                unlock["public"] = json!(BASE64.encode(&[4; 33]));
            }),
        ];
        for (what, damage) in damages {
            let mut value = good.clone();
            damage(&mut value["devices"][0]["unlock"]);
            let store = load(&dir, &value);
            assert_eq!(store.len(), 2, "{what}");
            assert!(store.get(&phone()).unwrap().unlock.is_none(), "{what}");
        }
    }

    #[test]
    fn nothing_that_is_printed_shows_the_box_or_a_key() {
        let new = unlock(1);
        assert_eq!(
            format!("{new:?}"),
            "NewUnlock { boxed: 40 bytes, sealed: SealedKey(113 bytes), sealing: EciesP256, \
             public: SealingKey(65 bytes) }"
        );
        let kept = set_up(1);
        assert_eq!(
            format!("{kept:?}"),
            format!(
                "PairedUnlock {{ target: {:?}, boxed: 40 bytes, sealed: 113 bytes, \
                 sealing: EciesP256, public: 65 bytes, set_up_at: 1234, usable: true }}",
                target().short()
            )
        );

        let (_dir, store) = store_with_phone();
        store
            .set_up_unlock(&phone(), target(), unlock(1), 1_234)
            .unwrap();
        let shown = format!("{:?}", store.get(&phone()).unwrap());
        assert!(shown.contains("PairedUnlock"), "{shown}");
        for secret in [boxed(1), sealed(1).as_bytes().to_vec(), point(0xc0)] {
            assert!(!shown.contains(&format!("{secret:?}")), "{shown}");
            assert!(!shown.contains(&BASE64.encode(&secret)), "{shown}");
        }
    }

    #[test]
    fn the_history_keeps_the_newest_hundred() {
        let mut history = UnlockHistory::default();
        assert_eq!(history.entries(), []);
        for at in 0..150 {
            history.push(entry(at, "Phone"));
        }
        assert_eq!(history.entries().len(), MAX_UNLOCK_HISTORY);
        assert_eq!(history.entries().first().unwrap().at, 50);
        assert_eq!(history.entries().last().unwrap().at, 149);
        let shown: Vec<u64> = history.newest_first().iter().map(|line| line.at).collect();
        assert_eq!((shown[0], shown[99]), (149, 50));
    }

    #[test]
    fn a_history_read_from_a_file_is_cut_to_the_newest_hundred_and_its_names_are_cleaned() {
        let (dir, store) = store_with_phone();
        let rude = "Pho\u{202e}ne\n\u{0}";
        let clean = sanitize_display_name(rude);
        assert_ne!(clean, rude, "the name must need cleaning");
        let mut value = file(&store);
        value["unlock_history"] = Value::Array(
            (0..130)
                .map(|at| serde_json::to_value(entry(at, rude)).unwrap())
                .collect(),
        );

        let history = load(&dir, &value).unlock_history();
        assert_eq!(history.len(), MAX_UNLOCK_HISTORY);
        assert_eq!((history[0].at, history[99].at), (129, 30));
        assert!(history.iter().all(|line| line.name == clean));
    }

    #[test]
    fn a_line_of_the_history_that_cannot_be_read_is_left_out() {
        let (dir, store) = store_with_phone();
        let mut value = file(&store);
        let line = |at| serde_json::to_value(entry(at, "Phone")).unwrap();
        let mut from_the_future = line(2);
        from_the_future["finish"] = json!({"FromTheFuture": 1});
        value["unlock_history"] = json!([line(1), from_the_future, "text", 5, line(3)]);

        let store = load(&dir, &value);
        let shown: Vec<u64> = store.unlock_history().iter().map(|line| line.at).collect();
        assert_eq!(shown, [3, 1]);
        assert!(store.contains(&phone()));
    }

    #[test]
    fn a_history_that_is_not_a_list_is_left_out() {
        let (dir, store) = store_with_phone();
        for bad in [json!("text"), json!(3), json!({}), Value::Null] {
            let mut value = file(&store);
            value["unlock_history"] = bad.clone();
            let store = load(&dir, &value);
            assert!(store.unlock_history().is_empty(), "{bad}");
            assert!(store.contains(&phone()), "{bad}");
        }
    }

    #[test]
    fn a_line_of_the_history_says_when_who_and_how_and_nothing_else() {
        let finishes = [
            (
                UnlockFinish::Done(UnlockOutcome::Unlocked),
                json!({"Done": "Unlocked"}),
            ),
            (
                UnlockFinish::Done(UnlockOutcome::Rejected),
                json!({"Done": "Rejected"}),
            ),
            (
                UnlockFinish::Cancelled(CancelReason::KeyLost),
                json!({"Cancelled": "KeyLost"}),
            ),
            (UnlockFinish::Ended, json!("Ended")),
        ];
        for (finish, text) in finishes {
            let line = UnlockEntry {
                finish,
                ..entry(5, "Phone")
            };
            let value = serde_json::to_value(&line).unwrap();
            assert_eq!(
                value,
                json!({"at": 5, "device": "02".repeat(32), "name": "Phone", "finish": text})
            );
            assert_eq!(serde_json::from_value::<UnlockEntry>(value).unwrap(), line);
        }
    }

    #[test]
    fn an_outcome_nobody_knows_is_kept_as_one() {
        let mut value = serde_json::to_value(entry(5, "Phone")).unwrap();
        value["finish"] = json!({"Done": "BrandNew"});
        let line: UnlockEntry = serde_json::from_value(value).unwrap();
        assert_eq!(line.finish, UnlockFinish::Done(UnlockOutcome::Other));
    }

    #[test]
    fn a_device_that_is_paired_has_no_history_in_its_file_until_there_is_one() {
        let (_dir, store) = store_with_phone();
        assert_eq!(store.unlock_history(), Vec::new());
        let text = fs::read_to_string(store.path()).unwrap();
        assert!(!text.contains("history"), "{text}");
    }
}
