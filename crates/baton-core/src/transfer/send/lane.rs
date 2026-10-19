// SPDX-License-Identifier: Apache-2.0

//! Writing one lane.
//!
//! A lane is a unidirectional stream that the sender opens and writes (section 8, Lanes). This is
//! the task that does it, one for each lane of an attempt:
//!
//! - **It opens the lane and writes what the rules say, in the order they say.** Lane 0 starts
//!   with the list, and says when the list is written, because no lane may write a span, an `End`
//!   or a `Skip` of the transfer before that (the attempt opens the others only then). After that
//!   every lane does the same: it takes the next chunk of frames from the [`Producer`], and writes
//!   it.
//! - **It reads while it writes.** A chunk is read from the files and hashed on a thread meant for
//!   blocking, and the lane has the next one ready by the time it has written the last, so that the
//!   disk and the network each wait for the other as little as they can.
//! - **It does not wait for ever.** A write that takes no byte for the stall time ends the lane as
//!   stalled, and so does a lane that cannot be opened and a read that does not come back. The
//!   receiver ends an attempt that moves no byte for as long, and a sender that is blocked in a read
//!   would never see it.
//! - **It never leaves a lane half written and finished.** A lane that is dropped before it was ended
//!   is reset, with the code that the attempt sets, whatever the reason it is dropped for: a task
//!   that is aborted stops at an `await`, and a stream that is dropped would otherwise be finished
//!   and look complete to the receiver.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use baton_proto::{
    close_code,
    frame::{self, FrameError},
    id::TransferId,
    limits::MAX_LIST_FRAME_ENTRIES,
    transfer::{
        lane::TAG_LIST,
        list::{ListChunk, ListEntry},
    },
};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

use super::{
    produce::{Chunk, Producer},
    progress::Tracker,
    wire::{LaneError, LaneOut, Link},
};

/// The list of a transfer as lane 0 carries it: a `List` frame for each piece of at most
/// [`MAX_LIST_FRAME_ENTRIES`] entries, in the form they have on the wire.
///
/// It is made once for a transfer and written again by each attempt, so that a list that cannot be
/// made into frames is known before anything is offered.
#[derive(Debug)]
pub(crate) struct ListFrames(Vec<Vec<u8>>);

impl ListFrames {
    /// The frames of `entries`, which are the whole list in the order of the format.
    pub(crate) fn new(entries: &[ListEntry]) -> Result<Self, FrameError> {
        entries
            .chunks(MAX_LIST_FRAME_ENTRIES)
            .map(|piece| {
                let chunk = ListChunk {
                    entries: piece.to_vec(),
                };
                let mut out = vec![TAG_LIST];
                out.extend_from_slice(&frame::encode(&chunk)?);
                Ok(out)
            })
            .collect::<Result<_, _>>()
            .map(Self)
    }

    fn frames(&self) -> &[Vec<u8>] {
        &self.0
    }
}

#[cfg(test)]
impl ListFrames {
    /// How many bytes the frames come to.
    pub(super) fn len(&self) -> usize {
        self.0.iter().map(Vec::len).sum()
    }

    /// The frames one after the other, as lane 0 writes them.
    pub(super) fn to_bytes(&self) -> Vec<u8> {
        self.0.concat()
    }
}

/// Why a lane did not end the way a lane should.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Halt {
    /// The receiver stopped the lane with this code.
    Stopped(u32),
    /// The stream or the connection failed, or the lane could not be opened.
    Lost,
    /// The lane could not be opened, took no byte, or waited for a read, for as long as the stall
    /// time.
    Stalled,
}

/// The code that open lanes are reset with when they are dropped before they were ended. The
/// attempt sets it to what it is ending for, before it drops them.
#[derive(Debug, Clone)]
pub(crate) struct ResetCode(Arc<AtomicU32>);

impl ResetCode {
    /// A reset code that starts out as 408, the timeout that ends an attempt (section 8).
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU32::new(close_code::TIMEOUT)))
    }

    pub(crate) fn set(&self, code: u32) {
        self.0.store(code, Ordering::Relaxed);
    }

    fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

/// What a lane needs to know, besides the connection it is opened on.
pub(crate) struct Plan {
    /// The number of the lane, from 0.
    pub(crate) number: u8,
    /// How long it may take no byte.
    pub(crate) stall: Duration,
    /// What it is reset with, if it is given up.
    pub(crate) reset: ResetCode,
    /// Where the frames of the files come from, for all the lanes of the attempt.
    pub(crate) producer: Arc<Producer>,
    /// Where the count of what was sent goes.
    pub(crate) tracker: Arc<Tracker>,
}

/// The list that lane 0 writes first, and the signal that it is written.
pub(crate) struct Listing {
    pub(crate) frames: Arc<ListFrames>,
    pub(crate) written: oneshot::Sender<()>,
}

/// A lane that is open. See the module documentation for what dropping it does.
struct Open {
    out: Box<dyn LaneOut>,
    reset: ResetCode,
    ended: bool,
}

impl Drop for Open {
    fn drop(&mut self) {
        if !self.ended {
            self.out.reset(self.reset.get());
        }
    }
}

/// Opens lane `plan.number` of the transfer `id` on `link`, writes it, and ends it. `listing` is
/// given to lane 0 only.
pub(crate) async fn run(
    link: Arc<dyn Link>,
    id: TransferId,
    plan: Plan,
    listing: Option<Listing>,
) -> Result<(), Halt> {
    let out = match timeout(plan.stall, link.open_lane(id, plan.number)).await {
        Ok(Ok(out)) => out,
        Ok(Err(_)) => return Err(Halt::Lost),
        Err(_) => return Err(Halt::Stalled),
    };
    let mut open = Open {
        out,
        reset: plan.reset.clone(),
        ended: false,
    };
    write(&mut open, &plan, listing).await?;
    end(&mut open)
}

/// Ends the lane. A stop that comes before the end is a stop.
fn end(open: &mut Open) -> Result<(), Halt> {
    match open.out.finish() {
        Ok(()) => {
            open.ended = true;
            Ok(())
        }
        Err(LaneError::Stopped(code)) => Err(Halt::Stopped(code)),
        Err(LaneError::Lost) => Err(Halt::Lost),
    }
}

async fn write(open: &mut Open, plan: &Plan, listing: Option<Listing>) -> Result<(), Halt> {
    if let Some(listing) = listing {
        for frame in listing.frames.frames() {
            write_all(open, frame, plan.stall).await?;
        }
        // The attempt opens the other lanes now. If it has gone, nothing else matters.
        let _ = listing.written.send(());
    }
    pump(open, plan).await
}

/// Writes every chunk the producer has, reading the next while the last is written.
async fn pump(open: &mut Open, plan: &Plan) -> Result<(), Halt> {
    let (mut ready, mut more) = read(&plan.producer, Chunk::new(), plan.stall).await?;
    let mut spare = Chunk::new();
    while more {
        let ahead = read_ahead(&plan.producer, spare);
        write_all(open, ready.bytes(), plan.stall).await?;
        plan.tracker.sent(ready.payload());
        let (next, again) = joined(ahead, plan.stall).await?;
        spare = std::mem::replace(&mut ready, next);
        more = again;
    }
    Ok(())
}

/// Writes all of `bytes`. Each write is given the stall time to take at least one byte.
async fn write_all(open: &mut Open, mut bytes: &[u8], stall: Duration) -> Result<(), Halt> {
    while !bytes.is_empty() {
        match timeout(stall, open.out.write(bytes)).await {
            Err(_) => return Err(Halt::Stalled),
            Ok(Err(LaneError::Stopped(code))) => return Err(Halt::Stopped(code)),
            // A stream that takes nothing and says nothing would be written to for ever.
            Ok(Err(LaneError::Lost) | Ok(0)) => return Err(Halt::Lost),
            Ok(Ok(taken)) => bytes = bytes.get(taken..).unwrap_or_default(),
        }
    }
    Ok(())
}

type Read = JoinHandle<(Chunk, bool)>;

/// Starts filling `chunk` on a thread meant for blocking.
fn read_ahead(producer: &Arc<Producer>, mut chunk: Chunk) -> Read {
    let producer = Arc::clone(producer);
    tokio::task::spawn_blocking(move || {
        let more = producer.fill(&mut chunk);
        (chunk, more)
    })
}

/// Fills `chunk` and waits for it.
async fn read(
    producer: &Arc<Producer>,
    chunk: Chunk,
    stall: Duration,
) -> Result<(Chunk, bool), Halt> {
    joined(read_ahead(producer, chunk), stall).await
}

/// Waits for a read that was started. A read that does not come back in the stall time is let go
/// of: its thread finishes by itself, and what it read is not used.
async fn joined(read: Read, stall: Duration) -> Result<(Chunk, bool), Halt> {
    match timeout(stall, read).await {
        Err(_) => Err(Halt::Stalled),
        Ok(Ok(filled)) => Ok(filled),
        Ok(Err(error)) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        // The runtime is shutting down.
        Ok(Err(_)) => Err(Halt::Lost),
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::transfer::list::ListEntry;
    use tokio::sync::oneshot;

    use super::*;
    use crate::transfer::{
        send::{
            selection::Selection,
            testing::{Closed, Folder, Script, Step, Take, id},
            wire::Wire,
        },
        testing::{bytes, file},
    };

    /// What the lanes of a transfer share.
    struct Fixture {
        producer: Arc<Producer>,
        tracker: Arc<Tracker>,
        reset: ResetCode,
    }

    impl Fixture {
        fn new(selection: &Arc<Selection>) -> Self {
            let summary = selection.summary();
            let (tracker, _watching) = Tracker::new(summary.files, summary.bytes);
            Self {
                producer: Arc::new(Producer::new(Arc::clone(selection))),
                tracker: Arc::new(tracker),
                reset: ResetCode::new(),
            }
        }

        fn plan(&self, number: u8, stall: Duration) -> Plan {
            Plan {
                number,
                stall,
                reset: self.reset.clone(),
                producer: Arc::clone(&self.producer),
                tracker: Arc::clone(&self.tracker),
            }
        }
    }

    /// A connection to a receiver whose lanes do what `take` says. It answers no request.
    async fn connection(
        take: impl Fn(usize, u8) -> Take + Send + Sync + 'static,
    ) -> (Script, Arc<dyn Link>) {
        let script = Script::new(|_| Step::Silent).lanes(take);
        let link: Arc<dyn Link> = Arc::from(script.connect().await.unwrap());
        (script, link)
    }

    /// A folder with one file of 300 000 bytes and one of four, and the list of it.
    fn sample() -> (Folder, Arc<Selection>, Arc<ListFrames>, Vec<u8>) {
        let folder = Folder::new();
        let content = bytes(1, 300_000);
        folder.make("a", &content);
        folder.make("b", b"tiny");
        let selection = folder.select(&["a", "b"]);
        let list = Arc::new(ListFrames::new(selection.entries()).unwrap());
        (folder, selection, list, content)
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    /// The stall time of the tests that wait for a stall. It is long enough for a read from the
    /// disk of a busy computer to come back in it, because a read that does not is a stall too.
    const STALL: Duration = Duration::from_secs(1);
    /// The stall time of every other test, which no test comes near: a busy computer can hold a
    /// read or a write for much longer than a short stall time, and a lane that stalls by then is
    /// not what the test is about.
    const PATIENT: Duration = Duration::from_secs(30);

    // The list.

    #[test]
    fn a_list_is_cut_into_frames_of_at_most_the_most_entries_one_frame_holds() {
        let entries: Vec<ListEntry> = (0..2500).map(|n| file(&format!("f{n}"), n)).collect();

        let list = ListFrames::new(&entries).unwrap();

        let frames = list.frames();
        assert_eq!(frames.len(), 3);
        let mut again = Vec::new();
        for (frame, count) in frames.iter().zip([1024, 1024, 452]) {
            assert_eq!(frame[0], TAG_LIST);
            let chunk: ListChunk = frame::decode(&frame[1..]).unwrap();
            assert_eq!(chunk.entries.len(), count);
            again.extend(chunk.entries);
        }
        assert_eq!(again, entries);
        assert_eq!(list.len(), frames.iter().map(Vec::len).sum::<usize>());
    }

    // A lane that goes well.

    #[tokio::test]
    async fn lane_zero_writes_the_list_first_says_so_and_then_the_files() {
        let (_folder, selection, list, content) = sample();
        let (script, link) = connection(|_, _| Take::All).await;
        let fixture = Fixture::new(&selection);
        let (written, signal) = oneshot::channel();
        let listing = Listing {
            frames: Arc::clone(&list),
            written,
        };

        let result = run(link, id(1), fixture.plan(0, PATIENT), Some(listing)).await;

        assert_eq!(result, Ok(()));
        assert_eq!(signal.await, Ok(()));
        let lane = script.lane(1, 0).unwrap();
        let taken = lane.bytes();
        assert_eq!(taken[..list.len()], list.to_bytes()[..]);
        // The bytes of the first file come after the list, in one piece, and the lane is ended.
        assert!(contains(&taken[list.len()..], &content));
        assert!(contains(&taken[list.len()..], b"tiny"));
        assert_eq!(lane.closed(), Some(Closed::Finished));
        // The count is of the data of the files.
        assert_eq!(fixture.tracker.now().sent, 300_004);
    }

    #[tokio::test]
    async fn a_lane_that_is_not_lane_zero_writes_no_list() {
        let (_folder, selection, _list, content) = sample();
        let (script, link) = connection(|_, _| Take::All).await;
        let fixture = Fixture::new(&selection);

        let result = run(link, id(1), fixture.plan(2, PATIENT), None).await;

        assert_eq!(result, Ok(()));
        let lane = script.lane(1, 2).unwrap();
        let taken = lane.bytes();
        assert_ne!(taken[0], TAG_LIST);
        assert!(contains(&taken, &content));
        assert_eq!(lane.closed(), Some(Closed::Finished));
    }

    #[tokio::test]
    async fn a_lane_that_ended_well_is_not_reset_when_it_is_let_go_of() {
        let (_folder, selection, _list, _content) = sample();
        let (script, link) = connection(|_, _| Take::All).await;
        let fixture = Fixture::new(&selection);

        run(link, id(1), fixture.plan(1, PATIENT), None)
            .await
            .unwrap();

        // The lane was let go of when `run` returned, and it was closed once.
        assert_eq!(script.log().closed(), [(1, 1, Closed::Finished)]);
    }

    // A lane that does not.

    #[tokio::test]
    async fn a_stop_from_the_receiver_ends_the_lane_and_resets_it_and_what_was_cut_off_is_not_counted()
     {
        let (_folder, selection, list, _content) = sample();
        let after = list.len() as u64 + 1000;
        let (script, link) = connection(move |_, _| Take::StopAfter {
            after,
            code: close_code::TOO_LARGE,
        })
        .await;
        let fixture = Fixture::new(&selection);
        let (written, _signal) = oneshot::channel();
        let listing = Listing {
            frames: list,
            written,
        };

        let result = run(link, id(1), fixture.plan(0, PATIENT), Some(listing)).await;

        assert_eq!(result, Err(Halt::Stopped(close_code::TOO_LARGE)));
        let lane = script.lane(1, 0).unwrap();
        assert_eq!(lane.closed(), Some(Closed::Reset(close_code::TIMEOUT)));
        // The chunk that was being written is not sent.
        assert_eq!(fixture.tracker.now().sent, 0);
    }

    #[tokio::test]
    async fn a_stop_when_the_lane_is_ended_is_a_stop_and_the_lane_is_reset() {
        let (_folder, selection, _list, _content) = sample();
        let (script, link) = connection(|_, _| Take::StopAtEnd {
            code: close_code::CANCELLED,
        })
        .await;
        let fixture = Fixture::new(&selection);

        let result = run(link, id(1), fixture.plan(1, PATIENT), None).await;

        assert_eq!(result, Err(Halt::Stopped(close_code::CANCELLED)));
        let lane = script.lane(1, 1).unwrap();
        assert_eq!(lane.closed(), Some(Closed::Reset(close_code::TIMEOUT)));
    }

    #[tokio::test]
    async fn a_lane_that_takes_no_byte_for_the_stall_time_stalls() {
        let (_folder, selection, _list, _content) = sample();
        let (script, link) = connection(|_, _| Take::HangAfter { after: 10 }).await;
        let fixture = Fixture::new(&selection);
        let started = std::time::Instant::now();

        let result = timeout(PATIENT, run(link, id(1), fixture.plan(1, STALL), None))
            .await
            .expect("the lane did not stall");

        assert_eq!(result, Err(Halt::Stalled));
        assert!(started.elapsed() >= STALL);
        let lane = script.lane(1, 1).unwrap();
        assert_eq!(lane.len(), 10);
        assert_eq!(lane.closed(), Some(Closed::Reset(close_code::TIMEOUT)));
    }

    #[tokio::test]
    async fn a_read_that_does_not_come_back_in_the_stall_time_stalls() {
        // The read waits for a message that is sent only after `joined` has answered, so an answer
        // that comes at all did not wait for the read. How long it took is asked only roughly: a
        // busy computer can be slow to wake any thread, but the answer is made at the stall time
        // and not at a multiple of it.
        let (release, held) = std::sync::mpsc::channel::<()>();
        let read = tokio::task::spawn_blocking(move || {
            let _ = held.recv();
            (Chunk::new(), false)
        });
        let started = std::time::Instant::now();

        let result = timeout(PATIENT, joined(read, STALL))
            .await
            .expect("the answer waited for the read");

        assert!(matches!(result, Err(Halt::Stalled)));
        assert!(
            started.elapsed() < 5 * STALL,
            "the answer took {:?}",
            started.elapsed()
        );
        // The read is let go of, and it ends by itself once it is told to.
        let _ = release.send(());
    }

    #[tokio::test]
    async fn a_stream_that_takes_nothing_and_says_nothing_is_lost() {
        let (_folder, selection, _list, _content) = sample();
        let (_script, link) = connection(|_, _| Take::Nothing).await;
        let fixture = Fixture::new(&selection);

        let result = run(link, id(1), fixture.plan(1, PATIENT), None).await;

        assert_eq!(result, Err(Halt::Lost));
    }

    #[tokio::test]
    async fn a_stream_that_fails_in_the_middle_is_lost() {
        let (_folder, selection, _list, _content) = sample();
        let (script, link) = connection(|_, _| Take::LoseAfter { after: 100 }).await;
        let fixture = Fixture::new(&selection);

        let result = run(link, id(1), fixture.plan(1, PATIENT), None).await;

        assert_eq!(result, Err(Halt::Lost));
        let lane = script.lane(1, 1).unwrap();
        assert_eq!(lane.closed(), Some(Closed::Reset(close_code::TIMEOUT)));
    }

    #[tokio::test]
    async fn a_lane_that_cannot_be_opened_is_lost_and_one_that_never_opens_stalls() {
        let (_folder, selection, _list, _content) = sample();
        let fixture = Fixture::new(&selection);

        let (script, link) = connection(|_, _| Take::OpenFails).await;
        let result = run(link, id(1), fixture.plan(1, PATIENT), None).await;
        assert_eq!(result, Err(Halt::Lost));
        assert_eq!(script.log().opened(), Vec::new());

        let (script, link) = connection(|_, _| Take::OpenHangs).await;
        let started = std::time::Instant::now();
        let result = timeout(PATIENT, run(link, id(1), fixture.plan(1, STALL), None))
            .await
            .expect("the lane did not stall");
        assert_eq!(result, Err(Halt::Stalled));
        assert!(started.elapsed() >= STALL);
        assert_eq!(script.log().opened(), Vec::new());
    }

    #[tokio::test]
    async fn a_lane_that_is_aborted_is_reset_with_the_code_that_is_set_by_then() {
        let (_folder, selection, _list, _content) = sample();
        let (script, link) = connection(|_, _| Take::HangAfter { after: 10 }).await;
        let fixture = Fixture::new(&selection);
        let task = tokio::spawn(run(
            link,
            id(1),
            fixture.plan(1, Duration::from_secs(60)),
            None,
        ));
        super::super::testing::until("the lane to take its bytes", || {
            script.lane(1, 1).is_some_and(|lane| lane.len() == 10)
        })
        .await;

        fixture.reset.set(close_code::CANCELLED);
        task.abort();
        let joined = task.await;

        assert!(joined.is_err_and(|error| error.is_cancelled()));
        let lane = script.lane(1, 1).unwrap();
        assert_eq!(lane.closed(), Some(Closed::Reset(close_code::CANCELLED)));
    }
}
