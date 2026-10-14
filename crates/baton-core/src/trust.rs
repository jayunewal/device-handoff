// SPDX-License-Identifier: Apache-2.0

//! The list of devices this one trusts.
//!
//! A device is on the list because a person approved pairing with it. The
//! transport consults the list after every handshake and refuses connections
//! from any key that is not on it, so removing a device here is what revokes
//! it. The list is kept in memory and saved to `devices.json` on every change.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use baton_proto::{DeviceId, DeviceInfo, Platform, device::sanitize_display_name};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    error::{Error, Result},
    fsutil::{read_text, write_atomic},
    policy::DevicePolicy,
};

/// Most devices that can be paired at once.
pub const MAX_DEVICES: usize = 256;

/// Most direct addresses remembered per device.
pub const MAX_ADDR_HINTS: usize = 8;

/// Longest relay URL remembered.
const MAX_RELAY_BYTES: usize = 255;

/// Newest file format this build reads and writes.
const FORMAT_VERSION: u32 = 1;

/// How stale `last_seen_at` may get before seeing a device again rewrites the file.
const SEEN_REFRESH_SECS: u64 = 300;

/// A device this one is paired with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    /// The device's identity.
    #[serde(with = "hex_id")]
    pub id: DeviceId,
    /// Its name, as last reported and cleaned up for display.
    pub name: String,
    /// Its operating system.
    pub platform: Platform,
    /// When it was paired, in seconds since the Unix epoch.
    pub added_at: u64,
    /// When this device last reached it or heard from it.
    #[serde(default)]
    pub last_seen_at: Option<u64>,
    /// Where its home relay was last seen. A hint for reaching it across networks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    /// Addresses it was last reachable at. Hints only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addrs: Vec<SocketAddr>,
    /// What this device does with what it receives from that one.
    #[serde(default)]
    pub policy: DevicePolicy,
}

impl PairedDevice {
    fn new(id: DeviceId, info: &DeviceInfo, now: u64) -> Self {
        Self {
            id,
            name: sanitize_display_name(&info.name),
            platform: info.platform,
            added_at: now,
            last_seen_at: Some(now),
            relay: None,
            addrs: Vec::new(),
            policy: DevicePolicy::default(),
        }
    }

    /// Re-establishes the invariants that anything read from a file or a peer must satisfy.
    fn normalize(&mut self) {
        self.name = sanitize_display_name(&self.name);
        self.addrs.truncate(MAX_ADDR_HINTS);
        if self.relay.as_deref().is_some_and(|r| {
            r.is_empty() || r.len() > MAX_RELAY_BYTES || !r.bytes().all(|b| b.is_ascii_graphic())
        }) {
            self.relay = None;
        }
    }
}

/// Where a device was last reachable. See [`TrustStore::record_contact`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactHints {
    /// Its home relay.
    pub relay: Option<String>,
    /// Its direct addresses.
    pub addrs: Vec<SocketAddr>,
}

/// What [`TrustStore::pair`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairOutcome {
    /// A device that was not on the list is now on it.
    Added,
    /// The device was already paired; its details were refreshed and its policy kept.
    Refreshed,
}

/// A change to the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustEvent {
    /// A device was paired.
    Added(DeviceId),
    /// A paired device's details or policy changed.
    Updated(DeviceId),
    /// A device was unpaired. Connections from it must be closed.
    Removed(DeviceId),
}

#[derive(Serialize, Deserialize)]
struct StoredTrust {
    version: u32,
    devices: Vec<PairedDevice>,
}

type Devices = BTreeMap<DeviceId, PairedDevice>;

struct Inner {
    path: PathBuf,
    devices: RwLock<Devices>,
    events: broadcast::Sender<TrustEvent>,
}

/// The shared, persistent list of paired devices. Cloning gives another handle to the same list.
#[derive(Clone)]
pub struct TrustStore {
    inner: Arc<Inner>,
}

impl TrustStore {
    /// Loads the list from `path`. A missing file is an empty list.
    ///
    /// A file that exists but cannot be understood is an error, never an empty
    /// list: silently forgetting everyone would be wrong, and so would
    /// overwriting the file.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let devices = match read_text(&path)? {
            None => Devices::new(),
            Some(text) => parse(&path, &text)?,
        };
        let (events, _) = broadcast::channel(64);
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                devices: RwLock::new(devices),
                events,
            }),
        })
    }

    /// The file this list is saved to.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    fn read(&self) -> RwLockReadGuard<'_, Devices> {
        // A panic while holding the lock cannot leave the map half-updated
        // (every change is swapped in whole), so a poisoned lock is safe to reuse.
        self.inner
            .devices
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Devices> {
        self.inner
            .devices
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `id` is paired. This is fast and does not touch the disk.
    pub fn contains(&self, id: &DeviceId) -> bool {
        self.read().contains_key(id)
    }

    /// The paired device with this id.
    pub fn get(&self, id: &DeviceId) -> Option<PairedDevice> {
        self.read().get(id).cloned()
    }

    /// All paired devices, sorted by name.
    pub fn list(&self) -> Vec<PairedDevice> {
        let mut devices: Vec<_> = self.read().values().cloned().collect();
        devices.sort_by_cached_key(|d| (d.name.to_lowercase(), d.id));
        devices
    }

    /// How many devices are paired.
    pub fn len(&self) -> usize {
        self.read().len()
    }

    /// Whether no device is paired.
    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    /// Receives a [`TrustEvent`] for every change made through any handle to this list.
    ///
    /// Events are sent in the order the changes were made. A receiver that falls
    /// behind gets [`RecvError::Lagged`](broadcast::error::RecvError::Lagged) and
    /// should then re-read the list instead of relying on the events it missed.
    pub fn subscribe(&self) -> broadcast::Receiver<TrustEvent> {
        self.inner.events.subscribe()
    }

    /// Adds a device, or refreshes it if it is already paired.
    ///
    /// Pairing again keeps the device's policy and original pairing time.
    pub fn pair(&self, id: DeviceId, info: &DeviceInfo, now: u64) -> Result<PairOutcome> {
        let mut guard = self.write();
        let mut next = guard.clone();
        let outcome = if let Some(existing) = next.get_mut(&id) {
            existing.name = sanitize_display_name(&info.name);
            existing.platform = info.platform;
            existing.last_seen_at = Some(now);
            PairOutcome::Refreshed
        } else {
            if next.len() >= MAX_DEVICES {
                return Err(Error::TooManyDevices { limit: MAX_DEVICES });
            }
            next.insert(id, PairedDevice::new(id, info, now));
            PairOutcome::Added
        };
        self.commit(&mut guard, next)?;
        self.emit(match outcome {
            PairOutcome::Added => TrustEvent::Added(id),
            PairOutcome::Refreshed => TrustEvent::Updated(id),
        });
        Ok(outcome)
    }

    /// Changes a paired device. Returns `false` if there is no such device.
    ///
    /// The id cannot be changed. Names and hints are cleaned up again afterwards.
    /// The closure runs while the list is locked, so it must not call back into it.
    pub fn update(&self, id: &DeviceId, change: impl FnOnce(&mut PairedDevice)) -> Result<bool> {
        let mut guard = self.write();
        let mut next = guard.clone();
        let Some(device) = next.get_mut(id) else {
            return Ok(false);
        };
        change(device);
        device.id = *id;
        device.normalize();
        self.commit(&mut guard, next)?;
        self.emit(TrustEvent::Updated(*id));
        Ok(true)
    }

    /// Unpairs a device. Returns what was removed, if anything.
    pub fn remove(&self, id: &DeviceId) -> Result<Option<PairedDevice>> {
        let mut guard = self.write();
        let mut next = guard.clone();
        let Some(removed) = next.remove(id) else {
            return Ok(None);
        };
        self.commit(&mut guard, next)?;
        self.emit(TrustEvent::Removed(*id));
        Ok(Some(removed))
    }

    /// Notes that a device was just reached, and where.
    ///
    /// This runs on every connection, so it writes to disk only when the hints
    /// changed or the last-seen time is stale. It sends no event. Returns
    /// whether anything was saved.
    pub fn record_contact(&self, id: &DeviceId, now: u64, hints: &ContactHints) -> Result<bool> {
        let mut guard = self.write();
        let Some(current) = guard.get(id) else {
            return Ok(false);
        };
        let mut updated = current.clone();
        updated.last_seen_at = Some(now);
        if hints.relay.is_some() {
            updated.relay.clone_from(&hints.relay);
        }
        if !hints.addrs.is_empty() {
            updated.addrs.clone_from(&hints.addrs);
        }
        updated.normalize();

        let stale = current
            .last_seen_at
            .is_none_or(|seen| now.saturating_sub(seen) >= SEEN_REFRESH_SECS);
        let hints_changed = updated.relay != current.relay || updated.addrs != current.addrs;
        if !stale && !hints_changed {
            return Ok(false);
        }
        let mut next = guard.clone();
        next.insert(*id, updated);
        self.commit(&mut guard, next)?;
        Ok(true)
    }

    /// Saves `next`, and only if that worked makes it the current list.
    fn commit(&self, current: &mut Devices, next: Devices) -> Result<()> {
        let stored = StoredTrust {
            version: FORMAT_VERSION,
            devices: next.values().cloned().collect(),
        };
        let json =
            serde_json::to_vec_pretty(&stored).map_err(|e| Error::invalid(&self.inner.path, &e))?;
        write_atomic(&self.inner.path, &json)?;
        *current = next;
        Ok(())
    }

    /// Sending never blocks, so this is called with the list still locked: that is
    /// what keeps events in the order of the changes.
    fn emit(&self, event: TrustEvent) {
        // Nobody listening is fine.
        let _ = self.inner.events.send(event);
    }
}

impl std::fmt::Debug for TrustStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustStore")
            .field("path", &self.inner.path)
            .field("devices", &self.len())
            .finish()
    }
}

fn parse(path: &Path, text: &str) -> Result<Devices> {
    let stored: StoredTrust = serde_json::from_str(text).map_err(|e| Error::invalid(path, &e))?;
    if stored.version > FORMAT_VERSION {
        return Err(Error::UnsupportedVersion {
            path: path.to_owned(),
            found: stored.version,
            supported: FORMAT_VERSION,
        });
    }
    if stored.devices.len() > MAX_DEVICES {
        return Err(Error::invalid(
            path,
            &format!(
                "lists {} devices, more than the limit of {MAX_DEVICES}",
                stored.devices.len()
            ),
        ));
    }
    let mut devices = Devices::new();
    for mut device in stored.devices {
        device.normalize();
        let id = device.id;
        if devices.insert(id, device).is_some() {
            return Err(Error::invalid(
                path,
                &format!("device {id} is listed twice"),
            ));
        }
    }
    Ok(devices)
}

/// Device ids are written as hexadecimal text so the file is readable.
mod hex_id {
    use baton_proto::DeviceId;
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub(super) fn serialize<S: Serializer>(
        id: &DeviceId,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(id)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<DeviceId, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use baton_proto::Platform;
    use tempfile::TempDir;

    use super::*;
    use crate::policy::{ClipboardPolicy, LinkPolicy};

    fn id(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; DeviceId::LEN])
    }

    fn info(name: &str) -> DeviceInfo {
        DeviceInfo {
            name: name.to_owned(),
            platform: Platform::Linux,
            app_version: "0.1.0".to_owned(),
        }
    }

    fn store() -> (TempDir, TrustStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::load(dir.path().join("devices.json")).unwrap();
        (dir, store)
    }

    fn reload(store: &TrustStore) -> TrustStore {
        TrustStore::load(store.path()).unwrap()
    }

    #[test]
    fn a_missing_file_is_an_empty_list_and_is_not_created() {
        let (dir, store) = store();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        assert!(store.list().is_empty());
        assert!(!dir.path().join("devices.json").exists());
    }

    #[test]
    fn pairing_adds_a_device_and_saves_it() {
        let (_dir, store) = store();
        assert!(!store.contains(&id(1)));
        assert_eq!(
            store.pair(id(1), &info("Laptop"), 1_000).unwrap(),
            PairOutcome::Added
        );
        assert!(store.contains(&id(1)));

        let device = store.get(&id(1)).unwrap();
        assert_eq!(device.name, "Laptop");
        assert_eq!(device.platform, Platform::Linux);
        assert_eq!(device.added_at, 1_000);
        assert_eq!(device.last_seen_at, Some(1_000));
        assert_eq!(device.policy, DevicePolicy::default());

        let again = reload(&store);
        assert_eq!(again.list(), store.list());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1).unwrap();
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn the_file_is_readable_json_with_hex_ids() {
        let (_dir, store) = store();
        store.pair(id(0xab), &info("Laptop"), 1).unwrap();
        let text = fs::read_to_string(store.path()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["devices"][0]["id"], "ab".repeat(32));
        assert_eq!(value["devices"][0]["policy"]["links"], "inbox");
    }

    #[test]
    fn pairing_again_refreshes_but_keeps_policy_and_history() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1_000).unwrap();
        store
            .update(&id(1), |d| {
                d.policy.links = LinkPolicy::Open;
                d.policy.sync_clipboard_to = true;
            })
            .unwrap();

        let outcome = store.pair(id(1), &info("Renamed"), 2_000).unwrap();
        assert_eq!(outcome, PairOutcome::Refreshed);
        let device = store.get(&id(1)).unwrap();
        assert_eq!(device.name, "Renamed");
        assert_eq!(device.added_at, 1_000);
        assert_eq!(device.last_seen_at, Some(2_000));
        assert_eq!(device.policy.links, LinkPolicy::Open);
        assert!(device.policy.sync_clipboard_to);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn names_from_peers_are_cleaned_before_they_are_stored() {
        let (_dir, store) = store();
        store
            .pair(id(1), &info("Ann\u{202e}\u{7}'s\nlaptop"), 1)
            .unwrap();
        let name = store.get(&id(1)).unwrap().name;
        assert!(!name.contains('\u{202e}'));
        assert!(!name.chars().any(char::is_control));
        assert_eq!(reload(&store).get(&id(1)).unwrap().name, name);

        store.pair(id(2), &info(&"x".repeat(500)), 1).unwrap();
        assert!(store.get(&id(2)).unwrap().name.chars().count() <= 64);
        store.pair(id(3), &info(""), 1).unwrap();
        assert!(!store.get(&id(3)).unwrap().name.is_empty());
    }

    #[test]
    fn update_changes_a_device_but_never_its_identity() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1).unwrap();
        let changed = store
            .update(&id(1), |d| {
                d.id = id(9);
                d.policy.clipboard = ClipboardPolicy::Apply;
                d.name = "Evil\u{202e}".to_owned();
            })
            .unwrap();
        assert!(changed);
        assert!(!store.contains(&id(9)));
        let device = store.get(&id(1)).unwrap();
        assert_eq!(device.id, id(1));
        assert_eq!(device.policy.clipboard, ClipboardPolicy::Apply);
        assert!(!device.name.contains('\u{202e}'));
        assert_eq!(reload(&store).get(&id(1)).unwrap(), device);
    }

    #[test]
    fn updating_an_unknown_device_does_nothing() {
        let (_dir, store) = store();
        let changed = store.update(&id(1), |_| panic!("must not run")).unwrap();
        assert!(!changed);
        assert!(!store.path().exists());
    }

    #[test]
    fn removing_forgets_the_device_for_good() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1).unwrap();
        store.pair(id(2), &info("Phone"), 1).unwrap();

        let removed = store.remove(&id(1)).unwrap().unwrap();
        assert_eq!(removed.name, "Laptop");
        assert!(!store.contains(&id(1)));
        assert!(store.contains(&id(2)));
        assert!(!reload(&store).contains(&id(1)));

        assert!(store.remove(&id(1)).unwrap().is_none());
    }

    #[test]
    fn the_list_is_sorted_by_name_ignoring_case() {
        let (_dir, store) = store();
        store.pair(id(1), &info("zebra"), 1).unwrap();
        store.pair(id(2), &info("Apple"), 1).unwrap();
        store.pair(id(3), &info("banana"), 1).unwrap();
        let names: Vec<_> = store.list().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Apple", "banana", "zebra"]);
    }

    #[test]
    fn clones_are_handles_to_the_same_list() {
        let (_dir, store) = store();
        let other = store.clone();
        store.pair(id(1), &info("Laptop"), 1).unwrap();
        assert!(other.contains(&id(1)));
        other.remove(&id(1)).unwrap();
        assert!(!store.contains(&id(1)));
    }

    #[test]
    fn changes_are_announced_in_order() {
        let (_dir, store) = store();
        let mut events = store.subscribe();

        store.pair(id(1), &info("Laptop"), 1).unwrap();
        store.pair(id(1), &info("Laptop"), 2).unwrap();
        store
            .update(&id(1), |d| d.policy.links = LinkPolicy::Ignore)
            .unwrap();
        store.remove(&id(1)).unwrap();
        // None of these change anything, so none is announced.
        store.remove(&id(1)).unwrap();
        store.update(&id(1), |_| {}).unwrap();
        store
            .record_contact(&id(1), 10_000, &ContactHints::default())
            .unwrap();

        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            seen.push(event);
        }
        assert_eq!(
            seen,
            [
                TrustEvent::Added(id(1)),
                TrustEvent::Updated(id(1)),
                TrustEvent::Updated(id(1)),
                TrustEvent::Removed(id(1)),
            ]
        );
    }

    #[test]
    fn noting_contact_is_not_announced() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1_000).unwrap();
        let mut events = store.subscribe();
        store
            .record_contact(&id(1), 9_000, &ContactHints::default())
            .unwrap();
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn contact_is_saved_only_when_something_changed() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1_000).unwrap();
        let none = ContactHints::default();

        // Seen again moments later: nothing worth a disk write.
        assert!(!store.record_contact(&id(1), 1_010, &none).unwrap());
        assert_eq!(store.get(&id(1)).unwrap().last_seen_at, Some(1_000));

        // Long enough later that the time is stale.
        assert!(
            store
                .record_contact(&id(1), 1_000 + SEEN_REFRESH_SECS, &none)
                .unwrap()
        );
        assert_eq!(
            store.get(&id(1)).unwrap().last_seen_at,
            Some(1_000 + SEEN_REFRESH_SECS)
        );

        // New addresses are saved at once.
        let hints = ContactHints {
            relay: Some("https://relay.example/".to_owned()),
            addrs: vec!["192.168.1.5:4000".parse().unwrap()],
        };
        let t = 1_000 + SEEN_REFRESH_SECS + 1;
        assert!(store.record_contact(&id(1), t, &hints).unwrap());
        let device = store.get(&id(1)).unwrap();
        assert_eq!(device.relay.as_deref(), Some("https://relay.example/"));
        assert_eq!(device.addrs, hints.addrs);
        assert_eq!(reload(&store).get(&id(1)).unwrap(), device);

        // The same again is not.
        assert!(!store.record_contact(&id(1), t + 1, &hints).unwrap());

        // Hints that say nothing do not erase what is known.
        assert!(!store.record_contact(&id(1), t + 2, &none).unwrap());
        assert_eq!(store.get(&id(1)).unwrap().addrs, hints.addrs);
    }

    #[test]
    fn contact_with_an_unknown_device_is_ignored() {
        let (_dir, store) = store();
        let hints = ContactHints {
            relay: Some("https://relay.example/".to_owned()),
            addrs: vec!["10.0.0.1:1".parse().unwrap()],
        };
        assert!(!store.record_contact(&id(7), 5, &hints).unwrap());
        assert!(store.is_empty());
        assert!(!store.path().exists());
    }

    #[test]
    fn contact_hints_are_bounded() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1).unwrap();
        let hints = ContactHints {
            relay: Some("https://r.example/".to_owned()),
            addrs: (1..=50u16)
                .map(|port| SocketAddr::from(([10, 0, 0, 1], port)))
                .collect(),
        };
        store.record_contact(&id(1), 2, &hints).unwrap();
        assert_eq!(store.get(&id(1)).unwrap().addrs.len(), MAX_ADDR_HINTS);

        let junk = ContactHints {
            relay: Some("not ascii \u{1f600} or short".to_owned()),
            addrs: Vec::new(),
        };
        store.record_contact(&id(1), 3, &junk).unwrap();
        assert_eq!(store.get(&id(1)).unwrap().relay, None);
    }

    #[test]
    fn the_list_has_a_size_limit() {
        let (_dir, store) = store();
        for n in 0..MAX_DEVICES {
            let mut bytes = [0u8; DeviceId::LEN];
            bytes[..8].copy_from_slice(&(n as u64).to_be_bytes());
            store
                .pair(DeviceId::from_bytes(bytes), &info("device"), 1)
                .unwrap();
        }
        assert_eq!(store.len(), MAX_DEVICES);

        let err = store.pair(id(0xff), &info("one too many"), 1).unwrap_err();
        assert!(
            matches!(err, Error::TooManyDevices { limit: MAX_DEVICES }),
            "{err}"
        );
        assert_eq!(store.len(), MAX_DEVICES);
        assert!(!store.contains(&id(0xff)));

        // Devices already on a full list can still be refreshed.
        let existing = store.list()[0].id;
        store.pair(existing, &info("still fine"), 2).unwrap();
        assert_eq!(reload(&store).len(), MAX_DEVICES);
    }

    #[test]
    fn a_failed_save_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // The directory does not exist, so saving cannot work.
        let store = TrustStore::load(dir.path().join("missing").join("devices.json")).unwrap();
        let mut events = store.subscribe();
        assert!(store.pair(id(1), &info("Laptop"), 1).is_err());
        assert!(store.is_empty());
        assert!(events.try_recv().is_err());

        // Make saving work, pair, then break saving again and try every kind of change.
        fs::create_dir(dir.path().join("missing")).unwrap();
        store.pair(id(1), &info("Laptop"), 1).unwrap();
        let before = store.list();
        fs::remove_dir_all(dir.path().join("missing")).unwrap();
        assert!(store.remove(&id(1)).is_err());
        assert!(
            store
                .update(&id(1), |d| d.name = "Other".to_owned())
                .is_err()
        );
        assert!(store.pair(id(2), &info("Phone"), 2).is_err());
        let hints = ContactHints {
            relay: None,
            addrs: vec!["10.0.0.1:1".parse().unwrap()],
        };
        assert!(store.record_contact(&id(1), 99_999, &hints).is_err());
        assert_eq!(
            store.list(),
            before,
            "memory must match what was last saved"
        );
    }

    fn write_file(dir: &TempDir, text: &str) -> PathBuf {
        let path = dir.path().join("devices.json");
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_damaged_file_is_an_error_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        for text in [
            "{not json",
            "",
            "[]",
            "{\"version\":1}",
            "{\"version\":1,\"devices\":3}",
        ] {
            let path = write_file(&dir, text);
            let err = TrustStore::load(&path).unwrap_err();
            assert!(matches!(err, Error::Invalid { .. }), "{text:?}: {err}");
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                text,
                "file must not be touched"
            );
        }
    }

    #[test]
    fn a_newer_format_is_refused_rather_than_misread() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, r#"{"version":2,"devices":[]}"#);
        let err = TrustStore::load(&path).unwrap_err();
        assert!(
            matches!(
                err,
                Error::UnsupportedVersion {
                    found: 2,
                    supported: 1,
                    ..
                }
            ),
            "{err}"
        );
    }

    fn device_json(id_hex: &str, name: &str) -> String {
        format!(
            r#"{{"id":"{id_hex}","name":{name:?},"platform":"linux","added_at":1,"last_seen_at":null}}"#
        )
    }

    #[test]
    fn bad_ids_and_duplicates_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let good = "01".repeat(32);
        let text =
            |devices: &[String]| format!(r#"{{"version":1,"devices":[{}]}}"#, devices.join(","));

        let path = write_file(&dir, &text(&[device_json("xyz", "a")]));
        assert!(TrustStore::load(&path).is_err(), "not hex");
        let path = write_file(&dir, &text(&[device_json(&"01".repeat(31), "a")]));
        assert!(TrustStore::load(&path).is_err(), "too short");
        let path = write_file(
            &dir,
            &text(&[device_json(&good, "a"), device_json(&good, "b")]),
        );
        let err = TrustStore::load(&path).unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");

        let path = write_file(&dir, &text(&[device_json(&good, "a")]));
        assert_eq!(TrustStore::load(&path).unwrap().len(), 1);
    }

    #[test]
    fn a_file_with_too_many_devices_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let devices: Vec<_> = (0..=MAX_DEVICES)
            .map(|n| device_json(&format!("{n:064x}"), "d"))
            .collect();
        let path = write_file(
            &dir,
            &format!(r#"{{"version":1,"devices":[{}]}}"#, devices.join(",")),
        );
        assert!(TrustStore::load(&path).is_err());
    }

    #[test]
    fn what_is_read_back_is_cleaned_like_what_is_received() {
        let dir = tempfile::tempdir().unwrap();
        let addrs: Vec<String> = (1..=20).map(|p| format!("10.0.0.1:{p}")).collect();
        let document = serde_json::json!({
            "version": 1,
            "devices": [{
                "id": "02".repeat(32),
                "name": "Ann\u{202e}\u{7}",
                "platform": "somethingnew",
                "added_at": 1,
                "relay": "has spaces",
                "addrs": addrs,
            }],
        });
        let path = write_file(&dir, &document.to_string());
        let store = TrustStore::load(&path).unwrap();
        let device = store.get(&id(2)).unwrap();
        assert!(!device.name.contains('\u{202e}'));
        assert!(!device.name.chars().any(char::is_control));
        assert_eq!(device.platform, Platform::Other);
        assert_eq!(device.relay, None);
        assert_eq!(device.addrs.len(), MAX_ADDR_HINTS);
        assert_eq!(device.policy, DevicePolicy::default());
        assert_eq!(device.last_seen_at, None);
    }

    #[test]
    fn debug_output_does_not_list_devices() {
        let (_dir, store) = store();
        store.pair(id(0x5a), &info("Laptop"), 1).unwrap();
        let text = format!("{store:?}");
        assert!(text.contains("devices: 1"), "{text}");
        assert!(!text.contains("Laptop"), "{text}");
    }
}
