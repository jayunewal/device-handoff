// SPDX-License-Identifier: Apache-2.0

//! The viewer's side of packets of sound: which to play next, and what to do about the ones that
//! do not come.
//!
//! Packets arrive on streams of their own, so they arrive in any order, and some never arrive
//! because the source reset them. A decoder takes the packets in order, and the player behind it
//! needs a steady supply, so the viewer does three things (ADR 0012):
//!
//! - It holds a packet that comes before the one it expects for a short time
//!   ([`Params::hold`]), since the repeat of a lost packet takes about a round trip and a home
//!   network has one that is shorter than the player's buffer. The packets are played in order
//!   as they become whole.
//! - It goes on without a packet that has not come by then, and says how many it skipped
//!   ([`Action::Play::lost_before`]), so that the player can cover the gap.
//! - It drops a packet that comes after a later one was played, or whose run has ended.
//!
//! A packet that begins a run ([`PacketHeader::is_first`]) tells the decoder to start afresh,
//! and tells the viewer that nothing numbered below it belongs to the run. The first packet that
//! is played after the viewer began to listen is played as the first of its run, whatever its
//! flag says, because the packet that began the run may have been the one that was lost.
//!
//! The viewer also bounds what a source can make it hold. It takes no more than
//! [`Params::max_streams`] packet streams at once and holds no more than [`Params::max_held`]
//! packets that wait for an earlier one. A stream that it does not take is stopped with code 499,
//! like a stream of a session that is not running: that is no fault of the source, because a
//! packet can arrive just as the viewer reaches its count or turns off.
//!
//! Like the sender, this is a state machine without input or output. It is told when a stream
//! starts, how much of a packet has been read, and when a packet has arrived or its stream broke
//! off, and says what to do with the packets. The caller keeps the bytes of a packet from the
//! time it has arrived until an action names it.

use std::{collections::BTreeMap, time::Duration};

use baton_proto::limits::MAX_SOUND_PACKET_BYTES;

use super::PacketHeader;

/// The numbers that the viewer's rules use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// How long a packet waits for the ones before it. A packet is played at the latest this long
    /// after it arrived.
    pub hold: Duration,
    /// How many packet streams are read at once, [`crate::MAX_SOUND_STREAMS`] unless a test says
    /// otherwise. The source keeps eight packets in flight and may have reset up to eight more
    /// that the viewer still counts.
    pub max_streams: usize,
    /// How many packets may wait for an earlier one. When there are more, the viewer goes on
    /// without what is missing before the hold is over.
    pub max_held: usize,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            hold: Duration::from_millis(40),
            max_streams: crate::MAX_SOUND_STREAMS,
            max_held: 64,
        }
    }
}

/// A packet stream that is being read. It is the viewer's own number for it, and is not used
/// again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId(u64);

/// Why the viewer does not take a packet stream, or the rest of one. The stream is forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The viewer listens to nothing: it has not turned sound on, or the run has ended. Stop the
    /// stream with code 499 and read no more of it. The source did not break a rule.
    #[error("the viewer is not listening")]
    NotListening,
    /// As many packet streams are open as the viewer allows. Stop the stream with code 499 and
    /// read no more of it. The source did not break a rule.
    #[error("the viewer is at its limit")]
    OverBudget,
    /// The packet is larger than the limit of section 6 of the protocol. Stop the stream with
    /// code 413.
    #[error("the packet is larger than the limit")]
    TooLarge,
    /// The viewer has no such stream: it never started, or it has ended or been refused.
    #[error("no such packet stream")]
    Unknown,
}

/// What a source did that no source that follows the protocol does. The session is over: the
/// viewer ends it with code 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Violation {
    /// A packet number came a second time, while the viewer still held the first packet. A
    /// packet has a number of its own and a packet that was reset is not sent again.
    #[error("packet {number} came a second time")]
    RepeatedNumber {
        /// The number of the packet.
        number: u64,
    },
}

/// What the viewer is to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Decode this packet and play it now. The actions come in the order in which packets are to
    /// be played.
    Play {
        /// The number of the packet.
        number: u64,
        /// Whether the decoder starts afresh at this packet, which is the first of its run.
        first: bool,
        /// How many packets before this one were skipped, because they did not come in time or
        /// were reset. They are not played if they come later. It is 0 for the first packet of
        /// a run.
        lost_before: u64,
    },
    /// Forget this packet, which will not be played.
    Discard(u64),
}

/// Counts of what happened, for the reports that the viewer sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Packets played.
    pub played: u64,
    /// Packets skipped: the ones the viewer went on without.
    pub lost: u64,
    /// Packets dropped because they came after a later one was played.
    pub late: u64,
    /// Packets dropped for another reason: the run ended, or they belonged to an earlier run.
    pub discarded: u64,
    /// Packet streams that the viewer did not take.
    pub refused: u64,
}

#[derive(Debug, Clone, Copy)]
struct Held {
    /// Whether the packet begins a run.
    first: bool,
    /// When the packet arrived.
    since: Duration,
}

/// The state of one session's sound on the viewer.
#[derive(Debug)]
pub struct Receiver {
    params: Params,
    /// Whether a run is going, so that packets are taken.
    running: bool,
    /// The packet streams that are being read, with the bytes of packet each has delivered.
    arriving: BTreeMap<StreamId, usize>,
    next_stream: u64,
    /// The packets that wait for an earlier one.
    held: BTreeMap<u64, Held>,
    /// The number of the packet that is played next, once one has been played in this run.
    next: Option<u64>,
    stats: Stats,
}

impl Receiver {
    /// Starts a session. Nothing is taken before [`begin_run`](Self::begin_run).
    pub fn new(params: Params) -> Self {
        Self {
            params,
            running: false,
            arriving: BTreeMap::new(),
            next_stream: 0,
            held: BTreeMap::new(),
            next: None,
            stats: Stats::default(),
        }
    }

    /// What has been counted so far.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Whether a run is going.
    pub fn running(&self) -> bool {
        self.running
    }

    /// The number of packet streams that are being read.
    pub fn streams(&self) -> usize {
        self.arriving.len()
    }

    /// The number of packets that wait for an earlier one.
    pub fn held(&self) -> usize {
        self.held.len()
    }

    /// The viewer reads a `Sound` and so listens: packet streams are taken from now on. Does
    /// nothing if a run is going. The packets are numbered and flagged, so a run that begins
    /// again without the viewer having read an end of the old one is seen in the packets.
    pub fn begin_run(&mut self) {
        if !self.running {
            self.running = true;
            self.next = None;
        }
    }

    /// The viewer turned sound off, or read a `Silent`: it listens to nothing. The streams that
    /// are being read are forgotten, and so are the packets that wait. Returns what to forget.
    pub fn end_run(&mut self) -> Vec<Action> {
        self.running = false;
        self.arriving.clear();
        self.next = None;
        let mut actions = Vec::new();
        self.discard_held(&mut actions, |_| true);
        actions
    }

    /// A packet stream has started: its first 17 bytes are read, and they name this session.
    ///
    /// The stream is refused when the viewer listens to nothing, or when as many are open as
    /// [`Params::max_streams`]. A refused stream is stopped with code 499 and not read.
    /// Otherwise the id that comes back is used for the rest of the stream, which ends with
    /// [`Receiver::arrived`] or [`Receiver::stream_aborted`].
    pub fn stream_started(&mut self) -> Result<StreamId, Refusal> {
        if !self.running {
            self.stats.refused += 1;
            return Err(Refusal::NotListening);
        }
        if self.arriving.len() >= self.params.max_streams {
            self.stats.refused += 1;
            return Err(Refusal::OverBudget);
        }
        let id = StreamId(self.next_stream);
        self.next_stream += 1;
        self.arriving.insert(id, 0);
        Ok(id)
    }

    /// `len` more bytes of the packet of a stream have been read, the header not counted.
    ///
    /// When the packet would be larger than [`MAX_SOUND_PACKET_BYTES`] the stream is refused and
    /// forgotten. It is then stopped with code 413 and not read any more.
    pub fn stream_read(&mut self, id: StreamId, len: usize) -> Result<(), Refusal> {
        let Some(&so_far) = self.arriving.get(&id) else {
            return Err(Refusal::Unknown);
        };
        let total = so_far.saturating_add(len);
        if total > MAX_SOUND_PACKET_BYTES {
            self.arriving.remove(&id);
            return Err(Refusal::TooLarge);
        }
        self.arriving.insert(id, total);
        Ok(())
    }

    /// A packet stream ended before its packet was whole: the source reset it, it was stopped,
    /// or the connection went. What it delivered is forgotten.
    pub fn stream_aborted(&mut self, id: StreamId) {
        self.arriving.remove(&id);
    }

    /// The packet of a stream has arrived in full at `now`.
    ///
    /// A stream that the viewer does not have, which means one that was refused, aborted, or
    /// forgotten when the run ended, brings a packet that is dropped. The `Discard` for such a
    /// stream names the packet that has just been read, whatever its number, and is not looked up
    /// by number: a packet with the same number may be held, and it stays.
    ///
    /// A packet whose number the viewer holds is a [`Violation`], whatever its header says. The
    /// session is to end. A packet with any other number is taken like any other, and is dropped
    /// as late if its number is below that of a packet that was played or skipped.
    ///
    /// # Errors
    ///
    /// [`Violation::RepeatedNumber`], as above.
    pub fn arrived(
        &mut self,
        now: Duration,
        id: StreamId,
        header: PacketHeader,
    ) -> Result<Vec<Action>, Violation> {
        let mut actions = Vec::new();
        if self.arriving.remove(&id).is_none() || !self.running {
            self.stats.discarded += 1;
            actions.push(Action::Discard(header.number));
        } else if self.held.contains_key(&header.number) {
            return Err(Violation::RepeatedNumber {
                number: header.number,
            });
        } else if header.is_first() && self.next.is_none_or(|next| header.number >= next) {
            // A run begins here. What is held and numbered below the packet is from an earlier
            // run, and its sound is of no use after the decoder starts afresh.
            self.discard_held(&mut actions, |number| number < header.number);
            self.next = None;
            self.hold(now, header);
        } else if self.next.is_some_and(|next| header.number < next) {
            // Late, and a decoder does not go back.
            self.stats.late += 1;
            actions.push(Action::Discard(header.number));
        } else {
            self.hold(now, header);
        }
        // Time has passed since the last call, whatever the packet was.
        self.drain(now, &mut actions);
        Ok(actions)
    }

    fn hold(&mut self, now: Duration, header: PacketHeader) {
        self.held.insert(
            header.number,
            Held {
                first: header.is_first(),
                since: now,
            },
        );
    }

    /// Looks at the clock: goes on without what has not come in time. Call it when
    /// [`next_deadline`](Self::next_deadline) is reached.
    pub fn tick(&mut self, now: Duration) -> Vec<Action> {
        let mut actions = Vec::new();
        self.drain(now, &mut actions);
        actions
    }

    /// The time at which a packet that waits has waited for [`Params::hold`], if one waits.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.held
            .values()
            .map(|held| held.since.saturating_add(self.params.hold))
            .min()
    }

    fn discard_held(&mut self, actions: &mut Vec<Action>, which: impl Fn(u64) -> bool) {
        let doomed: Vec<u64> = self.held.keys().copied().filter(|n| which(*n)).collect();
        for number in doomed {
            self.held.remove(&number);
            self.stats.discarded += 1;
            actions.push(Action::Discard(number));
        }
    }

    /// The highest number that has to be played now whether or not what is before it came: that
    /// of the packet that has waited for the hold, or the lowest if too many wait.
    fn forced_through(&self, now: Duration) -> Option<u64> {
        let expired = self
            .held
            .iter()
            .filter(|(_, held)| held.since.saturating_add(self.params.hold) <= now)
            .map(|(number, _)| *number)
            .max();
        let over = (self.held.len() > self.params.max_held.max(1))
            .then(|| self.held.keys().next().copied())
            .flatten();
        expired.max(over)
    }

    /// Plays every packet that can be played, oldest first.
    fn drain(&mut self, now: Duration, actions: &mut Vec<Action>) {
        loop {
            let forced = self.forced_through(now);
            let Some((&number, &held)) = self.held.first_key_value() else {
                return;
            };
            let (first, lost_before) = match self.next {
                Some(next) if number == next => (held.first, 0),
                None if held.first => (true, 0),
                _ if forced.is_some_and(|limit| number <= limit) => {
                    // The packets before this one did not come in time. The decoder is told to
                    // start afresh only if there is nothing to go on from.
                    match self.next {
                        Some(next) => (held.first, number.saturating_sub(next)),
                        None => (true, 0),
                    }
                }
                _ => return,
            };
            self.held.remove(&number);
            self.next = Some(number.saturating_add(1));
            self.stats.played += 1;
            self.stats.lost = self.stats.lost.saturating_add(lost_before);
            actions.push(Action::Play {
                number,
                first,
                lost_before,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use baton_proto::id::SessionId;
    use proptest::prelude::*;

    use super::Action::{Discard, Play};
    use super::*;
    use crate::sound::PacketFlags;

    const MS: Duration = Duration::from_millis(1);

    fn header(number: u64, first: bool) -> PacketHeader {
        PacketHeader {
            session: SessionId::from_bytes([3; SessionId::LEN]),
            number,
            capture_time: number.saturating_mul(21_333),
            flags: if first {
                PacketFlags::FIRST
            } else {
                PacketFlags::NONE
            },
        }
    }

    /// A receiver that listens.
    fn receiver() -> Receiver {
        let mut receiver = Receiver::new(Params::default());
        receiver.begin_run();
        receiver
    }

    /// Opens the stream of a packet of 340 bytes, reads it, and hands it over, at `now`.
    fn arrive(receiver: &mut Receiver, now: Duration, header: PacketHeader) -> Vec<Action> {
        let id = receiver.stream_started().expect("a stream is taken");
        receiver
            .stream_read(id, 340)
            .expect("a packet that is not too large");
        receiver
            .arrived(now, id, header)
            .expect("a number of its own")
    }

    fn play(number: u64, first: bool, lost_before: u64) -> Action {
        Play {
            number,
            first,
            lost_before,
        }
    }

    #[test]
    fn packets_that_come_in_order_are_played_at_once() {
        let mut r = receiver();
        assert_eq!(
            arrive(&mut r, Duration::ZERO, header(0, true)),
            [play(0, true, 0)]
        );
        assert_eq!(
            arrive(&mut r, 21 * MS, header(1, false)),
            [play(1, false, 0)]
        );
        assert_eq!(
            arrive(&mut r, 43 * MS, header(2, false)),
            [play(2, false, 0)]
        );
        assert_eq!(r.held(), 0);
        assert_eq!(r.next_deadline(), None);
        assert_eq!(
            r.stats(),
            Stats {
                played: 3,
                ..Stats::default()
            }
        );
    }

    #[test]
    fn a_packet_that_comes_early_waits_for_the_one_before_it() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        assert_eq!(arrive(&mut r, 10 * MS, header(2, false)), []);
        assert_eq!(r.held(), 1);
        assert_eq!(r.next_deadline(), Some(50 * MS));
        assert_eq!(
            arrive(&mut r, 20 * MS, header(1, false)),
            [play(1, false, 0), play(2, false, 0)]
        );
        assert_eq!(r.held(), 0);
        assert_eq!(r.stats().lost, 0);
    }

    #[test]
    fn a_packet_that_does_not_come_is_given_up_after_the_hold_and_counted() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        assert_eq!(arrive(&mut r, 10 * MS, header(2, false)), []);
        assert_eq!(r.tick(49 * MS), []);
        assert_eq!(r.tick(50 * MS), [play(2, false, 1)]);
        assert_eq!(r.next_deadline(), None);
        assert_eq!(r.stats().lost, 1);
        // The next one is in order again.
        assert_eq!(
            arrive(&mut r, 52 * MS, header(3, false)),
            [play(3, false, 0)]
        );
    }

    #[test]
    fn the_wait_of_a_packet_runs_from_its_own_arrival() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        assert_eq!(arrive(&mut r, Duration::ZERO, header(3, false)), []);
        assert_eq!(arrive(&mut r, 30 * MS, header(5, false)), []);
        assert_eq!(r.next_deadline(), Some(40 * MS));
        // Packet 3 has waited for the hold; packet 5 has not, so it waits for packet 4.
        assert_eq!(r.tick(40 * MS), [play(3, false, 2)]);
        assert_eq!(r.next_deadline(), Some(70 * MS));
        assert_eq!(r.tick(70 * MS), [play(5, false, 1)]);
        assert_eq!(r.stats().lost, 3);
    }

    #[test]
    fn a_packet_that_has_waited_takes_the_younger_ones_before_it_along() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(4, true));
        arrive(&mut r, 10 * MS, header(7, false));
        arrive(&mut r, 30 * MS, header(6, false));
        // Packet 7 has waited for the hold, and cannot be played before packet 6, which came
        // later and has not.
        assert_eq!(r.tick(50 * MS), [play(6, false, 1), play(7, false, 0)]);
        assert_eq!(r.stats().lost, 1);
    }

    #[test]
    fn a_packet_that_comes_after_the_hold_is_late_and_is_not_played() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, Duration::ZERO, header(2, false));
        assert_eq!(r.tick(40 * MS), [play(2, false, 1)]);
        assert_eq!(arrive(&mut r, 41 * MS, header(1, false)), [Discard(1)]);
        assert_eq!(r.stats().late, 1);
        // The place of the stream was given back.
        assert_eq!(r.streams(), 0);
    }

    #[test]
    fn a_packet_that_comes_a_second_time_is_late_when_the_first_was_played() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(1, false));
        assert_eq!(arrive(&mut r, 2 * MS, header(1, false)), [Discard(1)]);
        assert_eq!(arrive(&mut r, 3 * MS, header(0, true)), [Discard(0)]);
        assert_eq!(r.stats().late, 2);
    }

    #[test]
    fn a_number_that_comes_while_the_first_is_held_is_a_violation() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(2, false));
        let id = r.stream_started().unwrap();
        assert_eq!(
            r.arrived(2 * MS, id, header(2, false)),
            Err(Violation::RepeatedNumber { number: 2 })
        );
    }

    #[test]
    fn the_first_packet_of_a_run_makes_the_decoder_start_afresh_and_skips_nothing() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(1, false));
        // A run that begins again goes on from the numbers it had. What was skipped between the
        // runs is not counted as lost sound.
        assert_eq!(
            arrive(&mut r, 100 * MS, header(40, true)),
            [play(40, true, 0)]
        );
        assert_eq!(
            arrive(&mut r, 101 * MS, header(41, false)),
            [play(41, false, 0)]
        );
        assert_eq!(r.stats().lost, 0);
        // A packet of the earlier run that comes now is late.
        assert_eq!(arrive(&mut r, 102 * MS, header(39, false)), [Discard(39)]);
    }

    #[test]
    fn what_is_held_below_the_first_packet_of_a_run_belongs_to_the_run_before() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(1, false));
        arrive(&mut r, 2 * MS, header(5, false)); // waits for 2
        assert_eq!(
            arrive(&mut r, 3 * MS, header(10, true)),
            [Discard(5), play(10, true, 0)]
        );
        assert_eq!(r.held(), 0);
        assert_eq!(r.stats().discarded, 1);
    }

    #[test]
    fn packets_above_the_first_one_of_a_run_that_came_before_it_are_kept() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(11, false)); // the run that begins at 10 got ahead of its start
        assert_eq!(
            arrive(&mut r, 2 * MS, header(10, true)),
            [play(10, true, 0), play(11, false, 0)]
        );
    }

    #[test]
    fn a_packet_that_gets_ahead_of_the_first_of_the_session_waits_for_it() {
        let mut r = receiver();
        assert_eq!(arrive(&mut r, Duration::ZERO, header(1, false)), []);
        assert_eq!(
            arrive(&mut r, 5 * MS, header(0, true)),
            [play(0, true, 0), play(1, false, 0)]
        );
    }

    #[test]
    fn if_the_first_packet_does_not_come_the_first_that_does_starts_the_run_after_the_hold() {
        let mut r = receiver();
        assert_eq!(arrive(&mut r, Duration::ZERO, header(5, false)), []);
        assert_eq!(r.tick(39 * MS), []);
        assert_eq!(r.tick(40 * MS), [play(5, true, 0)]);
        // The one that was waited for comes too late.
        assert_eq!(arrive(&mut r, 41 * MS, header(4, true)), [Discard(4)]);
        assert_eq!(
            arrive(&mut r, 42 * MS, header(6, false)),
            [play(6, false, 0)]
        );
        assert_eq!(r.stats().lost, 0);
    }

    #[test]
    fn a_packet_of_the_run_before_that_comes_before_the_start_of_the_next_is_dropped() {
        let mut r = receiver();
        assert_eq!(arrive(&mut r, Duration::ZERO, header(95, false)), []);
        assert_eq!(
            arrive(&mut r, 5 * MS, header(100, true)),
            [Discard(95), play(100, true, 0)]
        );
    }

    #[test]
    fn nothing_is_taken_while_the_viewer_listens_to_nothing() {
        let mut r = Receiver::new(Params::default());
        assert!(!r.running());
        assert_eq!(r.stream_started(), Err(Refusal::NotListening));
        r.begin_run();
        assert!(r.running());
        let id = r.stream_started().unwrap();
        r.stream_aborted(id);
        assert_eq!(r.end_run(), []);
        assert!(!r.running());
        assert_eq!(r.stream_started(), Err(Refusal::NotListening));
        assert_eq!(r.stats().refused, 2);
    }

    #[test]
    fn ending_a_run_forgets_the_streams_and_the_held_packets() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(3, false));
        arrive(&mut r, 2 * MS, header(2, false));
        let reading = r.stream_started().unwrap();
        r.stream_read(reading, 100).unwrap();
        assert_eq!(r.end_run(), [Discard(2), Discard(3)]);
        assert_eq!((r.streams(), r.held()), (0, 0));
        assert_eq!(r.next_deadline(), None);
        // The packet that was being read comes to its end after the run did.
        assert_eq!(r.stream_read(reading, 100), Err(Refusal::Unknown));
        assert_eq!(
            r.arrived(3 * MS, reading, header(4, false)),
            Ok(vec![Discard(4)])
        );
        assert_eq!(r.tick(Duration::from_secs(1)), []);
    }

    #[test]
    fn a_run_that_begins_again_starts_with_nothing_played() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(1, false));
        r.end_run();
        r.begin_run();
        assert_eq!(
            arrive(&mut r, 100 * MS, header(50, true)),
            [play(50, true, 0)]
        );
        assert_eq!(
            arrive(&mut r, 101 * MS, header(51, false)),
            [play(51, false, 0)]
        );
    }

    #[test]
    fn beginning_a_run_while_one_goes_changes_nothing() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(2, false)); // waits for 1
        r.begin_run();
        assert_eq!(r.held(), 1);
        assert_eq!(
            arrive(&mut r, 2 * MS, header(1, false)),
            [play(1, false, 0), play(2, false, 0)]
        );
    }

    #[test]
    fn streams_beyond_the_count_are_refused_and_a_place_is_given_back() {
        let params = Params {
            max_streams: 3,
            ..Params::default()
        };
        let mut receiver = Receiver::new(params);
        receiver.begin_run();
        let aborted = receiver.stream_started().unwrap();
        let arriving = receiver.stream_started().unwrap();
        receiver.stream_started().unwrap();
        assert_eq!(receiver.streams(), 3);
        assert_eq!(receiver.stream_started(), Err(Refusal::OverBudget));
        assert_eq!(receiver.stats().refused, 1);
        // A stream that breaks off gives its place back.
        receiver.stream_aborted(aborted);
        receiver.stream_started().unwrap();
        assert_eq!(receiver.stream_started(), Err(Refusal::OverBudget));
        // So does one that arrives.
        receiver
            .arrived(Duration::ZERO, arriving, header(0, true))
            .unwrap();
        assert_eq!(receiver.streams(), 2);
        receiver.stream_started().unwrap();
    }

    #[test]
    fn the_count_of_streams_is_the_one_in_the_protocol() {
        assert_eq!(Params::default().max_streams, 16);
        assert_eq!(Params::default().max_streams, crate::MAX_SOUND_STREAMS);
    }

    #[test]
    fn a_packet_beyond_the_limit_is_refused_and_forgotten() {
        let mut r = receiver();
        let id = r.stream_started().unwrap();
        r.stream_read(id, 4096).unwrap();
        r.stream_read(id, 4096).unwrap();
        assert_eq!(MAX_SOUND_PACKET_BYTES, 8192);
        assert_eq!(r.stream_read(id, 1), Err(Refusal::TooLarge));
        assert_eq!(r.stream_read(id, 1), Err(Refusal::Unknown));
        assert_eq!(r.streams(), 0);
    }

    #[test]
    fn a_stream_that_is_not_known_has_no_bytes_and_brings_no_packet() {
        let mut r = receiver();
        let id = r.stream_started().unwrap();
        r.stream_aborted(id);
        assert_eq!(r.stream_read(id, 10), Err(Refusal::Unknown));
        assert_eq!(
            r.arrived(Duration::ZERO, id, header(7, true)),
            Ok(vec![Discard(7)])
        );
        assert_eq!(r.held(), 0);
        // The same stream cannot bring a second packet either.
        let id = r.stream_started().unwrap();
        r.arrived(Duration::ZERO, id, header(0, true)).unwrap();
        assert_eq!(
            r.arrived(Duration::ZERO, id, header(1, false)),
            Ok(vec![Discard(1)])
        );
    }

    #[test]
    fn a_dropped_packet_does_not_take_the_place_of_one_that_is_held() {
        // The Discard of a stream the viewer does not know names the packet just read, and the
        // packet with the same number that is held stays.
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(2, false));
        let id = r.stream_started().unwrap();
        r.stream_aborted(id);
        assert_eq!(
            r.arrived(2 * MS, id, header(2, false)),
            Ok(vec![Discard(2)])
        );
        assert_eq!(r.held(), 1);
    }

    #[test]
    fn when_too_many_wait_the_viewer_goes_on_without_what_is_missing() {
        let params = Params {
            max_held: 4,
            ..Params::default()
        };
        let mut r = Receiver::new(params);
        r.begin_run();
        arrive(&mut r, Duration::ZERO, header(10, true));
        for number in 12..16 {
            assert_eq!(arrive(&mut r, MS, header(number, false)), []);
        }
        assert_eq!(r.held(), 4);
        // One more than may wait: packet 11 is given up and the rest follow in order.
        assert_eq!(
            arrive(&mut r, 2 * MS, header(16, false)),
            [
                play(12, false, 1),
                play(13, false, 0),
                play(14, false, 0),
                play(15, false, 0),
                play(16, false, 0),
            ]
        );
        assert_eq!(r.held(), 0);
    }

    #[test]
    fn numbers_far_apart_are_counted_without_overflow() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, MS, header(u64::MAX, false));
        assert_eq!(r.tick(41 * MS), [play(u64::MAX, false, u64::MAX - 1)]);
        assert_eq!(r.stats().lost, u64::MAX - 1);
        // Nothing can come after the last number, and nothing panics when something does.
        assert_eq!(arrive(&mut r, 42 * MS, header(7, false)), [Discard(7)]);
    }

    #[test]
    fn a_packet_that_is_dropped_as_late_still_lets_the_ones_that_waited_too_long_go() {
        let mut r = receiver();
        assert_eq!(arrive(&mut r, Duration::ZERO, header(6, false)), []);
        assert_eq!(
            arrive(&mut r, 40 * MS, header(8, false)),
            [play(6, true, 0)]
        );
        assert_eq!(
            arrive(&mut r, 80 * MS, header(0, false)),
            [Discard(0), play(8, false, 1)]
        );
    }

    #[test]
    fn the_deadline_is_that_of_the_packet_that_has_waited_the_longest() {
        let mut r = receiver();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, 10 * MS, header(3, false));
        arrive(&mut r, 30 * MS, header(2, false));
        assert_eq!(r.next_deadline(), Some(50 * MS));
    }

    #[test]
    fn the_hold_can_be_set() {
        let params = Params {
            hold: 100 * MS,
            ..Params::default()
        };
        let mut r = Receiver::new(params);
        r.begin_run();
        arrive(&mut r, Duration::ZERO, header(0, true));
        arrive(&mut r, Duration::ZERO, header(2, false));
        assert_eq!(r.tick(99 * MS), []);
        assert_eq!(r.tick(100 * MS), [play(2, false, 1)]);
    }

    #[derive(Debug, Clone)]
    enum Event {
        Arrive { number: u64, first: bool },
        Tick,
        EndRun,
        BeginRun,
    }

    fn events() -> impl Strategy<Value = Vec<(u64, Event)>> {
        let event = prop_oneof![
            8 => (0_u64..30, prop_oneof![7 => Just(false), 1 => Just(true)])
                .prop_map(|(number, first)| Event::Arrive { number, first }),
            2 => Just(Event::Tick),
            1 => Just(Event::EndRun),
            1 => Just(Event::BeginRun),
        ];
        proptest::collection::vec((0_u64..70, event), 1..120)
    }

    proptest! {
        // Whatever order, time, repeats and restarts the packets come in, every packet that the
        // viewer is handed is named by exactly one action, the packets are played in the order
        // of their numbers within a run, what was skipped is counted, nothing is held beyond
        // the limit, and nothing waits longer than the hold.
        #[test]
        fn every_packet_is_played_or_dropped_once_and_in_order(events in events()) {
            let params = Params { max_held: 8, ..Params::default() };
            let mut r = Receiver::new(params);
            let mut now = Duration::ZERO;
            // The packets the caller holds the bytes of.
            let mut kept: BTreeSet<u64> = BTreeSet::new();
            let mut last: Option<u64> = None;
            let apply = |actions: Vec<Action>, kept: &mut BTreeSet<u64>, last: &mut Option<u64>| {
                for action in actions {
                    match action {
                        Play { number, first, lost_before } => {
                            prop_assert!(kept.remove(&number), "{number} was played without being held");
                            if first {
                                prop_assert_eq!(lost_before, 0);
                            } else {
                                let before = last.expect("a run begins with its first packet");
                                prop_assert!(number > before, "{number} after {before}");
                                prop_assert_eq!(lost_before, number - before - 1);
                            }
                            *last = Some(number);
                        }
                        Discard(number) => {
                            prop_assert!(kept.remove(&number), "{number} was dropped without being held");
                        }
                    }
                }
                Ok(())
            };
            for (step, event) in events {
                now += Duration::from_millis(step);
                // Only the calls that are told the time can act on it.
                let timed = matches!(event, Event::Arrive { .. } | Event::Tick);
                match event {
                    Event::Arrive { number, first } => match r.stream_started() {
                        Err(refusal) => {
                            prop_assert!(!r.running());
                            prop_assert_eq!(refusal, Refusal::NotListening);
                        }
                        Ok(id) => {
                            let header = header(number, first);
                            if kept.contains(&number) {
                                prop_assert_eq!(
                                    r.arrived(now, id, header),
                                    Err(Violation::RepeatedNumber { number })
                                );
                                // The session would end here, so the stream just goes.
                                continue;
                            }
                            kept.insert(number);
                            let actions = r.arrived(now, id, header).unwrap();
                            apply(actions, &mut kept, &mut last)?;
                        }
                    },
                    Event::Tick => {
                        let actions = r.tick(now);
                        apply(actions, &mut kept, &mut last)?;
                    }
                    Event::EndRun => {
                        let actions = r.end_run();
                        apply(actions, &mut kept, &mut last)?;
                        last = None;
                    }
                    Event::BeginRun => r.begin_run(),
                }
                prop_assert_eq!(r.held(), kept.len());
                prop_assert!(r.held() <= 8);
                // Nothing that waits has waited for the hold: it would have been played.
                if timed {
                    prop_assert!(r.next_deadline().is_none_or(|deadline| deadline > now));
                }
            }
            let actions = r.end_run();
            apply(actions, &mut kept, &mut last)?;
            prop_assert!(kept.is_empty());
        }
    }
}
