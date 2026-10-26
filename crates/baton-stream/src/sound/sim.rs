// SPDX-License-Identifier: Apache-2.0

//! A simulated link for sound, to try the sender and the receiver together.
//!
//! The link serialises the bytes of the packets in the order in which they were written, at a rate
//! that can change and stop. Bytes that were serialised cannot be taken back: a stream that is
//! reset loses what was not sent yet, and what is already on its way arrives. A packet that the
//! link loses takes four more one-way delays to arrive, as QUIC repeats it, so it arrives after
//! packets that were written later.
//!
//! The source makes a packet of 1024 samples at 48 kHz every 21.333 ms. There is no player: what
//! is measured is when each packet is handed to the decoder, and what the viewer says about the
//! ones it skipped.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap, VecDeque},
    time::Duration,
};

use baton_proto::id::SessionId;

use super::{
    PacketHeader,
    receive::{self, Action, Receiver},
    send::{self, Sender},
};
use crate::sim::{Link, Rng};

const STEP: Duration = Duration::from_millis(1);

/// 1024 samples at 48000 Hz.
const INTERVAL: Duration = Duration::from_micros(21_333);

/// What to simulate.
#[derive(Clone)]
pub(crate) struct Config {
    pub link: Link,
    pub run: Duration,
    /// The bytes of a packet, the header not counted.
    pub packet_bytes: usize,
    pub sender: send::Params,
    pub receiver: receive::Params,
    pub seed: u64,
    /// Times at which the viewer turns sound off and on again: off at, on at.
    pub pauses: Vec<(Duration, Duration)>,
    /// How much later than the first packet of a run its `Sound` arrives. Both leave the source
    /// at the same time on streams of their own.
    pub sound_late: Duration,
}

impl Config {
    pub(crate) fn new(link: Link, run: Duration) -> Self {
        Self {
            link,
            run,
            packet_bytes: 340,
            sender: send::Params::default(),
            receiver: receive::Params::default(),
            seed: 0x9e37_79b9_7f4a_7c15,
            pauses: Vec::new(),
            sound_late: Duration::ZERO,
        }
    }
}

/// A packet that was handed to the decoder.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Played {
    pub at: Duration,
    pub number: u64,
    pub captured: Duration,
    pub first: bool,
    pub lost_before: u64,
}

/// What a run came to.
pub(crate) struct Report {
    pub played: Vec<Played>,
    pub made: u64,
    pub sender: send::Stats,
    pub receiver: receive::Stats,
    pub max_in_flight: usize,
    pub max_held: usize,
}

impl Report {
    /// The delay from capture to the decoder of the packet at the given share, in thousandths, of
    /// the packets played at or after `from`.
    pub(crate) fn delay_at(&self, permille: usize, from: Duration) -> Duration {
        let mut delays: Vec<Duration> = self
            .played
            .iter()
            .filter(|p| p.at >= from)
            .map(|p| p.at.saturating_sub(p.captured))
            .collect();
        delays.sort();
        let at = delays.len().saturating_sub(1) * permille / 1000;
        delays.get(at).copied().unwrap_or_default()
    }

    /// The longest delay of a packet played in `from..to`.
    pub(crate) fn longest_delay(&self, from: Duration, to: Duration) -> Duration {
        self.played
            .iter()
            .filter(|p| p.at >= from && p.at < to)
            .map(|p| p.at.saturating_sub(p.captured))
            .max()
            .unwrap_or_default()
    }

    /// How many packets were played with a number that is not above the one played before it in
    /// the same run. A decoder does not go back, so there are none.
    pub(crate) fn out_of_order(&self) -> usize {
        let mut last: Option<u64> = None;
        let mut count = 0;
        for played in &self.played {
            if played.first {
                last = None;
            }
            if last.is_some_and(|last| played.number <= last) {
                count += 1;
            }
            last = Some(played.number);
        }
        count
    }

    /// The packets that the viewer skipped, as it said.
    pub(crate) fn skipped(&self) -> u64 {
        self.played.iter().map(|p| p.lost_before).sum()
    }
}

/// Something that happens at a time, outside the packets.
#[derive(Clone, Copy)]
enum Event {
    /// The viewer turns sound off.
    ViewerOff,
    /// The source learns that, and stops.
    SourceStops,
    /// The source learns that the viewer asks to hear, and begins a run.
    SourceBegins,
    /// The `Sound` of the new run reaches the viewer.
    ViewerBegins,
}

struct Writing {
    number: u64,
    total: u64,
    left: u64,
}

struct World<'a> {
    config: &'a Config,
    t: Duration,
    rng: Rng,
    sender: Sender,
    receiver: Receiver,
    source_on: bool,
    next_capture: Duration,
    events: Vec<(Duration, Event)>,
    /// When each packet was captured and its header.
    made: BTreeMap<u64, (Duration, PacketHeader)>,
    /// Packets being serialised, in the order they were written.
    writing: VecDeque<Writing>,
    /// (arrives at, packet number), earliest first.
    flying: BinaryHeap<Reverse<(Duration, u64)>>,
    /// What the source hears about a packet: (arrives at, number, whether the viewer stopped it).
    news: BinaryHeap<Reverse<(Duration, u64, bool)>>,
    credit: u64,
    report: Report,
}

/// Runs the simulation.
pub(crate) fn run(config: &Config) -> Report {
    let session = SessionId::from_bytes([9; SessionId::LEN]);
    let delay = config.link.delay;
    let mut events = Vec::new();
    for (off, on) in &config.pauses {
        events.push((*off, Event::ViewerOff));
        events.push((*off + delay, Event::SourceStops));
        events.push((*on + delay, Event::SourceBegins));
        events.push((*on + 2 * delay + config.sound_late, Event::ViewerBegins));
    }
    events.sort_by_key(|(at, _)| *at);
    events.reverse();
    let mut receiver = Receiver::new(config.receiver);
    receiver.begin_run();
    let mut world = World {
        config,
        t: Duration::ZERO,
        rng: Rng(config.seed),
        sender: Sender::new(session, config.sender),
        receiver,
        source_on: true,
        next_capture: Duration::ZERO,
        events,
        made: BTreeMap::new(),
        writing: VecDeque::new(),
        flying: BinaryHeap::new(),
        news: BinaryHeap::new(),
        credit: 0,
        report: Report {
            played: Vec::new(),
            made: 0,
            sender: send::Stats::default(),
            receiver: receive::Stats::default(),
            max_in_flight: 0,
            max_held: 0,
        },
    };
    while world.t < config.run {
        world.step();
        world.t += STEP;
    }
    world.report.sender = world.sender.stats();
    world.report.receiver = world.receiver.stats();
    world.report
}

impl World<'_> {
    fn step(&mut self) {
        self.serialise();
        self.deliver();
        self.hear();
        self.happen();
        let late = self.sender.poll(self.t);
        self.reset(&late);
        if self.source_on && self.t >= self.next_capture {
            self.next_capture += INTERVAL;
            self.capture();
        }
        let actions = self.receiver.tick(self.t);
        self.apply(actions);
        self.report.max_in_flight = self.report.max_in_flight.max(self.sender.in_flight());
        self.report.max_held = self.report.max_held.max(self.receiver.held());
    }

    /// What the connection would tell the source about the round trip: the delay both ways, and
    /// the time the queue needs to go out at the speed of the link when it is not stalled. A
    /// stall does not change it, because the estimate only moves when something is acknowledged.
    fn rtt(&self) -> Duration {
        let link = &self.config.link;
        let queued: u64 = self.writing.iter().map(|w| w.left).sum();
        2 * link.delay + Duration::from_micros(queued * 1_000_000 / link.rate.max(1))
    }

    fn capture(&mut self) {
        let captured = self.t;
        let micros = u64::try_from(captured.as_micros()).unwrap_or(u64::MAX);
        let sent = self.sender.sent(self.t, micros, self.rtt());
        self.reset(&sent.reset);
        let number = sent.header.number;
        self.report.made += 1;
        self.made.insert(number, (captured, sent.header));
        let total = u64::try_from(self.config.packet_bytes + PacketHeader::LEN).unwrap_or(u64::MAX);
        self.writing.push_back(Writing {
            number,
            total,
            left: total,
        });
    }

    /// The source resets streams. What the link has not started to send is gone, and a packet
    /// that it has started to send, or has sent, arrives.
    fn reset(&mut self, numbers: &[u64]) {
        for number in numbers {
            self.writing
                .retain(|w| w.number != *number || w.left < w.total);
        }
    }

    /// The link sends bytes. Rates are in bytes a second, so a millisecond gets a thousandth.
    fn serialise(&mut self) {
        let link = &self.config.link;
        self.credit = self.credit.saturating_add(link.rate_at(self.t));
        let mut budget = self.credit / 1000;
        self.credit %= 1000;
        while budget > 0 {
            let Some(front) = self.writing.front_mut() else {
                self.credit = 0;
                break;
            };
            let take = budget.min(front.left);
            front.left -= take;
            budget -= take;
            if front.left > 0 {
                break;
            }
            let Some(done) = self.writing.pop_front() else {
                break;
            };
            let extra = if self.rng.chance(link.loss_permille) {
                4 * link.delay
            } else {
                Duration::ZERO
            };
            // The first packet of a run may overtake its `Sound`, or the other way round.
            self.flying
                .push(Reverse((self.t + link.delay + extra, done.number)));
        }
    }

    /// Packets arrive at the viewer.
    fn deliver(&mut self) {
        let link = &self.config.link;
        while let Some(Reverse((at, number))) = self.flying.peek().copied() {
            if at > self.t {
                break;
            }
            self.flying.pop();
            let Some((_, header)) = self.made.get(&number).copied() else {
                continue;
            };
            let late = if self.rng.chance(link.ack_loss_permille) {
                6 * link.delay
            } else {
                Duration::ZERO
            };
            let heard = self.t + link.delay + late;
            let Ok(id) = self.receiver.stream_started() else {
                // Stopped with 499: the source hears that the stream is over.
                self.news.push(Reverse((heard, number, true)));
                continue;
            };
            self.receiver
                .stream_read(id, self.config.packet_bytes)
                .expect("a packet within the limit");
            self.news.push(Reverse((heard, number, false)));
            let actions = self
                .receiver
                .arrived(self.t, id, header)
                .expect("the sender gives each packet a number of its own");
            self.apply(actions);
        }
    }

    /// Acknowledgements and stops arrive at the source.
    fn hear(&mut self) {
        while let Some(Reverse((at, number, stopped))) = self.news.peek().copied() {
            if at > self.t {
                break;
            }
            self.news.pop();
            if stopped {
                self.sender.stopped(number);
            } else {
                self.sender.acked(number);
            }
        }
    }

    fn happen(&mut self) {
        while self.events.last().is_some_and(|(at, _)| *at <= self.t) {
            let Some((_, event)) = self.events.pop() else {
                break;
            };
            match event {
                Event::ViewerOff => {
                    let actions = self.receiver.end_run();
                    self.apply(actions);
                }
                Event::SourceStops => self.source_on = false,
                Event::SourceBegins => {
                    self.sender.begin_run();
                    self.source_on = true;
                    self.next_capture = self.t;
                }
                Event::ViewerBegins => self.receiver.begin_run(),
            }
        }
    }

    fn apply(&mut self, actions: Vec<Action>) {
        for action in actions {
            if let Action::Play {
                number,
                first,
                lost_before,
            } = action
            {
                let captured = self
                    .made
                    .get(&number)
                    .map_or(Duration::ZERO, |(captured, _)| *captured);
                self.report.played.push(Played {
                    at: self.t,
                    number,
                    captured,
                    first,
                    lost_before,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// The share of the packets made that the viewer played, in thousandths.
    fn played_permille(report: &Report) -> u64 {
        u64::try_from(report.played.len()).unwrap() * 1000 / report.made.max(1)
    }

    #[test]
    fn on_a_good_link_every_packet_is_played_in_order_at_the_delay_of_the_link() {
        let config = Config::new(Link::new(10, 5), secs(10));
        let report = run(&config);
        assert!(report.made > 450);
        // The last packets are still on their way when the run ends.
        assert!(report.played.len() as u64 + 3 >= report.made);
        assert_eq!(report.skipped(), 0);
        assert_eq!(report.out_of_order(), 0);
        assert_eq!(report.receiver.late, 0);
        assert_eq!(report.receiver.refused, 0);
        assert_eq!(report.sender.reset_late + report.sender.reset_over, 0);
        // One way, and a step of the simulation for the capture and one for the link.
        assert!(report.delay_at(1000, Duration::ZERO) <= 8 * MS);
        // The round trip is 10 ms and a packet is made every 21 ms.
        assert!(report.max_in_flight <= 2, "{}", report.max_in_flight);
        assert_eq!(report.played.iter().filter(|p| p.first).count(), 1);
    }

    #[test]
    fn a_link_that_loses_packets_is_put_back_in_order_without_skipping_any() {
        // A packet that is lost takes 40 ms more, so the one after it, and the one after that,
        // arrive first. The hold of 40 ms is for this.
        let mut link = Link::new(10, 10);
        link.loss_permille = 100;
        let config = Config::new(link, secs(20));
        let report = run(&config);
        assert!(report.played.len() as u64 + 4 >= report.made);
        assert_eq!(report.skipped(), 0, "no packet is given up on");
        assert_eq!(report.out_of_order(), 0);
        assert_eq!(report.receiver.late, 0);
        assert!(report.max_held >= 1, "the link did reorder packets");
        // A packet waits for the repeat of the one before it: at most the extra 40 ms, less the
        // time the later packet has been on its way.
        assert!(report.longest_delay(Duration::ZERO, secs(20)) <= 75 * MS);
        assert!(report.delay_at(500, Duration::ZERO) <= 15 * MS);
    }

    #[test]
    fn a_repeat_that_takes_too_long_is_skipped_and_the_packet_is_dropped_when_it_comes() {
        // A repeat takes 160 ms more than a packet that is not lost, which is longer than the
        // hold: the viewer goes on without the packet and drops it when it arrives.
        let mut link = Link::new(10, 40);
        link.loss_permille = 50;
        let config = Config::new(link, secs(30));
        let report = run(&config);
        assert!(report.skipped() > 10, "{}", report.skipped());
        assert_eq!(report.out_of_order(), 0);
        // Every packet that was skipped comes after all, and comes late.
        let stats = report.receiver;
        assert!(stats.late > 0);
        assert!(stats.late <= report.skipped());
        // Nobody waits for what does not come: a packet is played at most the hold after it
        // arrives, and the link takes 40 ms.
        assert!(report.delay_at(990, Duration::ZERO) <= 90 * MS);
        assert!(report.longest_delay(Duration::ZERO, secs(30)) <= 130 * MS);
    }

    #[test]
    fn a_stall_costs_the_sound_of_the_stall_and_the_delay_returns_to_normal() {
        let mut link = Link::new(10, 5);
        link.stalls.push((secs(2), secs(3)));
        let config = Config::new(link, secs(6));
        let report = run(&config);
        assert!(report.max_in_flight <= 8, "{}", report.max_in_flight);
        assert_eq!(report.out_of_order(), 0);
        // Before the stall the delay is that of the link.
        assert!(report.longest_delay(Duration::ZERO, secs(2)) <= 8 * MS);
        // Nothing is played while the link is down, except what was on its way.
        let during = report
            .played
            .iter()
            .filter(|p| p.at > Duration::from_millis(2_020) && p.at < secs(3))
            .count();
        assert_eq!(during, 0);
        // What the source kept for after the stall is the newest of what it made: at most the
        // eight packets in flight, and nothing older than the time a packet is given.
        let resumed = report
            .played
            .iter()
            .find(|p| p.at >= secs(3))
            .expect("sound resumes");
        assert!(
            resumed.lost_before >= 30,
            "the stall lasted 46 packets, {} were skipped",
            resumed.lost_before
        );
        assert!(report.longest_delay(secs(3), Duration::from_millis(3_500)) <= 240 * MS);
        // And then it is as it was.
        assert!(report.longest_delay(Duration::from_millis(3_300), secs(6)) <= 8 * MS);
        // The source reset what it could not send.
        assert!(report.sender.reset_over + report.sender.reset_late >= 30);
    }

    #[test]
    fn a_link_that_carries_half_of_the_sound_keeps_the_delay_bounded() {
        // A packet and its header are 374 bytes and 47 are made a second: 17.5 kB a second.
        let mut link = Link::new(1, 20);
        link.rate = 8_000;
        let config = Config::new(link, secs(30));
        let report = run(&config);
        assert!(report.max_in_flight <= 8, "{}", report.max_in_flight);
        assert_eq!(report.out_of_order(), 0);
        // What the link carries is played, and about half of it can be.
        let share = played_permille(&report);
        assert!((350..=550).contains(&share), "{share} of a thousand");
        // The delay is bounded by what a packet is given: the grace of 200 ms and two round
        // trips, and the time it takes to go out.
        assert!(
            report.longest_delay(secs(1), secs(30)) <= 350 * MS,
            "{:?}",
            report.longest_delay(secs(1), secs(30))
        );
    }

    #[test]
    fn acknowledgements_that_come_late_do_not_break_the_limit() {
        let mut link = Link::new(10, 5);
        link.ack_loss_permille = 300;
        let config = Config::new(link, secs(20));
        let report = run(&config);
        assert!(report.max_in_flight <= 8);
        assert!(report.max_in_flight >= 2);
        assert!(report.played.len() as u64 + 3 >= report.made);
        assert_eq!(report.skipped(), 0);
        assert_eq!(report.receiver.refused, 0);
    }

    #[test]
    fn the_viewer_never_refuses_the_streams_of_a_source_that_keeps_its_rules() {
        // The source has eight packets in flight and the viewer counts sixteen streams.
        for (megabits, delay) in [(10, 5), (10, 40), (1, 100)] {
            let mut link = Link::new(megabits, delay);
            link.loss_permille = 30;
            link.ack_loss_permille = 100;
            let report = run(&Config::new(link, secs(20)));
            assert_eq!(report.receiver.refused, 0, "{megabits} Mb/s, {delay} ms");
            assert!(report.max_in_flight <= 8);
        }
    }

    #[test]
    fn sound_that_is_turned_off_and_on_starts_with_the_first_packet_of_a_run() {
        let mut config = Config::new(Link::new(10, 30), secs(5));
        config.pauses.push((secs(2), secs(3)));
        let report = run(&config);
        assert_eq!(report.out_of_order(), 0);
        // Nothing is played from a little after the viewer turned it off until it was asked for
        // again and the source answered.
        assert!(
            report
                .played
                .iter()
                .all(|p| p.at < Duration::from_millis(2_000) + MS || p.at >= secs(3)),
        );
        let restart = report
            .played
            .iter()
            .find(|p| p.at >= secs(3))
            .expect("sound comes back");
        assert!(restart.first);
        assert_eq!(restart.lost_before, 0);
        assert_eq!(
            report.played.iter().filter(|p| p.first).count(),
            2,
            "one run before the pause and one after"
        );
        // The packets that were on their way when the viewer turned sound off were stopped.
        assert!(report.receiver.refused >= 1);
        assert!(report.sender.stopped >= 1);
        // And the delay is normal again at once.
        assert!(report.longest_delay(secs(3), secs(5)) <= 40 * MS);
    }

    #[test]
    fn a_packet_that_gets_ahead_of_its_sound_is_lost_and_the_run_starts_after_the_hold() {
        // The `Sound` of the new run arrives 30 ms after the first packet, so the first packet
        // is stopped, and the one after it is the first that is taken. It is not marked as the
        // first of a run, but there is nothing to follow, so the decoder starts there.
        let mut config = Config::new(Link::new(10, 30), secs(5));
        config.pauses.push((secs(2), secs(3)));
        config.sound_late = 30 * MS;
        let report = run(&config);
        assert_eq!(report.out_of_order(), 0);
        let restart = report
            .played
            .iter()
            .find(|p| p.at >= secs(3))
            .expect("sound comes back");
        assert!(restart.first);
        assert_eq!(restart.lost_before, 0);
        // It waited for the packet that began the run, for the hold.
        assert!(
            restart.at.saturating_sub(restart.captured) >= 40 * MS,
            "{:?}",
            restart.at.saturating_sub(restart.captured)
        );
        assert!(report.receiver.refused >= 2);
        // And then it is as it was.
        assert!(report.longest_delay(Duration::from_millis(3_200), secs(5)) <= 40 * MS);
    }
}
