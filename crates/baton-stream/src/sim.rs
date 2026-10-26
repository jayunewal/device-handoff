// SPDX-License-Identifier: Apache-2.0

//! A simulated link, with an encoder and a decoder that behave as a real hardware pair is
//! expected to, to try the sender and the viewer together.
//!
//! The link serialises the bytes of the frames that were written, in the order in which they were
//! written, at a rate that can change and stop. Bytes that were serialised cannot be taken back:
//! a stream that is reset loses what was not sent yet, and what is already on its way arrives.
//! That is the race that the rules have to survive, so the simulation keeps it.
//!
//! The encoder follows the model of ADR 0009: a key frame, then frames that refer to the newest
//! frame that something refers to, some of them marked as long-term references, and a recovery
//! frame that refers to the newest long-term reference that the application acknowledged. The
//! decoder checks what a decoder checks: that what a frame refers to is in its memory. A frame
//! that is decoded without it is a wrong picture, and the tests require that there are none.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    time::Duration,
};

use baton_proto::id::SessionId;

use crate::{
    header::{FrameHeader, FrameKind},
    sender::{self, Encoded, LtrToken, Path, Sender},
    viewer::{self, Action, Viewer},
};

const STEP: Duration = Duration::from_millis(1);

/// A small deterministic random source (xorshift64*).
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub(crate) fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub(crate) fn chance(&mut self, permille: u64) -> bool {
        self.below(1000) < permille
    }
}

/// The network between the two devices.
#[derive(Clone)]
pub(crate) struct Link {
    /// Bytes a second.
    pub rate: u64,
    /// One way.
    pub delay: Duration,
    /// Intervals in which nothing is delivered.
    pub stalls: Vec<(Duration, Duration)>,
    /// The rate from this time on.
    pub rate_changes: Vec<(Duration, u64)>,
    /// How many of a thousand frames lose a packet and need four more one-way delays.
    pub loss_permille: u64,
    /// How many of a thousand acknowledgements arrive six one-way delays late.
    pub ack_loss_permille: u64,
}

impl Link {
    pub(crate) fn new(megabits: u64, delay_ms: u64) -> Self {
        Self {
            rate: megabits * 125_000,
            delay: Duration::from_millis(delay_ms),
            stalls: Vec::new(),
            rate_changes: Vec::new(),
            loss_permille: 0,
            ack_loss_permille: 0,
        }
    }

    pub(crate) fn rate_at(&self, t: Duration) -> u64 {
        if self.stalls.iter().any(|(from, to)| *from <= t && t < *to) {
            return 0;
        }
        self.rate_changes
            .iter()
            .rfind(|(at, _)| *at <= t)
            .map_or(self.rate, |(_, rate)| *rate)
    }
}

/// What to simulate.
#[derive(Clone)]
pub(crate) struct Config {
    pub link: Link,
    pub fps: u64,
    pub run: Duration,
    pub seed: u64,
    /// Times at which the size of the window changes.
    pub resizes: Vec<Duration>,
    pub params: sender::Params,
    pub viewer: viewer::Params,
    /// Frame sizes: a key frame, a frame of the base layer, one of the other layer, a recovery.
    pub sizes: [usize; 4],
    /// How often, in thousandths, the decoder rejects a frame that is not a key frame and loses
    /// everything it held.
    pub decode_failure_permille: u64,
    /// How many key frames, the first ones, the decoder rejects.
    pub reject_keys: u64,
    /// How often, in thousandths, the decoder rejects a key frame after those.
    pub key_failure_permille: u64,
    /// Whether the source reads a request for a key frame before the acknowledgements that
    /// arrive at the same moment (`Some(true)`), after them (`Some(false)`), or either (`None`).
    /// Two streams do not say which of them arrives first.
    pub requests_first: Option<bool>,
}

impl Config {
    pub(crate) fn new(link: Link, run: Duration) -> Self {
        Self {
            link,
            fps: 60,
            run,
            seed: 0x9e37_79b9_7f4a_7c15,
            resizes: Vec::new(),
            params: sender::Params::default(),
            viewer: viewer::Params::default(),
            sizes: [150_000, 25_000, 8_000, 15_000],
            decode_failure_permille: 0,
            reject_keys: 0,
            key_failure_permille: 0,
            requests_first: Some(false),
        }
    }
}

/// What the encoder knew when it made a frame, which a decoder needs to decode it.
#[derive(Clone, Copy)]
enum Reference {
    None,
    /// The frame before it that something refers to.
    Short(u64),
    /// A long-term reference.
    Long(u64),
}

#[derive(Clone, Copy)]
struct Truth {
    kind: FrameKind,
    reference: Reference,
    ltr: Option<LtrToken>,
    /// Whether a later frame may refer to this one, so that a decoder keeps it.
    referenced: bool,
}

/// The encoder: it makes up what a hardware encoder in low-latency mode is documented to do.
struct Encoder {
    count: u64,
    last_referenced: Option<u64>,
    /// Long-term references by token, as frame numbers.
    ltrs: BTreeMap<LtrToken, u64>,
    acknowledged: BTreeSet<LtrToken>,
    next_token: u64,
}

impl Encoder {
    fn new() -> Self {
        Self {
            count: 0,
            last_referenced: None,
            ltrs: BTreeMap::new(),
            acknowledged: BTreeSet::new(),
            next_token: 1,
        }
    }

    fn encode(
        &mut self,
        number: u64,
        kind: FrameKind,
        capture_time: u64,
        sizes: &[usize; 4],
        rng: &mut Rng,
    ) -> (Encoded, Truth) {
        self.count += 1;
        let mut jitter = |size: usize| -> usize {
            let spread = u64::try_from(size / 2).unwrap_or(1);
            size - size / 5 + usize::try_from(rng.below(spread)).unwrap_or(0)
        };
        let (size, referenced, reference) = match kind {
            FrameKind::Key => {
                self.ltrs.clear();
                self.acknowledged.clear();
                (jitter(sizes[0]), true, Reference::None)
            }
            FrameKind::Recovery => {
                let newest = self
                    .acknowledged
                    .iter()
                    .next_back()
                    .and_then(|token| self.ltrs.get(token))
                    .expect("a recovery frame needs a long-term reference that was acknowledged");
                (jitter(sizes[3]), true, Reference::Long(*newest))
            }
            FrameKind::Delta => {
                // The base layer is every other frame, and the other layer refers to it.
                let base = self.count.is_multiple_of(2);
                let size = if base { sizes[1] } else { sizes[2] };
                let reference = Reference::Short(self.last_referenced.expect("after a key frame"));
                (jitter(size), base, reference)
            }
        };
        let ltr = (referenced && (kind != FrameKind::Delta || self.count.is_multiple_of(12))).then(
            || {
                let token = LtrToken(self.next_token);
                self.next_token += 1;
                self.ltrs.insert(token, number);
                token
            },
        );
        if referenced {
            self.last_referenced = Some(number);
        }
        (
            Encoded {
                size,
                referenced,
                ltr,
                capture_time,
            },
            Truth {
                kind,
                reference,
                ltr,
                referenced,
            },
        )
    }
}

/// The decoder: it keeps the newest reference frame that it decoded and the long-term references,
/// and finds out when a frame refers to something other than what it has.
///
/// A real decoder takes the frames in the order in which they were encoded. One that is given a
/// reference frame late replaces the reference that the frames after it were encoded against, so
/// the model has a single short-term reference, and decoding a frame whose number is below one
/// that was decoded is counted as a fault of its own.
#[derive(Default)]
struct Decoder {
    short: Option<u64>,
    long: BTreeSet<u64>,
    wrong: usize,
    newest: Option<u64>,
    backwards: usize,
}

impl Decoder {
    /// The decoder lost its state. What it got wrong before stays counted.
    fn lose_state(&mut self) {
        self.short = None;
        self.long.clear();
    }

    fn decode(&mut self, number: u64, truth: &Truth) {
        if self.newest.is_some_and(|newest| number < newest) {
            self.backwards += 1;
        }
        self.newest = Some(self.newest.map_or(number, |newest| newest.max(number)));
        match (truth.kind, truth.reference) {
            (FrameKind::Key, _) => {
                self.short = None;
                self.long.clear();
            }
            (_, Reference::Short(r)) => {
                if self.short != Some(r) {
                    self.wrong += 1;
                }
            }
            (_, Reference::Long(r)) => {
                if !self.long.contains(&r) {
                    self.wrong += 1;
                }
            }
            (_, Reference::None) => self.wrong += 1,
        }
        if truth.referenced {
            self.short = Some(number);
        }
        if truth.ltr.is_some() {
            self.long.insert(number);
        }
    }
}

/// One frame on the link.
struct Writing {
    number: u64,
    header: FrameHeader,
    /// The bytes of the picture, the header not counted.
    picture: usize,
    left: u64,
}

/// A picture that was shown.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Shown {
    pub at: Duration,
    pub number: u64,
    pub captured: Duration,
}

/// What a run came to.
pub(crate) struct Report {
    pub shown: Vec<Shown>,
    pub wrong: usize,
    /// Frames that were decoded after a frame with a higher number.
    pub backwards: usize,
    pub sender: sender::Stats,
    pub viewer: viewer::Stats,
    pub max_in_flight: usize,
    /// The most bytes of pictures the viewer held at once.
    pub max_held: usize,
    /// Frame streams that the viewer refused. A source that keeps to its rules has none.
    pub refused: usize,
}

impl Report {
    /// The delay from capture to glass at the given share of the pictures, in thousandths.
    pub(crate) fn delay_at(&self, permille: usize) -> Duration {
        self.delay_at_from(permille, Duration::ZERO)
    }

    /// The same, for the pictures shown at or after `from`.
    pub(crate) fn delay_at_from(&self, permille: usize, from: Duration) -> Duration {
        let mut delays: Vec<Duration> = self
            .shown
            .iter()
            .filter(|s| s.at >= from)
            .map(|s| s.at.saturating_sub(s.captured))
            .collect();
        delays.sort();
        let at = delays.len().saturating_sub(1) * permille / 1000;
        delays.get(at).copied().unwrap_or_default()
    }

    /// The longest time in `from..to` in which no new picture was shown.
    pub(crate) fn longest_gap(&self, from: Duration, to: Duration) -> Duration {
        let mut last = from;
        let mut longest = Duration::ZERO;
        for shown in self.shown.iter().filter(|s| s.at >= from && s.at < to) {
            longest = longest.max(shown.at.saturating_sub(last));
            last = shown.at;
        }
        longest.max(to.saturating_sub(last))
    }

    /// When the first picture was shown at or after `t`.
    pub(crate) fn first_after(&self, t: Duration) -> Option<Duration> {
        self.shown.iter().find(|s| s.at >= t).map(|s| s.at)
    }
}

struct World<'a> {
    config: &'a Config,
    t: Duration,
    rng: Rng,
    sender: Sender,
    viewer: Viewer,
    encoder: Encoder,
    decoder: Decoder,
    truths: BTreeMap<u64, Truth>,
    captured: BTreeMap<u64, Duration>,
    headers: BTreeMap<u64, (FrameHeader, usize)>,
    /// Frames being serialised, in the order they were written.
    writing: VecDeque<Writing>,
    /// (arrives at, frame number), earliest first.
    flying: BinaryHeap<Reverse<(Duration, u64)>>,
    acks: BinaryHeap<Reverse<(Duration, u64)>>,
    /// (arrives at, the frame the request names), earliest first.
    key_requests: BinaryHeap<Reverse<(Duration, u64)>>,
    /// Decides, when it is asked to, which of a request and an acknowledgement is read first.
    order: Rng,
    keys_rejected: u64,
    resizes: Vec<Duration>,
    credit: u64,
    next_capture: Duration,
    next_viewer_tick: Duration,
    report: Report,
}

/// Runs the simulation.
pub(crate) fn run(config: &Config) -> Report {
    let session = SessionId::from_bytes([9; SessionId::LEN]);
    let mut resizes = config.resizes.clone();
    resizes.sort_by(|a, b| b.cmp(a));
    let mut world = World {
        config,
        t: Duration::ZERO,
        rng: Rng(config.seed),
        sender: Sender::new(session, config.params),
        viewer: Viewer::new(config.viewer),
        encoder: Encoder::new(),
        decoder: Decoder::default(),
        truths: BTreeMap::new(),
        captured: BTreeMap::new(),
        headers: BTreeMap::new(),
        writing: VecDeque::new(),
        flying: BinaryHeap::new(),
        acks: BinaryHeap::new(),
        key_requests: BinaryHeap::new(),
        order: Rng(config.seed ^ 0x5851_f42d_4c95_7f2d),
        keys_rejected: 0,
        resizes,
        credit: 0,
        next_capture: Duration::ZERO,
        next_viewer_tick: Duration::ZERO,
        report: Report {
            shown: Vec::new(),
            wrong: 0,
            backwards: 0,
            sender: sender::Stats::default(),
            viewer: viewer::Stats::default(),
            max_in_flight: 0,
            max_held: 0,
            refused: 0,
        },
    };
    while world.t < config.run {
        world.step();
        world.t += STEP;
    }
    world.report.wrong = world.decoder.wrong;
    world.report.backwards = world.decoder.backwards;
    world.report.sender = world.sender.stats();
    world.report.viewer = world.viewer.stats();
    world.report
}

impl World<'_> {
    fn step(&mut self) {
        self.serialise();
        self.deliver();
        self.acknowledge();
        self.resize();
        let path = self.path();
        for number in self.sender.poll(self.t) {
            self.writing.retain(|w| w.number != number);
        }
        if self.t >= self.next_capture {
            self.next_capture += Duration::from_micros(1_000_000 / self.config.fps);
            self.capture(&path);
        }
        self.report.max_in_flight = self.report.max_in_flight.max(self.sender.in_flight());
        if self.t >= self.next_viewer_tick {
            self.next_viewer_tick += Duration::from_millis(100);
            let actions = self.viewer.tick(self.t);
            self.apply(actions);
        }
    }

    /// What the connection would tell the source.
    fn path(&self) -> Path {
        let link = &self.config.link;
        let rate = link.rate_at(self.t);
        let queued: u64 = self.writing.iter().map(|w| w.left).sum();
        let base = 2 * link.delay;
        Path {
            window: (2 * rate * u64::try_from(base.as_millis()).unwrap_or(0) / 1000).max(20_000),
            rtt: base + Duration::from_micros(queued * 1_000_000 / rate.max(1)),
            rate,
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
            self.flying
                .push(Reverse((self.t + link.delay + extra, done.number)));
            self.headers
                .insert(done.number, (done.header, done.picture));
        }
    }

    /// Frames arrive at the viewer.
    fn deliver(&mut self) {
        let link = &self.config.link;
        while let Some(Reverse((at, number))) = self.flying.peek().copied() {
            if at > self.t {
                break;
            }
            self.flying.pop();
            let Some((header, picture)) = self.headers.remove(&number) else {
                continue;
            };
            let late = if self.rng.chance(link.ack_loss_permille) {
                6 * link.delay
            } else {
                Duration::ZERO
            };
            self.acks
                .push(Reverse((self.t + link.delay + late, number)));
            let taken = self.viewer.stream_started().and_then(|id| {
                self.viewer.stream_read(id, picture)?;
                Ok(id)
            });
            let Ok(id) = taken else {
                self.report.refused += 1;
                continue;
            };
            self.report.max_held = self.report.max_held.max(self.viewer.held_bytes());
            let actions = self
                .viewer
                .arrived(self.t, id, header)
                .expect("the sender gives each frame a number of its own");
            self.apply(actions);
        }
    }

    /// Acknowledgements and requests arrive at the source.
    fn acknowledge(&mut self) {
        let requests_first = self
            .config
            .requests_first
            .unwrap_or_else(|| self.order.chance(500));
        if requests_first {
            self.requests_arrive();
            self.acks_arrive();
        } else {
            self.acks_arrive();
            self.requests_arrive();
        }
    }

    fn acks_arrive(&mut self) {
        while let Some(Reverse((at, number))) = self.acks.peek().copied() {
            if at > self.t {
                break;
            }
            self.acks.pop();
            for token in self.sender.acked(number) {
                self.encoder.acknowledged.insert(token);
            }
        }
    }

    fn requests_arrive(&mut self) {
        while let Some(Reverse((at, tried))) = self.key_requests.peek().copied() {
            if at > self.t {
                break;
            }
            self.key_requests.pop();
            self.sender
                .request_key_frame(self.t, tried)
                .expect("the viewer names a frame that was sent");
        }
    }

    /// The window changes its size.
    fn resize(&mut self) {
        while self.resizes.last().is_some_and(|at| *at <= self.t) {
            self.resizes.pop();
            for number in self.sender.new_epoch() {
                self.writing.retain(|w| w.number != number);
            }
        }
    }

    /// The source captures a picture, if the rules let it.
    fn capture(&mut self, path: &Path) {
        let Some(kind) = self.sender.begin(path) else {
            return;
        };
        let number = self.sender.next_number();
        let micros = u64::try_from(self.t.as_micros()).unwrap_or(u64::MAX);
        let (encoded, truth) =
            self.encoder
                .encode(number, kind, micros, &self.config.sizes, &mut self.rng);
        let sent = self
            .sender
            .sent(self.t, &encoded, path)
            .expect("a frame was announced");
        assert_eq!(sent.header.number, number);
        if let Reference::Short(r) = truth.reference {
            assert_eq!(
                sent.header.depends_on, r,
                "the header names what the encoder refers to"
            );
        }
        if kind == FrameKind::Key {
            // The encoder forgets the long-term references at a key frame, and so does the app.
            self.encoder.acknowledged.clear();
        }
        for number in &sent.reset {
            self.writing.retain(|w| w.number != *number);
        }
        self.truths.insert(number, truth);
        self.captured.insert(number, self.t);
        self.writing.push_back(Writing {
            number,
            header: sent.header,
            picture: encoded.size,
            left: u64::try_from(encoded.size + FrameHeader::LEN).unwrap_or(u64::MAX),
        });
    }

    fn apply(&mut self, actions: Vec<Action>) {
        // After a frame is rejected, what the viewer planned to decode and show next in the same
        // batch is of no use. Asking for a key frame and forgetting frames still goes through.
        let mut failed = false;
        for action in actions {
            match action {
                Action::Decode(_) | Action::Show(_) if failed => {}
                Action::Decode(number) => {
                    let truth = *self.truths.get(&number).expect("a frame that was made");
                    let rejected = if truth.kind == FrameKind::Key {
                        let rejected = self.keys_rejected < self.config.reject_keys
                            || (self.config.key_failure_permille > 0
                                && self.rng.chance(self.config.key_failure_permille));
                        self.keys_rejected += u64::from(rejected);
                        rejected
                    } else {
                        self.rng.chance(self.config.decode_failure_permille)
                    };
                    if rejected {
                        failed = true;
                        self.decoder.lose_state();
                        let more = self.viewer.decode_failed(self.t);
                        self.apply(more);
                    } else {
                        self.decoder.decode(number, &truth);
                    }
                }
                Action::Show(number) => {
                    let captured = self.captured.get(&number).copied().unwrap_or_default();
                    self.report.shown.push(Shown {
                        at: self.t,
                        number,
                        captured,
                    });
                }
                Action::Discard(_) => {}
                Action::RequestKeyFrame { tried } => self
                    .key_requests
                    .push(Reverse((self.t + self.config.link.delay, tried))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    /// What must hold in every run: no wrong picture, no more than the allowed frames in
    /// flight, and pictures are shown in the order they were captured.
    fn sound(report: &Report) {
        assert_eq!(report.wrong, 0, "a wrong picture was decoded");
        assert_eq!(report.backwards, 0, "a frame was decoded after a newer one");
        assert!(report.max_in_flight <= 6);
        assert_eq!(report.refused, 0, "the viewer refused a frame stream");
        assert!(report.max_held <= viewer::Params::default().max_held_bytes);
        assert!(
            report.shown.windows(2).all(|w| w[0].number < w[1].number),
            "a picture was shown after a newer one"
        );
    }

    fn summary(name: &str, config: &Config, report: &Report) {
        let cut = config.run / 2;
        eprintln!(
            "{name}: shown {} wrong {} | delay p50 {:?} p95 {:?} p100 {:?} | after half: p95 {:?} | \
             sender {:?} | viewer {:?} | max in flight {} | max held {} B",
            report.shown.len(),
            report.wrong,
            report.delay_at(500),
            report.delay_at(950),
            report.delay_at(1000),
            report.delay_at_from(950, cut),
            report.sender,
            report.viewer,
            report.max_in_flight,
            report.max_held,
        );
    }

    #[test]
    fn a_clean_link_shows_every_picture_with_a_small_delay() {
        let config = Config::new(Link::new(100, 5), 5 * S);
        let report = run(&config);
        summary("clean", &config, &report);
        sound(&report);
        assert_eq!(report.viewer.key_requests, 0);
        assert_eq!(report.sender.abandoned, 0);
        assert_eq!((report.sender.keys, report.sender.recoveries), (1, 0));
        assert!(report.shown.len() >= 290, "{} pictures", report.shown.len());
        assert!(report.delay_at(1000) < Duration::from_millis(60));
        assert!(report.max_in_flight <= 6);
    }

    #[test]
    fn a_stall_is_followed_by_a_picture_and_never_by_a_wrong_one() {
        for stall in [Duration::from_millis(200), 2 * S] {
            let mut link = Link::new(20, 10);
            link.stalls.push((3 * S, 3 * S + stall));
            let config = Config::new(link, 8 * S);
            let report = run(&config);
            summary(&format!("stall {stall:?}"), &config, &report);
            sound(&report);
            // Frames that were reset and arrived whole are dropped, and not a reason to ask.
            assert_eq!(report.viewer.key_requests, 0);
            assert!(report.sender.abandoned > 0);
            // A new picture within a round trip and a deadline of the end of the stall.
            let end = 3 * S + stall;
            let back = report.first_after(end).expect("the picture comes back");
            assert!(
                back.saturating_sub(end) < Duration::from_millis(400),
                "{:?} after the stall",
                back.saturating_sub(end)
            );
            // And it stays: no gap after that is as long as the stall was.
            assert!(report.longest_gap(end + S, 8 * S) < Duration::from_millis(100));
            assert!(report.delay_at_from(950, end + S) < Duration::from_millis(60));
        }
    }

    #[test]
    fn a_link_that_cannot_carry_the_stream_keeps_the_delay_bounded() {
        // The stream needs about 7 megabits a second and the link has two.
        let config = Config::new(Link::new(2, 20), 10 * S);
        let report = run(&config);
        summary("narrow", &config, &report);
        sound(&report);
        assert_eq!(report.viewer.key_requests, 0);
        assert!(report.shown.len() > 100);
        assert!(report.delay_at_from(950, 2 * S) < Duration::from_secs(1));
    }

    #[test]
    fn a_drop_of_the_rate_and_its_return_are_followed() {
        let mut link = Link::new(100, 10);
        link.rate_changes.push((2 * S, 3 * 125_000));
        link.rate_changes.push((5 * S, 100 * 125_000));
        let config = Config::new(link, 9 * S);
        let report = run(&config);
        summary("drop", &config, &report);
        sound(&report);
        assert_eq!(report.viewer.key_requests, 0);
        assert!(report.delay_at_from(950, 6 * S) < Duration::from_millis(60));
    }

    #[test]
    fn lost_packets_and_late_acknowledgements_do_not_make_a_wrong_picture() {
        for seed in 1..=8 {
            let mut link = Link::new(30, 15);
            link.loss_permille = 30;
            link.ack_loss_permille = 50;
            let mut config = Config::new(link, 8 * S);
            config.seed = seed * 0x1234_5678_9abc_def1;
            let report = run(&config);
            summary(&format!("lossy {seed}"), &config, &report);
            sound(&report);
            assert_eq!(report.viewer.key_requests, 0, "seed {seed}");
            assert!(report.shown.len() > 200, "seed {seed}");
        }
    }

    #[test]
    fn a_change_of_size_starts_a_new_picture() {
        let mut config = Config::new(Link::new(50, 10), 6 * S);
        config.resizes = vec![2 * S, 4 * S];
        let report = run(&config);
        summary("resize", &config, &report);
        sound(&report);
        assert_eq!(report.viewer.key_requests, 0);
        assert_eq!(report.sender.keys, 3);
        assert_eq!(report.sender.recoveries, 0);
        assert!(report.longest_gap(Duration::ZERO, 6 * S) < Duration::from_millis(150));
    }

    #[test]
    fn a_key_frame_that_needs_longer_than_the_longest_deadline_still_arrives() {
        // About 400 KB at a megabit a second is over three seconds, and the cap is two. A key
        // frame that was replaced when it was late would be replaced by one that is no smaller.
        let mut config = Config::new(Link::new(1, 20), 15 * S);
        config.sizes[0] = 400_000;
        let report = run(&config);
        summary("slow key frame", &config, &report);
        sound(&report);
        assert_eq!(report.sender.keys, 1, "the key frame was replaced");
        let first = report
            .first_after(Duration::ZERO)
            .expect("a picture is shown");
        assert!(
            first > 2 * S,
            "the first picture came at {first:?}, inside the cap"
        );
        assert!(first < 6 * S, "the first picture came at {first:?}");
        assert!(report.shown.len() > 50, "{} pictures", report.shown.len());
        assert_eq!(report.viewer.key_requests, 0);
    }

    #[test]
    fn a_recovery_after_a_stall_on_a_slow_link_does_not_starve() {
        // The stream needs more than the link has, a stall abandons what was in flight, and
        // the recovery frame has to cross a link that is slow for it.
        for seed in 1_u64..=6 {
            let mut link = Link::new(2, 30);
            link.stalls.push((3 * S, 4 * S));
            link.loss_permille = 20;
            let mut config = Config::new(link, 12 * S);
            config.sizes[3] = 90_000;
            config.seed = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let report = run(&config);
            summary(&format!("slow recovery {seed}"), &config, &report);
            sound(&report);
            let back = report.first_after(4 * S).expect("the picture comes back");
            assert!(back < 8 * S, "seed {seed}: back at {back:?}");
            assert!(report.shown.last().unwrap().at > 11 * S, "seed {seed}");
        }
    }

    #[test]
    fn a_decoder_that_rejects_a_frame_is_given_a_key_frame_and_shows_no_wrong_picture() {
        for seed in 1..=6 {
            let mut config = Config::new(Link::new(40, 10), 10 * S);
            config.decode_failure_permille = 5;
            config.seed = seed * 0x2545_f491_4f6c_dd1d;
            let report = run(&config);
            summary(&format!("rejects {seed}"), &config, &report);
            sound(&report);
            assert!(
                report.viewer.key_requests > 0,
                "seed {seed}: nothing was rejected"
            );
            assert!(
                report.sender.keys > 1,
                "seed {seed}: a request for a key frame was not answered with one"
            );
            // The picture comes back after each rejection: a request, a key frame that takes a
            // few round trips to cross the link, and no gap grows beyond that.
            assert!(
                report.longest_gap(Duration::ZERO, 10 * S) < Duration::from_millis(700),
                "seed {seed}: {:?}",
                report.longest_gap(Duration::ZERO, 10 * S)
            );
        }
    }

    #[test]
    fn a_key_frame_that_the_decoder_rejects_is_replaced_whichever_is_read_first() {
        // The viewer receives a key frame, cannot decode it, and says so. The acknowledgement of
        // the same frame and the request travel on different streams, and the source cannot tell
        // which of them it reads first.
        for requests_first in [false, true] {
            let mut config = Config::new(Link::new(50, 10), 5 * S);
            config.reject_keys = 1;
            config.requests_first = Some(requests_first);
            let report = run(&config);
            summary(
                &format!("rejected key, request first {requests_first}"),
                &config,
                &report,
            );
            sound(&report);
            assert_eq!(report.sender.keys, 2, "request first: {requests_first}");
            // The source answers by itself: the viewer does not have to ask again.
            assert_eq!(
                report.viewer.key_requests, 1,
                "request first: {requests_first}"
            );
            // The first key frame crosses in about 35 ms and is rejected. The next one is made 250
            // ms after the first and crosses in about 35 ms.
            let first = report
                .first_after(Duration::ZERO)
                .expect("a picture is shown");
            assert!(
                first < Duration::from_millis(400),
                "request first {requests_first}: the first picture came at {first:?}"
            );
            assert!(report.shown.len() > 250, "{} pictures", report.shown.len());
        }
    }

    #[test]
    fn a_key_frame_that_the_decoder_rejects_on_a_slow_link_is_replaced_at_once_too() {
        // 2 Mbit/s and 30 ms: a key frame needs about 0.6 s to cross. The request is read before
        // the acknowledgement of the key frame it names.
        let mut config = Config::new(Link::new(2, 30), 12 * S);
        config.reject_keys = 1;
        config.requests_first = Some(true);
        let report = run(&config);
        summary("rejected key, slow", &config, &report);
        sound(&report);
        // The viewer may ask again while the second key frame is crossing. That request names
        // the frame it named before, and the key frame on its way answers it.
        assert_eq!(report.sender.keys, 2);
        let first = report
            .first_after(Duration::ZERO)
            .expect("a picture is shown");
        assert!(
            first < Duration::from_millis(2500),
            "the first picture came at {first:?}"
        );
    }

    #[test]
    fn a_viewer_that_asks_as_fast_as_it_can_is_given_four_key_frames_a_second_at_most() {
        // Every key frame is rejected, and the viewer asks again the moment it has failed.
        let mut config = Config::new(Link::new(100, 5), 5 * S);
        config.key_failure_permille = 1000;
        config.viewer.key_request_interval = Duration::ZERO;
        let report = run(&config);
        summary("rejects every key frame", &config, &report);
        sound(&report);
        assert!(
            report.sender.keys >= 15,
            "{} key frames",
            report.sender.keys
        );
        assert!(
            report.sender.keys <= 21,
            "{} key frames",
            report.sender.keys
        );
        // Nothing is shown, and the stream is alive: the next key frame is always on its way.
        assert!(report.shown.is_empty());
    }

    /// A run in which a picture is due in the last two seconds even after a stall that ends at
    /// `until`: the stream has the time for five tries to get a key frame across. A try is the
    /// wait of the viewer before it asks, the way there and back with a packet lost on it, and the
    /// time a key frame needs on the link, up to 1.3 times the size in the config, which is more
    /// than a second at a megabit. The decoder rejects a key frame now and then, and a run in
    /// which only one try fits cannot be asked for a picture at its end.
    fn long_enough(config: &Config, until: Duration) -> Duration {
        let key = u64::try_from(config.sizes[0] * 13 / 10).unwrap_or(u64::MAX);
        let crossing = Duration::from_micros(key * 1_000_000 / config.link.rate);
        let attempt = config.viewer.key_request_interval + 6 * config.link.delay + crossing;
        (until + 5 * attempt + 2 * S).max(config.run)
    }

    #[test]
    fn a_link_of_a_megabit_that_stalls_late_shows_a_picture_again_after_a_key_frame_is_rejected() {
        // What the property below found on CI: a stall from 5 s to 7.6 s while a key frame was on
        // its way, which the decoder then rejected. The next one needs another 1.5 s on that
        // link, so it comes just after a run of ten seconds ends.
        let until = Duration::from_millis(7648);
        let mut link = Link::new(1, 4);
        link.loss_permille = 11;
        link.stalls.push((Duration::from_secs(5), until));
        let mut config = Config::new(link, 10 * S);
        config.run = long_enough(&config, until);
        config.decode_failure_permille = 9;
        config.key_failure_permille = 42;
        config.requests_first = None;
        config.seed = 17_678_794_754_649_973_101;
        let report = run(&config);
        summary(
            "a megabit, a late stall, a key frame rejected",
            &config,
            &report,
        );
        sound(&report);
        let last = report.shown.last().expect("a picture is shown");
        assert!(
            last.at + 2 * S > config.run,
            "the last picture came at {:?} of {:?}",
            last.at,
            config.run
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]

        /// Whatever the link does, no wrong picture is shown, the limit on frames in flight
        /// holds, and the picture keeps coming: one is shown in the last two seconds of a run
        /// that is long enough for the stream to get a few key frames across after the stall.
        #[test]
        fn no_link_makes_the_rules_show_a_wrong_picture(
            megabits in 1_u64..200,
            delay_ms in 1_u64..120,
            loss in 0_u64..80,
            ack_loss in 0_u64..120,
            stall_at in 1_u64..6,
            stall_ms in 0_u64..3000,
            rejects in 0_u64..20,
            key_rejects in 0_u64..50,
            seed in 1_u64..u64::MAX,
        ) {
            let mut link = Link::new(megabits, delay_ms);
            link.loss_permille = loss;
            link.ack_loss_permille = ack_loss;
            let from = Duration::from_secs(stall_at);
            let until = from + Duration::from_millis(stall_ms);
            link.stalls.push((from, until));
            let mut config = Config::new(link, 10 * S);
            config.run = long_enough(&config, until);
            config.decode_failure_permille = rejects;
            config.key_failure_permille = key_rejects;
            config.requests_first = None;
            config.seed = seed;
            let report = run(&config);
            sound(&report);
            proptest::prop_assert!(report.max_in_flight <= 6);
            proptest::prop_assert!(!report.shown.is_empty());
            let last = report.shown.last().unwrap();
            proptest::prop_assert!(
                last.at + 2 * S > config.run,
                "last picture at {:?} of {:?}",
                last.at,
                config.run
            );
        }
    }
}
