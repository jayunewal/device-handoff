// SPDX-License-Identifier: Apache-2.0

//! The transfers that another device may still hold for this one.
//!
//! A receiver keeps a transfer that it accepted until the sender finishes it or gives it up, or until
//! no attempt has been going for an hour (section 8, After a stop), and it counts the transfers that a
//! device has going: a device that has
//! [`GOING_PER_PEER`](crate::transfer::incoming::GOING_PER_PEER) of them is answered `RateLimited`
//! until the receiver lets one go. A sender that is closed or killed while it sends, or that gives a
//! transfer up when it cannot reach the receiver, leaves the receiver holding what it was sent. A
//! sender that starts again has forgotten the transfer, and could not give it up if it wanted to.
//!
//! [`Holds`] is what it remembers instead. A transfer is written down before its offer is made, and
//! struck out when the receiver is known to hold nothing of it: it finished, it said no, it ended the
//! transfer itself, or it was told that the transfer is given up. What is left is what some receiver
//! may be holding, and [`release`](super::release) tells that receiver to let go as soon as it can be
//! reached. A receiver that has let go already answers the same as one that has just done so, so a
//! word that was not needed does no harm.
//!
//! # The file
//!
//! `holds.json` in the [state directory](crate::DataDir), readable by its owner only. It names the
//! device and the id of each transfer, and nothing of what was sent. It is read leniently: a file that
//! cannot be read, or an entry in it that makes no sense, is left out. A device that is reminded of
//! one transfer less loses an hour at worst, and one that cannot start loses everything. A change that
//! cannot be written is logged and kept in memory, so that what the program has learned still reaches
//! the receivers while it runs.
//!
//! The methods that read or write the file block the calling thread, as those of the
//! [list of paired devices](crate::TrustStore) do. [`with`](Holds::with) runs them on a thread that is
//! meant for blocking, which is what the engine and the release do.
//!
//! # For how long
//!
//! A receiver lets go of a transfer by itself an hour after its last attempt ended, and it ends an
//! attempt that moves no byte after thirty seconds. Each entry carries the time of the last word that
//! this device had with the receiver about the transfer, as far as it knows: the time the transfer
//! began, while it is going, and the time it was left once it is not. An entry older than [`HELD_FOR`]
//! is dropped, because no receiver can hold it any more. An entry that a run of the program wrote and
//! did not live to end is taken as left when the next run starts, because nothing says when it was.

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use baton_proto::{DeviceId, id::TransferId};
use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::outcome::{self, Outcome};
use crate::{
    clock::{Clock, SystemClock},
    error::{Error, Result},
    fsutil::{Access, read_text, write_atomic},
    transfer::incoming::KEEP_IDLE,
    util::{blocking, lock},
};

/// Newest file format this build reads and writes.
const FORMAT_VERSION: u32 = 1;

/// How long a receiver may go on holding a transfer after this device last had a word with it: the
/// hour that it keeps a transfer that has no attempt going, the half minute that it takes to end an
/// attempt that moves no byte, and some slack.
pub(crate) const HELD_FOR: Duration = KEEP_IDLE.saturating_add(Duration::from_mins(5));

/// A transfer that a receiver may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hold {
    peer: DeviceId,
    id: TransferId,
    /// When this device last had a word with the receiver about the transfer, as far as it knows, in
    /// seconds since the Unix epoch.
    since: u64,
    /// Whether a transfer of this run of the program is going, which is not to be given up.
    going: bool,
}

struct Inner {
    /// Where the holds are kept, if they are kept anywhere.
    path: Option<PathBuf>,
    holds: Mutex<Vec<Hold>>,
    /// Counts the transfers that were left, so that whoever gives them back sees that there is
    /// another and does not wait for the next look.
    left: watch::Sender<u64>,
}

/// What other devices may be holding for this one: the transfers that were offered to them, and that
/// they have not been told are over. Clones share one list. See the module documentation.
#[derive(Clone)]
pub struct Holds(Arc<Inner>);

impl fmt::Debug for Holds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Holds").finish_non_exhaustive()
    }
}

impl Holds {
    /// A list that lives in memory and goes with the program. A transfer that is left is given up
    /// while the program runs, and forgotten when it ends.
    pub fn memory() -> Self {
        Self::new(None, Vec::new())
    }

    /// A list that is kept in the file at `path`, and starts with what is in it.
    ///
    /// This never fails. A file that cannot be read, or a part of it that makes no sense, is logged
    /// and left out, and a change that cannot be written is logged and kept in memory. The file
    /// belongs in the [state directory](crate::DataDir::holds_file).
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self::open_at(path.into(), SystemClock.now_unix())
    }

    fn open_at(path: PathBuf, now: u64) -> Self {
        let (holds, changed) = match read(&path, now) {
            Ok(read) => read,
            Err(error) => {
                tracing::warn!(%error, "the transfers that other devices may hold could not be read");
                (Vec::new(), false)
            }
        };
        let this = Self::new(Some(path), holds);
        if changed {
            this.save(&lock(&this.0.holds));
        }
        this
    }

    fn new(path: Option<PathBuf>, holds: Vec<Hold>) -> Self {
        Self(Arc::new(Inner {
            path,
            holds: Mutex::new(holds),
            left: watch::Sender::new(0),
        }))
    }

    /// Runs `work` on a thread that may block, since the file is on a disk. `None` if the program is
    /// stopping and `work` did not run.
    pub(crate) async fn with<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Self) -> T + Send + 'static,
    ) -> Option<T> {
        let holds = self.clone();
        blocking(move || work(&holds)).await.ok()
    }

    /// Writes down that `peer` may hold the transfer `id` from now on.
    pub(crate) fn begin(&self, peer: DeviceId, id: TransferId, now: u64) {
        let mut holds = lock(&self.0.holds);
        holds.push(Hold {
            peer,
            id,
            since: now,
            going: true,
        });
        self.save(&holds);
    }

    /// Writes down that the transfer `id` to `peer` is not going any more. `clear` says that the
    /// receiver holds nothing of it, and the transfer is struck out. Otherwise it is left for
    /// [`loose`](Self::loose) to give back, from now on.
    pub(crate) fn ended(&self, peer: &DeviceId, id: TransferId, clear: bool, now: u64) {
        let mut holds = lock(&self.0.holds);
        let Some(at) = holds
            .iter()
            .position(|hold| hold.peer == *peer && hold.id == id)
        else {
            return;
        };
        if clear {
            holds.remove(at);
        } else {
            holds[at].going = false;
            holds[at].since = now;
        }
        self.save(&holds);
        drop(holds);
        if !clear {
            self.0.left.send_modify(|count| *count += 1);
        }
    }

    /// The transfers to give back, by device: every one that is not going, except the ones that no
    /// receiver can hold any more (it is [`HELD_FOR`] since the last word) and those of a device that
    /// is not paired, which cannot be told anything. Those are struck out.
    pub(crate) fn loose(
        &self,
        paired: &dyn Fn(&DeviceId) -> bool,
        now: u64,
    ) -> Vec<(DeviceId, Vec<TransferId>)> {
        let mut holds = lock(&self.0.holds);
        let before = holds.len();
        holds.retain_mut(|hold| {
            if hold.going {
                return true;
            }
            // A clock that was set back: the last word cannot have been after now.
            hold.since = hold.since.min(now);
            paired(&hold.peer) && now - hold.since <= HELD_FOR.as_secs()
        });
        if holds.len() != before {
            self.save(&holds);
        }
        let mut by_peer: Vec<(DeviceId, Vec<TransferId>)> = Vec::new();
        for hold in holds.iter().filter(|hold| !hold.going) {
            match by_peer.iter_mut().find(|(peer, _)| *peer == hold.peer) {
                Some((_, ids)) => ids.push(hold.id),
                None => by_peer.push((hold.peer, vec![hold.id])),
            }
        }
        by_peer
    }

    /// Strikes out the transfer `id` of `peer`, which has let go of it.
    pub(crate) fn released(&self, peer: &DeviceId, id: TransferId) {
        self.strike(|hold| hold.peer == *peer && hold.id == id);
    }

    /// Strikes out every transfer of `peer` that is not going: nothing can be said to it.
    pub(crate) fn forget(&self, peer: &DeviceId) {
        self.strike(|hold| hold.peer == *peer);
    }

    /// Changes each time a transfer is left. Whoever gives transfers back marks the value as seen
    /// before it looks at the list, and waits for it to change when there is nothing to do, so that
    /// a transfer that is left in between is not missed.
    pub(crate) fn watch_left(&self) -> watch::Receiver<u64> {
        self.0.left.subscribe()
    }

    /// Strikes out the transfers that are not going and that `doomed` picks.
    fn strike(&self, doomed: impl Fn(&Hold) -> bool) {
        let mut holds = lock(&self.0.holds);
        let before = holds.len();
        holds.retain(|hold| hold.going || !doomed(hold));
        if holds.len() != before {
            self.save(&holds);
        }
    }

    /// Writes the list, if it is kept in a file. A list that cannot be written stays in memory.
    fn save(&self, holds: &[Hold]) {
        let Some(path) = &self.0.path else {
            return;
        };
        let stored: Vec<StoredHold> = holds.iter().map(StoredHold::from).collect();
        let written = serde_json::to_vec_pretty(&StoredRef {
            version: FORMAT_VERSION,
            holds: &stored,
        })
        .map_err(|e| Error::invalid(path, &e))
        .and_then(|bytes| write_atomic(path, &bytes));
        if let Err(error) = written {
            tracing::warn!(%error, "the transfers that other devices may hold could not be written down");
        }
    }

    /// What is in the list, for the tests: the device, the id and whether the transfer is going.
    #[cfg(test)]
    pub(crate) fn all(&self) -> Vec<(DeviceId, TransferId, bool)> {
        lock(&self.0.holds)
            .iter()
            .map(|hold| (hold.peer, hold.id, hold.going))
            .collect()
    }

    /// How many handles share this list, for the tests that show a task has let go of it.
    #[cfg(test)]
    pub(crate) fn sharers(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

/// What the engine tells the list about one transfer.
#[derive(Clone)]
pub(crate) struct Ledger {
    holds: Holds,
    peer: DeviceId,
    clock: Arc<dyn Clock>,
}

impl Ledger {
    pub(crate) fn new(holds: Holds, peer: DeviceId, clock: Arc<dyn Clock>) -> Self {
        Self { holds, peer, clock }
    }

    /// Writes down that the receiver may hold the transfer `id` from now on. The offer waits for it,
    /// so a transfer that this device does not live to end is remembered.
    pub(crate) async fn begin(&self, id: TransferId) {
        let (peer, now) = (self.peer, self.clock.now_unix());
        self.holds
            .with(move |holds| holds.begin(peer, id, now))
            .await;
    }

    /// Writes down how the transfer `id` ended. `offered` says whether an offer was ever made: a
    /// receiver that was not offered the transfer holds nothing of it, however it ended.
    pub(crate) async fn ended(&self, id: TransferId, outcome: &Outcome, offered: bool) {
        let (peer, now) = (self.peer, self.clock.now_unix());
        let clear = !offered || receiver_is_clear(outcome);
        self.holds
            .with(move |holds| holds.ended(&peer, id, clear, now))
            .await;
    }
}

/// Whether a receiver holds nothing of a transfer that ended like this: it finished, it said no, it
/// ended the transfer itself, it was told that the transfer is given up, or it turned this device
/// away for good, and nothing more can be said to it. Any other end leaves a receiver that may be
/// holding what it was sent.
pub(crate) fn receiver_is_clear(outcome: &Outcome) -> bool {
    match outcome {
        Outcome::Done(_)
        | Outcome::Declined(_)
        | Outcome::Stopped(_)
        | Outcome::Cancelled { told: true }
        | Outcome::Failed(outcome::Error::Refused(_)) => true,
        Outcome::Cancelled { told: false } | Outcome::Failed(_) => false,
    }
}

// The file.

/// A hold as it is written. The ids are hexadecimal text, so that the file can be read.
#[derive(Serialize, Deserialize)]
struct StoredHold {
    peer: String,
    id: String,
    since: u64,
    going: bool,
}

impl From<&Hold> for StoredHold {
    fn from(hold: &Hold) -> Self {
        Self {
            peer: hold.peer.to_string(),
            id: hold.id.to_string(),
            since: hold.since,
            going: hold.going,
        }
    }
}

#[derive(Serialize)]
struct StoredRef<'a> {
    version: u32,
    holds: &'a [StoredHold],
}

/// The file as it is read: each entry is judged on its own.
#[derive(Deserialize)]
struct Stored {
    version: u32,
    holds: Vec<serde_json::Value>,
}

/// The holds in the file at `path`, and whether the file says something that the list does not: an
/// entry that was left out, or one that a run of the program wrote and did not live to end.
fn read(path: &Path, now: u64) -> Result<(Vec<Hold>, bool)> {
    let Some(text) = read_text(path, Access::OwnerOnly)? else {
        return Ok((Vec::new(), false));
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
    let holds = stored
        .holds
        .into_iter()
        .filter_map(|entry| {
            let Some(mut hold) = hold_of(entry) else {
                changed = true;
                return None;
            };
            if hold.going {
                hold.going = false;
                hold.since = now;
                changed = true;
            }
            Some(hold)
        })
        .collect();
    Ok((holds, changed))
}

/// The hold that an entry of the file says, if it says one.
fn hold_of(entry: serde_json::Value) -> Option<Hold> {
    let stored: StoredHold = serde_json::from_value(entry).ok()?;
    let id: [u8; TransferId::LEN] = HEXLOWER
        .decode(stored.id.as_bytes())
        .ok()?
        .try_into()
        .ok()?;
    Some(Hold {
        peer: stored.peer.parse().ok()?,
        id: TransferId::from_bytes(id),
        since: stored.since,
        going: stored.going,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use baton_proto::{
        message::DeclineReason,
        transfer::{Done, lane::SkipReason},
    };
    use tempfile::TempDir;

    use super::*;
    use crate::{clock::ManualClock, transfer::send::outcome::Sent};

    const NOW: u64 = 1_000_000;

    fn peer(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; DeviceId::LEN])
    }

    fn id(n: u8) -> TransferId {
        TransferId::from_bytes([n; TransferId::LEN])
    }

    fn everyone(_: &DeviceId) -> bool {
        true
    }

    /// A folder, and the file in it that a list is kept in.
    fn kept() -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("holds.json");
        (dir, path)
    }

    /// Puts `contents` in the file as the program's own files are, readable by its owner only, so
    /// that what is wrong with them is what the test is about.
    fn put(path: &Path, contents: impl AsRef<[u8]>) {
        fs::write(path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn done() -> Outcome {
        Outcome::Done(Sent {
            done: Done {
                id: id(1),
                failed: Vec::new(),
                more_failed: 0,
            },
            files: 1,
            bytes: 5,
            skipped: Vec::<(u32, SkipReason)>::new(),
        })
    }

    #[test]
    fn a_transfer_is_held_from_its_beginning_and_struck_out_when_the_receiver_is_clear() {
        let holds = Holds::memory();

        holds.begin(peer(1), id(1), NOW);
        assert_eq!(holds.all(), [(peer(1), id(1), true)]);
        holds.ended(&peer(1), id(1), true, NOW + 5);

        assert_eq!(holds.all(), []);
        assert_eq!(holds.loose(&everyone, NOW + 5), []);
    }

    #[test]
    fn a_transfer_that_is_going_is_not_given_back() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), NOW);

        // However long it takes, and however long ago it began: it is going.
        assert_eq!(holds.loose(&everyone, NOW + 10 * HELD_FOR.as_secs()), []);
        assert_eq!(holds.all(), [(peer(1), id(1), true)]);
    }

    #[test]
    fn a_transfer_that_was_left_is_given_back_with_the_others_of_its_device() {
        let holds = Holds::memory();
        for (device, transfer) in [(1, 1), (2, 2), (1, 3), (1, 4)] {
            holds.begin(peer(device), id(transfer), NOW);
        }

        holds.ended(&peer(1), id(1), false, NOW + 1);
        holds.ended(&peer(2), id(2), false, NOW + 2);
        // The fourth is still going, and the third ended in a way that clears the receiver.
        holds.ended(&peer(1), id(3), true, NOW + 3);

        assert_eq!(
            holds.loose(&everyone, NOW + 4),
            [(peer(1), vec![id(1)]), (peer(2), vec![id(2)])]
        );
    }

    #[test]
    fn what_was_not_ended_is_a_transfer_that_was_left_when_the_next_run_starts() {
        let (_dir, path) = kept();
        let before = Holds::open_at(path.clone(), NOW);
        before.begin(peer(1), id(1), NOW);
        before.begin(peer(1), id(2), NOW + 10);
        before.ended(&peer(1), id(2), false, NOW + 20);
        before.begin(peer(2), id(3), NOW + 30);
        before.ended(&peer(2), id(3), true, NOW + 40);

        // The program is closed, or killed, and opened again an hour later.
        let later = NOW + 3600;
        let after = Holds::open_at(path.clone(), later);

        assert_eq!(
            after.all(),
            [(peer(1), id(1), false), (peer(1), id(2), false)]
        );
        // The first was not ended, so the last word with its receiver may have been any time up to
        // the start of this run, and an hour is counted from there. The second was left at NOW + 20.
        let just_in_time = later + HELD_FOR.as_secs();
        assert_eq!(
            after.loose(&everyone, just_in_time),
            [(peer(1), vec![id(1)])]
        );
        // What was struck out stays out, and what was dropped is not in the file either.
        let again = Holds::open_at(path, just_in_time);
        assert_eq!(again.all(), [(peer(1), id(1), false)]);
    }

    #[test]
    fn what_was_found_going_is_written_down_as_left_when_the_file_is_opened() {
        let (_dir, path) = kept();
        let before = Holds::open_at(path.clone(), NOW);
        before.begin(peer(1), id(1), NOW);
        drop(before);

        // Opened again, nothing is done with the list, and the program is closed.
        drop(Holds::open_at(path.clone(), NOW + 100));

        // The file says that it was left when that run started, so that the run after it does not
        // count from its own start, and keep the transfer for longer than a receiver does.
        let file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file["holds"][0]["going"], false);
        assert_eq!(file["holds"][0]["since"], NOW + 100);
    }

    #[test]
    fn a_hold_that_no_receiver_can_have_any_more_is_dropped() {
        let (_dir, path) = kept();
        let holds = Holds::open_at(path.clone(), NOW);
        holds.begin(peer(1), id(1), NOW);
        holds.ended(&peer(1), id(1), false, NOW);

        // On the last second that a receiver can have it, it is still given back.
        let last = NOW + HELD_FOR.as_secs();
        assert_eq!(holds.loose(&everyone, last), [(peer(1), vec![id(1)])]);
        // A second later it is not, and the file says so.
        assert_eq!(holds.loose(&everyone, last + 1), []);
        assert_eq!(Holds::open_at(path, last + 1).all(), []);
    }

    #[test]
    fn a_transfer_that_was_left_after_a_long_time_is_kept_from_when_it_was_left() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), NOW);

        // It went on for longer than a receiver keeps a transfer that has no attempt, and was left
        // then. The receiver's hour counts from there, and not from the beginning.
        let left = NOW + 3 * HELD_FOR.as_secs();
        holds.ended(&peer(1), id(1), false, left);

        let last = left + HELD_FOR.as_secs();
        assert_eq!(holds.loose(&everyone, last), [(peer(1), vec![id(1)])]);
        assert_eq!(holds.loose(&everyone, last + 1), []);
    }

    #[test]
    fn a_clock_that_was_set_back_does_not_make_a_hold_last_for_ever() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), 5_000_000);
        holds.ended(&peer(1), id(1), false, 5_000_000);

        // The clock now shows a time long before the last word. The word was no later than now, so
        // the hour counts from now.
        assert_eq!(holds.loose(&everyone, NOW), [(peer(1), vec![id(1)])]);
        assert_eq!(
            holds.loose(&everyone, NOW + HELD_FOR.as_secs()),
            [(peer(1), vec![id(1)])]
        );
        assert_eq!(holds.loose(&everyone, NOW + HELD_FOR.as_secs() + 1), []);
    }

    #[test]
    fn the_transfers_of_a_device_that_is_not_paired_are_dropped() {
        let (_dir, path) = kept();
        let holds = Holds::open_at(path.clone(), NOW);
        for device in [1, 2] {
            holds.begin(peer(device), id(device), NOW);
            holds.ended(&peer(device), id(device), false, NOW);
        }

        let paired = |device: &DeviceId| *device == peer(2);

        assert_eq!(holds.loose(&paired, NOW), [(peer(2), vec![id(2)])]);
        assert_eq!(Holds::open_at(path, NOW).all(), [(peer(2), id(2), false)]);
    }

    #[test]
    fn a_receiver_that_let_go_is_struck_out_and_so_is_a_device_that_cannot_be_told() {
        let holds = Holds::memory();
        for (device, transfer) in [(1, 1), (1, 2), (2, 3), (2, 4)] {
            holds.begin(peer(device), id(transfer), NOW);
            holds.ended(&peer(device), id(transfer), false, NOW);
        }
        // A transfer that is going is nobody's to strike out but the one that is making it.
        holds.begin(peer(2), id(5), NOW);

        holds.released(&peer(1), id(1));
        holds.forget(&peer(2));
        holds.released(&peer(2), id(5));

        assert_eq!(
            holds.all(),
            [(peer(1), id(2), false), (peer(2), id(5), true)]
        );
    }

    #[test]
    fn a_transfer_is_known_by_its_device_as_well_as_by_its_id() {
        // Nothing makes the ids of two devices differ, and one is not the other's.
        let holds = Holds::memory();
        for device in [1, 2] {
            holds.begin(peer(device), id(1), NOW);
            holds.ended(&peer(device), id(1), false, NOW);
        }

        holds.released(&peer(1), id(1));

        assert_eq!(holds.all(), [(peer(2), id(1), false)]);
    }

    #[test]
    fn a_transfer_that_is_ended_twice_or_was_never_begun_changes_nothing() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), NOW);
        holds.ended(&peer(1), id(1), true, NOW);

        holds.ended(&peer(1), id(1), false, NOW);
        holds.ended(&peer(2), id(2), false, NOW);

        assert_eq!(holds.all(), []);
    }

    #[test]
    fn a_file_that_cannot_be_used_is_no_reason_not_to_start_and_is_replaced_by_the_next_change() {
        // A file of a version that this one does not know may say anything, and is not read.
        let other_version = serde_json::json!({
            "version": 7,
            "holds": [{
                "peer": peer(1).to_string(),
                "id": id(1).to_string(),
                "since": NOW,
                "going": false,
            }],
        })
        .to_string();
        let broken: [&[u8]; 4] = [
            b"not json at all",
            other_version.as_bytes(),
            b"{\"version\": 1}",
            b"\xff\xfe not text",
        ];
        for contents in broken {
            let (_dir, path) = kept();
            put(&path, contents);

            let holds = Holds::open_at(path.clone(), NOW);

            assert_eq!(holds.all(), [], "{}", String::from_utf8_lossy(contents));
            holds.begin(peer(1), id(1), NOW);
            let reopened = Holds::open_at(path, NOW);
            assert_eq!(reopened.all(), [(peer(1), id(1), false)]);
        }
    }

    #[test]
    fn a_directory_where_the_file_belongs_is_no_reason_not_to_start() {
        let (_dir, path) = kept();
        fs::create_dir(&path).unwrap();

        let holds = Holds::open_at(path, NOW);
        holds.begin(peer(1), id(1), NOW);
        holds.ended(&peer(1), id(1), false, NOW);

        // The change could not be written, and the program goes on with what it has.
        assert_eq!(holds.loose(&everyone, NOW), [(peer(1), vec![id(1)])]);
    }

    #[test]
    fn an_entry_that_makes_no_sense_is_left_out_and_the_others_are_kept() {
        let good = |device: u8, transfer: u8| {
            serde_json::json!({
                "peer": peer(device).to_string(),
                "id": id(transfer).to_string(),
                "since": NOW,
                "going": false,
            })
        };
        let file = serde_json::json!({
            "version": 1,
            "holds": [
                good(1, 1),
                { "peer": "not a device", "id": id(2).to_string(), "since": NOW, "going": false },
                { "peer": peer(1).to_string(), "id": "abc", "since": NOW, "going": false },
                { "peer": peer(1).to_string(), "id": "z".repeat(32), "since": NOW, "going": false },
                { "peer": peer(1).to_string(), "id": id(3).to_string(), "going": false },
                { "peer": peer(1).to_string(), "id": id(3).to_string(), "since": "yesterday", "going": false },
                42,
                good(2, 4),
            ],
        });
        let (_dir, path) = kept();
        put(&path, file.to_string());

        let holds = Holds::open_at(path.clone(), NOW);

        assert_eq!(
            holds.all(),
            [(peer(1), id(1), false), (peer(2), id(4), false)]
        );
        // The entries that made no sense are gone from the file too, though nothing was changed
        // after it was read.
        let file: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file["holds"].as_array().unwrap().len(), 2);
        assert_eq!(Holds::open_at(path, NOW).all(), holds.all());
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_for_its_owner_only_and_one_that_others_can_use_is_not_read() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let (_dir, path) = kept();
        let holds = Holds::open_at(path.clone(), NOW);
        holds.begin(peer(1), id(1), NOW);
        assert_eq!(mode(&path), 0o600);

        // Somebody else made it readable, so it may not be what the program wrote.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let reopened = Holds::open_at(path.clone(), NOW);

        assert_eq!(reopened.all(), []);
        reopened.begin(peer(2), id(2), NOW);
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn the_file_names_the_device_and_the_transfer_and_nothing_of_what_was_sent() {
        let (_dir, path) = kept();
        let holds = Holds::open_at(path.clone(), NOW);
        holds.begin(peer(0xab), id(0xcd), NOW);

        let text = fs::read_to_string(&path).unwrap();

        assert!(text.contains(&"ab".repeat(32)), "{text}");
        assert!(text.contains(&"cd".repeat(16)), "{text}");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], 1);
        let entry = &value["holds"][0];
        let mut keys: Vec<&str> = entry
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["going", "id", "peer", "since"]);
    }

    #[test]
    fn a_transfer_that_is_going_is_kept_whatever_the_clock_and_the_pairing_say() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), NOW);

        // Far past what any receiver holds, and for a device that is not paired.
        let later = NOW + HELD_FOR.as_secs() + 1000;
        assert_eq!(holds.loose(&|_| false, later), []);

        assert_eq!(holds.all(), [(peer(1), id(1), true)]);
    }

    #[test]
    fn a_hold_outlasts_what_a_receiver_keeps_a_transfer_for() {
        // The receiver lets go by itself an hour after the last attempt ended, and ends an attempt
        // that moves no byte after half a minute: a hold that is dropped sooner could be one that a
        // receiver still has, and would never be given back.
        let (held, kept) = (HELD_FOR, KEEP_IDLE);
        assert!(held >= kept + Duration::from_secs(30), "{held:?} {kept:?}");
    }

    #[test]
    fn each_way_a_transfer_can_end_says_whether_the_receiver_is_clear() {
        let cases = [
            (done(), true),
            (Outcome::Declined(DeclineReason::RateLimited), true),
            (Outcome::Stopped(499), true),
            (Outcome::Stopped(507), true),
            (Outcome::Cancelled { told: true }, true),
            (Outcome::Cancelled { told: false }, false),
            (Outcome::Failed(outcome::Error::Refused(403)), true),
            (
                Outcome::Failed(outcome::Error::GaveUp(outcome::Why::Connection)),
                false,
            ),
            (
                Outcome::Failed(outcome::Error::GaveUp(outcome::Why::Forgotten)),
                false,
            ),
            (Outcome::Failed(outcome::Error::NoOutcome), false),
            (
                Outcome::Failed(outcome::Error::Protocol("for a reason")),
                false,
            ),
            (
                Outcome::Failed(outcome::Error::Internal("for a reason")),
                false,
            ),
            (Outcome::Failed(outcome::Error::Stopped), false),
        ];
        for (outcome, clear) in cases {
            assert_eq!(receiver_is_clear(&outcome), clear, "{outcome:?}");
        }
    }

    #[tokio::test]
    async fn the_engine_writes_down_a_transfer_before_its_offer_and_how_it_ended() {
        let (_dir, path) = kept();
        let holds = Holds::open_at(path.clone(), NOW);
        let clock = Arc::new(ManualClock::new(NOW));
        let ledger = Ledger::new(holds.clone(), peer(1), clock.clone());

        ledger.begin(id(1)).await;
        // It is on disk by the time `begin` returns: a program that is killed now still has it.
        assert_eq!(
            Holds::open_at(path.clone(), NOW).all(),
            [(peer(1), id(1), false)]
        );
        assert_eq!(holds.all(), [(peer(1), id(1), true)]);

        clock.advance(30);
        ledger
            .ended(id(1), &Outcome::Cancelled { told: false }, true)
            .await;
        assert_eq!(holds.all(), [(peer(1), id(1), false)]);
        assert_eq!(holds.loose(&everyone, NOW + 30), [(peer(1), vec![id(1)])]);

        ledger.begin(id(2)).await;
        ledger.ended(id(2), &done(), true).await;
        assert_eq!(holds.all(), [(peer(1), id(1), false)]);

        // A transfer that never got as far as an offer is nothing that a receiver can hold, and it
        // is struck out however it ended. One that was offered and that ended the same way is not.
        let gave_up = Outcome::Failed(outcome::Error::GaveUp(outcome::Why::Connection));
        ledger.begin(id(3)).await;
        ledger.ended(id(3), &gave_up, false).await;
        assert_eq!(holds.all(), [(peer(1), id(1), false)]);
        ledger.begin(id(4)).await;
        ledger.ended(id(4), &gave_up, true).await;
        assert_eq!(
            holds.all(),
            [(peer(1), id(1), false), (peer(1), id(4), false)]
        );
    }

    #[tokio::test]
    async fn a_transfer_that_is_left_wakes_whoever_gives_transfers_back() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), NOW);
        holds.begin(peer(1), id(2), NOW);
        let mut left = holds.watch_left();

        // A transfer that ends clear is nothing to give back.
        holds.ended(&peer(1), id(1), true, NOW);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), left.changed())
                .await
                .is_err()
        );

        holds.ended(&peer(1), id(2), false, NOW);
        tokio::time::timeout(Duration::from_secs(10), left.changed())
            .await
            .expect("nothing woke the release")
            .unwrap();
    }

    #[tokio::test]
    async fn a_transfer_that_is_left_after_the_last_look_is_not_missed() {
        let holds = Holds::memory();
        holds.begin(peer(1), id(1), NOW);
        let mut left = holds.watch_left();
        drop(left.borrow_and_update());
        assert_eq!(holds.loose(&everyone, NOW), []);

        // Left after the look, and before the wait.
        holds.ended(&peer(1), id(1), false, NOW);

        tokio::time::timeout(Duration::from_secs(10), left.changed())
            .await
            .expect("the change was lost")
            .unwrap();
    }
}
