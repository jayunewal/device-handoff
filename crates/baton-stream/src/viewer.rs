// SPDX-License-Identifier: Apache-2.0

//! The viewer's choice of which frame to decode next.
//!
//! Frames arrive on streams of their own, so they arrive in any order, and some never arrive
//! because the source reset them. Every header says which frame has to be decoded before it
//! ([`FrameHeader::depends_on`]), and that is all the viewer needs: it decodes a frame as soon as
//! the frame it depends on has been decoded, and shows the newest picture. It does not wait for a
//! frame that may be late, because the source tells it in the next frame what that one needs.
//!
//! The viewer never decodes backwards. A decoder takes the frames in the order in which they were
//! encoded, and a reference frame that is decoded late changes what the frames after it are
//! decoded against. So a frame that is numbered below one that has been decoded is dropped, also
//! one that the source reset and that arrived whole all the same, and so is a frame that waits
//! for a frame that is numbered below one that has been decoded. A late frame is not a reason to
//! ask for a key frame: only a decoder that fails is, and a frame that can never be decoded
//! although the source counted on it, which means that the two sides disagree about what the
//! viewer holds.
//!
//! The viewer also bounds what a source can make it hold. A picture has to be whole before it is
//! decoded, so the viewer reads it into memory of its own, and the bytes it has read no longer
//! count against the connection window of the transport. So the viewer counts them itself,
//! whether a stream is still delivering a picture or the picture waits for the frame it depends
//! on, and it takes no frame stream beyond [`Params::max_streams`] and no byte beyond
//! [`Params::max_held_bytes`]. A stream that it does not take is stopped with code 499, like a
//! stream of a session that is not running: that is no fault of the source, because a stream can
//! arrive just as the viewer reaches its count, and the source treats the frame as lost.
//!
//! A frame has a number of its own, and the viewer names a frame by its number in every action, so
//! a number that comes twice is a [`Violation`] and ends the session: the viewer could not say which
//! of two pictures an action means. Only a number that it still holds, or has decoded since the last
//! key frame, can be recognised. A frame with any other number is taken like any other, and is
//! dropped as late if its number is below that of a frame that has been decoded. That includes the
//! number of a frame that was dropped while it waited: the caller let go of the first copy when the
//! `Discard` told it to, so the second is counted as a new frame.
//!
//! Like the sender, this is a state machine without input or output. It is told when a frame
//! stream starts, how much of a picture has been read, and when a frame has arrived or its stream
//! broke off, and says what to do with the frames.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use baton_proto::limits::MAX_PICTURE_BYTES;

use crate::header::{FrameHeader, FrameKind};

/// How many frames may wait for the frame they depend on.
const MAX_PENDING: usize = 256;

/// How many decoded frames to remember.
const MAX_DECODED: usize = 4096;

/// The numbers that the viewer's rules use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// How long to wait before asking again for a key frame that did not come.
    pub key_request_interval: Duration,
    /// How many frame streams are read at once, [`crate::MAX_FRAME_STREAMS`] unless a test says
    /// otherwise. The source keeps six frames in flight and may have reset up to six more that
    /// the viewer still counts.
    pub max_streams: usize,
    /// How many bytes of pictures are held, whether a stream is still delivering them or they
    /// wait for the frame they depend on. It is the connection window of the reference
    /// implementation, which is what a paired device can otherwise make a device hold for each
    /// connection.
    pub max_held_bytes: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            key_request_interval: Duration::from_millis(500),
            max_streams: crate::MAX_FRAME_STREAMS,
            max_held_bytes: 16 * 1024 * 1024,
        }
    }
}

/// A frame stream that is being read. It is the viewer's own number for it, and is not used
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId(u64);

/// Why the viewer does not take a frame stream, or the rest of one. The stream is forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// As many frame streams are open as the viewer allows, or the pictures that it holds would
    /// go beyond its limit. Stop the stream with code 499 and read no more of it. The source did
    /// not break a rule.
    #[error("the viewer is at its limit")]
    OverBudget,
    /// The picture is larger than the limit of section 6 of the protocol. Stop the stream with
    /// code 413.
    #[error("the picture is larger than the limit")]
    TooLarge,
    /// The viewer has no such stream: it never started, or it has ended or been refused.
    #[error("no such frame stream")]
    Unknown,
}

/// What a source did that no source that follows the protocol does. The session is over: the
/// viewer ends it with code 400 and lets go of every picture it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Violation {
    /// A frame number came a second time, while the viewer still held the first frame or had
    /// decoded it since the last key frame. A frame has a number of its own and a frame that was
    /// reset is not sent again. The actions name a frame by its number, so the viewer could not
    /// say which of two pictures with one number to decode or to drop.
    #[error("frame {number} came a second time")]
    RepeatedNumber {
        /// The number of the frame.
        number: u64,
    },
}

/// What the viewer is to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Decode this frame now. The actions come in the order in which frames are to be decoded.
    Decode(u64),
    /// Show the picture of this frame. It is the newest one that was decoded.
    Show(u64),
    /// Forget this frame, which will not be decoded.
    Discard(u64),
    /// Ask the source for a key frame, saying the newest frame that the viewer has tried to
    /// decode. A key frame numbered above it answers the request, and one numbered at or below
    /// it does not: the viewer has tried that one, so it is not on its way.
    RequestKeyFrame {
        /// The number of the newest frame that the viewer has handed to its decoder.
        tried: u64,
    },
}

/// Counts of what happened, for the reports that the viewer sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Frames decoded.
    pub decoded: u64,
    /// Frames that were not decoded: they were replaced, late, or could not be decoded.
    pub discarded: u64,
    /// Requests for a key frame.
    pub key_requests: u64,
}

#[derive(Debug)]
struct Waiting {
    header: FrameHeader,
    /// The bytes of the picture, which the viewer holds until the frame is decoded or dropped.
    bytes: usize,
}

/// The state of one session on the viewer.
#[derive(Debug)]
pub struct Viewer {
    params: Params,
    epoch: u32,
    /// The frame streams that are being read, with the bytes of picture each has delivered.
    arriving: BTreeMap<StreamId, usize>,
    next_stream: u64,
    pending: BTreeMap<u64, Waiting>,
    decoded: BTreeSet<u64>,
    /// The newest key frame that was decoded. What is before it is gone from the decoder.
    last_key: Option<u64>,
    /// The newest frame that was decoded. Nothing below it is decoded any more.
    frontier: Option<u64>,
    shown: Option<u64>,
    /// The decoder failed: only a key frame helps.
    broken: bool,
    last_request: Option<Duration>,
    stats: Stats,
}

impl Viewer {
    /// Starts a session. Nothing can be shown before the first key frame.
    pub fn new(params: Params) -> Self {
        Self {
            params,
            epoch: 0,
            arriving: BTreeMap::new(),
            next_stream: 0,
            pending: BTreeMap::new(),
            decoded: BTreeSet::new(),
            last_key: None,
            frontier: None,
            shown: None,
            broken: false,
            last_request: None,
            stats: Stats::default(),
        }
    }

    /// What has been counted so far.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The newest frame that was shown.
    pub fn shown(&self) -> Option<u64> {
        self.shown
    }

    /// The bytes of pictures that the viewer holds: those that frame streams are still
    /// delivering, and those of frames that wait to be decoded. A picture that has been handed
    /// to the decoder, or dropped, is no longer counted.
    pub fn held_bytes(&self) -> usize {
        let arriving: usize = self.arriving.values().sum();
        let waiting: usize = self.pending.values().map(|w| w.bytes).sum();
        arriving + waiting
    }

    /// The number of frame streams that are being read.
    pub fn streams(&self) -> usize {
        self.arriving.len()
    }

    /// A frame stream has started: its first 17 bytes are read, and they name this session.
    ///
    /// The stream is refused when as many are open as [`Params::max_streams`], or when the
    /// pictures held already fill [`Params::max_held_bytes`]. A refused stream is stopped with
    /// code 499 and not read. Otherwise the id that comes back is used for the rest of the
    /// stream, which ends with [`Viewer::arrived`] or [`Viewer::stream_aborted`].
    pub fn stream_started(&mut self) -> Result<StreamId, Refusal> {
        if self.arriving.len() >= self.params.max_streams
            || self.held_bytes() >= self.params.max_held_bytes
        {
            return Err(Refusal::OverBudget);
        }
        let id = StreamId(self.next_stream);
        self.next_stream += 1;
        self.arriving.insert(id, 0);
        Ok(id)
    }

    /// `len` more bytes of the picture of a frame stream have been read, the header not counted.
    ///
    /// When the picture would be larger than [`MAX_PICTURE_BYTES`], or the pictures held would go
    /// beyond [`Params::max_held_bytes`], the stream is refused and forgotten, with what it
    /// delivered. It is then stopped, with code 413 for the first and 499 for the second, and
    /// not read any more.
    pub fn stream_read(&mut self, id: StreamId, len: usize) -> Result<(), Refusal> {
        let Some(&so_far) = self.arriving.get(&id) else {
            return Err(Refusal::Unknown);
        };
        let total = so_far.saturating_add(len);
        let refusal = if total > MAX_PICTURE_BYTES {
            Some(Refusal::TooLarge)
        } else if self.held_bytes().saturating_add(len) > self.params.max_held_bytes {
            Some(Refusal::OverBudget)
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.arriving.remove(&id);
            return Err(refusal);
        }
        self.arriving.insert(id, total);
        Ok(())
    }

    /// A frame stream ended before its frame was whole: the source reset it, it was stopped, or
    /// the connection went. What it delivered is forgotten.
    pub fn stream_aborted(&mut self, id: StreamId) {
        self.arriving.remove(&id);
    }

    /// The frame of a stream has arrived in full at `now`.
    ///
    /// A stream that the viewer does not have, which means one that was refused, aborted or has
    /// ended, brings a frame that is dropped: the viewer holds nothing that it has not counted.
    /// The `Discard` for such a stream names the picture of that stream, whatever its number, and
    /// is not looked up by number: a frame with the same number may be waiting, and it stays.
    ///
    /// A frame whose number the viewer holds, or has decoded since the last key frame, is a
    /// [`Violation`], whatever its header says. The session is to end, and the picture that came
    /// with it, which is no longer counted, goes with the others. A frame with any other number is
    /// taken like any other, and is dropped as late if its number is below that of a frame that has
    /// been decoded.
    ///
    /// # Errors
    ///
    /// [`Violation::RepeatedNumber`], as above.
    pub fn arrived(
        &mut self,
        now: Duration,
        id: StreamId,
        header: FrameHeader,
    ) -> Result<Vec<Action>, Violation> {
        if let Some(bytes) = self.arriving.remove(&id) {
            return self.arrive(now, header, bytes);
        }
        let mut actions = Vec::new();
        self.discard(header.number, &mut actions);
        Ok(actions)
    }

    fn arrive(
        &mut self,
        now: Duration,
        header: FrameHeader,
        bytes: usize,
    ) -> Result<Vec<Action>, Violation> {
        // Before the epoch and the order: every other way out of here either names this frame in
        // an action or keeps it in `pending`, and a number that is already held would make an
        // action that names it say which of two pictures it means.
        if self.pending.contains_key(&header.number) || self.decoded.contains(&header.number) {
            return Err(Violation::RepeatedNumber {
                number: header.number,
            });
        }
        let mut actions = Vec::new();
        if header.epoch < self.epoch {
            // The picture has changed its size since.
            self.discard(header.number, &mut actions);
            return Ok(actions);
        }
        if self
            .frontier
            .is_some_and(|frontier| header.number < frontier)
        {
            // Late, and a decoder does not go back: a frame that was encoded before one that has
            // been decoded would change what the frames after that one are decoded against.
            self.discard(header.number, &mut actions);
            return Ok(actions);
        }
        match header.kind() {
            FrameKind::Key => {
                self.discard_pending(&mut actions, |w| w.header.number < header.number);
            }
            FrameKind::Recovery => self.discard_pending(&mut actions, |w| {
                // What the old chain was going to say after the frame the recovery depends on is
                // of no use. What comes before it is still needed.
                w.header.chain < header.chain && w.header.number > header.depends_on
            }),
            FrameKind::Delta => {}
        }
        self.pending
            .insert(header.number, Waiting { header, bytes });
        self.drain(now, &mut actions);
        Ok(actions)
    }

    /// The decoder could not decode a frame. Nothing can be decoded until a key frame comes.
    ///
    /// The decoding and showing that an earlier call asked for and that has not been carried out
    /// yet is to be dropped: it depends on the frame that failed. The actions that this call
    /// returns say which frames to forget and whether to ask for a key frame.
    ///
    /// Frames that arrive from now on wait, and are not dropped: the source wrote some of them
    /// after the key frame that is on its way, and counts them as held, so they are needed when
    /// it comes. The ones from before it go when it arrives.
    pub fn decode_failed(&mut self, now: Duration) -> Vec<Action> {
        let mut actions = Vec::new();
        self.broken = true;
        self.discard_pending(&mut actions, |w| !w.header.is_key());
        self.request_key_frame(now, &mut actions);
        actions
    }

    /// Looks at the clock: asks again for a key frame that did not come.
    pub fn tick(&mut self, now: Duration) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.broken {
            self.request_key_frame(now, &mut actions);
        }
        actions
    }

    fn discard(&mut self, number: u64, actions: &mut Vec<Action>) {
        self.stats.discarded += 1;
        actions.push(Action::Discard(number));
    }

    fn discard_pending(&mut self, actions: &mut Vec<Action>, which: impl Fn(&Waiting) -> bool) {
        let doomed: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, w)| which(w))
            .map(|(n, _)| *n)
            .collect();
        for number in doomed {
            self.pending.remove(&number);
            self.discard(number, actions);
        }
    }

    fn request_key_frame(&mut self, now: Duration, actions: &mut Vec<Action>) {
        // Before anything has been tried the first key frame is still to come, and there is
        // nothing to say about what failed.
        let Some(tried) = self.frontier else {
            return;
        };
        let due = self
            .last_request
            .is_none_or(|last| now.saturating_sub(last) >= self.params.key_request_interval);
        if due {
            self.last_request = Some(now);
            self.stats.key_requests += 1;
            actions.push(Action::RequestKeyFrame { tried });
        }
    }

    /// Whether the decoder has everything that the frame needs.
    fn decodable(&self, header: &FrameHeader) -> bool {
        if header.is_key() {
            return true;
        }
        // A frame of a size that has not begun yet waits for its key frame.
        header.epoch == self.epoch
            && !self.broken
            && self.decoded.contains(&header.depends_on)
            && self.last_key.is_none_or(|key| header.depends_on >= key)
    }

    /// Whether the frame can never be decoded: what it depends on is below the newest frame that
    /// was decoded, and was not decoded itself, so it would be dropped if it came.
    fn stranded(&self, header: &FrameHeader) -> bool {
        !self.broken
            && !header.is_key()
            && header.epoch == self.epoch
            && self
                .frontier
                .is_some_and(|frontier| header.depends_on < frontier)
            && !self.decoded.contains(&header.depends_on)
    }

    /// Decodes every frame that can be decoded, oldest first, and shows the newest.
    fn drain(&mut self, now: Duration, actions: &mut Vec<Action>) {
        let mut newest = None;
        while let Some(number) = self
            .pending
            .iter()
            .find(|(_, w)| self.decodable(&w.header))
            .map(|(n, _)| *n)
        {
            let Some(waiting) = self.pending.remove(&number) else {
                break;
            };
            if waiting.header.is_key() {
                // The decoder forgets everything before a key frame, and so does the viewer. A
                // key frame of a new size ends the old size.
                self.epoch = self.epoch.max(waiting.header.epoch);
                self.decoded.clear();
                self.last_key = Some(number);
                self.broken = false;
                // The wait for a key frame is over. A decode error from here on, of this key
                // frame or of a later frame, is a new wait whose first request goes out at once;
                // the interval is for asking again while a key frame does not come. A source that
                // is asked too often keeps the request that comes early and answers it when its
                // own spacing of key frames allows (ADR 0009), so asking at once costs nothing.
                self.last_request = None;
            }
            self.decoded.insert(number);
            while self.decoded.len() > MAX_DECODED {
                self.decoded.pop_first();
            }
            self.frontier = Some(number);
            // Never backwards: what waits below this frame is dropped, and a key frame also
            // makes the pictures of the old size and the old frames useless.
            self.discard_pending(actions, |w| w.header.number < number);
            self.stats.decoded += 1;
            actions.push(Action::Decode(number));
            newest = Some(number);
        }
        // A frame that depends on something that can never be decoded was made when the source
        // counted on the viewer having it. The two sides disagree, and only a key frame mends it.
        let stranded: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, w)| self.stranded(&w.header))
            .map(|(n, _)| *n)
            .collect();
        if !stranded.is_empty() {
            for number in stranded {
                self.pending.remove(&number);
                self.discard(number, actions);
            }
            self.request_key_frame(now, actions);
        }
        while self.pending.len() > MAX_PENDING {
            if let Some((number, _)) = self.pending.pop_first() {
                self.discard(number, actions);
            }
        }
        if let Some(number) = newest.filter(|n| self.shown.is_none_or(|shown| shown < *n)) {
            self.shown = Some(number);
            actions.push(Action::Show(number));
        }
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::id::SessionId;

    use super::Action::{Decode, Discard, RequestKeyFrame, Show};
    use super::*;
    use crate::header::Flags;

    const MS: Duration = Duration::from_millis(1);

    fn header(number: u64, depends_on: u64, chain: u32, flags: Flags) -> FrameHeader {
        FrameHeader {
            session: SessionId::from_bytes([1; SessionId::LEN]),
            number,
            depends_on,
            chain,
            epoch: 0,
            capture_time: 0,
            flags,
        }
    }

    fn key(number: u64, chain: u32) -> FrameHeader {
        header(number, number, chain, Flags::KEY | Flags::REFERENCED)
    }

    fn delta(number: u64, depends_on: u64, chain: u32) -> FrameHeader {
        header(number, depends_on, chain, Flags::REFERENCED)
    }

    fn droppable(number: u64, depends_on: u64, chain: u32) -> FrameHeader {
        header(number, depends_on, chain, Flags::NONE)
    }

    fn recovery(number: u64, depends_on: u64, chain: u32) -> FrameHeader {
        header(
            number,
            depends_on,
            chain,
            Flags::RECOVERY | Flags::REFERENCED,
        )
    }

    fn viewer() -> Viewer {
        Viewer::new(Params::default())
    }

    /// A frame with no picture arrives on a stream of its own.
    fn arrive(v: &mut Viewer, now: Duration, header: FrameHeader) -> Vec<Action> {
        arrive_sized(v, now, header, 0)
    }

    /// A frame with a picture of `bytes` arrives on a stream of its own.
    fn arrive_sized(
        v: &mut Viewer,
        now: Duration,
        header: FrameHeader,
        bytes: usize,
    ) -> Vec<Action> {
        try_arrive_sized(v, now, header, bytes).expect("a number that is new")
    }

    /// As [`arrive_sized`], for a frame whose number may have come before.
    fn try_arrive_sized(
        v: &mut Viewer,
        now: Duration,
        header: FrameHeader,
        bytes: usize,
    ) -> Result<Vec<Action>, Violation> {
        let id = v.stream_started().expect("room for a stream");
        v.stream_read(id, bytes).expect("room for the picture");
        v.arrived(now, id, header)
    }

    #[test]
    fn frames_in_order_are_decoded_and_the_newest_is_shown() {
        let mut v = viewer();
        assert_eq!(arrive(&mut v, MS, key(0, 0)), vec![Decode(0), Show(0)]);
        assert_eq!(
            arrive(&mut v, 2 * MS, delta(1, 0, 0)),
            vec![Decode(1), Show(1)]
        );
        assert_eq!(v.shown(), Some(1));
        assert_eq!(v.stats().decoded, 2);
    }

    #[test]
    fn a_frame_waits_for_the_frame_it_depends_on() {
        let mut v = viewer();
        // Nothing can be decoded before a key frame.
        assert_eq!(arrive(&mut v, MS, delta(1, 0, 0)), vec![]);
        assert_eq!(arrive(&mut v, 2 * MS, delta(2, 1, 0)), vec![]);
        // The key frame brings the others with it, in order, and one picture is shown.
        assert_eq!(
            arrive(&mut v, 3 * MS, key(0, 0)),
            vec![Decode(0), Decode(1), Decode(2), Show(2)]
        );
        assert_eq!(v.stats().key_requests, 0);
    }

    #[test]
    fn frames_that_nothing_refers_to_do_not_hold_up_the_ones_after_them() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        // Frame 1 never arrives, and nothing refers to it: frame 2 depends on frame 0.
        assert_eq!(
            arrive(&mut v, 2 * MS, delta(2, 0, 0)),
            vec![Decode(2), Show(2)]
        );
        // Frame 3 is not referenced either. It is decoded and shown.
        assert_eq!(
            arrive(&mut v, 3 * MS, droppable(3, 2, 0)),
            vec![Decode(3), Show(3)]
        );
    }

    #[test]
    fn a_frame_numbered_below_one_that_was_decoded_is_dropped() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        // Frame 2 arrives first and is decoded. Frame 1 does not matter to it, nothing refers to
        // frame 1, and it would be decoded after frame 2: backwards. It is dropped.
        assert_eq!(
            arrive(&mut v, 2 * MS, delta(2, 0, 0)),
            vec![Decode(2), Show(2)]
        );
        assert_eq!(arrive(&mut v, 3 * MS, droppable(1, 0, 0)), vec![Discard(1)]);
        assert_eq!(v.shown(), Some(2));
        assert_eq!(v.stats().key_requests, 0);
    }

    #[test]
    fn a_reference_frame_that_the_source_reset_is_not_decoded_after_the_recovery_frame() {
        let mut v = viewer();
        arrive(&mut v, MS, key(10, 0));
        arrive(&mut v, 2 * MS, delta(11, 10, 0));
        // Frames 12 and 13 were reset. The recovery frame refers to frame 11.
        assert_eq!(
            arrive(&mut v, 3 * MS, recovery(14, 11, 1)),
            vec![Decode(14), Show(14)]
        );
        // Frame 12 arrives whole all the same. It names frame 11, which has been decoded, but
        // decoding it now would change what frame 15 is decoded against.
        assert_eq!(arrive(&mut v, 4 * MS, delta(12, 11, 0)), vec![Discard(12)]);
        assert_eq!(
            arrive(&mut v, 5 * MS, delta(15, 14, 1)),
            vec![Decode(15), Show(15)]
        );
        assert_eq!(v.stats().key_requests, 0);
    }

    #[test]
    fn a_frame_that_waits_for_a_frame_below_the_decoded_ones_is_dropped_without_a_request() {
        let mut v = viewer();
        arrive(&mut v, MS, key(10, 0));
        // Frame 13 needs frame 12, which was reset and has not come.
        assert_eq!(arrive(&mut v, 2 * MS, delta(13, 12, 0)), vec![]);
        // The recovery frame is decoded first, since frame 13 is not decodable, and then frame
        // 13 is below it.
        assert_eq!(
            arrive(&mut v, 3 * MS, recovery(14, 10, 1)),
            vec![Discard(13), Decode(14), Show(14)]
        );
        assert_eq!(v.stats().key_requests, 0);
    }

    #[test]
    fn a_recovery_frame_throws_away_the_rest_of_the_old_chain() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(1, 0, 0));
        // Frame 2 was given up on. Frame 3 arrived, but it needs frame 2.
        assert_eq!(arrive(&mut v, 3 * MS, delta(3, 2, 0)), vec![]);
        // The recovery frame refers to what was held, frame 1.
        assert_eq!(
            arrive(&mut v, 4 * MS, recovery(4, 1, 1)),
            vec![Discard(3), Decode(4), Show(4)]
        );
        // The frames of the new chain follow it.
        assert_eq!(
            arrive(&mut v, 5 * MS, delta(5, 4, 1)),
            vec![Decode(5), Show(5)]
        );
    }

    #[test]
    fn a_recovery_frame_can_arrive_before_the_frames_it_needs() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        // Frame 1 was acknowledged at the source but has not been read here yet.
        assert_eq!(arrive(&mut v, 2 * MS, recovery(3, 1, 1)), vec![]);
        assert_eq!(
            arrive(&mut v, 3 * MS, delta(1, 0, 0)),
            vec![Decode(1), Decode(3), Show(3)]
        );
    }

    #[test]
    fn a_key_frame_replaces_everything_before_it() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        assert_eq!(arrive(&mut v, 2 * MS, delta(2, 1, 0)), vec![]);
        assert_eq!(
            arrive(&mut v, 3 * MS, key(5, 1)),
            vec![Discard(2), Decode(5), Show(5)]
        );
        // A frame of the old picture that arrives late is thrown away, and no key frame is
        // asked for: the source never refers back across a key frame.
        assert_eq!(arrive(&mut v, 4 * MS, delta(3, 0, 0)), vec![Discard(3)]);
        assert_eq!(v.stats().discarded, 2);
        assert_eq!(v.stats().key_requests, 0);
    }

    #[test]
    fn a_key_frame_that_the_source_reset_is_not_decoded_after_a_newer_frame() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(1, 0, 0));
        arrive(&mut v, 3 * MS, recovery(7, 1, 2));
        // A key frame numbered 5 was reset when the recovery frame was written, and arrives
        // whole. Decoding it now would make the decoder forget the long-term references.
        assert_eq!(arrive(&mut v, 4 * MS, key(5, 1)), vec![Discard(5)]);
        assert_eq!(
            arrive(&mut v, 5 * MS, delta(8, 7, 2)),
            vec![Decode(8), Show(8)]
        );
    }

    #[test]
    fn a_frame_after_a_key_frame_that_depends_on_what_it_replaced_means_a_disagreement() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(1, 0, 0));
        // The source counted on the viewer holding frame 1, and then sent a recovery frame that
        // refers to it, although a key frame (5) that the source had given up on arrived and
        // made the decoder forget it. The two sides disagree, and a key frame mends it.
        arrive(&mut v, 3 * MS, key(5, 1));
        assert_eq!(
            arrive(&mut v, 4 * MS, recovery(6, 1, 2)),
            vec![Discard(6), RequestKeyFrame { tried: 5 }]
        );
        // Asking again at once is not done.
        assert_eq!(arrive(&mut v, 5 * MS, delta(7, 6, 2)), vec![]);
        assert_eq!(v.stats().key_requests, 1);
    }

    #[test]
    fn a_failed_decode_stops_everything_until_a_key_frame_comes() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(2, 1, 0));
        assert_eq!(
            v.decode_failed(3 * MS),
            vec![Discard(2), RequestKeyFrame { tried: 0 }]
        );
        // Frames that follow wait, and the request is not repeated at once.
        assert_eq!(arrive(&mut v, 4 * MS, delta(3, 0, 0)), vec![]);
        assert_eq!(v.tick(100 * MS), vec![]);
        // After the interval the request is made again.
        assert_eq!(v.tick(600 * MS), vec![RequestKeyFrame { tried: 0 }]);
        assert_eq!(v.tick(700 * MS), vec![]);
        assert_eq!(v.tick(1200 * MS), vec![RequestKeyFrame { tried: 0 }]);
        // A key frame mends it, and what waited from before it goes.
        assert_eq!(
            arrive(&mut v, 1300 * MS, key(9, 1)),
            vec![Discard(3), Decode(9), Show(9)]
        );
        assert_eq!(v.tick(5000 * MS), vec![]);
    }

    #[test]
    fn a_request_names_the_newest_frame_that_was_handed_to_the_decoder() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(1, 0, 0));
        // Frames 2 and 3 were handed to the decoder together, and the first one failed. The
        // second was never tried, but it will not be: the viewer names the newest handed over, so
        // that no key frame at or below it counts as one that is still to come.
        arrive(&mut v, 3 * MS, delta(3, 2, 0));
        assert_eq!(
            arrive(&mut v, 4 * MS, delta(2, 1, 0)),
            vec![Decode(2), Decode(3), Show(3)]
        );
        assert_eq!(v.decode_failed(5 * MS), vec![RequestKeyFrame { tried: 3 }]);
        // Asking again names the same frame: the key frame that answers it is numbered above.
        assert_eq!(v.tick(600 * MS), vec![RequestKeyFrame { tried: 3 }]);
    }

    #[test]
    fn a_key_frame_that_failed_is_named_so_that_it_does_not_answer_its_own_request() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        assert_eq!(v.decode_failed(2 * MS), vec![RequestKeyFrame { tried: 0 }]);
        // The next key frame is decoded and mends it. If it fails too, it is named in turn, and
        // at once: the wait that the first request began is over.
        assert_eq!(arrive(&mut v, 3 * MS, key(4, 1)), vec![Decode(4), Show(4)]);
        assert_eq!(v.decode_failed(4 * MS), vec![RequestKeyFrame { tried: 4 }]);
    }

    #[test]
    fn a_decode_error_soon_after_a_key_frame_is_asked_about_at_once() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        assert_eq!(v.decode_failed(10 * MS), vec![RequestKeyFrame { tried: 0 }]);
        assert_eq!(
            arrive(&mut v, 100 * MS, key(4, 1)),
            vec![Decode(4), Show(4)]
        );
        assert_eq!(
            arrive(&mut v, 150 * MS, delta(5, 4, 0)),
            vec![Decode(5), Show(5)]
        );
        // Less than the interval after the request of 10 ms, but the request was answered, and
        // this is a new wait. The picture does not stay frozen for the rest of the interval.
        assert_eq!(
            v.decode_failed(200 * MS),
            vec![RequestKeyFrame { tried: 5 }]
        );
        // Asking again while no key frame comes keeps its interval, counted from the last ask.
        assert_eq!(v.tick(300 * MS), vec![]);
        assert_eq!(v.tick(700 * MS), vec![RequestKeyFrame { tried: 5 }]);
        assert_eq!(v.stats().key_requests, 3);
    }

    #[test]
    fn nothing_is_asked_for_before_a_frame_has_been_tried() {
        let mut v = viewer();
        assert_eq!(v.decode_failed(MS), vec![]);
        assert_eq!(v.tick(2000 * MS), vec![]);
    }

    #[test]
    fn frames_behind_the_key_frame_that_arrive_before_it_are_decoded_after_it() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(1, 0, 0));
        assert_eq!(v.decode_failed(3 * MS), vec![RequestKeyFrame { tried: 1 }]);
        // The source counts frame 11 as held, since the viewer has acknowledged it, but it is
        // behind the key frame (10), which is still on its way. It is not thrown away.
        assert_eq!(arrive(&mut v, 4 * MS, delta(11, 10, 1)), vec![]);
        assert_eq!(
            arrive(&mut v, 5 * MS, key(10, 1)),
            vec![Decode(10), Decode(11), Show(11)]
        );
        assert_eq!(v.stats().key_requests, 1);
    }

    #[test]
    fn a_frame_that_waits_for_a_frame_that_has_not_come_waits() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        // Frame 3 needs frame 2, which has not arrived. It may be on its way, so nothing is
        // dropped and nothing is asked for, however long it takes.
        assert_eq!(arrive(&mut v, 2 * MS, delta(3, 2, 0)), vec![]);
        assert_eq!(v.tick(5000 * MS), vec![]);
        assert_eq!(
            arrive(&mut v, 5001 * MS, delta(2, 0, 0)),
            vec![Decode(2), Decode(3), Show(3)]
        );
    }

    #[test]
    fn a_frame_of_a_new_epoch_waits_for_its_key_frame() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        let mut resized = delta(3, 2, 1);
        resized.epoch = 1;
        // The key frame of the new size is bigger and slower than the frames behind it.
        assert_eq!(arrive(&mut v, 2 * MS, resized), vec![]);
        let mut new_key = key(2, 1);
        new_key.epoch = 1;
        assert_eq!(
            arrive(&mut v, 3 * MS, new_key),
            vec![Decode(2), Decode(3), Show(3)]
        );
        // A frame of the old size is thrown away, and no key frame is asked for.
        assert_eq!(arrive(&mut v, 4 * MS, delta(4, 3, 0)), vec![Discard(4)]);
        assert_eq!(v.stats().key_requests, 0);
    }

    #[test]
    fn a_key_frame_of_a_new_epoch_ends_the_old_size() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        assert_eq!(arrive(&mut v, 2 * MS, delta(2, 1, 0)), vec![]);
        let mut new_key = key(3, 1);
        new_key.epoch = 1;
        assert_eq!(
            arrive(&mut v, 3 * MS, new_key),
            vec![Discard(2), Decode(3), Show(3)]
        );
    }

    fn repeated(number: u64) -> Result<Vec<Action>, Violation> {
        Err(Violation::RepeatedNumber { number })
    }

    #[test]
    fn a_number_that_comes_twice_is_a_violation_whether_the_first_waits_or_was_decoded() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        // The first frame 0 was decoded.
        assert_eq!(try_arrive_sized(&mut v, 2 * MS, key(0, 0), 7), repeated(0));
        // The first frame 3 waits for frame 2, and its picture is held. The second is a
        // violation, and its bytes are not counted: the session ends and the caller lets go of
        // every picture it has.
        arrive_sized(&mut v, 3 * MS, delta(3, 2, 0), 40);
        assert_eq!(
            try_arrive_sized(&mut v, 4 * MS, delta(3, 2, 0), 60),
            repeated(3)
        );
        assert_eq!(v.held_bytes(), 40);
        assert_eq!(v.streams(), 0);
        // What header the second one has makes no difference: a key frame is a frame too.
        assert_eq!(try_arrive_sized(&mut v, 5 * MS, key(3, 1), 60), repeated(3));
        assert_eq!(v.held_bytes(), 40);
    }

    #[test]
    fn a_number_that_comes_twice_in_an_older_size_is_a_violation_too() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        // Frame 7 waits for frame 6. Then a key frame of a new size is decoded, and the old size
        // is over. Frame 7 is still held, because nothing numbered above it has been decoded.
        arrive_sized(&mut v, 2 * MS, delta(7, 6, 0), 40);
        let mut new_key = key(5, 1);
        new_key.epoch = 1;
        assert_eq!(arrive(&mut v, 3 * MS, new_key), vec![Decode(5), Show(5)]);
        assert_eq!(v.held_bytes(), 40);
        // A second frame 7 of the old size would have been dropped as of the old size, and the
        // action would have named the number of the frame that waits.
        assert_eq!(
            try_arrive_sized(&mut v, 4 * MS, delta(7, 6, 0), 40),
            repeated(7)
        );
        assert_eq!(v.held_bytes(), 40);
    }

    #[test]
    fn a_number_that_the_viewer_has_forgotten_is_late_and_not_a_violation() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(1, 0, 0));
        // A key frame ends what the decoder knew, and the viewer forgets the numbers before it.
        // Frame 1 is below the key frame, so it is dropped as late, and the action names a
        // frame of which the viewer holds nothing.
        arrive(&mut v, 3 * MS, key(4, 1));
        assert_eq!(arrive(&mut v, 4 * MS, delta(1, 0, 0)), vec![Discard(1)]);
        assert_eq!(v.held_bytes(), 0);
    }

    #[test]
    fn the_viewer_does_not_hold_more_than_it_may() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        for n in 0..1000_u64 {
            // Every one waits for a frame that never comes.
            let delta = delta(10_000 + n, 5000, 0);
            arrive_sized(&mut v, Duration::from_millis(n), delta, 1000);
        }
        assert!(v.pending.len() <= MAX_PENDING);
        assert!(v.decoded.len() <= MAX_DECODED);
        // The pictures of the frames that were pushed out are given back with them.
        assert_eq!(v.held_bytes(), 1000 * v.pending.len());
    }

    /// The numbers that make the limits easy to reach.
    fn small() -> Viewer {
        Viewer::new(Params {
            max_streams: 3,
            max_held_bytes: 100,
            ..Params::default()
        })
    }

    #[test]
    fn a_frame_stream_beyond_the_count_is_refused_until_one_ends() {
        let mut v = small();
        let first = v.stream_started().unwrap();
        let second = v.stream_started().unwrap();
        v.stream_started().unwrap();
        assert_eq!(v.streams(), 3);
        assert_eq!(v.stream_started(), Err(Refusal::OverBudget));
        // A stream that was refused is not counted, and the count is not shifted by it.
        assert_eq!(v.streams(), 3);
        // A frame that arrives ends its stream, and so does one that broke off.
        v.arrived(MS, first, key(0, 0)).unwrap();
        assert_eq!(v.streams(), 2);
        v.stream_started().unwrap();
        assert_eq!(v.stream_started(), Err(Refusal::OverBudget));
        v.stream_aborted(second);
        v.stream_started().unwrap();
        assert_eq!(v.streams(), 3);
    }

    #[test]
    fn a_picture_that_would_go_beyond_the_limit_is_refused_and_forgotten() {
        let mut v = small();
        let first = v.stream_started().unwrap();
        let second = v.stream_started().unwrap();
        v.stream_read(first, 60).unwrap();
        v.stream_read(second, 40).unwrap();
        assert_eq!(v.held_bytes(), 100);
        // One byte more than the limit: the stream is refused, and what it delivered is given
        // back, which leaves room for another.
        assert_eq!(v.stream_read(second, 1), Err(Refusal::OverBudget));
        assert_eq!(v.held_bytes(), 60);
        assert_eq!(v.streams(), 1);
        assert_eq!(v.stream_read(second, 1), Err(Refusal::Unknown));
        let third = v.stream_started().unwrap();
        v.stream_read(third, 40).unwrap();
        assert_eq!(v.held_bytes(), 100);
    }

    #[test]
    fn no_stream_starts_while_the_pictures_held_fill_the_limit() {
        let mut v = small();
        let id = v.stream_started().unwrap();
        v.stream_read(id, 100).unwrap();
        assert_eq!(v.stream_started(), Err(Refusal::OverBudget));
        // The frame is decoded, so its picture is the decoder's, and the next stream is taken.
        assert_eq!(v.arrived(MS, id, key(0, 0)), Ok(vec![Decode(0), Show(0)]));
        assert_eq!(v.held_bytes(), 0);
        v.stream_started().unwrap();
    }

    #[test]
    fn a_picture_that_waits_is_counted_until_it_is_decoded_or_dropped() {
        let mut v = Viewer::new(Params::default());
        arrive_sized(&mut v, MS, key(0, 0), 10);
        assert_eq!(v.held_bytes(), 0);
        // Frame 2 needs frame 1, which has not come: it waits, and its picture is held.
        assert_eq!(arrive_sized(&mut v, 2 * MS, delta(2, 1, 0), 300), vec![]);
        assert_eq!(v.held_bytes(), 300);
        assert_eq!(arrive_sized(&mut v, 3 * MS, delta(3, 2, 0), 200), vec![]);
        assert_eq!(v.held_bytes(), 500);
        // Frame 1 comes, and the three are decoded: nothing is held.
        assert_eq!(
            arrive_sized(&mut v, 4 * MS, delta(1, 0, 0), 50),
            vec![Decode(1), Decode(2), Decode(3), Show(3)]
        );
        assert_eq!(v.held_bytes(), 0);
        // A frame that waits and is thrown away by a key frame is given back as well.
        arrive_sized(&mut v, 5 * MS, delta(5, 4, 0), 70);
        assert_eq!(v.held_bytes(), 70);
        assert_eq!(
            arrive_sized(&mut v, 6 * MS, key(9, 1), 20),
            vec![Discard(5), Decode(9), Show(9)]
        );
        assert_eq!(v.held_bytes(), 0);
    }

    #[test]
    fn a_late_frame_is_dropped_with_its_picture() {
        let mut v = viewer();
        arrive(&mut v, MS, key(0, 0));
        arrive(&mut v, 2 * MS, delta(2, 0, 0));
        assert_eq!(
            arrive_sized(&mut v, 3 * MS, droppable(1, 0, 0), 500),
            vec![Discard(1)]
        );
        assert_eq!(v.held_bytes(), 0);
    }

    #[test]
    fn a_picture_over_the_size_limit_is_refused_whatever_the_budget() {
        let mut v = viewer();
        let id = v.stream_started().unwrap();
        v.stream_read(id, MAX_PICTURE_BYTES).unwrap();
        assert_eq!(v.stream_read(id, 1), Err(Refusal::TooLarge));
        assert_eq!(v.streams(), 0);
        assert_eq!(v.held_bytes(), 0);
        // Over the size limit and over the budget: the source is at fault for the first.
        let mut v = small();
        let id = v.stream_started().unwrap();
        assert_eq!(
            v.stream_read(id, MAX_PICTURE_BYTES + 1),
            Err(Refusal::TooLarge)
        );
        // A size that does not fit in the count does not wrap.
        let id = v.stream_started().unwrap();
        v.stream_read(id, 1).unwrap();
        assert_eq!(v.stream_read(id, usize::MAX), Err(Refusal::TooLarge));
    }

    #[test]
    fn a_stream_that_broke_off_gives_back_what_it_delivered() {
        let mut v = small();
        let id = v.stream_started().unwrap();
        v.stream_read(id, 80).unwrap();
        v.stream_aborted(id);
        assert_eq!(v.held_bytes(), 0);
        assert_eq!(v.streams(), 0);
        // Aborting twice, or a stream that never was, changes nothing.
        v.stream_aborted(id);
        v.stream_aborted(StreamId(99));
        assert_eq!(v.stream_read(id, 1), Err(Refusal::Unknown));
    }

    #[test]
    fn a_frame_that_comes_on_a_stream_the_viewer_does_not_have_is_dropped() {
        let mut v = small();
        let id = v.stream_started().unwrap();
        v.stream_aborted(id);
        assert_eq!(v.arrived(MS, id, key(0, 0)), Ok(vec![Discard(0)]));
        // Nor does one stream bring two frames.
        let id = v.stream_started().unwrap();
        assert_eq!(
            v.arrived(2 * MS, id, key(1, 0)),
            Ok(vec![Decode(1), Show(1)])
        );
        assert_eq!(v.arrived(3 * MS, id, delta(2, 1, 0)), Ok(vec![Discard(2)]));
        assert_eq!(v.held_bytes(), 0);
    }

    proptest::proptest! {
        /// A frame is never decoded before the frame it depends on, never after a frame with a
        /// higher number, whatever the order of arrival, and what the viewer keeps stays bounded.
        #[test]
        fn frames_in_any_order_are_decoded_after_what_they_depend_on_and_never_backwards(
            order in proptest::collection::vec(0_usize..40, 1..120)
        ) {
            // A chain of 40 frames: a key frame, then each frame depends on the one before.
            let frames: Vec<FrameHeader> = (0..40_u64)
                .map(|n| if n == 0 { key(0, 0) } else { delta(n, n - 1, 0) })
                .collect();
            let mut v = viewer();
            let mut decoded = Vec::new();
            // A frame arrives once, in some order. What a number that comes twice does is
            // another property.
            let mut seen = BTreeSet::new();
            for (i, pick) in order.into_iter().enumerate() {
                let now = Duration::from_millis(u64::try_from(i).unwrap());
                let Some(h) = frames.get(pick) else { continue };
                if !seen.insert(pick) {
                    continue;
                }
                for action in arrive(&mut v, now, *h) {
                    if let Decode(n) = action {
                        decoded.push(n);
                    }
                }
            }
            for (at, n) in decoded.iter().enumerate() {
                // Frame n depends on n - 1, which must have been decoded earlier.
                if *n > 0 {
                    proptest::prop_assert!(decoded.get(..at).unwrap().contains(&(n - 1)));
                }
            }
            proptest::prop_assert!(decoded.windows(2).all(|w| w[0] < w[1]));
            proptest::prop_assert!(v.pending.len() <= MAX_PENDING);
            proptest::prop_assert_eq!(v.stats().key_requests, 0);
        }

        /// Whatever a source does with its streams, numbers that come twice included, the viewer
        /// holds no more than its limits allow, and what it holds is exactly the pictures of the
        /// streams that are open and of the frames that wait: nothing is counted twice, and
        /// nothing is lost track of. A number that the viewer holds or has decoded since the last
        /// key frame is a violation, which ends the session, and the next event begins another.
        #[test]
        fn whatever_the_source_does_the_viewer_stays_within_its_limits_and_counts_exactly(
            events in proptest::collection::vec(event(), 1..300)
        ) {
            let mut v = small();
            let mut open: Vec<(StreamId, usize)> = Vec::new();
            // The frames that wait, with their bytes and whether they are key frames.
            let mut waiting: BTreeMap<u64, (usize, bool)> = BTreeMap::new();
            // The numbers that the viewer has decoded since the last key frame.
            let mut decoded = BTreeSet::new();
            let held = |open: &[(StreamId, usize)], waiting: &BTreeMap<u64, (usize, bool)>| {
                open.iter().map(|(_, b)| b).sum::<usize>()
                    + waiting.values().map(|(b, _)| b).sum::<usize>()
            };
            for (i, event) in events.into_iter().enumerate() {
                let now = Duration::from_millis(u64::try_from(i).unwrap());
                let mut actions = Vec::new();
                match event {
                    Event::Start => {
                        let room = open.len() < 3 && held(&open, &waiting) < 100;
                        match v.stream_started() {
                            Ok(id) => {
                                proptest::prop_assert!(room);
                                open.push((id, 0));
                            }
                            Err(refusal) => {
                                proptest::prop_assert!(!room);
                                proptest::prop_assert_eq!(refusal, Refusal::OverBudget);
                            }
                        }
                    }
                    Event::Read { which, len } => {
                        if open.is_empty() {
                            proptest::prop_assert_eq!(
                                v.stream_read(StreamId(1000), len),
                                Err(Refusal::Unknown)
                            );
                        } else {
                            let at = which % open.len();
                            let (id, so_far) = open[at];
                            let total = so_far.saturating_add(len);
                            let expected = if total > MAX_PICTURE_BYTES {
                                Err(Refusal::TooLarge)
                            } else if held(&open, &waiting) + len > 100 {
                                Err(Refusal::OverBudget)
                            } else {
                                Ok(())
                            };
                            proptest::prop_assert_eq!(v.stream_read(id, len), expected);
                            if expected.is_ok() {
                                open[at].1 = total;
                            } else {
                                open.remove(at);
                            }
                        }
                    }
                    Event::Abort { which } => {
                        if !open.is_empty() {
                            let (id, _) = open.remove(which % open.len());
                            v.stream_aborted(id);
                        }
                    }
                    Event::Arrive { which, number, key: is_key } => {
                        if !open.is_empty() {
                            let (id, bytes) = open.remove(which % open.len());
                            let h = if is_key || number == 0 {
                                key(number, 0)
                            } else {
                                delta(number, number - 1, 0)
                            };
                            let repeated = waiting.contains_key(&number) || decoded.contains(&number);
                            match v.arrived(now, id, h) {
                                Ok(taken) => {
                                    proptest::prop_assert!(!repeated);
                                    waiting.insert(number, (bytes, h.is_key()));
                                    actions = taken;
                                }
                                Err(violation) => {
                                    proptest::prop_assert!(repeated);
                                    proptest::prop_assert_eq!(
                                        violation,
                                        Violation::RepeatedNumber { number }
                                    );
                                    // What the viewer counts is still what it held before, and
                                    // the picture that came is not in it.
                                    proptest::prop_assert_eq!(v.streams(), open.len());
                                    proptest::prop_assert_eq!(
                                        v.held_bytes(),
                                        held(&open, &waiting)
                                    );
                                    // The session is over. Another one begins.
                                    v = small();
                                    open.clear();
                                    waiting.clear();
                                    decoded.clear();
                                }
                            }
                        }
                    }
                    Event::Stale { number } => {
                        let h = if number == 0 { key(0, 0) } else { delta(number, number - 1, 0) };
                        proptest::prop_assert_eq!(
                            v.arrived(now, StreamId(10_000), h),
                            Ok(vec![Discard(number)])
                        );
                    }
                    Event::Fail => actions = v.decode_failed(now),
                    Event::Tick => actions = v.tick(now),
                }
                // Every frame that is decoded or dropped was one that was held, and is given back.
                for action in actions {
                    match action {
                        Decode(n) => {
                            let Some((_, was_key)) = waiting.remove(&n) else {
                                proptest::prop_assert!(false, "decoded a frame that was not held");
                                continue;
                            };
                            if was_key {
                                decoded.clear();
                            }
                            decoded.insert(n);
                        }
                        Discard(n) => proptest::prop_assert!(waiting.remove(&n).is_some()),
                        Show(_) | RequestKeyFrame { .. } => {}
                    }
                }
                proptest::prop_assert!(v.streams() <= 3);
                proptest::prop_assert!(v.held_bytes() <= 100);
                proptest::prop_assert_eq!(v.streams(), open.len());
                proptest::prop_assert_eq!(v.held_bytes(), held(&open, &waiting));
            }
        }
    }

    #[derive(Debug, Clone)]
    enum Event {
        Start,
        Read {
            which: usize,
            len: usize,
        },
        Abort {
            which: usize,
        },
        Arrive {
            which: usize,
            number: u64,
            key: bool,
        },
        /// A frame comes on a stream that the viewer does not have.
        Stale {
            number: u64,
        },
        Fail,
        Tick,
    }

    fn event() -> impl proptest::strategy::Strategy<Value = Event> {
        use proptest::{prelude::any, prop_oneof, strategy::Just, strategy::Strategy};
        prop_oneof![
            3 => Just(Event::Start),
            6 => (0_usize..8, prop_oneof![9 => 0_usize..60, 1 => Just(MAX_PICTURE_BYTES + 1)])
                .prop_map(|(which, len)| Event::Read { which, len }),
            1 => (0_usize..8).prop_map(|which| Event::Abort { which }),
            4 => (0_usize..8, 0_u64..24, any::<bool>())
                .prop_map(|(which, number, key)| Event::Arrive { which, number, key }),
            1 => (0_u64..24).prop_map(|number| Event::Stale { number }),
            1 => Just(Event::Fail),
            1 => Just(Event::Tick),
        ]
    }
}
