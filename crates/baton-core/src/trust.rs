// SPDX-License-Identifier: Apache-2.0

//! The list of devices this one trusts.
//!
//! A device is on the list because a person approved pairing with it. The
//! transport consults the list after every handshake and refuses connections
//! from any key that is not on it, so removing a device here is what revokes
//! it. The list is kept in memory and saved to `devices.json` on every change.
//!
//! # Reading and writing
//!
//! Reading never waits for the disk. Every change is written to disk (and
//! flushed, which on some systems takes tens of milliseconds) *before* it takes
//! effect in memory, and the changes of all handles to one list are carried out
//! one at a time, but the readers only ever see a finished list: they take a
//! snapshot and let go of the lock at once. [`contains`](TrustStore::contains)
//! can therefore be called from async code on every request. The methods that
//! change the list do disk I/O and block the calling thread for as long as that
//! takes, so call them from `spawn_blocking` when running on an async runtime.
//!
//! # Revocation
//!
//! Removing a device has to stop it, including over a connection it already
//! has. Two things make that reliable:
//!
//! - By the time [`remove`](TrustStore::remove) returns, and by the time any
//!   [`TrustEvent::Removed`] can be received, [`contains`](TrustStore::contains)
//!   is already `false` for every handle.
//! - Code that serves a peer must check `contains` again for each request, and
//!   treat events only as a way to close connections sooner. An event alone is
//!   not enough: a removal that lands between a connection's check and its being
//!   registered finds nothing to close, and a receiver that lags never learns of
//!   it.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock},
};

use baton_proto::{
    DeviceId, DeviceInfo, Platform, device::sanitize_display_name, ticket::is_valid_relay_hint,
};
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

/// Newest file format this build reads and writes.
const FORMAT_VERSION: u32 = 1;

/// How stale `last_seen_at` may get before seeing a device again rewrites the file.
const SEEN_REFRESH_SECS: u64 = 300;

/// How soon after its last save a device's hints are saved again when they merely changed.
///
/// A peer that reports different addresses on every connection (a phone moving between
/// networks, or a paired device behaving badly) would otherwise turn every connection into
/// a write and an fsync. Hints are best effort, and the next contact after this long carries
/// the current ones.
const HINT_REFRESH_SECS: u64 = 60;

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
        self.addrs = canonical_addrs(std::mem::take(&mut self.addrs));
        if self
            .relay
            .as_deref()
            .is_some_and(|r| !is_valid_relay_hint(r))
        {
            self.relay = None;
        }
    }
}

/// The addresses worth remembering from `addrs`: usable ones, each once, in a fixed order.
///
/// A given set of addresses always comes out the same, whatever order it arrived in, so
/// comparing two results tells whether anything really changed. When there are more than
/// [`MAX_ADDR_HINTS`], which ones survive is arbitrary but stable.
fn canonical_addrs(mut addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    addrs.retain(|a| a.port() != 0 && !a.ip().is_unspecified() && !a.ip().is_multicast());
    addrs.sort_unstable();
    addrs.dedup();
    addrs.truncate(MAX_ADDR_HINTS);
    addrs
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

/// How a new version of the file reaches the disk. Tests replace it to make saving slow or fail.
type Saver = Box<dyn Fn(&Path, &[u8]) -> Result<()> + Send + Sync>;

struct Inner {
    path: PathBuf,
    /// The list as last saved. A reader copies the `Arc` and lets go of the lock at once, and a
    /// writer takes the write lock only to put a finished list in place, so nobody waits for the disk.
    current: RwLock<Arc<Devices>>,
    /// Held by a writer from the moment it starts working on a change until its event is sent,
    /// disk write included. It keeps changes, saves and events in one order.
    writer: Mutex<()>,
    events: broadcast::Sender<TrustEvent>,
    save: Saver,
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
        Self::load_with(path, Box::new(write_atomic))
    }

    fn load_with(path: impl Into<PathBuf>, save: Saver) -> Result<Self> {
        let path = path.into();
        let devices = match read_text(&path)? {
            None => Devices::new(),
            Some(text) => parse(&path, &text)?,
        };
        let (events, _) = broadcast::channel(64);
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                current: RwLock::new(Arc::new(devices)),
                writer: Mutex::new(()),
                events,
                save,
            }),
        })
    }

    /// The file this list is saved to.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// The list as it is right now. Cheap, and never waits for a save.
    fn snapshot(&self) -> Arc<Devices> {
        // Lists are swapped in whole, so a panic elsewhere cannot leave one half-updated and
        // a poisoned lock is safe to reuse.
        Arc::clone(
            &self
                .inner
                .current
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Takes the right to change the list. Held until the change has been announced.
    fn lock_writers(&self) -> MutexGuard<'_, ()> {
        self.inner
            .writer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether `id` is paired. This is fast and does not touch the disk, and it does not wait
    /// for a save that is in progress, so it can be called for every request.
    pub fn contains(&self, id: &DeviceId) -> bool {
        self.snapshot().contains_key(id)
    }

    /// The paired device with this id.
    pub fn get(&self, id: &DeviceId) -> Option<PairedDevice> {
        self.snapshot().get(id).cloned()
    }

    /// All paired devices, sorted by name.
    pub fn list(&self) -> Vec<PairedDevice> {
        let mut devices: Vec<_> = self.snapshot().values().cloned().collect();
        devices.sort_by_cached_key(|d| (d.name.to_lowercase(), d.id));
        devices
    }

    /// How many devices are paired.
    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    /// Whether no device is paired.
    pub fn is_empty(&self) -> bool {
        self.snapshot().is_empty()
    }

    /// Receives a [`TrustEvent`] for every change made through any handle to this list.
    ///
    /// Events are sent in the order the changes were made, and only after the change is
    /// visible to [`contains`](Self::contains) and the other readers. A receiver that falls
    /// behind gets [`RecvError::Lagged`](broadcast::error::RecvError::Lagged) and should
    /// then re-read the list instead of relying on the events it missed.
    ///
    /// Events are not enough to enforce a removal. See the module documentation: check
    /// [`contains`](Self::contains) for each request.
    pub fn subscribe(&self) -> broadcast::Receiver<TrustEvent> {
        self.inner.events.subscribe()
    }

    /// Adds a device, or refreshes it if it is already paired.
    ///
    /// Pairing again keeps the device's policy and original pairing time.
    pub fn pair(&self, id: DeviceId, info: &DeviceInfo, now: u64) -> Result<PairOutcome> {
        let writer = self.lock_writers();
        let mut next = Devices::clone(&self.snapshot());
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
        self.commit(&writer, next)?;
        self.emit(match outcome {
            PairOutcome::Added => TrustEvent::Added(id),
            PairOutcome::Refreshed => TrustEvent::Updated(id),
        });
        Ok(outcome)
    }

    /// Changes a paired device. Returns `false` if there is no such device.
    ///
    /// The id cannot be changed. Names and hints are cleaned up again afterwards.
    /// The closure runs while other changes to the list wait, so it must not change the list
    /// itself; reading it is fine.
    pub fn update(&self, id: &DeviceId, change: impl FnOnce(&mut PairedDevice)) -> Result<bool> {
        let writer = self.lock_writers();
        let mut next = Devices::clone(&self.snapshot());
        let Some(device) = next.get_mut(id) else {
            return Ok(false);
        };
        change(device);
        device.id = *id;
        device.normalize();
        self.commit(&writer, next)?;
        self.emit(TrustEvent::Updated(*id));
        Ok(true)
    }

    /// Unpairs a device. Returns what was removed, if anything.
    ///
    /// When this returns, [`contains`](Self::contains) is `false` for the device.
    pub fn remove(&self, id: &DeviceId) -> Result<Option<PairedDevice>> {
        let writer = self.lock_writers();
        let mut next = Devices::clone(&self.snapshot());
        let Some(removed) = next.remove(id) else {
            return Ok(None);
        };
        self.commit(&writer, next)?;
        self.emit(TrustEvent::Removed(*id));
        Ok(Some(removed))
    }

    /// Notes that a device was just reached, and where.
    ///
    /// This runs on every connection, so it writes to disk rarely: when the last-seen time
    /// is stale, when a first address or relay is learned, or when the hints changed and
    /// the device was last saved a minute or more ago. The same addresses in another order,
    /// repeated addresses, and addresses nobody can connect to (unspecified, multicast, port
    /// 0) are not changes. When nothing needs saving it neither waits for other changes nor
    /// makes them wait. It sends no event. Returns whether anything was saved.
    pub fn record_contact(&self, id: &DeviceId, now: u64, hints: &ContactHints) -> Result<bool> {
        // Nearly every call finds nothing to save. Decide on a snapshot, so that those calls
        // never queue behind a change that is being written.
        if contact_update(&self.snapshot(), id, now, hints).is_none() {
            return Ok(false);
        }
        let writer = self.lock_writers();
        // Another change may have got in first; decide again on the list as it is now.
        let current = self.snapshot();
        let Some(updated) = contact_update(&current, id, now, hints) else {
            return Ok(false);
        };
        let mut next = Devices::clone(&current);
        next.insert(*id, updated);
        self.commit(&writer, next)?;
        Ok(true)
    }

    /// Saves `next`, and only if that worked makes it the current list.
    ///
    /// The caller holds the writer lock, which is what the guard argument stands for.
    fn commit(&self, _writer: &MutexGuard<'_, ()>, next: Devices) -> Result<()> {
        let stored = StoredTrust {
            version: FORMAT_VERSION,
            devices: next.values().cloned().collect(),
        };
        let json =
            serde_json::to_vec_pretty(&stored).map_err(|e| Error::invalid(&self.inner.path, &e))?;
        (self.inner.save)(&self.inner.path, &json)?;
        *self
            .inner
            .current
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(next);
        Ok(())
    }

    /// Sending never blocks, so this is called while the writer lock is still held: that is
    /// what keeps events in the order of the changes.
    fn emit(&self, event: TrustEvent) {
        // Nobody listening is fine.
        let _ = self.inner.events.send(event);
    }
}

/// What a contact with `id` would change, if it is worth a disk write.
fn contact_update(
    devices: &Devices,
    id: &DeviceId,
    now: u64,
    hints: &ContactHints,
) -> Option<PairedDevice> {
    let current = devices.get(id)?;
    let mut updated = current.clone();
    // A clock that was set back must not make a device look as if it was last seen earlier.
    updated.last_seen_at = Some(now.max(current.last_seen_at.unwrap_or(0)));
    // Hints that say nothing, or nothing usable, do not erase what is known.
    if let Some(relay) = hints.relay.as_deref().filter(|r| is_valid_relay_hint(r)) {
        updated.relay = Some(relay.to_owned());
    }
    let addrs = canonical_addrs(hints.addrs.clone());
    if !addrs.is_empty() {
        updated.addrs = addrs;
    }
    updated.normalize();

    let since_saved = current
        .last_seen_at
        .map_or(u64::MAX, |seen| now.saturating_sub(seen));
    let stale = since_saved >= SEEN_REFRESH_SECS;
    // The first hint of a kind is saved at once. After that, changes wait their turn.
    let learned_something_new = (current.relay.is_none() && updated.relay.is_some())
        || (current.addrs.is_empty() && !updated.addrs.is_empty());
    let hints_changed = updated.relay != current.relay || updated.addrs != current.addrs;
    let hints_due = hints_changed && (learned_something_new || since_saved >= HINT_REFRESH_SECS);
    (stale || hints_due).then_some(updated)
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
    use std::{
        fs,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
            mpsc,
        },
        thread,
        time::Duration,
    };

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

    fn store_with(save: Saver) -> (TempDir, TrustStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::load_with(dir.path().join("devices.json"), save).unwrap();
        (dir, store)
    }

    /// Holds a save half-way, so a test can look at the list while the disk is busy.
    struct Gate {
        armed: Arc<AtomicBool>,
        entered: mpsc::Receiver<()>,
        release: mpsc::Sender<()>,
    }

    impl Gate {
        /// The next save will stop and wait. Saves made while it is not armed go straight through.
        fn arm(&self) {
            self.armed.store(true, SeqCst);
        }

        /// Waits until a save has started and is being held.
        fn wait_for_save(&self) {
            self.entered
                .recv_timeout(Duration::from_secs(10))
                .expect("a save should have started");
        }

        /// Lets the held save finish, and disarms the gate.
        fn let_save_finish(&self) {
            self.armed.store(false, SeqCst);
            self.release.send(()).unwrap();
        }
    }

    fn gated_store() -> (TempDir, TrustStore, Gate) {
        let armed = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let gate = Gate {
            armed: Arc::clone(&armed),
            entered,
            release,
        };
        let (dir, store) = store_with(Box::new(move |path, bytes| {
            if armed.load(SeqCst) {
                let _ = entered_tx.send(());
                // A test that gave up drops the sender, which also ends the wait.
                let _ = release_rx.lock().unwrap().recv();
            }
            write_atomic(path, bytes)
        }));
        (dir, store, gate)
    }

    /// Runs `work` on another thread and fails the test, rather than hanging it, if it is held up.
    fn promptly<T: Send + 'static>(what: &str, work: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(work());
        });
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(value) => value,
            Err(mpsc::RecvTimeoutError::Timeout) => panic!("{what} was held up"),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("{what} failed"),
        }
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

        // A relay that is not acceptable is ignored; it does not erase the good one.
        let junk = ContactHints {
            relay: Some("not ascii \u{1f600} or short".to_owned()),
            addrs: Vec::new(),
        };
        let later = 2 + HINT_REFRESH_SECS + 1;
        assert!(!store.record_contact(&id(1), later, &junk).unwrap());
        assert_eq!(
            store.get(&id(1)).unwrap().relay.as_deref(),
            Some("https://r.example/")
        );
    }

    #[test]
    fn the_same_addresses_in_another_order_are_not_a_change() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1_000).unwrap();
        let addr = |text: &str| text.parse::<SocketAddr>().unwrap();
        let first = ContactHints {
            relay: None,
            addrs: vec![
                addr("192.168.1.5:4000"),
                addr("10.0.0.2:4000"),
                addr("[2001:db8::1]:4000"),
            ],
        };
        assert!(store.record_contact(&id(1), 1_001, &first).unwrap());
        let saved = fs::read(store.path()).unwrap();

        // Long enough that a real change would be saved, so only the order and repeats are on trial.
        let mut later = 1_001 + HINT_REFRESH_SECS;
        for shuffled in [
            vec![
                addr("[2001:db8::1]:4000"),
                addr("192.168.1.5:4000"),
                addr("10.0.0.2:4000"),
            ],
            vec![
                addr("10.0.0.2:4000"),
                addr("10.0.0.2:4000"),
                addr("[2001:db8::1]:4000"),
                addr("192.168.1.5:4000"),
            ],
        ] {
            let hints = ContactHints {
                relay: None,
                addrs: shuffled,
            };
            later += 1;
            assert!(!store.record_contact(&id(1), later, &hints).unwrap());
            assert_eq!(
                fs::read(store.path()).unwrap(),
                saved,
                "the file must not be rewritten"
            );
        }

        // They are kept sorted and without repeats, so a file never differs only by order.
        let device = store.get(&id(1)).unwrap();
        assert_eq!(
            device.addrs,
            [
                addr("10.0.0.2:4000"),
                addr("192.168.1.5:4000"),
                addr("[2001:db8::1]:4000")
            ]
        );
    }

    #[test]
    fn addresses_nobody_can_connect_to_are_not_remembered() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 1_000).unwrap();
        let junk: Vec<SocketAddr> = [
            "0.0.0.0:4000",
            "[::]:4000",
            "224.0.0.251:5353",
            "[ff02::fb]:5353",
            "192.168.1.5:0",
        ]
        .into_iter()
        .map(|a| a.parse().unwrap())
        .collect();

        // Nothing usable: nothing changes, and nothing is saved.
        let only_junk = ContactHints {
            relay: None,
            addrs: junk.clone(),
        };
        assert!(!store.record_contact(&id(1), 1_001, &only_junk).unwrap());
        assert!(store.get(&id(1)).unwrap().addrs.is_empty());

        // Mixed with a usable one: only the usable one is kept.
        let usable: SocketAddr = "192.168.1.5:4000".parse().unwrap();
        let mixed = ContactHints {
            relay: None,
            addrs: [junk.clone(), vec![usable]].concat(),
        };
        assert!(store.record_contact(&id(1), 1_002, &mixed).unwrap());
        assert_eq!(store.get(&id(1)).unwrap().addrs, [usable]);

        // And a later report with only unusable ones does not erase it.
        assert!(
            !store
                .record_contact(&id(1), 1_002 + HINT_REFRESH_SECS, &only_junk)
                .unwrap()
        );
        assert_eq!(store.get(&id(1)).unwrap().addrs, [usable]);
    }

    #[test]
    fn a_peer_that_changes_its_addresses_every_time_is_not_saved_every_time() {
        let saves = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&saves);
        let (_dir, store) = store_with(Box::new(move |path, bytes| {
            counter.fetch_add(1, SeqCst);
            write_atomic(path, bytes)
        }));
        store.pair(id(1), &info("Laptop"), 1_000).unwrap();
        assert_eq!(saves.load(SeqCst), 1);

        let at = |port: u16| ContactHints {
            relay: None,
            addrs: vec![SocketAddr::from(([10, 0, 0, 1], port))],
        };
        // The first address ever heard of is worth saving at once.
        assert!(store.record_contact(&id(1), 1_001, &at(1)).unwrap());
        // A different one on each of the next connections is not.
        for (offset, port) in (2u16..20).enumerate() {
            let t = 1_002 + u64::try_from(offset).unwrap();
            assert!(
                !store.record_contact(&id(1), t, &at(port)).unwrap(),
                "port {port}"
            );
        }
        assert_eq!(saves.load(SeqCst), 2);
        assert_eq!(store.get(&id(1)).unwrap().addrs, at(1).addrs);

        // Once the minute is up the latest report is saved.
        let t = 1_001 + HINT_REFRESH_SECS;
        assert!(store.record_contact(&id(1), t, &at(99)).unwrap());
        assert_eq!(saves.load(SeqCst), 3);
        assert_eq!(store.get(&id(1)).unwrap().addrs, at(99).addrs);
    }

    #[test]
    fn a_clock_set_back_does_not_make_a_device_look_less_recently_seen() {
        let (_dir, store) = store();
        store.pair(id(1), &info("Laptop"), 5_000).unwrap();
        let hints = ContactHints {
            relay: Some("https://r.example/".to_owned()),
            addrs: Vec::new(),
        };
        assert!(store.record_contact(&id(1), 100, &hints).unwrap());
        assert_eq!(store.get(&id(1)).unwrap().last_seen_at, Some(5_000));
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
        let mut addrs: Vec<String> = (1..=20).rev().map(|p| format!("10.0.0.1:{p}")).collect();
        addrs.extend(["10.0.0.1:3".to_owned(), "0.0.0.0:9".to_owned()]);
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
        let expected: Vec<SocketAddr> = (1..=8)
            .map(|p| format!("10.0.0.1:{p}").parse().unwrap())
            .collect();
        assert_eq!(
            device.addrs, expected,
            "sorted, without repeats, and capped"
        );
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

    #[test]
    fn readers_do_not_wait_for_a_save() {
        let (_dir, store, gate) = gated_store();
        store.pair(id(2), &info("Phone"), 1_000).unwrap();
        let mut events = store.subscribe();

        gate.arm();
        let writer = thread::spawn({
            let store = store.clone();
            move || store.pair(id(1), &info("Laptop"), 2_000)
        });
        gate.wait_for_save();

        // The disk is busy with a change. Reading, and noting a contact that needs no write, go on.
        let reader = store.clone();
        promptly("a reader", move || {
            assert!(reader.contains(&id(2)));
            assert!(
                !reader.contains(&id(1)),
                "a change must not show before it is saved"
            );
            assert_eq!(reader.get(&id(2)).unwrap().name, "Phone");
            assert_eq!(reader.len(), 1);
            assert!(!reader.is_empty());
            assert_eq!(reader.list().len(), 1);
            drop(reader.subscribe());
            let saved = reader
                .record_contact(&id(2), 1_010, &ContactHints::default())
                .unwrap();
            assert!(!saved);
        });
        assert!(
            events.try_recv().is_err(),
            "nothing is announced before it is in effect"
        );

        gate.let_save_finish();
        assert_eq!(writer.join().unwrap().unwrap(), PairOutcome::Added);
        assert!(store.contains(&id(1)));
        assert_eq!(events.try_recv().unwrap(), TrustEvent::Added(id(1)));
    }

    #[test]
    fn a_removal_is_in_effect_before_anyone_hears_of_it() {
        let (_dir, store) = store();
        for n in 1..=50 {
            let target = id(n);
            store.pair(target, &info("Laptop"), 1).unwrap();
            let mut events = store.subscribe();
            let watcher = thread::spawn({
                let store = store.clone();
                move || loop {
                    match events.blocking_recv() {
                        Ok(TrustEvent::Removed(gone)) if gone == target => {
                            return store.contains(&gone);
                        }
                        Ok(_) => {}
                        Err(e) => panic!("{e}"),
                    }
                }
            });
            store.remove(&target).unwrap();
            assert!(
                !watcher.join().unwrap(),
                "heard of a removal while the device was still trusted"
            );
        }
    }

    #[test]
    fn a_contact_that_raced_a_removal_does_not_bring_the_device_back() {
        let (_dir, store, gate) = gated_store();
        store.pair(id(1), &info("Laptop"), 1_000).unwrap();

        gate.arm();
        let remover = thread::spawn({
            let store = store.clone();
            move || store.remove(&id(1))
        });
        gate.wait_for_save();
        assert!(
            store.contains(&id(1)),
            "the removal is not in effect until it is saved"
        );

        // Late enough to be worth saving, so it queues behind the removal.
        let contact = thread::spawn({
            let store = store.clone();
            move || store.record_contact(&id(1), 99_999, &ContactHints::default())
        });
        // Waiting makes the interesting order likely. The assertions below hold either way.
        thread::sleep(Duration::from_millis(200));

        gate.let_save_finish();
        remover.join().unwrap().unwrap();
        let saved = contact.join().unwrap().unwrap();
        assert!(!saved, "there is nothing left to save for a removed device");
        assert!(!store.contains(&id(1)));
        assert!(!reload(&store).contains(&id(1)));
    }

    #[test]
    fn changes_from_many_threads_are_all_kept_and_saved_one_at_a_time() {
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let (during, peak) = (Arc::clone(&running), Arc::clone(&most));
        let (_dir, store) = store_with(Box::new(move |path, bytes| {
            peak.fetch_max(during.fetch_add(1, SeqCst) + 1, SeqCst);
            thread::sleep(Duration::from_millis(2));
            let result = write_atomic(path, bytes);
            during.fetch_sub(1, SeqCst);
            result
        }));
        let mut events = store.subscribe();

        let threads: Vec<_> = (1..=30)
            .map(|n| {
                let store = store.clone();
                thread::spawn(move || store.pair(id(n), &info("Laptop"), 1).unwrap())
            })
            .collect();
        for handle in threads {
            handle.join().unwrap();
        }

        assert_eq!(store.len(), 30);
        assert_eq!(reload(&store).len(), 30, "no change was overwritten");
        assert_eq!(most.load(SeqCst), 1, "two saves overlapped");
        assert_eq!(running.load(SeqCst), 0);
        let mut added = 0;
        while let Ok(event) = events.try_recv() {
            assert!(matches!(event, TrustEvent::Added(_)), "{event:?}");
            added += 1;
        }
        assert_eq!(added, 30);
    }

    #[test]
    fn a_panic_while_saving_does_not_wedge_the_list() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let (_dir, store) = store_with(Box::new(move |path, bytes| {
            assert!(counter.fetch_add(1, SeqCst) > 0, "the disk caught fire");
            write_atomic(path, bytes)
        }));

        let outcome = catch_unwind(AssertUnwindSafe(|| store.pair(id(1), &info("Laptop"), 1)));
        assert!(outcome.is_err());
        assert!(store.is_empty(), "a change that was not saved is not made");

        store.pair(id(1), &info("Laptop"), 2).unwrap();
        assert!(store.contains(&id(1)));
        assert!(store.remove(&id(1)).unwrap().is_some());
        assert!(!reload(&store).contains(&id(1)));
    }
}
