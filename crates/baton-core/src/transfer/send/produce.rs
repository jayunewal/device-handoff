// SPDX-License-Identifier: Apache-2.0

//! Reading the files of an attempt into the frames that the lanes write.
//!
//! A lane asks the [`Producer`] to fill a [`Chunk`]. The producer takes pieces from the
//! [`Book`], reads each from its file, hashes it, tells the book what came of it, and puts the
//! frames in the chunk in the form they have on the wire. The lane writes the chunk as it is.
//!
//! - **A chunk holds at most one span of file data.** One span of a large file fills a chunk, and
//!   the small files that come together share one, with their `End` frames. Many small files are
//!   therefore read and written in a few large steps and not one step each, which is what makes
//!   10,000 files of 10 KiB nearly as quick as one file of 100 MB (ADR 0008).
//! - **The bytes of a file are read into the chunk where they are sent.** The head of the frame is
//!   written in front of them once they are in, so a span is not copied.
//! - **A file that fails fails alone.** A file that cannot be read, or has changed, gets a `Skip`
//!   frame, and the chunk goes on with the next piece. The producer keeps what each skip was for,
//!   for the report at the end.
//! - **The producer reads, so a lane runs it on a thread meant for blocking.** It takes a chunk
//!   that is full of frames the lane has written already, and a thread of the lane's can read the
//!   next one while the lane writes the last.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use baton_proto::transfer::{
    hash::{SPAN_LEN, hash_span},
    lane::{FrameHead, SkipReason},
};
use blake3::Hash;

use super::{
    book::{Book, Claim, Piece, Recorded, Totals},
    selection::Selection,
    source::Source,
};
use crate::util::lock;

/// The most file data that a chunk holds, in bytes.
pub(crate) const CHUNK_PAYLOAD: u64 = SPAN_LEN;

/// [`CHUNK_PAYLOAD`] as a length of memory.
const PAYLOAD_LEN: usize = 1 << 20;
const _: () = assert!(PAYLOAD_LEN as u64 == CHUNK_PAYLOAD);

/// The most pieces that a chunk holds.
const CHUNK_PIECES: usize = 256;

/// The bytes before the data of a span: its tag, the number of the file and the number of the span.
const SPAN_HEAD_LEN: usize = 1 + 4 + 4;

/// The bytes of an `End` frame: its tag, the number of the file and the hash.
const END_LEN: usize = 1 + 4 + blake3::OUT_LEN;

/// What a chunk can hold: the data, and the frames around it of as many pieces as it takes.
const CAPACITY: usize = PAYLOAD_LEN + CHUNK_PIECES * (SPAN_HEAD_LEN + END_LEN);

/// Frames, ready to be written to a lane.
#[derive(Debug)]
pub(crate) struct Chunk {
    buf: Vec<u8>,
    len: usize,
    /// The bytes of files in it.
    payload: u64,
    /// The pieces in it that made a frame.
    pieces: usize,
    /// Where a frame head is put together.
    scratch: Vec<u8>,
}

impl Chunk {
    pub(crate) fn new() -> Self {
        Self {
            buf: vec![0; CAPACITY],
            len: 0,
            payload: 0,
            pieces: 0,
            scratch: Vec::with_capacity(FrameHead::MAX_REST_LEN + 1),
        }
    }

    /// Empties the chunk, so that it can be filled again.
    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.payload = 0;
        self.pieces = 0;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The frames to write.
    pub(crate) fn bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or_default()
    }

    /// How many bytes of files the frames carry.
    pub(crate) fn payload(&self) -> u64 {
        self.payload
    }

    /// Whether no more pieces should be put in the chunk.
    fn is_full(&self) -> bool {
        self.payload >= CHUNK_PAYLOAD || self.pieces >= CHUNK_PIECES
    }

    /// The place where the data of a span of `len` bytes goes.
    fn room_for_span(&mut self, len: usize) -> Option<&mut [u8]> {
        let start = self.len + SPAN_HEAD_LEN;
        self.buf.get_mut(start..start + len)
    }

    /// Makes the data that was read into [`room_for_span`](Self::room_for_span) a frame.
    fn keep_span(&mut self, file: u32, span: u32, len: usize) {
        self.scratch.clear();
        FrameHead::Span { file, span }.encode_into(&mut self.scratch);
        if let Some(head) = self.buf.get_mut(self.len..self.len + SPAN_HEAD_LEN) {
            head.copy_from_slice(&self.scratch);
        }
        self.len += SPAN_HEAD_LEN + len;
        self.payload += len as u64;
        self.pieces += 1;
    }

    fn push(&mut self, head: &FrameHead) {
        self.scratch.clear();
        head.encode_into(&mut self.scratch);
        if let Some(room) = self.buf.get_mut(self.len..self.len + self.scratch.len()) {
            room.copy_from_slice(&self.scratch);
        }
        self.len += self.scratch.len();
    }

    fn end(&mut self, file: u32, hash: Hash) {
        self.push(&FrameHead::End { file, hash });
    }

    fn skip(&mut self, file: u32, reason: SkipReason) {
        self.push(&FrameHead::Skip { file, reason });
        self.pieces += 1;
    }
}

/// `len` bytes as a length of memory.
fn length(len: u32) -> usize {
    usize::try_from(len).unwrap_or(usize::MAX)
}

/// An open file that several lanes read at the same time, or the reason it could not be opened.
type Shared = Arc<OnceLock<Result<Source, SkipReason>>>;

/// Reads the files of one attempt. See the module documentation.
#[derive(Debug)]
pub(crate) struct Producer {
    selection: Arc<Selection>,
    book: Book,
    /// The files of more than one span that have been started and are not settled.
    shared: Mutex<HashMap<u32, Shared>>,
    /// Why each file that failed did.
    skipped: Mutex<Vec<(u32, SkipReason)>>,
}

impl Producer {
    /// A producer of the files of `selection`, from the first.
    pub(crate) fn new(selection: Arc<Selection>) -> Self {
        let book = Book::new(selection.entries());
        Self {
            selection,
            book,
            shared: Mutex::new(HashMap::new()),
            skipped: Mutex::new(Vec::new()),
        }
    }

    /// What has become of the files so far.
    pub(crate) fn totals(&self) -> Totals {
        self.book.totals()
    }

    /// The files that failed, and why, in the order they failed.
    pub(crate) fn skipped(&self) -> Vec<(u32, SkipReason)> {
        lock(&self.skipped).clone()
    }

    /// Fills `chunk`, which it empties first, with the frames of the next pieces. It says whether
    /// there is anything in it: `false` means that every piece has been claimed, and the lane is
    /// done.
    ///
    /// This reads files and hashes what it reads, so a lane calls it on a thread meant for blocking.
    pub(crate) fn fill(&self, chunk: &mut Chunk) -> bool {
        chunk.clear();
        loop {
            let room = CHUNK_PAYLOAD.saturating_sub(chunk.payload());
            match self.book.claim(room) {
                // `NoRoom`: the piece is for the next lane, and this chunk is as full as it will be.
                Claim::Done | Claim::NoRoom => break,
                Claim::Piece(piece) => {
                    self.read(piece, chunk);
                    if chunk.is_full() {
                        break;
                    }
                }
            }
        }
        !chunk.is_empty()
    }

    /// Reads one piece into `chunk`, and puts the frames that follow from it there.
    fn read(&self, piece: Piece, chunk: &mut Chunk) {
        let Some(entry) = self.selection.file(piece.file) else {
            // The book only gives pieces of files that are in the list.
            self.fail(piece.file, SkipReason::Other, chunk);
            return;
        };
        let Some(path) = self.selection.source(piece.file) else {
            self.fail(piece.file, SkipReason::Other, chunk);
            return;
        };
        let shared;
        let own;
        let opened = if piece.shared {
            shared = self.shared_of(piece.file);
            shared.get_or_init(|| Source::open(&path, entry))
        } else {
            own = Source::open(&path, entry);
            &own
        };
        let recorded = match opened {
            Ok(source) => self.read_from(source, entry.size, piece, chunk),
            Err(reason) => Err(*reason),
        };
        match recorded {
            Ok(Recorded::Kept) => {
                if piece.len > 0 {
                    chunk.keep_span(piece.file, piece.span, length(piece.len));
                }
            }
            Ok(Recorded::Complete(hash)) => {
                if piece.len > 0 {
                    chunk.keep_span(piece.file, piece.span, length(piece.len));
                } else {
                    chunk.pieces += 1;
                }
                chunk.end(piece.file, hash);
            }
            // The file failed in another lane, and its `Skip` is that lane's.
            Ok(Recorded::Dropped) => {}
            Err(reason) => self.fail(piece.file, reason, chunk),
        }
        if piece.shared && self.book.is_settled(piece.file) {
            lock(&self.shared).remove(&piece.file);
        }
    }

    /// Reads the bytes of `piece` into the room for them in `chunk`, hashes them, and records
    /// them in the book. The frame is made by the caller, once it is known to be wanted.
    fn read_from(
        &self,
        source: &Source,
        size: u64,
        piece: Piece,
        chunk: &mut Chunk,
    ) -> Result<Recorded, SkipReason> {
        if piece.len == 0 {
            // The file was checked when it was opened, and has no bytes to read.
            return Ok(self.book.record_empty(piece.file));
        }
        let room = chunk
            .room_for_span(length(piece.len))
            .ok_or(SkipReason::Other)?;
        source.read_span(piece.span, room)?;
        let value = hash_span(size, piece.span, room).map_err(|_| SkipReason::Other)?;
        Ok(self.book.record(piece.file, piece.span, value))
    }

    /// Fails `file`, and puts its `Skip` in `chunk` if this is the first to fail it.
    fn fail(&self, file: u32, reason: SkipReason, chunk: &mut Chunk) {
        if self.book.fail(file) {
            lock(&self.skipped).push((file, reason));
            chunk.skip(file, reason);
        }
    }

    fn shared_of(&self, file: u32) -> Shared {
        Arc::clone(lock(&self.shared).entry(file).or_default())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::Path, thread, time::Duration};

    use baton_proto::transfer::{
        hash::{span_count, span_size},
        lane::TAG_SPAN,
    };

    use super::*;
    use crate::transfer::testing::bytes;

    /// What a lane writes, taken apart.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Frame {
        Span { file: u32, span: u32, data: Vec<u8> },
        End { file: u32, hash: Hash },
        Skip { file: u32, reason: SkipReason },
    }

    /// Takes apart the frames of a chunk, as the receiver reads them, with the sizes the list
    /// gives for the spans.
    fn frames(selection: &Selection, mut rest: &[u8]) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Some((&tag, after)) = rest.split_first() {
            let head_len = FrameHead::rest_len(tag).unwrap();
            let (head, after) = after.split_at(head_len);
            rest = after;
            match FrameHead::decode(tag, head).unwrap() {
                FrameHead::Span { file, span } => {
                    assert_eq!(tag, TAG_SPAN);
                    let size = selection.file(file).unwrap().size;
                    let len = length(span_size(size, span).unwrap());
                    let (data, after) = rest.split_at(len);
                    rest = after;
                    out.push(Frame::Span {
                        file,
                        span,
                        data: data.to_vec(),
                    });
                }
                FrameHead::End { file, hash } => out.push(Frame::End { file, hash }),
                FrameHead::Skip { file, reason } => out.push(Frame::Skip { file, reason }),
                FrameHead::List => panic!("the producer made a list frame"),
            }
        }
        out
    }

    /// Runs `lanes` threads that each fill chunks until the producer is done.
    fn produce(selection: &Arc<Selection>, lanes: usize) -> (Arc<Producer>, Vec<Frame>) {
        let producer = Arc::new(Producer::new(Arc::clone(selection)));
        let threads: Vec<_> = (0..lanes)
            .map(|_| {
                let producer = Arc::clone(&producer);
                let selection = Arc::clone(selection);
                thread::spawn(move || {
                    let mut chunk = Chunk::new();
                    let mut got = Vec::new();
                    while producer.fill(&mut chunk) {
                        assert!(chunk.payload() <= CHUNK_PAYLOAD);
                        assert!(chunk.bytes().len() <= CAPACITY);
                        got.extend(frames(&selection, chunk.bytes()));
                    }
                    got
                })
            })
            .collect();
        let mut all = Vec::new();
        for thread in threads {
            all.extend(thread.join().unwrap());
        }
        (producer, all)
    }

    /// Makes a tree of the files in `files`, with the bytes `bytes(seed, size)`, and walks it.
    fn tree(dir: &Path, files: &[(&str, usize)]) -> Arc<Selection> {
        for (n, (name, size)) in files.iter().enumerate() {
            let path = dir.join("t").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes(u8::try_from(n % 200).unwrap(), *size)).unwrap();
        }
        Arc::new(Selection::walk(&[dir.join("t")]).unwrap())
    }

    /// What the frames amount to: the bytes of each file that ended, assembled from its spans in
    /// any order, and the files that were skipped. Checks the hash of every file that ended and
    /// that each file got one `End` or one `Skip`.
    fn assemble(
        selection: &Selection,
        frames: &[Frame],
    ) -> (BTreeMap<u32, Vec<u8>>, BTreeMap<u32, SkipReason>) {
        let mut spans: BTreeMap<u32, BTreeMap<u32, Vec<u8>>> = BTreeMap::new();
        let mut ended: BTreeMap<u32, Hash> = BTreeMap::new();
        let mut skipped = BTreeMap::new();
        for frame in frames {
            match frame {
                Frame::Span { file, span, data } => {
                    assert!(
                        spans
                            .entry(*file)
                            .or_default()
                            .insert(*span, data.clone())
                            .is_none(),
                        "span {span} of file {file} was sent twice"
                    );
                }
                Frame::End { file, hash } => {
                    assert!(ended.insert(*file, *hash).is_none(), "{file} ended twice");
                }
                Frame::Skip { file, reason } => {
                    assert!(
                        skipped.insert(*file, *reason).is_none(),
                        "{file} skipped twice"
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        for (index, entry) in selection.entries().iter().enumerate() {
            if entry.folder {
                continue;
            }
            let file = u32::try_from(index).unwrap();
            assert!(
                ended.contains_key(&file) != skipped.contains_key(&file),
                "file {file} must have an End or a Skip, and not both"
            );
            if let Some(hash) = ended.get(&file) {
                let held = spans.remove(&file).unwrap_or_default();
                let count = usize::try_from(span_count(entry.size)).unwrap();
                assert_eq!(held.len(), count, "file {file}");
                let data: Vec<u8> = held.into_values().flatten().collect();
                assert_eq!(data.len() as u64, entry.size);
                assert_eq!(*hash, blake3::hash(&data), "the hash of file {file}");
                files.insert(file, data);
            }
        }
        (files, skipped)
    }

    /// Checks that the frames carry exactly the files in `selection`, byte for byte.
    fn assert_sends_everything(selection: &Selection, frames: &[Frame]) {
        let (files, skipped) = assemble(selection, frames);
        assert!(skipped.is_empty(), "{skipped:?}");
        for (file, data) in files {
            let path = selection.source(file).unwrap();
            assert_eq!(data, fs::read(path).unwrap(), "file {file}");
        }
    }

    const MIB: usize = 1 << 20;

    #[test]
    fn files_of_every_size_that_matters_are_sent_whole_with_the_hash_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let selection = tree(
            dir.path(),
            &[
                ("empty", 0),
                ("one", 1),
                ("kib", 1024),
                ("just-under", MIB - 1),
                ("a-span", MIB),
                ("a-span-and-one", MIB + 1),
                ("three-and-a-bit", 3 * MIB + 17),
                ("in/a/folder", 5),
            ],
        );
        for lanes in [1, 3, 8] {
            let (producer, frames) = produce(&selection, lanes);
            assert_sends_everything(&selection, &frames);
            assert_eq!(producer.totals().ended_files, 8);
            assert_eq!(producer.totals().skipped_files, 0);
            assert_eq!(producer.skipped(), Vec::new());
        }
    }

    #[test]
    fn many_small_files_share_chunks_and_a_big_file_gets_one_each() {
        let dir = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..1000).map(|n| format!("small/{n:04}")).collect();
        let mut files: Vec<(&str, usize)> = names.iter().map(|n| (n.as_str(), 10 << 10)).collect();
        files.push(("zzz-big", 3 * MIB));
        let selection = tree(dir.path(), &files);

        let producer = Producer::new(Arc::clone(&selection));
        let mut chunk = Chunk::new();
        let mut chunks = 0;
        let mut longest = 0;
        while producer.fill(&mut chunk) {
            chunks += 1;
            longest = longest.max(chunk.bytes().len());
            assert!(chunk.payload() <= CHUNK_PAYLOAD);
        }
        // 1000 files of 10 KiB are 10 MB: about ten chunks, and the big file is three more.
        assert!((10..=16).contains(&chunks), "{chunks} chunks");
        assert!(longest <= CAPACITY);
    }

    #[test]
    fn a_chunk_never_holds_more_than_its_pieces_even_when_every_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..700).map(|n| format!("e{n:03}")).collect();
        let files: Vec<(&str, usize)> = names.iter().map(|n| (n.as_str(), 0)).collect();
        let selection = tree(dir.path(), &files);

        let producer = Producer::new(Arc::clone(&selection));
        let mut chunk = Chunk::new();
        let mut chunks = 0;
        while producer.fill(&mut chunk) {
            chunks += 1;
            assert!(chunk.bytes().len() <= CHUNK_PIECES * END_LEN);
        }
        assert_eq!(chunks, 3);
        assert_eq!(producer.totals().ended_files, 700);
    }

    #[test]
    fn a_file_that_is_gone_or_changed_is_skipped_and_the_others_are_sent() {
        let dir = tempfile::tempdir().unwrap();
        let selection = tree(
            dir.path(),
            &[
                ("a-gone", 100),
                ("b-fine", 100),
                ("c-edited", 100),
                ("d-grown", MIB + 5),
                ("e-fine-too", 2 * MIB),
            ],
        );
        let root = dir.path().join("t");
        fs::remove_file(root.join("a-gone")).unwrap();
        let file = fs::OpenOptions::new()
            .write(true)
            .open(root.join("c-edited"))
            .unwrap();
        let modified = fs::metadata(root.join("c-edited"))
            .unwrap()
            .modified()
            .unwrap();
        file.set_modified(modified + Duration::from_secs(1))
            .unwrap();
        fs::write(root.join("d-grown"), bytes(9, 2 * MIB)).unwrap();

        let (producer, frames) = produce(&selection, 4);

        let (files, skipped) = assemble(&selection, &frames);
        // Position 0 is the folder.
        assert_eq!(
            skipped,
            BTreeMap::from([
                (1, SkipReason::Missing),
                (3, SkipReason::Changed),
                (4, SkipReason::Changed)
            ])
        );
        assert_eq!(files.keys().copied().collect::<Vec<_>>(), [2, 5]);
        assert_eq!(producer.totals().ended_files, 2);
        assert_eq!(producer.totals().skipped_files, 3);
        let mut why = producer.skipped();
        why.sort_by_key(|(file, _)| *file);
        assert_eq!(
            why,
            [
                (1, SkipReason::Missing),
                (3, SkipReason::Changed),
                (4, SkipReason::Changed)
            ]
        );
    }

    #[test]
    fn a_large_file_that_is_cut_short_after_its_first_span_is_skipped_once_and_never_ends() {
        let dir = tempfile::tempdir().unwrap();
        let selection = tree(dir.path(), &[("a-big", 3 * MIB), ("b-after", 10)]);

        let producer = Producer::new(Arc::clone(&selection));
        let mut chunk = Chunk::new();
        assert!(producer.fill(&mut chunk));
        let first = frames(&selection, chunk.bytes());
        assert!(matches!(
            first.as_slice(),
            [Frame::Span {
                file: 1,
                span: 0,
                ..
            }]
        ));

        let file = fs::OpenOptions::new()
            .write(true)
            .open(dir.path().join("t/a-big"))
            .unwrap();
        file.set_len(MIB as u64 + 3).unwrap();
        let mut rest = Vec::new();
        while producer.fill(&mut chunk) {
            rest.extend(frames(&selection, chunk.bytes()));
        }

        let skips = rest
            .iter()
            .filter(|frame| matches!(frame, Frame::Skip { file: 1, .. }))
            .count();
        assert_eq!(skips, 1, "{rest:?}");
        assert!(
            !rest
                .iter()
                .any(|frame| matches!(frame, Frame::End { file: 1, .. }))
        );
        // The file after it is sent.
        assert!(
            rest.iter()
                .any(|frame| matches!(frame, Frame::End { file: 2, .. }))
        );
        assert_eq!(producer.totals().skipped_files, 1);
    }

    #[test]
    fn a_span_that_is_read_after_its_file_failed_in_another_lane_is_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let selection = tree(dir.path(), &[("a-big", 3 * MIB), ("b-after", 10)]);
        let producer = Producer::new(Arc::clone(&selection));

        // One lane has taken the first span of the big file and is about to read it ...
        let Claim::Piece(piece) = producer.book.claim(CHUNK_PAYLOAD) else {
            panic!("the first piece");
        };
        assert_eq!((piece.file, piece.span), (1, 0));
        // ... when another lane fails the file, and its `Skip` is that lane's.
        assert!(producer.book.fail(1));

        let mut chunk = Chunk::new();
        producer.read(piece, &mut chunk);
        assert!(chunk.is_empty(), "{:?}", frames(&selection, chunk.bytes()));
        assert_eq!(chunk.payload(), 0);

        // The file is not sent by the lanes that come after, and the others are.
        let mut rest = Vec::new();
        while producer.fill(&mut chunk) {
            rest.extend(frames(&selection, chunk.bytes()));
        }
        assert!(rest.iter().all(|frame| match frame {
            Frame::Span { file, .. } | Frame::End { file, .. } | Frame::Skip { file, .. } =>
                *file == 2,
        }));
        assert!(
            rest.iter()
                .any(|frame| matches!(frame, Frame::End { file: 2, .. }))
        );
        assert_eq!(producer.totals().skipped_files, 1);
    }

    #[test]
    fn a_read_that_fails_after_another_lane_failed_the_file_sends_no_skip() {
        let dir = tempfile::tempdir().unwrap();
        let selection = tree(dir.path(), &[("a-big", 3 * MIB), ("b-after", 10)]);
        let producer = Producer::new(Arc::clone(&selection));

        // Two lanes hold a span of the big file each ...
        let Claim::Piece(mine) = producer.book.claim(CHUNK_PAYLOAD) else {
            panic!("the first piece");
        };
        let Claim::Piece(theirs) = producer.book.claim(CHUNK_PAYLOAD) else {
            panic!("the second piece");
        };
        assert_eq!(
            (mine.file, mine.span, theirs.file, theirs.span),
            (1, 0, 1, 1)
        );
        // ... the other one fails the file first, and its `Skip` is that lane's. The file is gone
        // before this lane reads, so that this read fails too. A second `Skip` for a file in one
        // attempt is a violation that the receiver ends the transfer for (section 8), so this
        // lane sends none.
        assert!(producer.book.fail(1));
        fs::remove_file(dir.path().join("t/a-big")).unwrap();
        let mut chunk = Chunk::new();
        producer.read(mine, &mut chunk);

        assert!(chunk.is_empty(), "{:?}", frames(&selection, chunk.bytes()));
        assert_eq!(producer.skipped(), Vec::new());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_and_a_pipe_that_took_the_place_of_files_are_skipped_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::write(&outside, b"must not be sent").unwrap();
        let selection = tree(
            dir.path(),
            &[("a-link", 20), ("b-pipe", 20), ("c-file", 20)],
        );
        let root = dir.path().join("t");
        fs::remove_file(root.join("a-link")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("a-link")).unwrap();
        fs::remove_file(root.join("b-pipe")).unwrap();
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join("b-pipe"))
                .status()
                .unwrap()
                .success()
        );

        let (_, frames) = produce(&selection, 2);

        let (files, skipped) = assemble(&selection, &frames);
        assert_eq!(
            skipped,
            BTreeMap::from([(1, SkipReason::Unreadable), (2, SkipReason::Unreadable)])
        );
        assert_eq!(files.keys().copied().collect::<Vec<_>>(), [3]);
        assert!(!frames.iter().any(|frame| matches!(
            frame,
            Frame::Span { data, .. } if data.starts_with(b"must not")
        )));
    }

    #[test]
    fn lanes_that_read_at_the_same_time_send_every_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..120).map(|n| format!("f{n:03}")).collect();
        let sizes = [0, 7, 4096, MIB - 1, MIB, MIB + 1, 2 * MIB + 3, 90_000];
        let files: Vec<(&str, usize)> = names
            .iter()
            .enumerate()
            .map(|(n, name)| (name.as_str(), sizes[n % sizes.len()]))
            .collect();
        let selection = tree(dir.path(), &files);
        for _ in 0..5 {
            let (producer, frames) = produce(&selection, 8);
            assert_sends_everything(&selection, &frames);
            assert_eq!(producer.totals().ended_files, 120);
        }
    }

    #[test]
    fn open_files_are_let_go_of_when_their_file_is_settled() {
        let dir = tempfile::tempdir().unwrap();
        let selection = tree(dir.path(), &[("one", 3 * MIB), ("two", 3 * MIB)]);
        let (producer, _) = produce(&selection, 4);
        assert!(lock(&producer.shared).is_empty());
    }
}
