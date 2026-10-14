// SPDX-License-Identifier: Apache-2.0

//! The inbox: links and text that paired devices handed this one, kept until the person
//! deals with them.
//!
//! The inbox is where a received item rests. Nothing is opened or put on the clipboard
//! because it arrived: the person looks at the inbox and decides. Settings that open links
//! or replace the clipboard by themselves belong to the agent, which comes with the
//! locked-device rules in `docs/locked-devices.md`. Until it exists, a device set to open
//! links or apply clipboard text keeps them in the inbox like any other.
//!
//! # Where it lives
//!
//! The inbox is one file, `inbox.json`, in the [state directory](crate::DataDir). Like the
//! list of paired devices it is readable by its owner only, because clipboard text can be a
//! password. Unlike the rest of the state it is used by more than one process at a time: a
//! running listener adds to it while the person reads and clears it from another terminal.
//! So every change takes an exclusive lock on `inbox.lock` beside the file, reads the
//! file again, changes it and replaces it in one step. A reader that changes nothing takes
//! no lock and sees the old file or the new one, never half of one.
//!
//! The methods that read or change the file block the calling thread, for as long as the disk
//! and another process take, so call them from `spawn_blocking` when running on an async
//! runtime. [`InboxHandler`] does.
//!
//! # Limits
//!
//! At most `handoff.max_inbox_items` items are kept and the oldest are dropped first. The
//! file never grows past 3 MiB, which is under what Baton will read back, and older items are
//! dropped to keep it there. An item that does not fit even alone is declined as too large.
//! Every item gets a number that counts up from 1 and is never reused, so "item 7" means the
//! same thing from one command to the next.

use std::{
    fmt,
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use baton_proto::{
    DeviceId,
    device::{FALLBACK_NAME, sanitize_display_name},
    limits::{MAX_CLIPBOARD_TEXT_BYTES, MAX_TITLE_BYTES, MAX_URL_BYTES},
    message::{DeclineReason, Handoff, Item, Outcome, capability},
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use url::Url;

use crate::{
    clock::{Clock, SystemClock},
    config::MAX_INBOX_ITEMS_LIMIT,
    error::{Error, Result},
    fsutil::{Access, MAX_STATE_FILE_BYTES, read_text, write_atomic},
    policy::{ClipboardPolicy, LinkPolicy},
    session::HandoffHandler,
    trust::{PairedDevice, hex_id},
    util::{BoxFuture, blocking},
};

/// Newest file format this build reads and writes.
const FORMAT_VERSION: u32 = 1;

/// Largest inbox file this build writes. The limit for reading a state file is higher, so
/// that anything written here can be read back.
const MAX_FILE_BYTES: usize = 3 * 1024 * 1024;

/// Room for everything in the file that is not an item.
const HEADER_ALLOWANCE: usize = 128;

const _: () = assert!(MAX_FILE_BYTES as u64 <= MAX_STATE_FILE_BYTES);

/// How long a change waits for another process that is using the inbox.
const LOCK_WAIT: Duration = Duration::from_secs(5);

/// How often a change that has to wait looks at the lock again.
const LOCK_POLL: Duration = Duration::from_millis(10);

/// How many received items a slow subscriber can fall behind by.
const EVENT_BACKLOG: usize = 64;

/// What was handed over.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum InboxItem {
    /// A web link. It is kept as a browser would send it: plain ASCII, with the host in
    /// lower case and anything else percent-encoded.
    Link {
        /// The address.
        url: String,
        /// The page title, cleaned like a device name, if the sender knew one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// Text the sender copied.
    Text {
        /// The text, exactly as sent.
        text: String,
    },
}

// A link carries secrets in its query and credentials, and text may be a password, so
// neither is printed: this appears in logs.
impl fmt::Debug for InboxItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link { url, title } => f
                .debug_struct("Link")
                .field("url_bytes", &url.len())
                .field("title_bytes", &title.as_ref().map(String::len))
                .finish(),
            Self::Text { text } => write!(f, "Text({} bytes)", text.len()),
        }
    }
}

impl InboxItem {
    /// What is wrong with an item read from disk, if anything.
    fn problem(&self) -> Option<String> {
        match self {
            Self::Link { url, title } => {
                if url.is_empty() {
                    return Some("a link is empty".to_owned());
                }
                if url.len() > MAX_URL_BYTES {
                    return Some(format!("a link is {} bytes long", url.len()));
                }
                title
                    .as_ref()
                    .filter(|title| title.len() > MAX_TITLE_BYTES)
                    .map(|title| format!("a title is {} bytes long", title.len()))
            }
            Self::Text { text } => (text.len() > MAX_CLIPBOARD_TEXT_BYTES)
                .then(|| format!("a text is {} bytes long", text.len())),
        }
    }
}

/// One received item, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxEntry {
    /// Counts up from 1 in this inbox and is never reused.
    pub number: u64,
    /// The device that sent it.
    #[serde(with = "hex_id")]
    pub from: DeviceId,
    /// The name that device was shown under when this arrived, cleaned for display.
    pub from_name: String,
    /// When it arrived, in seconds since the Unix epoch.
    pub received_at: u64,
    /// What it is.
    pub item: InboxItem,
}

/// What [`Inbox::add`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// The item is in the inbox.
    Stored(InboxEntry),
    /// The item is too big to keep, even with every older item dropped. Nothing changed.
    TooLarge,
}

/// The file as read.
#[derive(Debug)]
struct State {
    next_number: u64,
    entries: Vec<InboxEntry>,
}

impl State {
    fn empty() -> Self {
        Self {
            next_number: 1,
            entries: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
struct Stored {
    version: u32,
    next_number: u64,
    items: Vec<InboxEntry>,
}

#[derive(Serialize)]
struct StoredRef<'a> {
    version: u32,
    next_number: u64,
    items: &'a [InboxEntry],
}

struct Inner {
    path: PathBuf,
    lock_path: PathBuf,
    max_items: usize,
    lock_wait: Duration,
    received: broadcast::Sender<Arc<InboxEntry>>,
}

/// The items other devices handed this one. Cloning gives another handle to the same inbox.
#[derive(Clone)]
pub struct Inbox {
    inner: Arc<Inner>,
}

impl fmt::Debug for Inbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inbox")
            .field("path", &self.inner.path)
            .field("max_items", &self.inner.max_items)
            .finish()
    }
}

impl Inbox {
    /// Opens the inbox kept in `path`, which keeps at most `max_items` items.
    ///
    /// A missing file is an empty inbox and is not created. A file that exists but cannot be
    /// understood is an error, never an empty inbox: the items in it are not Baton's to throw
    /// away. On Unix a file that other users can read or change is refused too, since it can
    /// hold passwords.
    ///
    /// # Errors
    ///
    /// If the file cannot be read or is not a valid inbox.
    pub fn load(path: impl Into<PathBuf>, max_items: usize) -> Result<Self> {
        Self::load_waiting(path, max_items, LOCK_WAIT)
    }

    fn load_waiting(
        path: impl Into<PathBuf>,
        max_items: usize,
        lock_wait: Duration,
    ) -> Result<Self> {
        let path = path.into();
        read_state(&path)?;
        let (received, _) = broadcast::channel(EVENT_BACKLOG);
        Ok(Self {
            inner: Arc::new(Inner {
                lock_path: path.with_extension("lock"),
                path,
                max_items: max_items.clamp(1, MAX_INBOX_ITEMS_LIMIT),
                lock_wait,
                received,
            }),
        })
    }

    /// The file the inbox is kept in.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Every item, oldest first.
    ///
    /// # Errors
    ///
    /// If the file cannot be read or is not a valid inbox.
    pub fn list(&self) -> Result<Vec<InboxEntry>> {
        Ok(read_state(&self.inner.path)?.entries)
    }

    /// The item with this number, if it is still in the inbox.
    ///
    /// # Errors
    ///
    /// If the file cannot be read or is not a valid inbox.
    pub fn get(&self, number: u64) -> Result<Option<InboxEntry>> {
        Ok(read_state(&self.inner.path)?
            .entries
            .into_iter()
            .find(|entry| entry.number == number))
    }

    /// Announces each item as it is added through any handle to this inbox in this process.
    ///
    /// Items added by other processes are not announced. A receiver that falls behind gets
    /// [`RecvError::Lagged`](broadcast::error::RecvError::Lagged) and should read the
    /// inbox again.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<InboxEntry>> {
        self.inner.received.subscribe()
    }

    /// Keeps `item`, which `from` sent at `now`, and drops the oldest items that no longer fit.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, is not a valid inbox, cannot be written, or stays locked by
    /// another process for several seconds.
    pub fn add(&self, from: &PairedDevice, item: InboxItem, now: u64) -> Result<Added> {
        let _held = self.lock()?;
        let mut state = read_state(&self.inner.path)?;
        let number = state.next_number;
        let entry = InboxEntry {
            number,
            from: from.id,
            from_name: from.display_name().to_owned(),
            received_at: now,
            item,
        };
        state.next_number = number
            .checked_add(1)
            .ok_or_else(|| Error::Internal("the inbox ran out of item numbers".to_owned()))?;
        state.entries.push(entry.clone());
        let over_count = state.entries.len().saturating_sub(self.inner.max_items);
        state.entries.drain(..over_count);
        if !fit_in_file(&mut state.entries)? {
            return Ok(Added::TooLarge);
        }
        self.save(&state)?;
        // Nobody listening is fine.
        let _ = self.inner.received.send(Arc::new(entry.clone()));
        Ok(Added::Stored(entry))
    }

    /// Removes the item with this number and returns it, or `None` if there is no such item.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, is not a valid inbox, cannot be written, or stays locked by
    /// another process for several seconds.
    pub fn remove(&self, number: u64) -> Result<Option<InboxEntry>> {
        let _held = self.lock()?;
        let mut state = read_state(&self.inner.path)?;
        let Some(at) = state.entries.iter().position(|e| e.number == number) else {
            return Ok(None);
        };
        let removed = state.entries.remove(at);
        self.save(&state)?;
        Ok(Some(removed))
    }

    /// Removes every item and returns how many there were. Numbers are not reused.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, is not a valid inbox, cannot be written, or stays locked by
    /// another process for several seconds.
    pub fn clear(&self) -> Result<usize> {
        let _held = self.lock()?;
        let mut state = read_state(&self.inner.path)?;
        let removed = state.entries.len();
        if removed > 0 {
            state.entries.clear();
            self.save(&state)?;
        }
        Ok(removed)
    }

    /// Takes the exclusive right to change the inbox, waiting a few seconds for another
    /// process to finish if it has it.
    fn lock(&self) -> Result<Held> {
        let path = &self.inner.lock_path;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Nothing in the file matters, but another user who can open it could hold the lock.
            options.mode(0o600);
        }
        let file = options.open(path).map_err(Error::io("open", path))?;
        let give_up = Instant::now() + self.inner.lock_wait;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Held { _file: file }),
                Err(TryLockError::WouldBlock) if Instant::now() < give_up => {
                    std::thread::sleep(LOCK_POLL);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(Error::Timeout(
                        "waiting for another Baton process to finish with the inbox",
                    ));
                }
                Err(TryLockError::Error(e)) => return Err(Error::io("lock", path)(e)),
            }
        }
    }

    fn save(&self, state: &State) -> Result<()> {
        let json = serde_json::to_vec(&StoredRef {
            version: FORMAT_VERSION,
            next_number: state.next_number,
            items: &state.entries,
        })
        .map_err(|e| Error::invalid(&self.inner.path, &e))?;
        if json.len() > MAX_FILE_BYTES {
            return Err(Error::Internal(format!(
                "the inbox came to {} bytes after it was trimmed to fit in {MAX_FILE_BYTES}",
                json.len()
            )));
        }
        write_atomic(&self.inner.path, &json)
    }
}

/// Proof that this thread may change the inbox. The lock is released when it is dropped, and
/// by the operating system if the process dies.
struct Held {
    _file: File,
}

/// Drops the oldest entries until the file that holds the rest fits in [`MAX_FILE_BYTES`].
///
/// Returns `false`, having changed nothing, if the newest entry does not fit even alone.
fn fit_in_file(entries: &mut Vec<InboxEntry>) -> Result<bool> {
    let sizes = entries
        .iter()
        .map(|entry| {
            serde_json::to_vec(entry)
                .map(|json| json.len() + 1)
                .map_err(|e| Error::Internal(e.to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut total = HEADER_ALLOWANCE + sizes.iter().sum::<usize>();
    let mut dropped = 0;
    for size in sizes.iter().take(entries.len().saturating_sub(1)) {
        if total <= MAX_FILE_BYTES {
            break;
        }
        total -= size;
        dropped += 1;
    }
    if total > MAX_FILE_BYTES {
        return Ok(false);
    }
    entries.drain(..dropped);
    Ok(true)
}

/// Reads and checks the inbox file. A missing file is an empty inbox.
fn read_state(path: &Path) -> Result<State> {
    match read_text(path, Access::OwnerOnly)? {
        None => Ok(State::empty()),
        Some(text) => parse(path, &text),
    }
}

fn parse(path: &Path, text: &str) -> Result<State> {
    let mut stored: Stored = serde_json::from_str(text).map_err(|e| Error::invalid(path, &e))?;
    if stored.version > FORMAT_VERSION {
        return Err(Error::UnsupportedVersion {
            path: path.to_owned(),
            found: stored.version,
            supported: FORMAT_VERSION,
        });
    }
    if stored.items.len() > MAX_INBOX_ITEMS_LIMIT {
        return Err(Error::invalid(
            path,
            &format!(
                "holds {} items, more than the limit of {MAX_INBOX_ITEMS_LIMIT}",
                stored.items.len()
            ),
        ));
    }
    let mut previous = 0;
    for entry in &mut stored.items {
        if entry.number <= previous || entry.number >= stored.next_number {
            return Err(Error::invalid(
                path,
                &format!(
                    "item {} is out of order: numbers must start above 0 and count up",
                    entry.number
                ),
            ));
        }
        previous = entry.number;
        if let Some(problem) = entry.item.problem() {
            return Err(Error::invalid(
                path,
                &format!("item {}: {problem}", entry.number),
            ));
        }
        // A name is cleaned when it is received, but this file is only as trustworthy as the
        // disk, and a name read back is shown to a person.
        entry.from_name = sanitize_display_name(&entry.from_name);
    }
    Ok(State {
        next_number: stored.next_number,
        entries: stored.items,
    })
}

/// Keeps what paired devices hand over in an [`Inbox`].
///
/// A device whose settings say to ignore links, or clipboard text, has them declined as
/// blocked. Everything else is kept for the person to look at, including links set to open
/// and clipboard text set to apply: those settings are for the agent, which does not exist
/// yet, and opening or applying an item needs the locked-device rules it will carry. A link
/// must be a well-formed address without spaces or control characters, which is kept in its
/// normal form, and clipboard text must not be empty.
#[derive(Debug)]
pub struct InboxHandler {
    inbox: Inbox,
    clock: Arc<dyn Clock>,
}

impl InboxHandler {
    /// A handler that keeps items in `inbox`.
    pub fn new(inbox: Inbox) -> Self {
        Self {
            inbox,
            clock: Arc::new(SystemClock),
        }
    }

    /// Replaces the clock that stamps items, mainly so tests can move time.
    #[must_use]
    pub fn with_clock(mut self, clock: impl Clock) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    async fn take(&self, from: &PairedDevice, item: Item) -> Outcome {
        let item = match item {
            Item::Link(link) => {
                if from.policy.links == LinkPolicy::Ignore {
                    return Outcome::Declined(DeclineReason::Blocked);
                }
                let url = match normalise_link(&link.url) {
                    Ok(url) => url,
                    Err(reason) => return Outcome::Declined(reason),
                };
                InboxItem::Link {
                    url,
                    title: link.title.as_deref().and_then(clean_title),
                }
            }
            Item::ClipboardText(clip) => {
                if from.policy.clipboard == ClipboardPolicy::Ignore {
                    return Outcome::Declined(DeclineReason::Blocked);
                }
                if clip.text.is_empty() {
                    return Outcome::Declined(DeclineReason::Invalid);
                }
                InboxItem::Text { text: clip.text }
            }
        };
        let inbox = self.inbox.clone();
        let from = from.clone();
        let now = self.clock.now_unix();
        match blocking(move || inbox.add(&from, item, now)).await {
            Ok(Ok(Added::Stored(_))) => Outcome::Accepted,
            Ok(Ok(Added::TooLarge)) => Outcome::Declined(DeclineReason::TooLarge),
            Ok(Err(e)) | Err(e) => {
                tracing::warn!(error = %e, "could not keep a received item in the inbox");
                Outcome::Declined(DeclineReason::Other)
            }
        }
    }
}

impl HandoffHandler for InboxHandler {
    fn capabilities(&self) -> &'static [&'static str] {
        &[capability::LINK, capability::CLIPBOARD_TEXT]
    }

    fn handle<'a>(&'a self, from: &'a PairedDevice, handoff: Handoff) -> BoxFuture<'a, Outcome> {
        Box::pin(self.take(from, handoff.item))
    }
}

/// A link in the form it is kept in: plain ASCII, as a browser would send it.
///
/// Spaces and control characters are refused before anything is parsed, because the parser
/// silently drops tabs and line breaks, which would make the stored link differ from the
/// one that was sent.
fn normalise_link(raw: &str) -> std::result::Result<String, DeclineReason> {
    if raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(DeclineReason::Invalid);
    }
    let url = Url::parse(raw).map_err(|_| DeclineReason::Invalid)?;
    if url.as_str().len() > MAX_URL_BYTES {
        return Err(DeclineReason::TooLarge);
    }
    Ok(url.into())
}

/// A page title that is safe to show, or `None` if nothing visible is left of it.
fn clean_title(raw: &str) -> Option<String> {
    let cleaned = sanitize_display_name(raw);
    // Nothing left to show is no title, not a title that says "unnamed device".
    (cleaned != FALLBACK_NAME || raw.trim() == FALLBACK_NAME).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use std::{fs, thread};

    use baton_proto::{
        DeviceInfo, Platform,
        message::{ClipboardText, Link},
    };
    use tempfile::TempDir;

    use super::*;
    use crate::{clock::ManualClock, policy::DevicePolicy, trust::TrustStore};

    const NOW: u64 = 1_800_000_000;

    fn paired(n: u8, name: &str, policy: DevicePolicy) -> PairedDevice {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::load(dir.path().join("devices.json")).unwrap();
        let id = DeviceId::from_bytes([n; 32]);
        let info = DeviceInfo {
            name: name.to_owned(),
            platform: Platform::Linux,
            app_version: "0.0.0-test".to_owned(),
        };
        store.pair(id, &info, NOW).unwrap();
        store.update(&id, |device| device.policy = policy).unwrap();
        store.get(&id).unwrap()
    }

    fn laptop() -> PairedDevice {
        paired(1, "Laptop", DevicePolicy::default())
    }

    fn inbox_in(dir: &TempDir, max_items: usize) -> Inbox {
        Inbox::load(dir.path().join("inbox.json"), max_items).unwrap()
    }

    fn inbox(max_items: usize) -> (TempDir, Inbox) {
        let dir = tempfile::tempdir().unwrap();
        let inbox = inbox_in(&dir, max_items);
        (dir, inbox)
    }

    fn link(url: &str) -> InboxItem {
        InboxItem::Link {
            url: url.to_owned(),
            title: None,
        }
    }

    fn text(text: &str) -> InboxItem {
        InboxItem::Text {
            text: text.to_owned(),
        }
    }

    fn stored(added: Added) -> InboxEntry {
        match added {
            Added::Stored(entry) => entry,
            Added::TooLarge => panic!("the item was refused as too large"),
        }
    }

    fn numbers(inbox: &Inbox) -> Vec<u64> {
        inbox.list().unwrap().iter().map(|e| e.number).collect()
    }

    #[test]
    fn a_missing_file_is_an_empty_inbox_and_is_not_created() {
        let (dir, inbox) = inbox(10);
        assert_eq!(inbox.list().unwrap(), Vec::new());
        assert_eq!(inbox.get(1).unwrap(), None);
        assert!(!dir.path().join("inbox.json").exists());
    }

    #[test]
    fn an_item_is_kept_with_its_sender_and_a_number() {
        let (dir, inbox) = inbox(10);
        let entry = stored(
            inbox
                .add(&laptop(), link("https://example.com/a"), NOW)
                .unwrap(),
        );
        assert_eq!(entry.number, 1);
        assert_eq!(entry.from, DeviceId::from_bytes([1; 32]));
        assert_eq!(entry.from_name, "Laptop");
        assert_eq!(entry.received_at, NOW);
        assert_eq!(inbox.list().unwrap(), vec![entry.clone()]);
        assert_eq!(inbox.get(1).unwrap(), Some(entry));

        // The file is readable JSON with the sender written as hexadecimal text.
        let json = fs::read_to_string(dir.path().join("inbox.json")).unwrap();
        assert!(json.contains(&"01".repeat(32)), "{json}");
        assert!(json.contains(r#""kind":"link""#), "{json}");
    }

    #[test]
    fn what_was_kept_is_still_there_in_a_new_handle() {
        let (dir, inbox) = inbox(10);
        inbox.add(&laptop(), text("hello"), NOW).unwrap();
        inbox
            .add(&laptop(), link("https://example.com/"), NOW + 1)
            .unwrap();
        let again = inbox_in(&dir, 10);
        assert_eq!(numbers(&again), [1, 2]);
        assert_eq!(again.get(1).unwrap().unwrap().item, text("hello"));
    }

    #[test]
    fn numbers_count_up_and_are_not_reused() {
        let (_dir, inbox) = inbox(10);
        for n in 1..=3 {
            let entry = stored(inbox.add(&laptop(), text("x"), NOW).unwrap());
            assert_eq!(entry.number, n);
        }
        assert_eq!(inbox.remove(3).unwrap().unwrap().number, 3);
        assert_eq!(
            stored(inbox.add(&laptop(), text("x"), NOW).unwrap()).number,
            4
        );
        assert_eq!(inbox.clear().unwrap(), 3);
        assert_eq!(
            stored(inbox.add(&laptop(), text("x"), NOW).unwrap()).number,
            5
        );
    }

    #[test]
    fn the_oldest_items_go_first_past_the_limit() {
        let (_dir, inbox) = inbox(3);
        for _ in 0..5 {
            inbox.add(&laptop(), text("x"), NOW).unwrap();
        }
        assert_eq!(numbers(&inbox), [3, 4, 5]);
    }

    #[test]
    fn the_file_stays_within_its_size_budget() {
        let (dir, inbox) = inbox(200);
        let big = "a".repeat(1024 * 1024);
        for _ in 0..7 {
            inbox.add(&laptop(), text(&big), NOW).unwrap();
        }
        let size = fs::metadata(dir.path().join("inbox.json")).unwrap().len();
        assert!(size <= MAX_FILE_BYTES as u64, "{size} bytes");
        // Two of these fit under 3 MiB, and the newest is always one of them.
        assert_eq!(numbers(&inbox), [6, 7]);
        // And what was written can be read back by a new handle.
        assert_eq!(numbers(&inbox_in(&dir, 200)), [6, 7]);
    }

    #[test]
    fn an_item_that_does_not_fit_alone_is_refused_and_changes_nothing() {
        let (dir, inbox) = inbox(10);
        inbox.add(&laptop(), text("keep me"), NOW).unwrap();
        let before = fs::read(dir.path().join("inbox.json")).unwrap();
        // Each NUL is written as six bytes of JSON, so this is far over the budget.
        let heavy = "\0".repeat(1024 * 1024);
        assert_eq!(
            inbox.add(&laptop(), text(&heavy), NOW).unwrap(),
            Added::TooLarge
        );
        assert_eq!(fs::read(dir.path().join("inbox.json")).unwrap(), before);
        // The refused item did not use up a number.
        assert_eq!(
            stored(inbox.add(&laptop(), text("next"), NOW).unwrap()).number,
            2
        );
    }

    #[test]
    fn removing_and_clearing() {
        let (dir, inbox) = inbox(10);
        for _ in 0..3 {
            inbox.add(&laptop(), text("x"), NOW).unwrap();
        }
        assert_eq!(inbox.remove(2).unwrap().unwrap().number, 2);
        assert_eq!(numbers(&inbox), [1, 3]);

        // An item that is not there is not an error and does not touch the file.
        let before = fs::read(dir.path().join("inbox.json")).unwrap();
        assert_eq!(inbox.remove(2).unwrap(), None);
        assert_eq!(inbox.remove(99).unwrap(), None);
        assert_eq!(fs::read(dir.path().join("inbox.json")).unwrap(), before);

        assert_eq!(inbox.clear().unwrap(), 2);
        assert_eq!(inbox.list().unwrap(), Vec::new());
        assert_eq!(inbox.clear().unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn the_files_are_private_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, inbox) = inbox(10);
        inbox.add(&laptop(), text("secret"), NOW).unwrap();
        for name in ["inbox.json", "inbox.lock"] {
            let mode = fs::metadata(dir.path().join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_other_users_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, inbox) = inbox(10);
        inbox.add(&laptop(), text("secret"), NOW).unwrap();
        let path = dir.path().join("inbox.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            Inbox::load(&path, 10),
            Err(Error::InsecurePermissions { .. })
        ));
        assert!(matches!(
            inbox.list(),
            Err(Error::InsecurePermissions { .. })
        ));
    }

    fn write_file(dir: &TempDir, json: &str) -> PathBuf {
        let path = dir.path().join("inbox.json");
        fs::write(&path, json).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    fn entry_json(number: u64, name: &str, item: &str) -> String {
        let name = serde_json::to_string(name).unwrap();
        format!(
            r#"{{"number":{number},"from":"{}","from_name":{name},"received_at":5,"item":{item}}}"#,
            "02".repeat(32)
        )
    }

    fn file_json(next_number: u64, entries: &[String]) -> String {
        format!(
            r#"{{"version":1,"next_number":{next_number},"items":[{}]}}"#,
            entries.join(",")
        )
    }

    #[test]
    fn a_damaged_file_is_an_error_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, "this is not json");
        assert!(matches!(Inbox::load(&path, 10), Err(Error::Invalid { .. })));

        // An inbox that was fine when it was opened and was damaged afterwards.
        let (dir, inbox) = inbox(10);
        inbox.add(&laptop(), text("x"), NOW).unwrap();
        let path = write_file(&dir, "{\"version\":1,");
        assert!(inbox.list().is_err());
        assert!(inbox.add(&laptop(), text("y"), NOW).is_err());
        assert!(inbox.remove(1).is_err());
        assert!(inbox.clear().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "{\"version\":1,");
    }

    #[test]
    fn a_newer_format_is_refused_rather_than_misread() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, r#"{"version":2,"next_number":1,"items":[]}"#);
        assert!(matches!(
            Inbox::load(&path, 10),
            Err(Error::UnsupportedVersion {
                found: 2,
                supported: 1,
                ..
            })
        ));
    }

    #[test]
    fn numbers_that_do_not_count_up_are_refused() {
        let item = r#"{"kind":"text","text":"x"}"#;
        let entry = |n| entry_json(n, "Laptop", item);
        let cases = [
            ("a number of zero", file_json(5, &[entry(0)])),
            ("a repeated number", file_json(5, &[entry(2), entry(2)])),
            ("numbers going down", file_json(5, &[entry(3), entry(2)])),
            (
                "a number the next one would repeat",
                file_json(3, &[entry(3)]),
            ),
        ];
        for (what, json) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = write_file(&dir, &json);
            assert!(
                matches!(Inbox::load(&path, 10), Err(Error::Invalid { .. })),
                "{what}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, &file_json(5, &[entry(2), entry(4)]));
        assert_eq!(numbers(&Inbox::load(&path, 10).unwrap()), [2, 4]);
    }

    #[test]
    fn items_that_break_the_limits_are_refused() {
        let long_link = format!(
            r#"{{"kind":"link","url":"https://example.com/{}"}}"#,
            "a".repeat(MAX_URL_BYTES)
        );
        let long_title = format!(
            r#"{{"kind":"link","url":"https://example.com/","title":"{}"}}"#,
            "t".repeat(MAX_TITLE_BYTES + 1)
        );
        let long_text = format!(
            r#"{{"kind":"text","text":"{}"}}"#,
            "x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1)
        );
        let empty_link = r#"{"kind":"link","url":""}"#.to_owned();
        for item in [long_link, long_title, long_text, empty_link] {
            let dir = tempfile::tempdir().unwrap();
            let path = write_file(&dir, &file_json(2, &[entry_json(1, "Laptop", &item)]));
            assert!(matches!(Inbox::load(&path, 10), Err(Error::Invalid { .. })));
        }
    }

    #[test]
    fn a_file_with_too_many_items_is_refused() {
        let item = r#"{"kind":"text","text":"x"}"#;
        let entries: Vec<String> = (1..=MAX_INBOX_ITEMS_LIMIT as u64 + 1)
            .map(|n| entry_json(n, "Laptop", item))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(&dir, &file_json(5_000, &entries));
        assert!(matches!(Inbox::load(&path, 10), Err(Error::Invalid { .. })));
    }

    #[test]
    fn names_read_back_are_cleaned_like_names_received() {
        let item = r#"{"kind":"text","text":"x"}"#;
        let dir = tempfile::tempdir().unwrap();
        let path = write_file(
            &dir,
            &file_json(2, &[entry_json(1, "Lap\u{202e}top\n\u{1b}[2J", item)]),
        );
        let inbox = Inbox::load(&path, 10).unwrap();
        assert_eq!(inbox.list().unwrap()[0].from_name, "Laptop [2J");
    }

    #[test]
    fn handles_in_different_threads_do_not_lose_each_others_items() {
        let (dir, first) = inbox(200);
        // A second handle, as another process would have.
        let second = inbox_in(&dir, 200);
        let workers: Vec<_> = [first.clone(), second]
            .into_iter()
            .chain([first])
            .enumerate()
            .map(|(i, inbox)| {
                thread::spawn(move || {
                    let from = paired(
                        u8::try_from(i).unwrap() + 1,
                        "Laptop",
                        DevicePolicy::default(),
                    );
                    for _ in 0..20 {
                        stored(inbox.add(&from, text("x"), NOW).unwrap());
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let all = numbers(&inbox_in(&dir, 200));
        assert_eq!(all, (1..=60).collect::<Vec<u64>>());
    }

    #[test]
    fn a_busy_inbox_gives_up_after_the_wait_but_can_still_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::load_waiting(
            dir.path().join("inbox.json"),
            10,
            Duration::from_millis(100),
        )
        .unwrap();
        inbox.add(&laptop(), text("x"), NOW).unwrap();

        // Another process in the middle of a change.
        let other = OpenOptions::new()
            .write(true)
            .open(dir.path().join("inbox.lock"))
            .unwrap();
        other.try_lock().unwrap();

        let started = Instant::now();
        assert!(matches!(
            inbox.add(&laptop(), text("y"), NOW),
            Err(Error::Timeout(_))
        ));
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(matches!(inbox.remove(1), Err(Error::Timeout(_))));
        assert!(matches!(inbox.clear(), Err(Error::Timeout(_))));
        assert_eq!(numbers(&inbox), [1]);

        drop(other);
        stored(inbox.add(&laptop(), text("y"), NOW).unwrap());
        assert_eq!(numbers(&inbox), [1, 2]);
    }

    #[tokio::test]
    async fn new_items_are_announced_in_order() {
        let (_dir, inbox) = inbox(10);
        let mut heard = inbox.subscribe();
        inbox.add(&laptop(), text("one"), NOW).unwrap();
        inbox
            .add(&laptop(), link("https://example.com/"), NOW)
            .unwrap();
        let first = heard.recv().await.unwrap();
        let second = heard.recv().await.unwrap();
        assert_eq!((first.number, second.number), (1, 2));
        assert_eq!(first.item, text("one"));
        // Removing is not announced, and neither is a refused item.
        inbox.remove(1).unwrap();
        assert!(heard.try_recv().is_err());
    }

    #[test]
    fn debug_output_hides_what_was_received() {
        let (_dir, inbox) = inbox(10);
        let entry = stored(
            inbox
                .add(
                    &laptop(),
                    InboxItem::Link {
                        url: "https://user:hunter2@example.com/?token=abc123".to_owned(),
                        title: Some("Private page".to_owned()),
                    },
                    NOW,
                )
                .unwrap(),
        );
        let secret = stored(
            inbox
                .add(&laptop(), text("correct horse battery"), NOW)
                .unwrap(),
        );
        let shown = format!("{entry:?} {secret:?} {inbox:?}");
        for hidden in ["hunter2", "abc123", "Private page", "correct horse"] {
            assert!(!shown.contains(hidden), "{shown}");
        }
    }

    // --- the handler ---

    fn handler(inbox: &Inbox) -> InboxHandler {
        InboxHandler::new(inbox.clone()).with_clock(ManualClock::new(NOW + 7))
    }

    fn link_item(url: &str, title: Option<&str>) -> Item {
        Item::Link(Link {
            url: url.to_owned(),
            title: title.map(str::to_owned),
        })
    }

    fn text_item(text: &str) -> Item {
        Item::ClipboardText(ClipboardText {
            text: text.to_owned(),
        })
    }

    async fn offer(handler: &InboxHandler, from: &PairedDevice, item: Item) -> Outcome {
        handler.take(from, item).await
    }

    #[tokio::test]
    async fn links_and_text_are_kept_and_accepted() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        let from = laptop();
        assert_eq!(
            offer(
                &handler,
                &from,
                link_item("https://example.com/a?b=c", Some("A page"))
            )
            .await,
            Outcome::Accepted
        );
        assert_eq!(
            offer(&handler, &from, text_item("some text")).await,
            Outcome::Accepted
        );
        let kept = inbox.list().unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(
            kept[0].item,
            InboxItem::Link {
                url: "https://example.com/a?b=c".to_owned(),
                title: Some("A page".to_owned()),
            }
        );
        assert_eq!(kept[1].item, text("some text"));
        assert_eq!(
            (kept[0].from_name.as_str(), kept[0].received_at),
            ("Laptop", NOW + 7)
        );
    }

    #[tokio::test]
    async fn settings_that_open_or_apply_still_keep_the_item_in_the_inbox() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        let eager = paired(
            3,
            "Eager",
            DevicePolicy {
                links: LinkPolicy::Open,
                clipboard: ClipboardPolicy::Apply,
                ..DevicePolicy::default()
            },
        );
        assert_eq!(
            offer(&handler, &eager, link_item("https://example.com/", None)).await,
            Outcome::Accepted
        );
        assert_eq!(
            offer(&handler, &eager, text_item("replace the clipboard")).await,
            Outcome::Accepted
        );
        assert_eq!(numbers(&inbox), [1, 2]);
    }

    #[tokio::test]
    async fn a_kind_set_to_ignore_is_declined_as_blocked_and_not_kept() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        let quiet = paired(
            4,
            "Quiet",
            DevicePolicy {
                links: LinkPolicy::Ignore,
                ..DevicePolicy::default()
            },
        );
        assert_eq!(
            offer(&handler, &quiet, link_item("https://example.com/", None)).await,
            Outcome::Declined(DeclineReason::Blocked)
        );
        // Only links were set to be ignored.
        assert_eq!(
            offer(&handler, &quiet, text_item("hello")).await,
            Outcome::Accepted
        );
        let muted = paired(
            5,
            "Muted",
            DevicePolicy {
                clipboard: ClipboardPolicy::Ignore,
                ..DevicePolicy::default()
            },
        );
        assert_eq!(
            offer(&handler, &muted, text_item("hello")).await,
            Outcome::Declined(DeclineReason::Blocked)
        );
        assert_eq!(numbers(&inbox), [1]);
    }

    #[tokio::test]
    async fn links_that_are_not_well_formed_are_declined() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        let from = laptop();
        for bad in [
            "example.com",
            "not a link",
            "https://example.com/a b",
            "https://example.com/\n",
            "https://exa\tmple.com/",
            "https://",
            "/relative/path",
        ] {
            assert_eq!(
                offer(&handler, &from, link_item(bad, None)).await,
                Outcome::Declined(DeclineReason::Invalid),
                "{bad:?}"
            );
        }
        assert_eq!(
            offer(&handler, &from, text_item("")).await,
            Outcome::Declined(DeclineReason::Invalid)
        );
        assert_eq!(inbox.list().unwrap(), Vec::new());
    }

    #[tokio::test]
    async fn a_link_is_kept_in_its_normal_form() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        let from = laptop();
        offer(
            &handler,
            &from,
            link_item("HTTPS://Example.COM:443/p\u{e4}th?q=1#top", None),
        )
        .await;
        offer(
            &handler,
            &from,
            link_item("mailto:someone@example.com", None),
        )
        .await;
        // Characters that reorder text are percent-encoded like any other non-ASCII character.
        offer(
            &handler,
            &from,
            link_item("https://example.com/\u{202e}x", None),
        )
        .await;
        let kept: Vec<_> = inbox
            .list()
            .unwrap()
            .into_iter()
            .map(|entry| entry.item)
            .collect();
        assert_eq!(
            kept,
            [
                link("https://example.com/p%C3%A4th?q=1#top"),
                link("mailto:someone@example.com"),
                link("https://example.com/%E2%80%AEx"),
            ]
        );
    }

    #[tokio::test]
    async fn titles_are_cleaned_and_empty_ones_dropped() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        let from = laptop();
        let url = "https://example.com/";
        offer(
            &handler,
            &from,
            link_item(url, Some("Pay\u{202e}ment\n\u{1b}[31m page")),
        )
        .await;
        offer(&handler, &from, link_item(url, Some(" \u{200b}\n "))).await;
        let titles: Vec<_> = inbox
            .list()
            .unwrap()
            .into_iter()
            .map(|entry| match entry.item {
                InboxItem::Link { title, .. } => title,
                InboxItem::Text { .. } => unreachable!(),
            })
            .collect();
        assert_eq!(titles, [Some("Payment [31m page".to_owned()), None]);
    }

    #[tokio::test]
    async fn an_item_too_big_for_the_inbox_is_declined_as_too_large() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        let heavy = "\0".repeat(1024 * 1024);
        assert_eq!(
            offer(&handler, &laptop(), text_item(&heavy)).await,
            Outcome::Declined(DeclineReason::TooLarge)
        );
        let long = format!("https://example.com/{}", "%7F".repeat(MAX_URL_BYTES / 2));
        assert_eq!(
            offer(&handler, &laptop(), link_item(&long, None)).await,
            Outcome::Declined(DeclineReason::TooLarge)
        );
    }

    #[tokio::test]
    async fn an_inbox_that_cannot_be_used_declines_instead_of_failing() {
        let (dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        // The file is not an inbox any more.
        write_file(&dir, "not json");
        assert_eq!(
            offer(&handler, &laptop(), text_item("hello")).await,
            Outcome::Declined(DeclineReason::Other)
        );
    }

    #[test]
    fn the_handler_accepts_both_kinds_of_item() {
        let (_dir, inbox) = inbox(10);
        let handler = handler(&inbox);
        assert_eq!(
            handler.capabilities(),
            [capability::LINK, capability::CLIPBOARD_TEXT]
        );
    }
}
