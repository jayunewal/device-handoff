// SPDX-License-Identifier: Apache-2.0

//! The outbox: links and text that the person sent to a device that could not be reached, kept
//! until it can be.
//!
//! A person sends a link to their phone while Baton is closed on it. The send fails there and
//! then, because nobody is there to take it, and the person has to remember to send it again. The
//! outbox keeps the item on the sending device instead, and the program that owns it tries again
//! when the other device can be reached. It does not wait for ever: an item that has not been
//! delivered when [`LIFETIME`] is over is dropped, and the person is told.
//!
//! This module is the list and its file. It does not send anything: the program that owns the
//! list asks it what waits, tries each item, and strikes out the ones that went.
//!
//! What waits is only what the person chose to send, to the device that they chose, and it goes to
//! no other device. Unpairing a device drops everything that waits for it
//! ([`remove_for`](Outbox::remove_for)). Files do not wait: an offer of files needs the person at
//! the other end to say yes, and the files at the sender to be where they were.
//!
//! # The file
//!
//! `outbox.json` in the [state directory](crate::DataDir), readable by its owner only, because
//! text can be a password. It holds what waits, in the order it was queued, and the number that
//! the next item gets. It is read leniently: a file that cannot be read, or an entry in it that
//! makes no sense, is left out, because an outbox that is lost loses a few links and one that
//! cannot start loses the app. A change that cannot be written is logged and kept in memory, so
//! that what waits is still sent while the program runs.
//!
//! The methods that change the list write the file, so they block the calling thread for as long
//! as the disk takes, as those of the [list of paired devices](crate::TrustStore) do. The list is
//! small: [`MOST_WAITING`] items.
//!
//! # Limits
//!
//! At most [`MOST_WAITING`] items wait, at most [`MOST_FOR_A_DEVICE`] of them for one device, and
//! the file never grows past 3 MiB. An item that does not fit is refused ([`Full`]) and the person
//! is told. What waits is never dropped to make room for another: it was promised.
//!
//! # For how long
//!
//! An item is kept for [`LIFETIME`] from the time it was queued, counted on the wall clock,
//! because that is the only clock that goes on while the program is not running. A clock that is
//! set forward cuts the time short, and one that is set back does not lengthen it: an item that is
//! said to have been queued after now is taken as queued now.

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use baton_proto::{
    DeviceId,
    device::sanitize_display_name,
    limits::{MAX_CLIPBOARD_TEXT_BYTES, MAX_TITLE_BYTES, MAX_URL_BYTES},
    message::{ClipboardText, Item, Link},
};
use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, Result},
    fsutil::{Access, MAX_STATE_FILE_BYTES, read_text, write_atomic},
    util::lock,
};

/// Newest file format this build reads and writes.
const FORMAT_VERSION: u32 = 1;

/// How long an item waits for its device before it is dropped.
pub const LIFETIME: Duration = Duration::from_hours(24);

/// The most items that wait, for all devices together.
pub const MOST_WAITING: usize = 50;

/// The most items that wait for one device.
pub const MOST_FOR_A_DEVICE: usize = 20;

/// The largest file this build writes. The limit for reading a state file is higher, so that what
/// is written here can be read back.
const MAX_FILE_BYTES: usize = 3 * 1024 * 1024;

const _: () = assert!(MAX_FILE_BYTES as u64 <= MAX_STATE_FILE_BYTES);

/// An item that waits to be sent to a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// Counts up from 1 and is never reused while the file lasts, so that a screen that shows
    /// "cancel this one" cancels the item that it showed and no other.
    pub id: u64,
    /// The device it is for, and no other.
    pub to: DeviceId,
    /// The name that the device had when the item was queued, to tell the person about it if the
    /// device has been unpaired since.
    pub device_name: String,
    /// When the person sent it, in seconds since the Unix epoch.
    pub queued_at: u64,
    /// What to send. Its `Debug` hides what it says.
    pub item: Item,
}

/// Why an item was not put in the outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Full {
    /// [`MOST_WAITING`] items wait already.
    Everywhere,
    /// [`MOST_FOR_A_DEVICE`] items wait already for that device.
    ForTheDevice,
    /// The file would pass its limit with it.
    TooMuch,
}

struct State {
    /// The number that the next item gets.
    next: u64,
    /// What waits, the oldest first.
    items: Vec<Waiting>,
}

struct Inner {
    /// Where the list is kept, if it is kept anywhere.
    path: Option<PathBuf>,
    state: Mutex<State>,
}

/// What waits to be sent to devices that could not be reached. Clones share one list. See the
/// module documentation.
#[derive(Clone)]
pub struct Outbox(Arc<Inner>);

impl fmt::Debug for Outbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outbox").finish_non_exhaustive()
    }
}

impl Outbox {
    /// A list that lives in memory and goes with the program.
    pub fn memory() -> Self {
        Self::new(None, State::empty())
    }

    /// A list that is kept in the file at `path`, and starts with what is in it. The file belongs
    /// in the [state directory](crate::DataDir::outbox_file).
    ///
    /// This never fails. A file that cannot be read, or an entry in it that makes no sense, is
    /// logged and left out, and a change that cannot be written is logged and kept in memory.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let (state, changed) = match read(&path) {
            Ok(read) => read,
            Err(error) => {
                tracing::warn!(%error, "what waits to be sent to other devices could not be read");
                (State::empty(), false)
            }
        };
        let this = Self::new(Some(path), state);
        if changed {
            this.save(&lock(&this.0.state));
        }
        this
    }

    fn new(path: Option<PathBuf>, state: State) -> Self {
        Self(Arc::new(Inner {
            path,
            state: Mutex::new(state),
        }))
    }

    /// Puts `item` in the outbox, to be sent to `to` when it can be reached, and returns the
    /// number that it waits under. `device_name` is what the device is called now, and `now` is
    /// the time in seconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// [`Full`] says which limit it would pass. Nothing is changed then.
    pub fn add(
        &self,
        to: DeviceId,
        device_name: &str,
        item: Item,
        now: u64,
    ) -> std::result::Result<u64, Full> {
        let mut state = lock(&self.0.state);
        if state.items.len() >= MOST_WAITING {
            return Err(Full::Everywhere);
        }
        if state
            .items
            .iter()
            .filter(|waiting| waiting.to == to)
            .count()
            >= MOST_FOR_A_DEVICE
        {
            return Err(Full::ForTheDevice);
        }
        let id = state.next;
        state.items.push(Waiting {
            id,
            to,
            device_name: sanitize_display_name(device_name),
            queued_at: now,
            item,
        });
        // The file as it would be, to know that it fits before the item is kept.
        let bytes = encode(&state);
        if bytes
            .as_ref()
            .is_ok_and(|bytes| bytes.len() > MAX_FILE_BYTES)
        {
            state.items.pop();
            return Err(Full::TooMuch);
        }
        state.next = id + 1;
        match bytes {
            Ok(bytes) => self.write(&bytes),
            // Not a thing that an item can cause: it stays in memory and is sent from there.
            Err(error) => {
                tracing::warn!(%error, "what waits to be sent to other devices could not be written down");
            }
        }
        Ok(id)
    }

    /// Everything that waits, the oldest first.
    pub fn list(&self) -> Vec<Waiting> {
        lock(&self.0.state).items.clone()
    }

    /// Whether nothing waits.
    pub fn is_empty(&self) -> bool {
        lock(&self.0.state).items.is_empty()
    }

    /// The devices that something waits for, the one whose item has waited longest first.
    pub fn devices(&self) -> Vec<DeviceId> {
        let mut devices: Vec<DeviceId> = Vec::new();
        for waiting in &lock(&self.0.state).items {
            if !devices.contains(&waiting.to) {
                devices.push(waiting.to);
            }
        }
        devices
    }

    /// The item that has waited longest for `to`, which is the one to send next.
    pub fn next_for(&self, to: &DeviceId) -> Option<Waiting> {
        lock(&self.0.state)
            .items
            .iter()
            .find(|waiting| waiting.to == *to)
            .cloned()
    }

    /// Strikes out the item `id`, which was sent, or which the person gave up on. `None` if it is
    /// not there, because something else struck it out first.
    pub fn remove(&self, id: u64) -> Option<Waiting> {
        let mut state = lock(&self.0.state);
        let at = state.items.iter().position(|waiting| waiting.id == id)?;
        let removed = state.items.remove(at);
        self.save(&state);
        Some(removed)
    }

    /// Strikes out everything that waits for `to`, which is not paired any more, and returns it.
    pub fn remove_for(&self, to: &DeviceId) -> Vec<Waiting> {
        let mut state = lock(&self.0.state);
        let gone = take_out(&mut state.items, |waiting| waiting.to == *to);
        if !gone.is_empty() {
            self.save(&state);
        }
        gone
    }

    /// Strikes out the items that have waited for [`LIFETIME`] by `now`, and returns them, the
    /// oldest first.
    pub fn expire(&self, now: u64) -> Vec<Waiting> {
        let mut state = lock(&self.0.state);
        for waiting in &mut state.items {
            // A clock that was set back: nothing was queued after now.
            waiting.queued_at = waiting.queued_at.min(now);
        }
        let gone = take_out(&mut state.items, |waiting| {
            now - waiting.queued_at >= LIFETIME.as_secs()
        });
        if !gone.is_empty() {
            self.save(&state);
        }
        gone
    }

    /// Writes the list, if it is kept in a file. A list that cannot be written stays in memory.
    fn save(&self, state: &State) {
        if self.0.path.is_none() {
            return;
        }
        match encode(state) {
            Ok(bytes) => self.write(&bytes),
            Err(error) => {
                tracing::warn!(%error, "what waits to be sent to other devices could not be written down");
            }
        }
    }

    fn write(&self, bytes: &[u8]) {
        let Some(path) = &self.0.path else {
            return;
        };
        if let Err(error) = write_atomic(path, bytes) {
            tracing::warn!(%error, "what waits to be sent to other devices could not be written down");
        }
    }
}

impl State {
    fn empty() -> Self {
        Self {
            next: 1,
            items: Vec::new(),
        }
    }
}

/// Takes the items that `doomed` picks out of `items`, and returns them in the order they had.
fn take_out(items: &mut Vec<Waiting>, doomed: impl Fn(&Waiting) -> bool) -> Vec<Waiting> {
    let (gone, kept) = std::mem::take(items).into_iter().partition(doomed);
    *items = kept;
    gone
}

// The file.

/// What an entry of the file says. The id of the device is hexadecimal text, so that the file can
/// be read.
#[derive(Serialize, Deserialize)]
struct StoredWaiting {
    id: u64,
    to: String,
    name: String,
    queued_at: u64,
    item: StoredItem,
}

/// An item as it is written. It is not the type that goes over the wire, so that the file does not
/// change with it.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredItem {
    Link {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Text {
        text: String,
    },
}

#[derive(Serialize)]
struct StoredRef<'a> {
    version: u32,
    next: u64,
    items: &'a [StoredWaiting],
}

/// The file as it is read: each entry is judged on its own.
#[derive(Deserialize)]
struct Stored {
    version: u32,
    #[serde(default)]
    next: u64,
    items: Vec<serde_json::Value>,
}

fn encode(state: &State) -> serde_json::Result<Vec<u8>> {
    let items: Vec<StoredWaiting> = state.items.iter().map(StoredWaiting::from).collect();
    serde_json::to_vec(&StoredRef {
        version: FORMAT_VERSION,
        next: state.next,
        items: &items,
    })
}

impl From<&Waiting> for StoredWaiting {
    fn from(waiting: &Waiting) -> Self {
        Self {
            id: waiting.id,
            to: waiting.to.to_string(),
            name: waiting.device_name.clone(),
            queued_at: waiting.queued_at,
            item: match &waiting.item {
                Item::Link(Link { url, title }) => StoredItem::Link {
                    url: url.clone(),
                    title: title.clone(),
                },
                Item::ClipboardText(ClipboardText { text }) => {
                    StoredItem::Text { text: text.clone() }
                }
            },
        }
    }
}

/// What is in the file at `path`, and whether the file says something that the list does not: an
/// entry that was left out.
fn read(path: &Path) -> Result<(State, bool)> {
    let Some(text) = read_text(path, Access::OwnerOnly)? else {
        return Ok((State::empty(), false));
    };
    let stored: Stored = serde_json::from_str(&text).map_err(|e| Error::invalid(path, &e))?;
    if stored.version != FORMAT_VERSION {
        return Err(Error::invalid(
            path,
            &format!(
                "it is of format {}, and this version reads format {FORMAT_VERSION}",
                stored.version
            ),
        ));
    }
    let mut changed = false;
    let mut items: Vec<Waiting> = Vec::new();
    for entry in stored.items {
        match waiting_of(entry) {
            // A number that is there already would make "cancel this one" ambiguous.
            Some(waiting) if items.iter().all(|kept| kept.id != waiting.id) => items.push(waiting),
            _ => changed = true,
        }
    }
    // More than there is room for, which only a file that was edited can hold: the oldest stay.
    if items.len() > MOST_WAITING {
        items.truncate(MOST_WAITING);
        changed = true;
    }
    let next = items
        .iter()
        .map(|waiting| waiting.id + 1)
        .max()
        .unwrap_or(1)
        .max(stored.next);
    Ok((State { next, items }, changed))
}

/// The item that an entry of the file says, if it says one that can be sent.
fn waiting_of(entry: serde_json::Value) -> Option<Waiting> {
    let stored: StoredWaiting = serde_json::from_value(entry).ok()?;
    let item = match stored.item {
        StoredItem::Link { url, title } => {
            if url.trim().is_empty()
                || url.len() > MAX_URL_BYTES
                || title
                    .as_ref()
                    .is_some_and(|title| title.len() > MAX_TITLE_BYTES)
            {
                return None;
            }
            Item::Link(Link { url, title })
        }
        StoredItem::Text { text } => {
            if text.trim().is_empty() || text.len() > MAX_CLIPBOARD_TEXT_BYTES {
                return None;
            }
            Item::ClipboardText(ClipboardText { text })
        }
    };
    Some(Waiting {
        id: stored.id,
        to: stored.to.parse().ok()?,
        device_name: sanitize_display_name(&stored.name),
        queued_at: stored.queued_at,
        item,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = LIFETIME.as_secs();

    fn peer(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; DeviceId::LEN])
    }

    fn link(url: &str) -> Item {
        Item::Link(Link {
            url: url.to_owned(),
            title: None,
        })
    }

    fn text(text: &str) -> Item {
        Item::ClipboardText(ClipboardText {
            text: text.to_owned(),
        })
    }

    fn file(dir: &TempDir) -> PathBuf {
        dir.path().join("outbox.json")
    }

    fn ids(outbox: &Outbox) -> Vec<u64> {
        outbox.list().iter().map(|waiting| waiting.id).collect()
    }

    /// An entry as a person might write it into the file: some text for the device `to`.
    fn entry(id: u64, to: u8, item: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "to": peer(to).to_string(),
            "name": "Phone",
            "queued_at": NOW,
            "item": item,
        })
    }

    fn stored_text(id: u64, text: &str) -> serde_json::Value {
        entry(id, 1, &serde_json::json!({ "text": { "text": text } }))
    }

    /// Writes a file of `version` that holds `entries`, where the outbox looks for one.
    fn write_by_hand(dir: &TempDir, version: u32, next: u64, entries: &[serde_json::Value]) {
        let body = serde_json::json!({ "version": version, "next": next, "items": entries });
        write_atomic(&file(dir), body.to_string().as_bytes()).unwrap();
    }

    #[test]
    fn what_is_added_waits_in_the_order_it_came_with_numbers_that_count_up() {
        let outbox = Outbox::memory();
        assert!(outbox.is_empty());
        let first = outbox.add(peer(1), "Pocket Phone", link("https://a.example/"), NOW);
        let second = outbox.add(peer(2), "Kitchen iPad", text("hello"), NOW + 1);
        assert_eq!((first, second), (Ok(1), Ok(2)));
        let list = outbox.list();
        assert_eq!(list[0].to, peer(1));
        assert_eq!(list[0].device_name, "Pocket Phone");
        assert_eq!(list[0].queued_at, NOW);
        assert_eq!(list[1].item, text("hello"));
        assert!(!outbox.is_empty());
    }

    #[test]
    fn a_number_is_not_used_again_after_its_item_is_struck_out() {
        let outbox = Outbox::memory();
        outbox.add(peer(1), "Phone", text("a"), NOW).unwrap();
        outbox.add(peer(1), "Phone", text("b"), NOW).unwrap();
        assert_eq!(
            outbox.remove(2).map(|waiting| waiting.item),
            Some(text("b"))
        );
        assert_eq!(outbox.add(peer(1), "Phone", text("c"), NOW), Ok(3));
        assert_eq!(ids(&outbox), [1, 3]);
        assert!(outbox.remove(2).is_none(), "it is not there any more");
    }

    #[test]
    fn the_item_that_waited_longest_for_a_device_is_the_next_for_it() {
        let outbox = Outbox::memory();
        outbox.add(peer(1), "A", text("a1"), NOW).unwrap();
        outbox.add(peer(2), "B", text("b1"), NOW).unwrap();
        outbox.add(peer(1), "A", text("a2"), NOW).unwrap();
        assert_eq!(outbox.devices(), [peer(1), peer(2)]);
        assert_eq!(outbox.next_for(&peer(1)).unwrap().item, text("a1"));
        assert_eq!(outbox.next_for(&peer(2)).unwrap().item, text("b1"));
        assert!(outbox.next_for(&peer(3)).is_none());
        outbox.remove(1);
        assert_eq!(outbox.next_for(&peer(1)).unwrap().item, text("a2"));
    }

    #[test]
    fn unpairing_a_device_strikes_out_what_waited_for_it_and_nothing_else() {
        let outbox = Outbox::memory();
        outbox.add(peer(1), "A", text("a1"), NOW).unwrap();
        outbox.add(peer(2), "B", text("b1"), NOW).unwrap();
        outbox.add(peer(1), "A", text("a2"), NOW).unwrap();
        let gone = outbox.remove_for(&peer(1));
        assert_eq!(
            gone.iter().map(|waiting| waiting.id).collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(ids(&outbox), [2]);
        assert_eq!(outbox.remove_for(&peer(1)), Vec::new());
    }

    #[test]
    fn an_item_is_dropped_when_a_day_is_over_and_not_before() {
        let outbox = Outbox::memory();
        outbox.add(peer(1), "A", text("old"), NOW).unwrap();
        outbox.add(peer(1), "A", text("young"), NOW + 60).unwrap();
        assert_eq!(outbox.expire(NOW + DAY - 1), Vec::new());
        let gone = outbox.expire(NOW + DAY);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].item, text("old"));
        assert_eq!(ids(&outbox), [2]);
        let gone = outbox.expire(NOW + 60 + DAY);
        assert_eq!(gone.len(), 1);
        assert!(outbox.is_empty());
    }

    #[test]
    fn a_clock_that_was_set_back_does_not_lengthen_the_time() {
        let outbox = Outbox::memory();
        outbox.add(peer(1), "A", text("x"), NOW + 10 * DAY).unwrap();
        // Now is earlier than the time that the item says it was queued: it was queued now.
        assert_eq!(outbox.expire(NOW), Vec::new());
        assert_eq!(outbox.expire(NOW + DAY - 1), Vec::new());
        assert_eq!(outbox.expire(NOW + DAY).len(), 1);
    }

    #[test]
    fn no_more_than_so_many_wait_for_one_device_and_for_all() {
        let outbox = Outbox::memory();
        for n in 0..MOST_FOR_A_DEVICE {
            outbox.add(peer(1), "A", text(&n.to_string()), NOW).unwrap();
        }
        assert_eq!(
            outbox.add(peer(1), "A", text("one more"), NOW),
            Err(Full::ForTheDevice)
        );
        // Another device is not held up by it, until the whole is full.
        let mut device = 2;
        while outbox.list().len() < MOST_WAITING {
            let room = MOST_WAITING - outbox.list().len();
            for _ in 0..room.min(MOST_FOR_A_DEVICE) {
                outbox.add(peer(device), "B", text("x"), NOW).unwrap();
            }
            device += 1;
        }
        assert_eq!(
            outbox.add(peer(200), "C", text("one more"), NOW),
            Err(Full::Everywhere)
        );
        assert_eq!(
            outbox.list().len(),
            MOST_WAITING,
            "nothing was dropped to make room"
        );
        assert_eq!(outbox.list()[0].id, 1);
    }

    #[test]
    fn an_item_that_would_make_the_file_too_large_is_refused_and_changes_nothing() {
        let outbox = Outbox::memory();
        let big = "x".repeat(MAX_CLIPBOARD_TEXT_BYTES);
        for _ in 0..2 {
            outbox.add(peer(1), "A", text(&big), NOW).unwrap();
        }
        // Two of a mebibyte fit in three, and a third does not.
        assert_eq!(
            outbox.add(peer(1), "A", text(&big), NOW),
            Err(Full::TooMuch)
        );
        assert_eq!(ids(&outbox), [1, 2]);
        // A small one still goes in, and has the next number: the refused one used none.
        assert_eq!(outbox.add(peer(1), "A", text("small"), NOW), Ok(3));
    }

    #[test]
    fn what_waits_is_kept_in_the_file_and_is_there_when_the_program_starts_again() {
        let dir = TempDir::new().unwrap();
        let outbox = Outbox::open(file(&dir));
        outbox
            .add(
                peer(1),
                "Pocket Phone",
                Item::Link(Link {
                    url: "https://a.example/".to_owned(),
                    title: Some("The page".to_owned()),
                }),
                NOW,
            )
            .unwrap();
        outbox
            .add(peer(2), "Kitchen iPad", text("hello"), NOW + 5)
            .unwrap();
        outbox
            .add(peer(1), "Pocket Phone", text("later"), NOW + 9)
            .unwrap();
        outbox.remove(3);

        let again = Outbox::open(file(&dir));
        assert_eq!(again.list(), outbox.list());
        assert_eq!(ids(&again), [1, 2]);
        // The numbers go on from where they were, also past the one that was struck out last.
        assert_eq!(
            again.add(peer(1), "Pocket Phone", text("new"), NOW + 20),
            Ok(4)
        );
    }

    #[test]
    fn every_change_is_in_the_file_before_the_program_starts_again() {
        let dir = TempDir::new().unwrap();
        let outbox = Outbox::open(file(&dir));
        let same_when_read_again = |after: &str| {
            assert_eq!(
                Outbox::open(file(&dir)).list(),
                outbox.list(),
                "after {after}"
            );
        };
        outbox.add(peer(1), "A", text("a1"), NOW).unwrap();
        outbox.add(peer(2), "B", text("b1"), NOW + 60).unwrap();
        outbox.add(peer(3), "C", text("c1"), NOW + 120).unwrap();
        outbox.add(peer(3), "C", text("c2"), NOW + 120).unwrap();
        same_when_read_again("adding");
        assert!(outbox.remove(2).is_some());
        same_when_read_again("striking one out");
        assert_eq!(outbox.remove_for(&peer(3)).len(), 2);
        same_when_read_again("unpairing a device");
        assert_eq!(outbox.expire(NOW + DAY).len(), 1);
        same_when_read_again("a day was over");
        assert!(outbox.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_can_be_read_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let outbox = Outbox::open(file(&dir));
        outbox.add(peer(1), "A", text("a password"), NOW).unwrap();
        let mode = fs::metadata(file(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "mode is {mode:o}");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_other_users_can_read_is_not_read() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        Outbox::open(file(&dir))
            .add(peer(1), "A", text("a password"), NOW)
            .unwrap();
        assert_eq!(Outbox::open(file(&dir)).list().len(), 1);
        // It is not the file that this program made, and what is in it may have been read: it is
        // not gone on with, as with the list of paired devices.
        fs::set_permissions(file(&dir), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Outbox::open(file(&dir)).is_empty());
    }

    #[test]
    fn a_file_that_nothing_wrote_is_an_empty_outbox_and_makes_no_file() {
        let dir = TempDir::new().unwrap();
        let outbox = Outbox::open(file(&dir));
        assert!(outbox.is_empty());
        assert!(!file(&dir).exists());
    }

    #[test]
    fn an_entry_that_makes_no_sense_is_left_out_and_the_rest_is_kept() {
        let dir = TempDir::new().unwrap();
        let good = |id: u64, to: u8| {
            format!(
                r#"{{"id":{id},"to":"{}","name":"Phone","queued_at":{NOW},"item":{{"text":{{"text":"hello {id}"}}}}}}"#,
                peer(to)
            )
        };
        let entries = [
            good(1, 1),
            r#"{"id":2,"to":"not a device","name":"Phone","queued_at":5,"item":{"text":{"text":"x"}}}"#.to_owned(),
            r#"{"id":3}"#.to_owned(),
            good(4, 1),
            // The same number twice.
            good(4, 2),
            // Nothing to send.
            format!(
                r#"{{"id":6,"to":"{}","name":"Phone","queued_at":{NOW},"item":{{"text":{{"text":"   "}}}}}}"#,
                peer(1)
            ),
            // A link with nothing in it.
            format!(
                r#"{{"id":7,"to":"{}","name":"Phone","queued_at":{NOW},"item":{{"link":{{"url":""}}}}}}"#,
                peer(1)
            ),
        ];
        let body = format!(
            r#"{{"version":1,"next":2,"items":[{}]}}"#,
            entries.join(",")
        );
        write_atomic(&file(&dir), body.as_bytes()).unwrap();

        let outbox = Outbox::open(file(&dir));
        assert_eq!(ids(&outbox), [1, 4]);
        // What was left out is gone from the file too, before anything else is written to it.
        let written = fs::read_to_string(file(&dir)).unwrap();
        assert!(!written.contains("not a device"), "{written}");
        assert_eq!(ids(&Outbox::open(file(&dir))), [1, 4]);
        // The numbers go on past the ones that are kept.
        assert_eq!(outbox.add(peer(1), "Phone", text("new"), NOW), Ok(5));
    }

    #[test]
    fn an_entry_over_a_limit_of_the_protocol_is_left_out_and_one_at_the_limit_stays() {
        let dir = TempDir::new().unwrap();
        let stored_link = |id: u64, url: String, title: Option<String>| {
            let mut link = serde_json::json!({ "url": url });
            if let Some(title) = title {
                link["title"] = title.into();
            }
            entry(id, 1, &serde_json::json!({ "link": link }))
        };
        let entries = [
            stored_link(1, "u".repeat(MAX_URL_BYTES), None),
            stored_link(2, "u".repeat(MAX_URL_BYTES + 1), None),
            stored_link(
                3,
                "https://a.example/".to_owned(),
                Some("t".repeat(MAX_TITLE_BYTES)),
            ),
            stored_link(
                4,
                "https://a.example/".to_owned(),
                Some("t".repeat(MAX_TITLE_BYTES + 1)),
            ),
            stored_text(5, &"x".repeat(MAX_CLIPBOARD_TEXT_BYTES)),
            stored_text(6, &"x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1)),
        ];
        write_by_hand(&dir, 1, 7, &entries);

        assert_eq!(ids(&Outbox::open(file(&dir))), [1, 3, 5]);
    }

    #[test]
    fn a_file_with_more_than_there_is_room_for_keeps_the_oldest_and_is_cut_down() {
        let dir = TempDir::new().unwrap();
        let most = MOST_WAITING as u64;
        let entries: Vec<_> = (1..=most + 10).map(|id| stored_text(id, "x")).collect();
        write_by_hand(&dir, 1, most + 11, &entries);

        let outbox = Outbox::open(file(&dir));
        assert_eq!(ids(&outbox), (1..=most).collect::<Vec<_>>());
        // The rest is gone from the file too, and nothing more fits.
        assert_eq!(ids(&Outbox::open(file(&dir))), ids(&outbox));
        assert_eq!(
            outbox.add(peer(2), "Other", text("one more"), NOW),
            Err(Full::Everywhere)
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_empty_outbox_and_the_program_starts() {
        let dir = TempDir::new().unwrap();
        write_atomic(&file(&dir), b"this is not json").unwrap();
        let outbox = Outbox::open(file(&dir));
        assert!(outbox.is_empty());
        // What is queued from now on is kept, and replaces the file.
        outbox.add(peer(1), "A", text("a"), NOW).unwrap();
        assert_eq!(Outbox::open(file(&dir)).list().len(), 1);
    }

    #[test]
    fn a_file_of_another_format_is_not_read() {
        let dir = TempDir::new().unwrap();
        // An entry that makes sense in the format that this build reads, so that it is the format
        // that decides.
        let entries = [stored_text(1, "hello")];
        write_by_hand(&dir, 2, 9, &entries);
        assert!(Outbox::open(file(&dir)).is_empty());
        write_by_hand(&dir, 1, 9, &entries);
        assert_eq!(ids(&Outbox::open(file(&dir))), [1]);
    }

    #[test]
    fn what_a_device_is_called_is_made_safe_to_show() {
        let shown = |outbox: &Outbox| {
            let name = outbox.list()[0].device_name.clone();
            assert!(
                !name.contains('\u{202e}') && !name.contains('\n'),
                "{name:?}"
            );
        };
        // As it is added.
        let outbox = Outbox::memory();
        outbox
            .add(peer(1), "Pocket\u{202e}Phone\n", text("a"), NOW)
            .unwrap();
        shown(&outbox);
        // And as it is read from a file that somebody else wrote.
        let dir = TempDir::new().unwrap();
        let mut entry = stored_text(1, "a");
        entry["name"] = "Pocket\u{202e}Phone\n".into();
        write_by_hand(&dir, 1, 2, &[entry]);
        shown(&Outbox::open(file(&dir)));
    }

    #[test]
    fn what_waits_is_kept_out_of_the_logs() {
        let outbox = Outbox::memory();
        outbox
            .add(
                peer(1),
                "A",
                link("https://token.example/?secret=hunter2"),
                NOW,
            )
            .unwrap();
        outbox
            .add(peer(1), "A", text("the password is hunter2"), NOW)
            .unwrap();
        let shown = format!("{:?} {:?}", outbox.list(), outbox);
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
