// SPDX-License-Identifier: Apache-2.0

//! What is left to send of a transfer, and what has become of each file.
//!
//! The lanes of an attempt share one book. A lane asks it for the next **piece**: the next span of
//! the next file that has one, in the order of the list, or a whole empty file. It reads the piece
//! and tells the book what came of it. Pieces of one file go to different lanes, and the lanes read
//! them at the same time, so the book is where the order and the count of everything is kept:
//!
//! - **A piece is claimed once.** Two lanes never read the same span.
//! - **A file ends once.** Every file of the list gets exactly one `End` or one `Skip` in an
//!   attempt (section 8, End and Skip), and the lane that is told it by the book is the only one
//!   that sends it: the lane that records the last span of a file gets the hash, and the lane that
//!   fails a file first gets the right to say so. A file that failed is not completed by the lanes
//!   that still hold pieces of it, and its spans that are not claimed yet are never claimed.
//! - **The hash is made from spans in any order.** The value of each span goes into the file's
//!   [`FileHasher`], and the hash is known when the last one is in.
//!
//! The book does no I/O and has no clock, so what it promises can be tested with many threads and
//! no files. The reading of files is in [`produce`](super::produce).

use std::{collections::HashMap, sync::Mutex};

use baton_proto::transfer::{
    hash::{FileHasher, SpanValue, span_count, span_size},
    list::ListEntry,
};
use blake3::Hash;

use crate::util::lock;

/// A piece of a file to read: one span of it, or all of a file that has no bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Piece {
    /// Position of the file in the list.
    pub(crate) file: u32,
    /// Number of the span. It is 0 for a file with no bytes, which has no span.
    pub(crate) span: u32,
    /// How many bytes the piece has: the length of the span, and 0 for an empty file.
    pub(crate) len: u32,
    /// Whether other lanes may be reading other pieces of the file at the same time, so that its
    /// open file is shared.
    pub(crate) shared: bool,
}

/// What a lane gets when it asks for work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Claim {
    /// The next piece. It is the lane's until it records it or fails the file.
    Piece(Piece),
    /// The next piece has more bytes than the room that was offered. It is left for the next lane
    /// that asks, and the lane that is full goes and writes.
    NoRoom,
    /// Every piece has been claimed.
    Done,
}

/// What became of a piece that was read and hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Recorded {
    /// The piece is in. The file still has pieces that are not.
    Kept,
    /// The piece was the last one. This is the hash of the whole file, and the lane that was told
    /// sends its `End`.
    Complete(Hash),
    /// The file has failed, or ended, and the piece is not needed. The lane drops it.
    Dropped,
}

/// What has become of the files so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    /// Files that were read in full, and whose `End` the lane that completed them sends.
    pub(crate) ended_files: u32,
    /// What those files add up to, in bytes.
    pub(crate) ended_bytes: u64,
    /// Files that failed, and whose `Skip` the lane that failed them sends.
    pub(crate) skipped_files: u32,
}

/// The pieces of a transfer, and what became of them. See the module documentation.
#[derive(Debug)]
pub(crate) struct Book {
    /// The size of each entry in bytes. A folder has none.
    sizes: Vec<u64>,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    /// Where each entry is. A folder has nothing to send, so it starts out settled.
    phases: Vec<Phase>,
    /// The first entry that may still have a piece to claim.
    cursor: usize,
    /// The files that have been started and are not settled yet.
    active: HashMap<u32, Active>,
    totals: Totals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No piece of it has been claimed.
    Waiting,
    /// A piece has been claimed, and the file has neither ended nor failed with every piece in.
    Started,
    /// Nothing more will happen to it: it ended, or it failed and no piece of it is out.
    Settled,
}

#[derive(Debug)]
struct Active {
    /// How many pieces the file has: its spans, and one for a file with no bytes.
    pieces: u32,
    /// The next piece to claim. It is `pieces` when all are claimed, or when the file has failed.
    next: u32,
    /// Pieces that are claimed and not recorded yet.
    outstanding: u32,
    hasher: FileHasher,
    failed: bool,
}

impl Active {
    /// Whether nothing more will happen to the file: no piece is out, and none is left to claim.
    fn is_over(&self) -> bool {
        self.outstanding == 0 && self.next >= self.pieces
    }
}

impl Book {
    /// A book of the entries of a list.
    pub(crate) fn new(entries: &[ListEntry]) -> Self {
        Self {
            sizes: entries.iter().map(|entry| entry.size).collect(),
            state: Mutex::new(State {
                phases: entries
                    .iter()
                    .map(|entry| {
                        if entry.folder {
                            Phase::Settled
                        } else {
                            Phase::Waiting
                        }
                    })
                    .collect(),
                cursor: 0,
                active: HashMap::new(),
                totals: Totals::default(),
            }),
        }
    }

    /// Gives the next piece, if it has no more than `room` bytes.
    pub(crate) fn claim(&self, room: u64) -> Claim {
        let mut guard = lock(&self.state);
        let state = &mut *guard;
        loop {
            let index = state.cursor;
            let Some(phase) = state.phases.get(index).copied() else {
                return Claim::Done;
            };
            let file = u32::try_from(index).unwrap_or(u32::MAX);
            let size = self.sizes.get(index).copied().unwrap_or(0);
            let (pieces, next) = match (phase, state.active.get(&file)) {
                (Phase::Settled, _) => {
                    state.cursor += 1;
                    continue;
                }
                (Phase::Started, Some(active)) if active.failed || active.next >= active.pieces => {
                    state.cursor += 1;
                    continue;
                }
                (Phase::Started, Some(active)) => (active.pieces, active.next),
                (Phase::Started | Phase::Waiting, _) => (pieces_of(size), 0),
            };
            let len = span_size(size, next).unwrap_or(0);
            if u64::from(len) > room {
                return Claim::NoRoom;
            }
            let Ok(hasher) = FileHasher::new(size) else {
                // A file larger than a transfer can hold is not in a list that was walked. It is
                // not claimed, and its `End` is missing from the count at `Finish`.
                state.phases[index] = Phase::Settled;
                state.cursor += 1;
                continue;
            };
            let active = state.active.entry(file).or_insert(Active {
                pieces,
                next: 0,
                outstanding: 0,
                hasher,
                failed: false,
            });
            active.next += 1;
            active.outstanding += 1;
            state.phases[index] = Phase::Started;
            return Claim::Piece(Piece {
                file,
                span: next,
                len,
                shared: pieces > 1,
            });
        }
    }

    /// Records the value of a span that was read, and says what became of the piece.
    pub(crate) fn record(&self, file: u32, span: u32, value: SpanValue) -> Recorded {
        let mut guard = lock(&self.state);
        let state = &mut *guard;
        let Some(active) = state.active.get_mut(&file) else {
            return Recorded::Dropped;
        };
        if active.failed {
            active.outstanding = active.outstanding.saturating_sub(1);
            Self::settle_if_over(state, file);
            return Recorded::Dropped;
        }
        // A span that is not one of the file's, or is in already, is not a piece that a lane holds,
        // and does not bring the file any nearer to being over.
        if active.hasher.insert(span, value).is_err() {
            return Recorded::Dropped;
        }
        active.outstanding = active.outstanding.saturating_sub(1);
        self.complete_if_whole(state, file)
    }

    /// Records that a file with no bytes was found as the list says.
    pub(crate) fn record_empty(&self, file: u32) -> Recorded {
        let mut guard = lock(&self.state);
        let state = &mut *guard;
        let Some(active) = state.active.get_mut(&file) else {
            return Recorded::Dropped;
        };
        active.outstanding = active.outstanding.saturating_sub(1);
        if active.failed {
            Self::settle_if_over(state, file);
            return Recorded::Dropped;
        }
        self.complete_if_whole(state, file)
    }

    /// Ends the file if its last piece is in.
    fn complete_if_whole(&self, state: &mut State, file: u32) -> Recorded {
        let Some(active) = state.active.get(&file) else {
            return Recorded::Dropped;
        };
        if !active.hasher.is_complete() {
            return Recorded::Kept;
        }
        let Ok(hash) = active.hasher.finish() else {
            return Recorded::Kept;
        };
        state.active.remove(&file);
        let index = usize::try_from(file).unwrap_or(usize::MAX);
        if let Some(phase) = state.phases.get_mut(index) {
            *phase = Phase::Settled;
        }
        state.totals.ended_files += 1;
        state.totals.ended_bytes = state
            .totals
            .ended_bytes
            .saturating_add(self.sizes.get(index).copied().unwrap_or(0));
        Recorded::Complete(hash)
    }

    /// Settles a file that has failed, once the last piece of it that is out is back.
    fn settle_if_over(state: &mut State, file: u32) {
        if state.active.get(&file).is_some_and(Active::is_over) {
            state.active.remove(&file);
            let index = usize::try_from(file).unwrap_or(usize::MAX);
            if let Some(phase) = state.phases.get_mut(index) {
                *phase = Phase::Settled;
            }
        }
    }

    /// Gives up the piece of `file` that the caller holds, and fails the file. It says whether
    /// this call failed it: the first does, and the lane that gets `true` sends the `Skip`. The
    /// pieces of the file that nobody has claimed are never claimed.
    pub(crate) fn fail(&self, file: u32) -> bool {
        let mut guard = lock(&self.state);
        let state = &mut *guard;
        let Some(active) = state.active.get_mut(&file) else {
            return false;
        };
        active.outstanding = active.outstanding.saturating_sub(1);
        let first = !active.failed;
        active.failed = true;
        active.next = active.pieces;
        if first {
            state.totals.skipped_files += 1;
        }
        Self::settle_if_over(state, file);
        first
    }

    /// Whether `file` is open: it has started, and has neither ended nor failed.
    #[cfg(test)]
    pub(crate) fn is_open(&self, file: u32) -> bool {
        lock(&self.state)
            .active
            .get(&file)
            .is_some_and(|active| !active.failed)
    }

    /// Whether nothing more will happen to `file`: no piece of it is out and none is left to
    /// claim, so whatever is held open for it can be let go of.
    pub(crate) fn is_settled(&self, file: u32) -> bool {
        let state = lock(&self.state);
        usize::try_from(file)
            .ok()
            .and_then(|index| state.phases.get(index))
            .is_some_and(|phase| *phase == Phase::Settled)
    }

    /// What has become of the files so far.
    pub(crate) fn totals(&self) -> Totals {
        lock(&self.state).totals
    }
}

/// How many pieces a file of `size` bytes has: its spans, and one for a file with no bytes.
fn pieces_of(size: u64) -> u32 {
    u32::try_from(span_count(size).max(1)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
        thread,
    };

    use baton_proto::transfer::hash::{SPAN_LEN, hash_span};

    use super::*;
    use crate::transfer::testing::{file, folder};

    /// The bytes of the span `span` of a file of `size` bytes, as the tests make them.
    fn bytes_of(file: u32, span: u32, len: u32) -> Vec<u8> {
        let seed = u8::try_from((file + span) % 251).unwrap();
        (0..len)
            .map(|i| seed.wrapping_add(u8::try_from(i % 251).unwrap()))
            .collect()
    }

    /// The value of a span of the bytes that `bytes_of` makes.
    fn value_of(size: u64, file: u32, span: u32) -> SpanValue {
        let len = span_size(size, span).unwrap();
        hash_span(size, span, &bytes_of(file, span, len)).unwrap()
    }

    /// The hash of the bytes that `bytes_of` makes for all the spans of a file.
    fn hash_of(size: u64, file: u32) -> Hash {
        let mut data = Vec::new();
        for span in 0..u32::try_from(span_count(size)).unwrap() {
            data.extend(bytes_of(file, span, span_size(size, span).unwrap()));
        }
        blake3::hash(&data)
    }

    fn piece(book: &Book, room: u64) -> Piece {
        match book.claim(room) {
            Claim::Piece(piece) => piece,
            other => panic!("expected a piece, got {other:?}"),
        }
    }

    #[test]
    fn pieces_come_in_the_order_of_the_list_and_folders_have_none() {
        let book = Book::new(&[
            folder("d"),
            file("d/a", 10),
            file("d/b", SPAN_LEN + 1),
            folder("e"),
            file("z", 0),
        ]);
        let all = [
            (1, 0, 10, false),
            (2, 0, u32::try_from(SPAN_LEN).unwrap(), true),
            (2, 1, 1, true),
            (4, 0, 0, false),
        ];
        for (file, span, len, shared) in all {
            assert_eq!(
                piece(&book, u64::MAX),
                Piece {
                    file,
                    span,
                    len,
                    shared
                }
            );
        }
        assert_eq!(book.claim(u64::MAX), Claim::Done);
        assert_eq!(book.claim(u64::MAX), Claim::Done);
    }

    #[test]
    fn a_piece_that_does_not_fit_in_the_room_is_left_for_the_next_lane() {
        let book = Book::new(&[file("a", 100), file("b", 300), file("c", 50)]);
        assert_eq!(piece(&book, 250).file, 0);
        // 150 are left of the room of this lane, and `b` has 300.
        assert_eq!(book.claim(150), Claim::NoRoom);
        assert_eq!(book.claim(150), Claim::NoRoom);
        // A lane with room for it takes it, and `c` is next.
        assert_eq!(piece(&book, 300).file, 1);
        assert_eq!(piece(&book, 50).file, 2);
        assert_eq!(book.claim(0), Claim::Done);
    }

    #[test]
    fn a_file_with_no_bytes_fits_in_no_room() {
        let book = Book::new(&[file("empty", 0)]);
        assert_eq!(piece(&book, 0).len, 0);
    }

    #[test]
    fn a_list_of_folders_alone_has_nothing_to_claim() {
        let book = Book::new(&[folder("a"), folder("a/b")]);
        assert_eq!(book.claim(u64::MAX), Claim::Done);
        assert_eq!(Book::new(&[]).claim(u64::MAX), Claim::Done);
    }

    #[test]
    fn the_last_span_recorded_completes_the_file_whatever_order_they_come_in() {
        let size = 3 * SPAN_LEN + 5;
        for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1], [1, 3, 0, 2]] {
            let book = Book::new(&[file("big", size)]);
            let pieces: Vec<_> = (0..4).map(|_| piece(&book, u64::MAX)).collect();
            for (n, span) in order.into_iter().enumerate() {
                let recorded = book.record(0, span, value_of(size, 0, span));
                if n < 3 {
                    assert_eq!(recorded, Recorded::Kept, "{order:?}");
                    assert!(!book.is_settled(0));
                } else {
                    assert_eq!(recorded, Recorded::Complete(hash_of(size, 0)), "{order:?}");
                }
            }
            assert_eq!(pieces.len(), 4);
            assert!(book.is_settled(0));
            assert_eq!(
                book.totals(),
                Totals {
                    ended_files: 1,
                    ended_bytes: size,
                    skipped_files: 0
                }
            );
        }
    }

    #[test]
    fn a_file_of_one_span_is_complete_when_its_span_is_in_and_the_hash_is_the_files() {
        for size in [1, 1024, SPAN_LEN - 1, SPAN_LEN] {
            let book = Book::new(&[file("f", size)]);
            let p = piece(&book, u64::MAX);
            assert!(!p.shared);
            assert_eq!(
                book.record(0, 0, value_of(size, 0, 0)),
                Recorded::Complete(hash_of(size, 0)),
                "{size}"
            );
        }
    }

    #[test]
    fn a_file_with_no_bytes_is_complete_at_once_with_the_hash_of_nothing() {
        let book = Book::new(&[file("empty", 0)]);
        piece(&book, 0);
        assert_eq!(book.record_empty(0), Recorded::Complete(blake3::hash(b"")));
        assert_eq!(book.totals().ended_files, 1);
        assert_eq!(book.totals().ended_bytes, 0);
    }

    #[test]
    fn the_first_failure_of_a_file_is_the_one_that_says_so() {
        let size = 3 * SPAN_LEN;
        let book = Book::new(&[file("big", size), file("after", 5)]);
        let (a, b) = (piece(&book, u64::MAX), piece(&book, u64::MAX));
        assert_eq!((a.span, b.span), (0, 1));
        assert!(book.is_open(0));

        assert!(book.fail(0));
        assert!(!book.is_open(0));
        // The other lane fails the same file a moment later, and is told it is not the first.
        assert!(!book.fail(0));
        assert!(book.is_settled(0));
        assert_eq!(book.totals().skipped_files, 1);
        assert_eq!(book.totals().ended_files, 0);

        // Span 2 was never claimed, and never will be: the next piece is the next file's.
        assert_eq!(piece(&book, u64::MAX).file, 1);
    }

    #[test]
    fn a_span_that_arrives_after_the_file_failed_is_dropped_and_completes_nothing() {
        let size = 2 * SPAN_LEN;
        let book = Book::new(&[file("big", size)]);
        let (a, b) = (piece(&book, u64::MAX), piece(&book, u64::MAX));
        assert!(book.fail(a.file));
        assert!(!book.is_settled(0), "a piece of the file is still out");

        assert_eq!(
            book.record(b.file, b.span, value_of(size, 0, 1)),
            Recorded::Dropped
        );
        assert!(book.is_settled(0));
        assert_eq!(book.totals().ended_files, 0);
    }

    #[test]
    fn a_file_that_ended_cannot_be_failed_or_completed_again() {
        let book = Book::new(&[file("f", 5)]);
        piece(&book, u64::MAX);
        assert!(matches!(
            book.record(0, 0, value_of(5, 0, 0)),
            Recorded::Complete(_)
        ));
        assert!(!book.fail(0));
        assert_eq!(book.record(0, 0, value_of(5, 0, 0)), Recorded::Dropped);
        assert_eq!(book.totals().ended_files, 1);
        assert_eq!(book.totals().skipped_files, 0);
    }

    #[test]
    fn a_span_recorded_twice_is_dropped_and_does_not_complete_the_file() {
        let size = 2 * SPAN_LEN;
        let book = Book::new(&[file("big", size)]);
        piece(&book, u64::MAX);
        piece(&book, u64::MAX);
        assert_eq!(book.record(0, 0, value_of(size, 0, 0)), Recorded::Kept);
        assert_eq!(book.record(0, 0, value_of(size, 0, 0)), Recorded::Dropped);
        assert!(matches!(
            book.record(0, 1, value_of(size, 0, 1)),
            Recorded::Complete(_)
        ));
    }

    #[test]
    fn a_file_is_open_between_its_first_claim_and_its_end() {
        let book = Book::new(&[file("f", 5)]);
        assert!(!book.is_open(0));
        piece(&book, u64::MAX);
        assert!(book.is_open(0));
        let _ = book.record(0, 0, value_of(5, 0, 0));
        assert!(!book.is_open(0));
    }

    #[test]
    fn a_file_is_settled_only_once_every_piece_is_claimed_and_recorded() {
        let size = 2 * SPAN_LEN;
        let book = Book::new(&[file("big", size)]);
        piece(&book, u64::MAX);
        // One piece is out and one is unclaimed.
        assert!(!book.is_settled(0));
        piece(&book, u64::MAX);
        let _ = book.record(0, 0, value_of(size, 0, 0));
        assert!(!book.is_settled(0));
        let _ = book.record(0, 1, value_of(size, 0, 1));
        assert!(book.is_settled(0));
    }

    /// Whether a lane fails a piece, from the number of the file and of its span alone, so that a
    /// run does not depend on which thread got what.
    fn fails(seed: u64, file: u32, span: u32) -> bool {
        let mixed = u64::from(file)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(u64::from(span).wrapping_mul(0xBF58_476D_1CE4_E5B9))
            .wrapping_add(seed)
            .rotate_left(17)
            .wrapping_mul(0x94D0_49BB_1331_11EB);
        // About one piece in seven.
        (mixed >> 33) % 7 == 0
    }

    /// The size of the `n`th file of the stress test: empty, small, one span, one span and a byte,
    /// and several spans.
    fn stress_size(n: u32) -> u64 {
        match n % 9 {
            0 => 0,
            1 | 2 => 1 + u64::from(n),
            3 => SPAN_LEN,
            4 => SPAN_LEN + 1,
            5 => 3 * SPAN_LEN + u64::from(n),
            _ => 100 + u64::from(n) * 7,
        }
    }

    const FILES: u32 = 400;

    #[test]
    fn many_lanes_claim_every_piece_once_and_every_file_ends_once() {
        for seed in 0..6_u64 {
            let mut entries = vec![folder("d")];
            entries.extend((0..FILES).map(|n| file(&format!("d/f{n:03}"), stress_size(n))));
            let book = Arc::new(Book::new(&entries));
            let claimed = Arc::new(Mutex::new(HashSet::new()));
            // How many times each file got its `End` and its `Skip`.
            let ends = Arc::new(Mutex::new(HashMap::<u32, (u32, u32)>::new()));

            let lanes: Vec<_> = (0..8)
                .map(|_| {
                    let (book, claimed, ends) =
                        (Arc::clone(&book), Arc::clone(&claimed), Arc::clone(&ends));
                    thread::spawn(move || {
                        let mut room = SPAN_LEN;
                        loop {
                            let p = match book.claim(room) {
                                Claim::Done => break,
                                Claim::NoRoom => {
                                    room = SPAN_LEN;
                                    continue;
                                }
                                Claim::Piece(p) => p,
                            };
                            assert!(
                                lock(&claimed).insert((p.file, p.span)),
                                "a piece was claimed twice: {p:?}"
                            );
                            room = room.saturating_sub(u64::from(p.len)).max(1);
                            let size = stress_size(p.file - 1);
                            // `true` if this lane ended the file with a `Skip`, `false` with an
                            // `End`, and `None` if it ended nothing.
                            let skipped = if fails(seed, p.file, p.span) {
                                book.fail(p.file).then_some(true)
                            } else if p.len == 0 {
                                matches!(book.record_empty(p.file), Recorded::Complete(_))
                                    .then_some(false)
                            } else {
                                match book.record(p.file, p.span, value_of(size, p.file, p.span)) {
                                    Recorded::Complete(hash) => {
                                        assert_eq!(hash, hash_of(size, p.file));
                                        Some(false)
                                    }
                                    Recorded::Kept | Recorded::Dropped => None,
                                }
                            };
                            if let Some(skipped) = skipped {
                                let mut ends = lock(&ends);
                                let counts = ends.entry(p.file).or_default();
                                if skipped {
                                    counts.1 += 1;
                                } else {
                                    counts.0 += 1;
                                }
                            }
                        }
                    })
                })
                .collect();
            for lane in lanes {
                lane.join().unwrap();
            }

            let ends = lock(&ends);
            let (mut completed, mut skipped, mut bytes) = (0_u32, 0_u32, 0_u64);
            for file in 1..=FILES {
                let (done, failed) = ends.get(&file).copied().unwrap_or_default();
                assert_eq!(
                    done + failed,
                    1,
                    "file {file} ended {done} + {failed} times"
                );
                completed += done;
                skipped += failed;
                if done == 1 {
                    bytes += stress_size(file - 1);
                }
                assert!(book.is_settled(file), "file {file} is not settled");
            }
            assert_eq!(
                book.totals(),
                Totals {
                    ended_files: completed,
                    ended_bytes: bytes,
                    skipped_files: skipped
                }
            );
            assert!(completed > 0 && skipped > 0, "{completed} {skipped}");
        }
    }
}
