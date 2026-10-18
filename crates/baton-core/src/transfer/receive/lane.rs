// SPDX-License-Identifier: Apache-2.0

//! Reading the frames of one lane.
//!
//! A lane is a stream of frames after its header (`baton_proto::transfer::lane`). [`LaneReader`]
//! reads them one at a time off any stream that can be read asynchronously, and applies the
//! rules that need nothing but the list: where a piece of the list may come, which frames may
//! come before the list is complete, and how many bytes a span has. What needs the book of the
//! attempt (an `End` for a folder, a file that is ended twice) is the [`Ledger`](super::Ledger)'s.
//!
//! The reader allocates nothing on the word of the sender, other than a piece of the list, which
//! grows as its bytes arrive and is at most the size of a frame. The data of a span is read into a
//! buffer of the largest span, which the caller gives back when it is done with it, so that a
//! lane that carries many spans allocates once.

use std::{
    fmt, io,
    ops::Deref,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use baton_proto::{
    frame::{self, HEADER_LEN},
    transfer::{
        hash::SPAN_LEN,
        lane::{FrameHead, SkipReason},
        list::{ListChunk, Listing},
    },
};
use blake3::Hash;
use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

use super::Violation;

/// The most bytes a span has.
#[allow(clippy::cast_possible_truncation)] // 1 MiB fits in a `usize` on every system.
const SPAN_BYTES: usize = SPAN_LEN as usize;

/// The bytes of one span.
///
/// It is a buffer of the largest span, filled as far as this span goes. It dereferences to the
/// bytes of the span, and nothing else of the buffer can be seen.
pub struct SpanBuf {
    bytes: Box<[u8]>,
    len: usize,
}

impl SpanBuf {
    fn new() -> Self {
        Self {
            bytes: vec![0; SPAN_BYTES].into_boxed_slice(),
            len: 0,
        }
    }
}

impl Deref for SpanBuf {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or_default()
    }
}

// The bytes of a file are the person's, so they are not printed.
impl fmt::Debug for SpanBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpanBuf")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

/// A frame of a lane, with the bytes that belong to it.
#[derive(Debug)]
pub enum Frame {
    /// A piece of the list of files. It has been decoded, and it has not been checked against the
    /// offer: that is for the list checker, which sees the pieces in order.
    List(ListChunk),
    /// Span `span` of file `file`, with all its bytes. The list says that the file is one, and that
    /// the file has the span.
    Span {
        /// Position of the file in the list.
        file: u32,
        /// Number of the span in the file.
        span: u32,
        /// The bytes of the span, exactly as many as the list says. Give the buffer back with
        /// [`LaneReader::recycle`] when it is written.
        data: SpanBuf,
    },
    /// Every span of file `file` has been sent, and its hash is `hash`.
    End {
        /// Position of the file in the list. Whether it is a file is for the book to say.
        file: u32,
        /// The hash of the whole file.
        hash: Hash,
    },
    /// File `file` will not be sent. Whether it is a file is for the book to say.
    Skip {
        /// Position of the file in the list.
        file: u32,
        /// Why not. The receiver has no use for it other than a log line.
        reason: SkipReason,
    },
}

/// Why a frame could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Fault {
    /// The sender broke a rule of the transfer.
    #[error(transparent)]
    Violation(#[from] Violation),
    /// The lane ended in the middle of a frame, without an error: the sender finished it too
    /// early. A sender that does that broke a rule of the transfer ([`Violation::Cut`]), and the
    /// caller says so. A lane that is reset is [`Io`](Self::Io) instead.
    #[error("the lane ended in the middle of a frame")]
    Cut,
    /// The stream failed, as when the sender reset it or the connection was lost. The error is the
    /// stream's own.
    #[error("the lane failed: {0}")]
    Io(#[from] io::Error),
}

/// A stream that counts the bytes read from it, into a counter that others can read.
///
/// The counter of a transfer is the sum of its lanes. It is how the receiver tells that a
/// transfer has stopped moving (section 8, Time).
#[derive(Debug)]
pub struct Counted<R> {
    inner: R,
    bytes: Arc<AtomicU64>,
}

impl<R> Counted<R> {
    /// Wraps `inner`, and adds every byte read from it to `bytes`.
    pub const fn new(inner: R, bytes: Arc<AtomicU64>) -> Self {
        Self { inner, bytes }
    }

    /// Gives back the stream that was wrapped.
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for Counted<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(poll, Poll::Ready(Ok(()))) {
            let read = buf.filled().len().saturating_sub(before);
            self.bytes
                .fetch_add(u64::try_from(read).unwrap_or(u64::MAX), Ordering::Relaxed);
        }
        poll
    }
}

/// Reads the frames of one lane.
///
/// [`next`](Self::next) is not safe to cancel. A call that is dropped before it returned has read
/// part of a frame, and the lane is lost: the caller stops it, as it does with a lane that ends.
#[derive(Debug)]
pub struct LaneReader<R> {
    stream: R,
    lane: u8,
    /// A buffer that was given back, for the next span.
    spare: Option<SpanBuf>,
}

impl<R: AsyncRead + Unpin> LaneReader<R> {
    /// Reads lane number `lane` from `stream`, which is positioned just after the header of the
    /// lane.
    pub const fn new(stream: R, lane: u8) -> Self {
        Self {
            stream,
            lane,
            spare: None,
        }
    }

    /// Gives back the buffer of a span that has been written, to be used for the next one.
    pub fn recycle(&mut self, mut buf: SpanBuf) {
        buf.len = 0;
        self.spare = Some(buf);
    }

    /// Gives back the stream, as it is.
    pub fn into_inner(self) -> R {
        self.stream
    }

    /// Reads the next frame, or `None` if the lane ended after the last one.
    ///
    /// `listing` is the list of the transfer once it is complete, and `None` until then. Until
    /// then only lane 0 is read, and nothing but pieces of the list may come on it. After that, a
    /// piece of the list may not come at all.
    pub async fn next(&mut self, listing: Option<&Listing>) -> Result<Option<Frame>, Fault> {
        let mut tag = [0_u8; 1];
        if self.stream.read(&mut tag).await? == 0 {
            return Ok(None);
        }
        let [tag] = tag;
        let unknown = || Violation::BadTag(tag);
        let rest_len = FrameHead::rest_len(tag).map_err(|_| unknown())?;
        let mut rest = [0_u8; FrameHead::MAX_REST_LEN];
        let rest = rest.get_mut(..rest_len).ok_or_else(unknown)?;
        self.fill(rest).await?;
        let head = FrameHead::decode(tag, rest).map_err(|_| unknown())?;

        match head {
            FrameHead::List => {
                if self.lane != 0 || listing.is_some() {
                    return Err(Violation::ListOutOfPlace.into());
                }
                Ok(Some(Frame::List(self.read_chunk().await?)))
            }
            FrameHead::Span { file, span } => {
                let listing = listing.ok_or(Violation::BeforeTheList)?;
                let len = listing
                    .span_size(file, span)
                    .and_then(|len| usize::try_from(len).ok())
                    .ok_or(Violation::NoSuchSpan)?;
                let mut data = self.spare.take().unwrap_or_else(SpanBuf::new);
                data.len = 0;
                let bytes = data.bytes.get_mut(..len).ok_or(Violation::NoSuchSpan)?;
                self.fill(bytes).await?;
                data.len = len;
                Ok(Some(Frame::Span { file, span, data }))
            }
            FrameHead::End { file, hash } => {
                listing.ok_or(Violation::BeforeTheList)?;
                Ok(Some(Frame::End { file, hash }))
            }
            FrameHead::Skip { file, reason } => {
                listing.ok_or(Violation::BeforeTheList)?;
                Ok(Some(Frame::Skip { file, reason }))
            }
        }
    }

    /// Reads a frame of section 2 that holds a piece of the list.
    async fn read_chunk(&mut self) -> Result<ListChunk, Fault> {
        let mut header = [0_u8; HEADER_LEN];
        self.fill(&mut header).await?;
        let len = frame::payload_len::<ListChunk>(header).map_err(|_| Violation::BadChunk)?;
        // The length is the sender's word. The buffer grows as the bytes arrive.
        let mut payload = Vec::new();
        let wanted = u64::try_from(len).map_err(|_| Violation::BadChunk)?;
        let read = (&mut self.stream)
            .take(wanted)
            .read_to_end(&mut payload)
            .await?;
        if read < len {
            return Err(Fault::Cut);
        }
        Ok(frame::decode_payload::<ListChunk>(&payload).map_err(|_| Violation::BadChunk)?)
    }

    /// Fills `buf` from the stream, or says that the lane ended before it was full.
    async fn fill(&mut self, buf: &mut [u8]) -> Result<(), Fault> {
        match self.stream.read_exact(buf).await {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Err(Fault::Cut),
            Err(e) => Err(Fault::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::transfer::lane::{TAG_END, TAG_LIST, TAG_SKIP, TAG_SPAN};
    use tokio::io::AsyncWriteExt;

    use super::*;
    use crate::transfer::testing::{
        bytes, chunk, end, file, folder, lane, list, listing, skip, span,
    };

    /// `a` (a folder), `a/x` (10 bytes), `a/y` (a mebibyte and 3 bytes), `z` (5 bytes), `zero`
    /// (empty).
    const A: u32 = 0;
    const X: u32 = 1;
    const Y: u32 = 2;
    const Z: u32 = 3;
    const ZERO: u32 = 4;
    const MIB: usize = 1 << 20;

    fn sample() -> Listing {
        listing(&[
            folder("a"),
            file("a/x", 10),
            file("a/y", MIB as u64 + 3),
            file("z", 5),
            file("zero", 0),
        ])
    }

    async fn next_of(
        lane: &[u8],
        number: u8,
        listing: Option<&Listing>,
    ) -> Result<Option<Frame>, Fault> {
        LaneReader::new(lane, number).next(listing).await
    }

    fn violation(result: Result<Option<Frame>, Fault>) -> Violation {
        match result {
            Err(Fault::Violation(violation)) => violation,
            other => panic!("expected a violation, got {other:?}"),
        }
    }

    // What comes before the list is complete.

    #[tokio::test]
    async fn a_lane_that_has_nothing_in_it_ends_cleanly() {
        assert!(next_of(&[], 0, None).await.unwrap().is_none());
        assert!(next_of(&[], 3, Some(&sample())).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn pieces_of_the_list_come_on_lane_0_until_the_list_is_complete() {
        let first = chunk(vec![folder("a"), file("a/x", 10)]);
        let second = chunk(vec![file("z", 5)]);
        let bytes = lane(&[&list(&first), &list(&second)]);
        let mut reader = LaneReader::new(&bytes[..], 0);

        let Some(Frame::List(got)) = reader.next(None).await.unwrap() else {
            panic!("a piece of the list");
        };
        assert_eq!(got, first);
        let Some(Frame::List(got)) = reader.next(None).await.unwrap() else {
            panic!("a piece of the list");
        };
        assert_eq!(got, second);
        assert!(reader.next(None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_piece_of_the_list_on_another_lane_is_a_violation() {
        let bytes = list(&chunk(vec![file("z", 5)]));

        for number in [1, 2, 7] {
            assert_eq!(
                violation(next_of(&bytes, number, None).await),
                Violation::ListOutOfPlace,
                "lane {number}"
            );
        }
    }

    #[tokio::test]
    async fn a_piece_of_the_list_after_the_list_is_complete_is_a_violation() {
        let bytes = list(&chunk(vec![file("z", 5)]));

        assert_eq!(
            violation(next_of(&bytes, 0, Some(&sample())).await),
            Violation::ListOutOfPlace
        );
    }

    #[tokio::test]
    async fn a_span_an_end_or_a_skip_before_the_list_is_complete_is_a_violation() {
        for bytes in [
            span(Z, 0, b"12345"),
            end(Z, 1),
            skip(Z, SkipReason::Missing),
        ] {
            assert_eq!(
                violation(next_of(&bytes, 0, None).await),
                Violation::BeforeTheList
            );
        }
    }

    // The frames themselves.

    #[tokio::test]
    async fn a_span_has_the_bytes_that_the_list_gives_and_the_next_frame_starts_after_them() {
        let listing = sample();
        let bytes = lane(&[
            &span(X, 0, b"0123456789"),
            &span(Z, 0, b"abcde"),
            &end(X, 4),
        ]);
        let mut reader = LaneReader::new(&bytes[..], 1);

        let Some(Frame::Span { file, span, data }) = reader.next(Some(&listing)).await.unwrap()
        else {
            panic!("a span");
        };
        assert_eq!((file, span), (X, 0));
        assert_eq!(&*data, b"0123456789");
        let Some(Frame::Span { file, span, data }) = reader.next(Some(&listing)).await.unwrap()
        else {
            panic!("a span");
        };
        assert_eq!((file, span), (Z, 0));
        assert_eq!(&*data, b"abcde");
        let Some(Frame::End { file, hash }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("an end");
        };
        assert_eq!(file, X);
        assert_eq!(hash, Hash::from_bytes([4; blake3::OUT_LEN]));
        assert!(reader.next(Some(&listing)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_last_span_of_a_large_file_is_shorter_than_the_others() {
        let listing = sample();
        let first = bytes(1, MIB);
        let second = bytes(2, 3);
        let bytes = lane(&[&span(Y, 0, &first), &span(Y, 1, &second)]);
        let mut reader = LaneReader::new(&bytes[..], 0);

        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };
        assert_eq!(&*data, &first[..]);
        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };
        assert_eq!(&*data, &second[..]);
    }

    #[tokio::test]
    async fn an_end_and_a_skip_are_read_and_the_reason_for_a_skip_is_kept() {
        let listing = sample();
        let bytes = lane(&[
            &end(ZERO, 9),
            &skip(X, SkipReason::Changed),
            &skip(Z, SkipReason::Other),
        ]);
        let mut reader = LaneReader::new(&bytes[..], 2);

        let Some(Frame::End { file, hash }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("an end");
        };
        assert_eq!(file, ZERO);
        assert_eq!(hash.as_bytes(), &[9; blake3::OUT_LEN]);
        let Some(Frame::Skip { file, reason }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a skip");
        };
        assert_eq!((file, reason), (X, SkipReason::Changed));
        let Some(Frame::Skip { file, reason }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a skip");
        };
        assert_eq!((file, reason), (Z, SkipReason::Other));
    }

    #[tokio::test]
    async fn an_end_or_a_skip_for_a_folder_is_read_and_left_to_the_book() {
        let listing = sample();
        let bytes = lane(&[&end(A, 1), &skip(900, SkipReason::Missing)]);
        let mut reader = LaneReader::new(&bytes[..], 0);

        assert!(matches!(
            reader.next(Some(&listing)).await.unwrap(),
            Some(Frame::End { file: A, .. })
        ));
        assert!(matches!(
            reader.next(Some(&listing)).await.unwrap(),
            Some(Frame::Skip { file: 900, .. })
        ));
    }

    #[tokio::test]
    async fn a_span_of_a_folder_a_file_not_in_the_list_or_a_span_not_in_the_file_is_a_violation() {
        let listing = sample();

        for (file, number) in [(A, 0), (99, 0), (u32::MAX, 0), (Z, 1), (X, 1), (ZERO, 0)] {
            let bytes = span(file, number, b"12345");
            assert_eq!(
                violation(next_of(&bytes, 0, Some(&listing)).await),
                Violation::NoSuchSpan,
                "file {file} span {number}"
            );
        }
    }

    #[tokio::test]
    async fn a_tag_that_is_not_known_is_a_violation_even_if_it_is_alone() {
        for tag in [0_u8, 5, 9, 255] {
            assert_eq!(
                violation(next_of(&[tag], 0, Some(&sample())).await),
                Violation::BadTag(tag)
            );
            assert_eq!(
                violation(next_of(&[tag], 0, None).await),
                Violation::BadTag(tag)
            );
        }
    }

    // A lane that ends or fails in the middle of a frame.

    #[tokio::test]
    async fn a_lane_that_ends_inside_a_frame_is_cut_wherever_it_ends() {
        let listing = sample();
        let first = span(X, 0, b"0123456789");
        let whole = lane(&[&first, &end(X, 3)]);

        for len in 1..whole.len() {
            // A lane that ends between two frames ends cleanly: that is the one place it may.
            if len == first.len() {
                continue;
            }
            let mut reader = LaneReader::new(&whole[..len], 1);
            let mut frames = 0;
            let fault = loop {
                match reader.next(Some(&listing)).await {
                    Ok(Some(_)) => frames += 1,
                    Ok(None) => panic!("{len} bytes: a cut lane ended cleanly"),
                    Err(fault) => break fault,
                }
            };
            assert!(matches!(fault, Fault::Cut), "{len} bytes: {fault:?}");
            assert_eq!(frames, usize::from(len > first.len()), "{len} bytes");
        }
    }

    #[tokio::test]
    async fn a_lane_that_ends_between_two_frames_ends_cleanly() {
        let listing = sample();
        let first = span(X, 0, b"0123456789");
        let whole = lane(&[&first, &end(X, 3)]);
        let mut reader = LaneReader::new(&whole[..first.len()], 1);

        assert!(matches!(
            reader.next(Some(&listing)).await.unwrap(),
            Some(Frame::Span { .. })
        ));
        assert!(reader.next(Some(&listing)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_piece_of_the_list_that_is_cut_short_is_cut_and_not_a_violation() {
        let whole = list(&chunk(vec![file("z", 5), file("zz", 6)]));

        for len in 1..whole.len() {
            let result = next_of(&whole[..len], 0, None).await;
            assert!(matches!(result, Err(Fault::Cut)), "{len} bytes: {result:?}");
        }
    }

    /// A stream that gives `bytes` and then fails as a stream does that the other side reset.
    struct Resets<'a>(&'a [u8]);

    impl AsyncRead for Resets<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.0.is_empty() {
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset")));
            }
            let len = self.0.len().min(buf.remaining());
            let (now, later) = self.0.split_at(len);
            buf.put_slice(now);
            self.0 = later;
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn a_stream_that_fails_is_an_io_fault_at_the_start_or_inside_a_frame() {
        let listing = sample();
        let whole = span(X, 0, b"0123456789");

        for len in [0, 1, 5, whole.len() - 1] {
            let mut reader = LaneReader::new(Resets(&whole[..len]), 0);
            match reader.next(Some(&listing)).await {
                Err(Fault::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::ConnectionReset),
                other => panic!("{len} bytes: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_stream_that_fails_inside_a_piece_of_the_list_is_an_io_fault() {
        let whole = list(&chunk(vec![file("z", 5)]));

        let mut reader = LaneReader::new(Resets(&whole[..whole.len() - 1]), 0);

        assert!(matches!(reader.next(None).await, Err(Fault::Io(_))));
    }

    // A piece of the list that is not valid.

    fn with_length(length: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![TAG_LIST];
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[tokio::test]
    async fn a_piece_of_the_list_that_is_not_valid_is_a_violation() {
        let valid = frame::encode(&chunk(vec![file("z", 5)])).unwrap();
        let payload = valid[HEADER_LEN..].to_vec();
        let mut with_trailing = payload.clone();
        with_trailing.push(0);
        let empty_chunk = frame::encode(&chunk(Vec::new()));
        // A chunk with no entries does not encode, so make the bytes of one by hand: an empty
        // CBOR map for `entries`.
        assert!(empty_chunk.is_err());
        let no_entries = [0xa1, 0x67, b'e', b'n', b't', b'r', b'i', b'e', b's', 0x80];

        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("no payload", with_length(0, &[])),
            (
                "more than a frame holds",
                with_length(2 * 1024 * 1024 + 1, &[]),
            ),
            ("not CBOR", with_length(4, &[0xff, 0xff, 0xff, 0xff])),
            (
                "an extra byte",
                with_length(u32::try_from(with_trailing.len()).unwrap(), &with_trailing),
            ),
            (
                "no entries",
                with_length(u32::try_from(no_entries.len()).unwrap(), &no_entries),
            ),
        ];
        for (what, bytes) in cases {
            assert_eq!(
                violation(next_of(&bytes, 0, None).await),
                Violation::BadChunk,
                "{what}"
            );
        }
    }

    #[tokio::test]
    async fn a_piece_that_says_it_is_larger_than_it_is_gets_nothing_allocated_for_the_difference() {
        // The sender says 2 MiB and sends 5 bytes before it finishes the lane.
        let bytes = with_length(2 * 1024 * 1024, &[1, 2, 3, 4, 5]);

        let result = next_of(&bytes, 0, None).await;

        assert!(matches!(result, Err(Fault::Cut)), "{result:?}");
    }

    // The buffer, the counter and streams that give a few bytes at a time.

    #[tokio::test]
    async fn a_buffer_that_is_given_back_is_used_again_for_a_span_of_any_size() {
        let listing = sample();
        let big = bytes(1, MIB);
        let bytes = lane(&[&span(Y, 0, &big), &span(Z, 0, b"abcde")]);
        let mut reader = LaneReader::new(&bytes[..], 0);
        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };
        let address = data.as_ptr();
        reader.recycle(data);

        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };

        assert_eq!(data.as_ptr(), address, "the same buffer");
        assert_eq!(&*data, b"abcde", "and only the bytes of this span");
    }

    #[tokio::test]
    async fn a_span_buffer_shows_nothing_of_what_it_held_before() {
        let listing = sample();
        let bytes = lane(&[&span(X, 0, b"0123456789"), &span(Z, 0, b"abcde")]);
        let mut reader = LaneReader::new(&bytes[..], 0);
        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };
        reader.recycle(data);

        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };

        assert_eq!(data.len(), 5);
        assert_eq!(&*data, b"abcde");
        // The print has the length and none of the bytes, in any form.
        assert_eq!(format!("{data:?}"), "SpanBuf { len: 5, .. }");
    }

    #[tokio::test]
    async fn every_byte_read_is_counted_even_those_of_a_frame_that_is_cut() {
        let listing = sample();
        let whole = lane(&[&span(X, 0, b"0123456789"), &end(X, 3)]);
        let counter = Arc::new(AtomicU64::new(0));
        let mut reader = LaneReader::new(Counted::new(&whole[..], counter.clone()), 0);
        while reader.next(Some(&listing)).await.unwrap().is_some() {}
        assert_eq!(counter.load(Ordering::Relaxed), whole.len() as u64);

        let counter = Arc::new(AtomicU64::new(0));
        let cut = &whole[..whole.len() - 5];
        let mut reader = LaneReader::new(Counted::new(cut, counter.clone()), 0);
        while reader.next(Some(&listing)).await.is_ok() {}
        assert_eq!(counter.load(Ordering::Relaxed), cut.len() as u64);
    }

    #[tokio::test]
    async fn the_stream_can_be_taken_back_with_what_was_not_read() {
        let listing = sample();
        let bytes = lane(&[&end(X, 1), &end(Z, 2)]);
        let counter = Arc::new(AtomicU64::new(0));
        let mut reader = LaneReader::new(Counted::new(&bytes[..], counter), 0);
        reader.next(Some(&listing)).await.unwrap();

        let rest = reader.into_inner().into_inner();

        assert_eq!(rest, &end(Z, 2)[..]);
    }

    #[tokio::test]
    async fn a_lane_that_arrives_a_few_bytes_at_a_time_is_read_the_same() {
        let listing = sample();
        let first = bytes(3, MIB);
        let tail = bytes(4, 3);
        let pieces = chunk(vec![folder("a"), file("a/x", 10)]);
        let whole = lane(&[
            &list(&pieces),
            &span(Y, 0, &first),
            &span(Y, 1, &tail),
            &end(Y, 8),
            &skip(X, SkipReason::Unreadable),
        ]);
        let (mut tx, rx) = tokio::io::duplex(7);
        let sent = whole.clone();
        let writer = tokio::spawn(async move {
            // In pieces of different sizes, so that frames are cut at every kind of place.
            for (index, part) in sent.chunks(5).enumerate() {
                tx.write_all(part).await.unwrap();
                if index % 50 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });
        let mut reader = LaneReader::new(rx, 0);

        let Some(Frame::List(got)) = reader.next(None).await.unwrap() else {
            panic!("a piece of the list");
        };
        assert_eq!(got, pieces);
        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };
        assert_eq!(&*data, &first[..]);
        reader.recycle(data);
        let Some(Frame::Span { data, .. }) = reader.next(Some(&listing)).await.unwrap() else {
            panic!("a span");
        };
        assert_eq!(&*data, &tail[..]);
        assert!(matches!(
            reader.next(Some(&listing)).await.unwrap(),
            Some(Frame::End { file: Y, .. })
        ));
        assert!(matches!(
            reader.next(Some(&listing)).await.unwrap(),
            Some(Frame::Skip {
                file: X,
                reason: SkipReason::Unreadable
            })
        ));
        writer.await.unwrap();
        assert!(reader.next(Some(&listing)).await.unwrap().is_none());
    }

    #[test]
    fn the_tags_in_these_tests_are_those_of_the_protocol() {
        assert_eq!([TAG_LIST, TAG_SPAN, TAG_END, TAG_SKIP], [1, 2, 3, 4]);
    }
}
