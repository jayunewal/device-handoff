// SPDX-License-Identifier: Apache-2.0

//! The source's accounting of packets of sound in flight.
//!
//! Each packet travels on a stream of its own, as a frame of video does, and the source learns
//! two things about it from QUIC: that the peer has all of it (the stream is acknowledged), and,
//! by resetting the stream, that it no longer wants to send it. A packet is a few hundred bytes,
//! so its size never holds the path back. What goes wrong is time: a path that stops, or is slow,
//! or loses a packet and waits for the repeat. A packet that arrives after the player has gone on
//! is of no use, and the packets behind it wait on the same path, so letting a late packet go on
//! makes every packet after it late too. Two rules keep the delay bounded (ADR 0012):
//!
//! 1. At most [`Params::max_in_flight`] packets are in flight. A source with one more to send
//!    resets the oldest, which is the one nearest to being too late.
//! 2. A packet that is still in flight [`Params::grace`] and a number of round trips after it was
//!    written is late and is reset.
//!
//! A packet that is reset is not sent again, and the next packet takes the next number, so the
//! viewer can tell what is missing. A packet that the session has no room for, because the viewer
//! takes only so many streams at once, is skipped in the same way ([`Sender::skip`]).
//!
//! Like the other rules of this crate, this is a state machine with no input or output. It is told
//! what happened (a packet was made, a stream was acknowledged, time passed) and answers with what
//! to do (reset these streams), and it reads no clock, so a test can drive it with a simulated
//! link.

use std::{collections::BTreeMap, mem, time::Duration};

use baton_proto::id::SessionId;

use super::{PacketFlags, PacketHeader};

/// The numbers that the rules use. The values are where ADR 0012 starts; the tests settle them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// Most packets in flight at once. A source with one more to send resets the oldest.
    pub max_in_flight: usize,
    /// How late a packet may be on top of the round trips.
    pub grace: Duration,
    /// How many round trips come on top of the grace.
    pub round_trips: u32,
    /// The longest a packet is given, however slow the path.
    pub longest: Duration,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            max_in_flight: 8,
            grace: Duration::from_millis(200),
            round_trips: 2,
            longest: Duration::from_secs(1),
        }
    }
}

/// The result of [`Sender::sent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// The header to write at the start of the packet's stream.
    pub header: PacketHeader,
    /// The streams of earlier packets that are to be reset: the oldest if there were too many in
    /// flight, and those that are late.
    pub reset: Vec<u64>,
}

/// Counts of what happened, for reports and benchmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Packets made and written.
    pub sent: u64,
    /// Packets whose stream the peer acknowledged.
    pub acked: u64,
    /// Packets reset because they were late.
    pub reset_late: u64,
    /// Packets reset because one more was to be sent and too many were in flight.
    pub reset_over: u64,
    /// Packets whose stream the peer stopped.
    pub stopped: u64,
    /// Packets that were made and not written, because the session had no room for one more
    /// stream.
    pub skipped: u64,
}

/// The accounting of one session's packets of sound, on the source.
#[derive(Debug)]
pub struct Sender {
    params: Params,
    session: SessionId,
    next_number: u64,
    /// Whether the next packet begins a run.
    first: bool,
    /// The packets whose streams are written and neither acknowledged nor reset, with the time
    /// from which each is late.
    in_flight: BTreeMap<u64, Duration>,
    stats: Stats,
}

impl Sender {
    /// Starts the accounting for a session. The first packet begins a run.
    pub fn new(session: SessionId, params: Params) -> Self {
        Self {
            params,
            session,
            next_number: 0,
            first: true,
            in_flight: BTreeMap::new(),
            stats: Stats::default(),
        }
    }

    /// The next packet begins a run of sound: the capture started afresh, or the viewer asked
    /// to hear again. The numbers go on from where they were.
    pub fn begin_run(&mut self) {
        self.first = true;
    }

    /// How many packets are in flight.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// What has been counted so far.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The number that the next packet will get.
    pub fn next_number(&self) -> u64 {
        self.next_number
    }

    /// The time from which the earliest packet in flight is late, if one is in flight. A caller
    /// that has nothing else to wake it can sleep until then and call [`poll`](Self::poll).
    pub fn next_deadline(&self) -> Option<Duration> {
        self.in_flight.values().min().copied()
    }

    /// A packet was made, its sound captured at `capture_time` microseconds into the session, and
    /// its stream is written at `now`. `rtt` is the round-trip time of the path.
    ///
    /// Returns the header to write and the streams to reset. Every packet that is made is sent:
    /// it is sound that has just been played, and the packets before it are older.
    pub fn sent(&mut self, now: Duration, capture_time: u64, rtt: Duration) -> Sent {
        let mut reset = self.late(now);
        while self.in_flight.len() >= self.params.max_in_flight.max(1) {
            let Some((oldest, _)) = self.in_flight.pop_first() else {
                break;
            };
            self.stats.reset_over += 1;
            reset.push(oldest);
        }
        let number = self.next_number;
        self.next_number += 1;
        self.in_flight
            .insert(number, now.saturating_add(self.allowance(rtt)));
        self.stats.sent += 1;
        let flags = if mem::take(&mut self.first) {
            PacketFlags::FIRST
        } else {
            PacketFlags::NONE
        };
        Sent {
            header: PacketHeader {
                session: self.session,
                number,
                capture_time,
                flags,
            },
            reset,
        }
    }

    /// A packet was made and cannot be written, because the session has no room for another
    /// stream: the viewer takes only so many at once, and the streams of the packets before it are
    /// open or being reset. The packet takes a number and is not sent, so the viewer sees that it
    /// is missing and covers the gap, and the number is never used again. Returns the number.
    ///
    /// The packet that follows still begins a run if this one was to.
    pub fn skip(&mut self) -> u64 {
        let number = self.next_number;
        self.next_number += 1;
        self.stats.skipped += 1;
        number
    }

    /// The peer has acknowledged all of the packet's stream.
    pub fn acked(&mut self, number: u64) {
        if self.in_flight.remove(&number).is_some() {
            self.stats.acked += 1;
        }
    }

    /// The peer stopped reading the packet's stream, so the packet will not arrive. There is
    /// nothing to reset. A viewer does this for a packet that comes when it listens to nothing,
    /// or beyond its count.
    pub fn stopped(&mut self, number: u64) {
        if self.in_flight.remove(&number).is_some() {
            self.stats.stopped += 1;
        }
    }

    /// Looks at the clock. Returns the streams to reset because their packets are late.
    pub fn poll(&mut self, now: Duration) -> Vec<u64> {
        self.late(now)
    }

    fn late(&mut self, now: Duration) -> Vec<u64> {
        let late: Vec<u64> = self
            .in_flight
            .iter()
            .filter(|(_, due)| now > **due)
            .map(|(number, _)| *number)
            .collect();
        for number in &late {
            self.in_flight.remove(number);
        }
        self.stats.reset_late += u64::try_from(late.len()).unwrap_or(u64::MAX);
        late
    }

    /// How long a packet may be in flight if it is written on a path with this round-trip time.
    fn allowance(&self, rtt: Duration) -> Duration {
        self.params
            .grace
            .saturating_add(rtt.saturating_mul(self.params.round_trips))
            .min(self.params.longest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn session() -> SessionId {
        SessionId::from_bytes([7; SessionId::LEN])
    }

    fn sender() -> Sender {
        Sender::new(session(), Params::default())
    }

    fn rtt() -> Duration {
        20 * MS
    }

    /// Sends a packet at `now` captured at the same time, in microseconds.
    fn send(sender: &mut Sender, now: Duration) -> Sent {
        let capture = u64::try_from(now.as_micros()).unwrap();
        sender.sent(now, capture, rtt())
    }

    #[test]
    fn the_first_packet_begins_a_run_and_the_ones_after_it_do_not() {
        let mut s = sender();
        let first = send(&mut s, Duration::ZERO);
        let second = send(&mut s, 21 * MS);
        assert!(first.header.is_first());
        assert!(!second.header.is_first());
        assert_eq!((first.header.number, second.header.number), (0, 1));
        assert_eq!(first.header.session, session());
        assert_eq!(second.header.capture_time, 21_000);
        assert_eq!(first.reset, [] as [u64; 0]);
    }

    #[test]
    fn a_run_that_begins_again_is_marked_and_the_numbers_go_on() {
        let mut s = sender();
        for i in 0..3 {
            let sent = send(&mut s, i * 21 * MS);
            s.acked(sent.header.number);
        }
        s.begin_run();
        let again = send(&mut s, 70 * MS);
        assert!(again.header.is_first());
        assert_eq!(again.header.number, 3);
        assert!(!send(&mut s, 91 * MS).header.is_first());
    }

    #[test]
    fn the_number_of_a_packet_is_never_used_again() {
        let mut s = sender();
        let mut numbers = Vec::new();
        for i in 0..40 {
            let sent = send(&mut s, i * 21 * MS);
            numbers.push(sent.header.number);
            // Nothing is acknowledged, so packets are reset to make room and for being late.
        }
        assert_eq!(numbers, (0..40).collect::<Vec<u64>>());
        assert_eq!(s.next_number(), 40);
    }

    #[test]
    fn one_packet_too_many_in_flight_resets_the_oldest() {
        let mut s = sender();
        for i in 0..8 {
            assert_eq!(send(&mut s, i * MS).reset, [] as [u64; 0]);
        }
        assert_eq!(s.in_flight(), 8);
        let ninth = send(&mut s, 8 * MS);
        assert_eq!(ninth.reset, [0]);
        assert_eq!(s.in_flight(), 8);
        let tenth = send(&mut s, 9 * MS);
        assert_eq!(tenth.reset, [1]);
        assert_eq!(s.stats().reset_over, 2);
        assert_eq!(s.stats().reset_late, 0);
    }

    #[test]
    fn an_acknowledged_packet_makes_room_and_nothing_is_reset() {
        let mut s = sender();
        for i in 0..8 {
            send(&mut s, i * MS);
        }
        s.acked(3);
        assert_eq!(send(&mut s, 8 * MS).reset, [] as [u64; 0]);
        assert_eq!(s.in_flight(), 8);
        assert_eq!(s.stats().acked, 1);
    }

    #[test]
    fn a_packet_that_the_peer_stopped_makes_room_and_is_not_reset() {
        let mut s = sender();
        for i in 0..8 {
            send(&mut s, i * MS);
        }
        s.stopped(0);
        assert_eq!(send(&mut s, 8 * MS).reset, [] as [u64; 0]);
        assert_eq!(s.stats().stopped, 1);
    }

    #[test]
    fn an_acknowledgement_for_a_packet_that_is_not_in_flight_changes_nothing() {
        let mut s = sender();
        let sent = send(&mut s, Duration::ZERO);
        s.acked(sent.header.number);
        // Twice, a number that was reset, and a number that was never sent.
        s.acked(sent.header.number);
        s.stopped(sent.header.number);
        s.acked(99);
        s.stopped(99);
        assert_eq!(
            s.stats(),
            Stats {
                sent: 1,
                acked: 1,
                ..Stats::default()
            }
        );
    }

    #[test]
    fn a_packet_is_late_after_the_grace_and_two_round_trips() {
        let mut s = sender();
        send(&mut s, Duration::ZERO);
        // 200 ms of grace and 2 x 20 ms: late after 240 ms, and not at 240 ms.
        assert_eq!(s.poll(240 * MS), [] as [u64; 0]);
        assert_eq!(s.next_deadline(), Some(240 * MS));
        assert_eq!(s.poll(241 * MS), [0]);
        assert_eq!(s.in_flight(), 0);
        assert_eq!(s.stats().reset_late, 1);
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn the_time_a_packet_is_given_grows_with_the_round_trip_and_stops_at_the_longest() {
        let mut s = sender();
        s.sent(Duration::ZERO, 0, 100 * MS);
        assert_eq!(s.next_deadline(), Some(400 * MS));
        let mut s = sender();
        s.sent(Duration::ZERO, 0, Duration::from_secs(5));
        assert_eq!(s.next_deadline(), Some(Duration::from_secs(1)));
        // A round trip that cannot be added up is still a packet that is reset in the end.
        let mut s = sender();
        s.sent(Duration::ZERO, 0, Duration::MAX);
        assert_eq!(s.next_deadline(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn the_late_are_reset_oldest_first_and_the_ones_that_are_not_late_stay() {
        let mut s = sender();
        for i in 0..5 {
            send(&mut s, i * 50 * MS);
        }
        // Packets 0 and 1 were written at 0 and 50 ms and are late after 240 and 290 ms.
        assert_eq!(s.poll(300 * MS), [0, 1]);
        assert_eq!(s.in_flight(), 3);
        assert_eq!(s.stats().reset_late, 2);
    }

    #[test]
    fn a_packet_that_is_written_also_resets_the_late_ones() {
        let mut s = sender();
        send(&mut s, Duration::ZERO);
        send(&mut s, 10 * MS);
        let sent = send(&mut s, 300 * MS);
        assert_eq!(sent.reset, [0, 1]);
        assert_eq!(s.in_flight(), 1);
        assert_eq!(s.stats().reset_late, 2);
    }

    #[test]
    fn late_packets_and_too_many_packets_are_both_reset_by_one_packet() {
        let params = Params {
            max_in_flight: 3,
            ..Params::default()
        };
        let mut s = Sender::new(session(), params);
        s.sent(Duration::ZERO, 0, rtt()); // 0, late after 240 ms
        s.sent(200 * MS, 0, rtt()); // 1
        s.sent(210 * MS, 0, rtt()); // 2
        let sent = s.sent(250 * MS, 0, rtt()); // 0 is late, 1 and 2 stay
        assert_eq!(sent.reset, [0]);
        let sent = s.sent(251 * MS, 0, rtt()); // 1, 2, 3 are in flight: one too many
        assert_eq!(sent.reset, [1]);
        assert_eq!(s.in_flight(), 3);
        assert_eq!(s.stats().reset_late, 1);
        assert_eq!(s.stats().reset_over, 1);
    }

    #[test]
    fn a_packet_that_cannot_be_written_takes_a_number_and_is_never_in_flight() {
        let mut s = sender();
        assert_eq!(send(&mut s, Duration::ZERO).header.number, 0);
        assert_eq!(s.skip(), 1);
        let third = send(&mut s, 42 * MS);
        assert_eq!(third.header.number, 2);
        assert_eq!(third.reset, [] as [u64; 0]);
        // Only the two that were written are in flight, and the skipped one has no deadline.
        assert_eq!(s.in_flight(), 2);
        assert_eq!(s.next_number(), 3);
        assert_eq!(s.stats().skipped, 1);
        assert_eq!(s.stats().sent, 2);
        assert_eq!(s.poll(241 * MS), [0]);
        assert_eq!(s.poll(500 * MS), [2]);
    }

    #[test]
    fn a_packet_that_cannot_be_written_does_not_take_the_start_of_a_run() {
        let mut s = sender();
        assert_eq!(s.skip(), 0);
        let first = send(&mut s, 21 * MS);
        assert!(first.header.is_first());
        assert_eq!(first.header.number, 1);
        assert!(!send(&mut s, 42 * MS).header.is_first());
    }

    #[test]
    fn a_limit_of_zero_still_lets_one_packet_through() {
        let params = Params {
            max_in_flight: 0,
            ..Params::default()
        };
        let mut s = Sender::new(session(), params);
        s.sent(Duration::ZERO, 0, rtt());
        let sent = s.sent(MS, 0, rtt());
        assert_eq!(sent.reset, [0]);
        assert_eq!(s.in_flight(), 1);
    }
}
