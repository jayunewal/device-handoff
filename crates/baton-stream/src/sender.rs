// SPDX-License-Identifier: Apache-2.0

//! The source's accounting of frames in flight.
//!
//! Each encoded frame travels on a stream of its own. QUIC acknowledges a stream when the peer
//! has all of it, and the source can reset a stream it no longer wants. This module decides,
//! from those two facts and from the clock, three things: whether to encode the next picture at
//! all, when to give up on a frame, and what the frame after that has to be so that the viewer
//! can decode it.
//!
//! It is a state machine. It is told what happened (a frame was sent, a frame was acknowledged,
//! time passed) and answers with what to do (reset these streams, encode a key frame). It does
//! no input or output and reads no clock, so a test can drive it with a simulated link.
//!
//! The vocabulary:
//!
//! - A frame is **in flight** from the moment its stream is written until its stream is
//!   acknowledged or reset.
//! - A frame is **referenced** if a later frame may refer to it. A frame that is not referenced
//!   can be given up on alone.
//! - A frame is **held** when the peer is known to have decoded it, or can: it is acknowledged,
//!   and so is every frame that it depends on. The source never needs a message from the viewer
//!   to know this, because the only frames that never arrive are those it reset itself.
//! - A **chain** is a run of frames that are decoded one after the other. It begins at a key
//!   frame or at a recovery frame. Giving up on a referenced frame breaks the chain: every later
//!   frame of it depends on the lost one, so the next frame begins a new chain.

use std::{collections::BTreeMap, time::Duration};

use baton_proto::id::SessionId;

use crate::header::{Flags, FrameHeader, FrameKind};

/// How many frames to keep a record of, at most, once they are settled.
const KEPT: usize = 64;

/// The numbers that the rules use. The values are where ADR 0009 starts; the tests settle them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// Most frames in flight at once.
    pub max_in_flight: usize,
    /// Below this many frames in flight a picture is always encoded, whatever the window says.
    pub min_in_flight: usize,
    /// How late a frame may be on top of the round trips and the time to send it.
    pub grace: Duration,
    /// How many round trips come on top of the grace.
    pub round_trips: u32,
    /// The longest a frame is given, however large it is and however slow the path.
    pub longest: Duration,
    /// The shortest time between two key frames that are made because the viewer asked for one.
    pub min_key_interval: Duration,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            max_in_flight: 6,
            min_in_flight: 2,
            grace: Duration::from_millis(100),
            round_trips: 2,
            longest: Duration::from_secs(2),
            min_key_interval: Duration::from_millis(250),
        }
    }
}

/// What the connection says about the path at this moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Path {
    /// The congestion window, in bytes.
    pub window: u64,
    /// The smoothed round-trip time.
    pub rtt: Duration,
    /// How fast the path delivers, in bytes a second, or 0 to use the window over the round trip,
    /// which is what a path under Cubic delivers.
    pub rate: u64,
}

/// Names a frame that the encoder marked as a long-term reference.
///
/// The encoder asks to be told which of these the viewer holds, and refers to one of them when it
/// recovers from a frame that was given up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LtrToken(pub u64);

/// What the encoder made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoded {
    /// The size of the encoded frame, in bytes.
    pub size: usize,
    /// Whether the encoder says that a later frame may refer to this one. Key frames, recovery
    /// frames and long-term references count as referenced whatever is said here.
    pub referenced: bool,
    /// The token of the frame, if the encoder marked it as a long-term reference.
    pub ltr: Option<LtrToken>,
    /// When the picture was captured, in microseconds since the session began.
    pub capture_time: u64,
}

/// The result of [`Sender::sent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    /// The header to write at the start of the frame's stream.
    pub header: FrameHeader,
    /// The streams of earlier frames that this frame makes useless, which are to be reset.
    pub reset: Vec<u64>,
}

/// Counts of what happened, for reports and benchmarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Frames sent.
    pub sent: u64,
    /// Key frames sent.
    pub keys: u64,
    /// Recovery frames sent.
    pub recoveries: u64,
    /// Frames given up on and reset.
    pub abandoned: u64,
    /// Pictures that were not encoded because too much was in flight.
    pub skipped: u64,
}

/// A request for a key frame named a frame that was never sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a request for a key frame names frame {0}, which was never sent")]
pub struct UnknownFrame(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The stream is written and not acknowledged.
    InFlight,
    /// Acknowledged, but a frame it depends on is not.
    Acked,
    /// The viewer holds it.
    Held,
    /// Given up on, or useless: the viewer will never decode it.
    Abandoned,
}

#[derive(Debug, Clone)]
struct Frame {
    kind: FrameKind,
    chain: u32,
    depends_on: u64,
    referenced: bool,
    ltr: Option<LtrToken>,
    size: usize,
    /// When the frame is late: it is given up on if it is still in flight after this.
    due: Duration,
    state: State,
}

/// The accounting of one session's frames, on the source.
#[derive(Debug)]
pub struct Sender {
    params: Params,
    session: SessionId,
    epoch: u32,
    /// The chain of the newest frame.
    chain: u32,
    /// The chain that the next key or recovery frame begins.
    next_chain: u32,
    next_number: u64,
    /// The newest frame of the current chain that a later frame may refer to.
    last_referenced: Option<u64>,
    frames: BTreeMap<u64, Frame>,
    /// What the frame that is being encoded has to be, and the epoch it was begun in, if one is.
    encoding: Option<(FrameKind, u32)>,
    want_key: bool,
    /// The current chain lost a frame, so the next frame has to begin a new one.
    broken: bool,
    /// When the newest key frame was written.
    last_key_at: Option<Duration>,
    /// A key frame that the viewer asked for is made from this time on, and not before.
    key_due: Option<Duration>,
    /// The newest frame that the viewer holds and that a later frame may refer to.
    held_tip: Option<u64>,
    /// How many long-term references the viewer holds.
    held_ltrs: usize,
    stats: Stats,
}

impl Sender {
    /// Starts the accounting for a session. The first frame will be a key frame.
    pub fn new(session: SessionId, params: Params) -> Self {
        Self {
            params,
            session,
            epoch: 0,
            chain: 0,
            next_chain: 0,
            next_number: 0,
            last_referenced: None,
            frames: BTreeMap::new(),
            encoding: None,
            want_key: true,
            broken: false,
            last_key_at: None,
            key_due: None,
            held_tip: None,
            held_ltrs: 0,
            stats: Stats::default(),
        }
    }

    /// How many frames are in flight.
    pub fn in_flight(&self) -> usize {
        self.frames
            .values()
            .filter(|f| f.state == State::InFlight)
            .count()
    }

    fn in_flight_bytes(&self) -> u64 {
        self.frames
            .values()
            .filter(|f| f.state == State::InFlight)
            .map(|f| u64::try_from(f.size).unwrap_or(u64::MAX))
            .fold(0, u64::saturating_add)
    }

    /// What has been counted so far.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The number that the next frame will get.
    pub fn next_number(&self) -> u64 {
        self.next_number
    }

    /// The epoch of the frames that are made now. It changes with
    /// [`new_epoch`](Self::new_epoch), and the first frame of an epoch is a key frame.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Whether the next frame has to be something other than a plain delta, so the source
    /// should encode again even if the screen has not changed. A screen that has not changed
    /// delivers no new picture, and without one a lost frame would never be replaced.
    pub fn recovery_pending(&self) -> bool {
        self.want_key || self.broken
    }

    /// Asks whether the next picture may be encoded, and what it has to be.
    ///
    /// Returns `None` when too much is in flight, or when a frame is still being encoded. The
    /// picture that was skipped is not lost: the capture API hands over the newest picture when
    /// it is asked for one next, so what is skipped is the old.
    ///
    /// A frame that begins a chain (a key frame or a recovery frame) is written alone: nothing
    /// is written behind it until the viewer has acknowledged it. Nothing behind it can be
    /// decoded before it, and on a path where it needs a long time to cross, the frames behind it
    /// would share the path with it, slow it down further, and be waiting at the viewer when it
    /// arrives, a picture or more stale than the one that is written after the acknowledgement.
    /// The one exception is a key frame that the viewer asked for (see
    /// [`request_key_frame`](Self::request_key_frame)): the viewer has said that it cannot use
    /// what it has, which includes the frame in flight, so the key frame goes at once and the
    /// frame in flight is reset with it.
    ///
    /// A delta is encoded if fewer than [`Params::min_in_flight`] frames are in flight, or if the
    /// bytes in flight are below the congestion window; and never if [`Params::max_in_flight`]
    /// are. Counting frames alone would be wrong twice: a path with a long round trip needs more
    /// frames in flight to be full, and a receiver may hold an acknowledgement back for a while,
    /// so a run of small frames would wait for no reason.
    ///
    /// A frame that begins a chain is counted after the resets it makes. It makes every frame
    /// that is not held useless, and when it is written the ones in flight are reset (see
    /// [`sent`](Self::sent)), so it is alone in flight and under both limits: waiting for the
    /// acknowledgement of a frame that it is about to reset would only delay the picture that the
    /// viewer is waiting for.
    pub fn begin(&mut self, path: &Path) -> Option<FrameKind> {
        if self.encoding.is_some() {
            return None;
        }
        let kind = self.next_kind();
        let room = if kind == FrameKind::Delta {
            let in_flight = self.in_flight();
            !self.begins_chain_in_flight()
                && in_flight < self.params.max_in_flight
                && (in_flight < self.params.min_in_flight || self.in_flight_bytes() < path.window)
        } else {
            self.want_key || !self.begins_chain_in_flight()
        };
        if !room {
            self.stats.skipped += 1;
            return None;
        }
        self.encoding = Some((kind, self.epoch));
        Some(kind)
    }

    /// Whether a frame that begins a chain has been written and not acknowledged.
    fn begins_chain_in_flight(&self) -> bool {
        self.frames
            .values()
            .any(|f| f.state == State::InFlight && f.kind != FrameKind::Delta)
    }

    fn next_kind(&self) -> FrameKind {
        if self.want_key || self.last_referenced.is_none() {
            FrameKind::Key
        } else if self.broken {
            if self.held_tip.is_some() && self.held_ltrs > 0 {
                FrameKind::Recovery
            } else {
                FrameKind::Key
            }
        } else {
            FrameKind::Delta
        }
    }

    /// The frame that [`begin`](Self::begin) announced could not be made. Nothing was sent.
    pub fn cancel(&mut self) {
        self.encoding = None;
    }

    /// The frame that [`begin`](Self::begin) announced was made and its stream written at `now`.
    ///
    /// Returns the header to write at the start of the stream, and the earlier frames that this
    /// one makes useless. Returns `None` if no frame was being encoded, and also if the one that
    /// was is of no use any more: the size of the picture changed, or a frame that a delta was to
    /// follow was given up on, while it was being made. Nothing was sent then, as after
    /// [`cancel`](Self::cancel), and the caller drops the picture. The next frame is the key frame
    /// of the new size, or the frame that begins a new chain.
    ///
    /// The time by which the frame is late is fixed here, from what the path is like now. It is
    /// the grace, plus a number of round trips, plus the time that the path needs for the bytes
    /// in flight and for this frame. It is not worked out again later: a path that has emptied
    /// its queue since looks faster, but that does not make a frame that was written into the
    /// queue any later.
    pub fn sent(&mut self, now: Duration, encoded: &Encoded, path: &Path) -> Option<Sent> {
        let (kind, epoch) = self.encoding.take()?;
        if epoch != self.epoch || (kind == FrameKind::Delta && self.broken) {
            return None;
        }
        let due = now + self.allowance(encoded.size, path);
        let number = self.next_number;
        self.next_number += 1;
        let starts_chain = kind != FrameKind::Delta;
        let referenced = starts_chain || encoded.referenced || encoded.ltr.is_some();
        let depends_on = match kind {
            FrameKind::Key => number,
            FrameKind::Recovery => self.held_tip.unwrap_or(0),
            FrameKind::Delta => self.last_referenced.unwrap_or(0),
        };
        let mut reset = Vec::new();
        if starts_chain {
            // A frame that is not held by now is of no use to the viewer: a key frame makes the
            // decoder forget everything before it, and a recovery frame refers to what is held.
            reset = self.abandon_unheld(number);
            self.chain = self.next_chain;
            self.next_chain += 1;
            self.broken = false;
            self.last_referenced = Some(number);
            if kind == FrameKind::Key {
                self.held_tip = None;
                self.held_ltrs = 0;
                self.last_key_at = Some(now);
                // The requests that were waiting for a key frame are answered by this one. A
                // recovery frame does not answer them: a request that came while it was being
                // encoded is for the next frame.
                self.want_key = false;
                self.key_due = None;
                self.stats.keys += 1;
            } else {
                self.stats.recoveries += 1;
            }
        } else if referenced {
            self.last_referenced = Some(number);
        }
        self.frames.insert(
            number,
            Frame {
                kind,
                chain: self.chain,
                depends_on,
                referenced,
                ltr: encoded.ltr,
                size: encoded.size,
                due,
                state: State::InFlight,
            },
        );
        self.stats.sent += 1;
        let mut flags = match kind {
            FrameKind::Key => Flags::KEY,
            FrameKind::Recovery => Flags::RECOVERY,
            FrameKind::Delta => Flags::NONE,
        };
        if referenced {
            flags = flags | Flags::REFERENCED;
        }
        let header = FrameHeader {
            session: self.session,
            number,
            depends_on,
            chain: self.chain,
            epoch: self.epoch,
            capture_time: encoded.capture_time,
            flags,
        };
        Some(Sent { header, reset })
    }

    /// Every frame older than `number` that is not held is given up on. Returns the streams
    /// among them that are still open.
    fn abandon_unheld(&mut self, number: u64) -> Vec<u64> {
        let mut reset = Vec::new();
        for (n, frame) in self.frames.range_mut(..number) {
            match frame.state {
                State::InFlight => {
                    reset.push(*n);
                    frame.state = State::Abandoned;
                    self.stats.abandoned += 1;
                }
                State::Acked => frame.state = State::Abandoned,
                State::Held | State::Abandoned => {}
            }
        }
        reset
    }

    /// The peer has acknowledged all of the frame's stream.
    ///
    /// Returns the long-term references that the viewer is now known to hold, which the encoder
    /// is to be told about with the next frame.
    pub fn acked(&mut self, number: u64) -> Vec<LtrToken> {
        let mut newly_held = Vec::new();
        match self.frames.get_mut(&number) {
            Some(frame) if frame.state == State::InFlight => frame.state = State::Acked,
            // Given up on already, or acknowledged twice, or a frame that was forgotten.
            _ => return newly_held,
        }
        let mut work = vec![number];
        while let Some(n) = work.pop() {
            if !self.promote(n, &mut newly_held) {
                continue;
            }
            work.extend(
                self.frames
                    .range(n + 1..)
                    .filter(|(_, f)| f.state == State::Acked && f.depends_on == n)
                    .map(|(m, _)| *m),
            );
        }
        self.prune();
        newly_held
    }

    /// Makes an acknowledged frame held if everything it depends on is held. Returns whether it
    /// did.
    fn promote(&mut self, number: u64, newly_held: &mut Vec<LtrToken>) -> bool {
        let Some(frame) = self.frames.get(&number) else {
            return false;
        };
        if frame.state != State::Acked {
            return false;
        }
        let depends_on = frame.depends_on;
        let held = depends_on == number
            || self
                .frames
                .get(&depends_on)
                // A frame that is no longer remembered was held when it was forgotten.
                .is_none_or(|dependency| dependency.state == State::Held);
        if !held {
            return false;
        }
        if let Some(frame) = self.frames.get_mut(&number) {
            frame.state = State::Held;
            if frame.referenced && self.held_tip.is_none_or(|tip| tip < number) {
                self.held_tip = Some(number);
            }
            if let Some(token) = frame.ltr {
                self.held_ltrs += 1;
                newly_held.push(token);
            }
        }
        true
    }

    /// Looks at the clock. Returns the streams to reset because their frames are late.
    ///
    /// A key frame that the viewer asked for, and that had to wait for its time, is let go here,
    /// so this is to be called on a timer. [`request_key_frame`](Self::request_key_frame) lets it
    /// go too, when the viewer asks again.
    ///
    /// A frame that is not referenced is given up on alone. A frame that is referenced breaks
    /// the chain: the later frames of the chain are given up on with it, and the next frame
    /// begins a new chain.
    ///
    /// A frame that begins a chain (a key frame or a recovery frame) is never given up on for
    /// being late. It is replaced by another that is no smaller, so on a path where one needs
    /// longer than [`Params::longest`] to cross, every one would be given up on, the bytes that
    /// had crossed would be thrown away each time, and no picture would ever arrive. Nothing is
    /// written behind such a frame (see [`begin`](Self::begin)), so there is nothing else to be
    /// late. The rule resumes with the frame after it. A path that is gone is for the session to
    /// end, by its rule for liveness.
    pub fn poll(&mut self, now: Duration) -> Vec<u64> {
        self.release_key_due(now);
        if self.begins_chain_in_flight() {
            return Vec::new();
        }
        let late: Vec<u64> = self
            .frames
            .iter()
            .filter(|(_, f)| f.state == State::InFlight && now > f.due)
            .map(|(n, _)| *n)
            .collect();
        let mut reset = Vec::new();
        for number in late {
            // The oldest comes first, and it may already have taken this one with it.
            if self
                .frames
                .get(&number)
                .is_some_and(|f| f.state == State::InFlight)
            {
                self.abandon(number, &mut reset);
            }
        }
        self.prune();
        reset
    }

    /// The peer stopped reading the frame's stream, so it will never arrive. Returns the streams
    /// to reset with it.
    pub fn lost(&mut self, number: u64) -> Vec<u64> {
        let mut reset = Vec::new();
        if self
            .frames
            .get(&number)
            .is_some_and(|f| f.state == State::InFlight)
        {
            self.abandon(number, &mut reset);
        }
        reset
    }

    /// Lets go of a key frame that the viewer asked for and that waited for its time, if the time
    /// has come.
    fn release_key_due(&mut self, now: Duration) {
        if self.key_due.is_some_and(|due| due <= now) {
            self.key_due = None;
            self.want_key = true;
        }
    }

    /// How long a frame of `size` bytes may be in flight if it is written now.
    fn allowance(&self, size: usize, path: &Path) -> Duration {
        let bytes = self
            .in_flight_bytes()
            .saturating_add(u64::try_from(size).unwrap_or(u64::MAX));
        // The rate the path delivers: what the connection says, or else the window over the round
        // trip, which is what Cubic delivers (ADR 0009, rule 2).
        let rate = match path.rate {
            0 => (u128::from(path.window) * 1_000_000).checked_div(path.rtt.as_micros()),
            rate => Some(u128::from(rate)),
        };
        let transmit = rate
            .and_then(|rate| (u128::from(bytes) * 1_000_000).checked_div(rate))
            .map_or(Duration::ZERO, |micros| {
                Duration::from_micros(u64::try_from(micros).unwrap_or(u64::MAX))
            });
        (self.params.grace + path.rtt.saturating_mul(self.params.round_trips) + transmit)
            .min(self.params.longest)
    }

    fn abandon(&mut self, number: u64, reset: &mut Vec<u64>) {
        let Some(frame) = self.frames.get_mut(&number) else {
            return;
        };
        frame.state = State::Abandoned;
        let referenced = frame.referenced;
        let chain = frame.chain;
        reset.push(number);
        self.stats.abandoned += 1;
        if !referenced || chain != self.chain {
            return;
        }
        self.broken = true;
        for (n, later) in self.frames.range_mut(number + 1..) {
            match later.state {
                State::InFlight => {
                    reset.push(*n);
                    later.state = State::Abandoned;
                    self.stats.abandoned += 1;
                }
                State::Acked => later.state = State::Abandoned,
                State::Held | State::Abandoned => {}
            }
        }
    }

    /// The viewer cannot decode what it has, and says the newest frame that it has tried to
    /// decode. Returns an error if that frame was never sent, which is a malformed message.
    ///
    /// The request is answered with a key frame at the next picture, unless a key frame that is
    /// numbered above the named frame is on its way or held: the viewer has not tried that one
    /// yet, and it will answer the request. A request that names the key frame itself is not
    /// answered by it, because a viewer that names a frame has tried it and failed. So the answer
    /// does not depend on whether the request or the acknowledgement of the frame is read first,
    /// which two streams do not decide.
    ///
    /// A frame that begins a chain and is still in flight is reset with the others: a viewer that
    /// cannot decode takes nothing but a key frame, so a recovery frame is of no use to it.
    ///
    /// Key frames made on request are at least [`Params::min_key_interval`] apart, so that a
    /// viewer cannot make the source encode as fast as it can ask. A request that comes sooner
    /// waits for its time, and [`poll`](Self::poll) lets it go, or the next request that comes
    /// after that time, which the viewer repeats while it waits.
    pub fn request_key_frame(&mut self, now: Duration, tried: u64) -> Result<(), UnknownFrame> {
        if tried >= self.next_number {
            return Err(UnknownFrame(tried));
        }
        self.release_key_due(now);
        let on_its_way = self.frames.range(tried + 1..).any(|(_, f)| {
            f.kind == FrameKind::Key && matches!(f.state, State::InFlight | State::Held)
        });
        // A key frame that is wanted already, or waits for its time, comes after this request.
        if on_its_way || self.want_key || self.key_due.is_some() {
            return Ok(());
        }
        match self
            .last_key_at
            .map(|at| at + self.params.min_key_interval)
            .filter(|earliest| *earliest > now)
        {
            Some(earliest) => self.key_due = Some(earliest),
            None => self.want_key = true,
        }
        Ok(())
    }

    /// The size of the picture changed. Starts a new epoch, which begins with a key frame, and
    /// returns the streams of the frames in flight, which are of the old size and are to be
    /// reset. A picture that is being encoded is of the old size too: [`sent`](Self::sent)
    /// returns `None` for it.
    pub fn new_epoch(&mut self) -> Vec<u64> {
        self.epoch += 1;
        self.want_key = true;
        let mut reset = Vec::new();
        for (n, frame) in &mut self.frames {
            match frame.state {
                State::InFlight => {
                    reset.push(*n);
                    frame.state = State::Abandoned;
                    self.stats.abandoned += 1;
                }
                State::Acked => frame.state = State::Abandoned,
                State::Held | State::Abandoned => {}
            }
        }
        reset
    }

    /// Forgets frames that are settled and old, so that a long session does not grow.
    fn prune(&mut self) {
        if self.frames.len() <= KEPT {
            return;
        }
        let oldest_open = self
            .frames
            .iter()
            .find(|(_, f)| matches!(f.state, State::InFlight | State::Acked))
            .map_or(self.next_number, |(n, _)| *n);
        // Nothing needs the record of a settled frame: a frame that depends on one that is
        // forgotten counts it as held (see `promote`), and a recovery frame takes the number of
        // the newest held frame from `held_tip`, not from a record.
        let doomed: Vec<u64> = self
            .frames
            .range(..oldest_open)
            .map(|(n, _)| *n)
            .take(self.frames.len() - KEPT)
            .collect();
        for number in doomed {
            self.frames.remove(&number);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn session() -> SessionId {
        SessionId::from_bytes([7; SessionId::LEN])
    }

    fn path() -> Path {
        Path {
            window: 1_000_000,
            rtt: 20 * MS,
            rate: 0,
        }
    }

    fn sender() -> Sender {
        Sender::new(session(), Params::default())
    }

    fn frame(size: usize) -> Encoded {
        Encoded {
            size,
            referenced: true,
            ltr: None,
            capture_time: 0,
        }
    }

    fn droppable(size: usize) -> Encoded {
        Encoded {
            referenced: false,
            ..frame(size)
        }
    }

    fn ltr(size: usize, token: u64) -> Encoded {
        Encoded {
            ltr: Some(LtrToken(token)),
            ..frame(size)
        }
    }

    /// Asks for a frame and sends it.
    fn send(sender: &mut Sender, now: Duration, encoded: &Encoded) -> Sent {
        sender.begin(&path()).expect("room for a frame");
        sender
            .sent(now, encoded, &path())
            .expect("a frame was announced")
    }

    /// Sends the first frame, a key frame, and has the viewer acknowledge it: nothing is written
    /// behind a key frame before that.
    fn start(sender: &mut Sender, encoded: &Encoded) {
        let sent = send(sender, Duration::ZERO, encoded);
        sender.acked(sent.header.number);
    }

    #[test]
    fn the_first_frame_is_a_key_frame_that_depends_on_itself() {
        let mut s = sender();
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let sent = s.sent(Duration::ZERO, &frame(1000), &path()).unwrap();
        let h = sent.header;
        assert_eq!((h.number, h.depends_on, h.chain, h.epoch), (0, 0, 0, 0));
        assert!(h.is_key() && h.is_referenced());
        assert_eq!(h.session, session());
        assert_eq!(sent.reset, [] as [u64; 0]);
        h.validate().unwrap();
    }

    #[test]
    fn a_delta_depends_on_the_newest_frame_that_something_may_refer_to() {
        let mut flight = sender();
        start(&mut flight, &frame(1000)); // 0, key
        let first = send(&mut flight, MS, &frame(100)).header; // 1, referenced
        let second = send(&mut flight, 2 * MS, &droppable(100)).header; // 2, nothing refers to it
        let third = send(&mut flight, 3 * MS, &droppable(100)).header; // 3, nothing refers to it
        let fourth = send(&mut flight, 4 * MS, &frame(100)).header; // 4
        assert_eq!(
            (
                first.depends_on,
                second.depends_on,
                third.depends_on,
                fourth.depends_on
            ),
            (0, 1, 1, 1)
        );
        let fifth = send(&mut flight, 5 * MS, &frame(100)).header; // 5
        assert_eq!(fifth.depends_on, 4);
        assert!(
            [first, second, third, fourth, fifth]
                .iter()
                .all(|h| h.chain == 0 && h.kind() == FrameKind::Delta)
        );
        assert!(!second.is_referenced() && fourth.is_referenced());
    }

    #[test]
    fn a_long_term_reference_counts_as_referenced() {
        let mut s = sender();
        start(&mut s, &frame(1000));
        let marked = Encoded {
            referenced: false,
            ..ltr(100, 1)
        };
        let h = send(&mut s, MS, &marked).header;
        assert!(h.is_referenced());
        assert_eq!(send(&mut s, 2 * MS, &frame(100)).header.depends_on, 1);
    }

    #[test]
    fn at_most_six_frames_are_in_flight() {
        let mut s = sender();
        start(&mut s, &frame(10));
        for i in 1..=6 {
            send(&mut s, i * MS, &frame(10));
        }
        assert_eq!(s.in_flight(), 6);
        assert_eq!(s.begin(&path()), None);
        assert_eq!(s.stats().skipped, 1);
        s.acked(1);
        assert_eq!(s.in_flight(), 5);
        assert!(s.begin(&path()).is_some());
    }

    #[test]
    fn a_picture_waits_when_the_bytes_in_flight_fill_the_window() {
        let mut s = sender();
        let narrow = Path {
            window: 5000,
            ..path()
        };
        start(&mut s, &frame(10));
        s.begin(&narrow).unwrap();
        s.sent(Duration::ZERO, &frame(4000), &path()).unwrap();
        // Fewer than two frames in flight: always room, whatever the window.
        s.begin(&narrow).unwrap();
        s.sent(MS, &frame(4000), &path()).unwrap();
        // Two frames, 8000 bytes, over a window of 5000.
        assert_eq!(s.begin(&narrow), None);
        // A wider window makes room, and so does an acknowledgement.
        assert!(s.begin(&path()).is_some());
        s.cancel();
        s.acked(1);
        s.acked(2);
        assert!(s.begin(&narrow).is_some());
    }

    #[test]
    fn only_one_frame_is_encoded_at_a_time() {
        let mut s = sender();
        assert!(s.begin(&path()).is_some());
        assert_eq!(s.begin(&path()), None);
        s.cancel();
        assert!(s.begin(&path()).is_some());
        assert!(s.sent(Duration::ZERO, &frame(1), &path()).is_some());
        assert!(s.sent(Duration::ZERO, &frame(1), &path()).is_none());
    }

    #[test]
    fn a_frame_is_held_only_when_the_frames_it_depends_on_are() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 100)); // 0, key, long-term reference
        assert_eq!(s.acked(0), vec![LtrToken(100)]);
        send(&mut s, MS, &ltr(100, 101)); // 1, depends on 0
        send(&mut s, 2 * MS, &ltr(100, 102)); // 2, depends on 1
        send(&mut s, 3 * MS, &ltr(100, 103)); // 3, depends on 2
        // Frame 3 arrives first: it is not held, because 2 and 1 might never arrive.
        assert_eq!(s.acked(3), [] as [LtrToken; 0]);
        assert_eq!(s.acked(2), [] as [LtrToken; 0]);
        // Frame 1 settles all three.
        let mut held = s.acked(1);
        held.sort();
        assert_eq!(held, vec![LtrToken(101), LtrToken(102), LtrToken(103)]);
        assert_eq!(s.held_tip, Some(3));
        assert_eq!(s.held_ltrs, 4);
        // Acknowledging again changes nothing.
        assert_eq!(s.acked(1), [] as [LtrToken; 0]);
    }

    #[test]
    fn a_frame_that_was_given_up_on_alone_does_not_stop_the_ones_after_it_being_held() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0, key
        send(&mut s, MS, &droppable(100)); // 1, nothing refers to it
        send(&mut s, 2 * MS, &frame(100)); // 2, depends on 0
        send(&mut s, 3 * MS, &frame(100)); // 3, depends on 2
        // Frame 1 never arrives and is given up on. The frames after it do not name it.
        s.acked(2);
        s.acked(3);
        assert_eq!(s.held_tip, Some(3));
        assert_eq!(s.lost(1), vec![1]);
        assert_eq!(s.held_tip, Some(3));
    }

    #[test]
    fn a_frame_that_nothing_refers_to_is_held_without_moving_the_tip() {
        let mut s = sender();
        start(&mut s, &frame(1000));
        send(&mut s, MS, &droppable(100));
        s.acked(1);
        assert_eq!(s.held_tip, Some(0));
    }

    #[test]
    fn a_late_frame_that_nothing_refers_to_is_given_up_on_alone() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0
        send(&mut s, MS, &droppable(100)); // 1, will be late
        send(&mut s, 2 * MS, &frame(100)); // 2, depends on 0
        s.acked(2);
        let late = Duration::from_millis(500);
        assert_eq!(s.poll(late), vec![1]);
        assert!(!s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Delta));
        // Frame 2 does not depend on frame 1, so it is held.
        assert_eq!(s.held_tip, Some(2));
        assert_eq!(s.stats().abandoned, 1);
    }

    #[test]
    fn a_late_frame_that_something_refers_to_breaks_the_chain() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 1)); // 0, key
        assert_eq!(s.acked(0), vec![LtrToken(1)]);
        send(&mut s, MS, &frame(100)); // 1
        send(&mut s, 2 * MS, &frame(100)); // 2, late
        send(&mut s, 3 * MS, &frame(100)); // 3, depends on 2
        send(&mut s, 4 * MS, &frame(100)); // 4, depends on 3
        s.acked(1);
        s.acked(4); // arrived, but it cannot be decoded without 2 and 3
        let reset = s.poll(Duration::from_millis(500));
        assert_eq!(reset, vec![2, 3]);
        assert!(s.recovery_pending());
        // Held: 0 and 1. The recovery frame refers to the newest frame that is held.
        assert_eq!(s.begin(&path()), Some(FrameKind::Recovery));
        let h = s
            .sent(Duration::from_millis(501), &frame(300), &path())
            .unwrap()
            .header;
        assert_eq!(h.kind(), FrameKind::Recovery);
        assert_eq!((h.number, h.depends_on, h.chain), (5, 1, 1));
        assert!(h.is_referenced());
        assert!(!s.recovery_pending());
        // Two streams were reset. Frame 4 was acknowledged, so there is nothing to reset, but
        // it can never be held now.
        assert_eq!(s.stats().abandoned, 2);
        assert_eq!(s.acked(4), [] as [LtrToken; 0]);
        // The recovery frame is held once the viewer has it and what it depends on.
        s.acked(5);
        assert_eq!(s.held_tip, Some(5));
        // The frames after the recovery frame depend on it, and are written once it is held.
        let next = send(&mut s, Duration::from_millis(502), &frame(100)).header;
        assert_eq!((next.depends_on, next.chain), (5, 1));
    }

    #[test]
    fn without_a_long_term_reference_the_recovery_is_a_key_frame() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0, key, no token
        send(&mut s, MS, &frame(100)); // 1, late
        assert_eq!(s.poll(Duration::from_millis(500)), vec![1]);
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
    }

    #[test]
    fn nothing_is_held_when_the_first_frame_is_lost() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 1));
        // The viewer stopped reading its stream.
        assert_eq!(s.lost(0), vec![0]);
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let h = s
            .sent(Duration::from_secs(1), &frame(1000), &path())
            .unwrap()
            .header;
        assert_eq!((h.number, h.depends_on, h.chain), (1, 1, 1));
    }

    #[test]
    fn a_key_frame_is_never_late_and_nothing_is_written_behind_it() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &frame(230_000)); // 0, a key frame that needs a long time
        // Far past every deadline, and nothing is given up on: a key frame that was replaced
        // would be replaced by one that is no smaller, and none would ever arrive.
        assert_eq!(s.poll(Duration::from_secs(30)), [] as [u64; 0]);
        assert_eq!(s.stats().abandoned, 0);
        assert_eq!(s.in_flight(), 1);
        assert!(!s.recovery_pending());
        // Nothing is written behind it: the frames could not be decoded before it, and would
        // only share the path with it.
        assert_eq!(s.begin(&path()), None);
        assert_eq!(s.stats().skipped, 1);
        // The frame after it waits for the acknowledgement, and then goes at once.
        s.acked(0);
        assert_eq!(s.begin(&path()), Some(FrameKind::Delta));
    }

    #[test]
    fn a_recovery_frame_is_written_alone_and_is_never_late_either() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 1)); // 0, key, long-term reference
        s.acked(0);
        send(&mut s, MS, &frame(100)); // 1, late
        assert_eq!(s.poll(Duration::from_millis(500)), vec![1]);
        assert_eq!(s.begin(&path()), Some(FrameKind::Recovery));
        let recovery = s
            .sent(Duration::from_millis(501), &frame(300), &path())
            .unwrap();
        assert_eq!(recovery.header.kind(), FrameKind::Recovery);
        assert_eq!(s.begin(&path()), None);
        assert_eq!(s.poll(Duration::from_secs(30)), [] as [u64; 0]);
        s.acked(recovery.header.number);
        assert_eq!(s.begin(&path()), Some(FrameKind::Delta));
    }

    /// The time of the first key frame in the tests that follow, and a time after the shortest
    /// interval between key frames that are made on request.
    const LATER: Duration = Duration::from_millis(300);

    #[test]
    fn a_request_is_answered_by_a_key_frame_numbered_above_the_frame_it_names() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0, key, held
        send(&mut s, LATER, &frame(100)); // 1
        // The viewer could not decode frame 1. Nothing above it is on its way.
        s.request_key_frame(LATER + MS, 1).unwrap();
        assert!(s.recovery_pending());
        let key = send(&mut s, LATER + 2 * MS, &frame(1000)); // 2, key
        assert!(key.header.is_key());
        assert_eq!(key.reset, vec![1]);
        // Key frame 2 is on its way and the viewer has not tried it, so it answers a request that
        // names frame 1 again, and so does a key frame that is held.
        s.request_key_frame(2 * LATER, 1).unwrap();
        assert!(!s.recovery_pending());
        s.acked(2);
        s.request_key_frame(3 * LATER, 1).unwrap();
        assert!(!s.recovery_pending());
        // A request that names the key frame itself says it has been tried and failed.
        s.request_key_frame(3 * LATER, 2).unwrap();
        assert!(s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
    }

    #[test]
    fn a_key_frame_that_the_viewer_rejected_is_replaced_before_its_acknowledgement_is_read() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 1)); // 0, key, in flight
        // The viewer received it, could not decode it, and says so. The request is read before
        // the acknowledgement of the same frame, so the key frame still looks as if it were on
        // its way, and the request names it.
        s.request_key_frame(LATER, 0).unwrap();
        assert!(s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let sent = s.sent(LATER + MS, &frame(1000), &path()).unwrap();
        assert_eq!(sent.reset, vec![0]);
        // The late acknowledgement of the key frame that failed counts for nothing.
        assert_eq!(s.acked(0), [] as [LtrToken; 0]);
        assert_eq!(s.held_tip, None);
        s.acked(sent.header.number);
        assert_eq!(s.held_tip, Some(1));
    }

    #[test]
    fn a_request_resets_a_recovery_frame_that_is_on_its_way() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 1)); // 0, key, long-term reference
        s.acked(0);
        send(&mut s, MS, &frame(100)); // 1, late
        assert_eq!(s.poll(Duration::from_millis(500)), vec![1]);
        assert_eq!(s.begin(&path()), Some(FrameKind::Recovery));
        let recovery = s
            .sent(Duration::from_millis(501), &frame(300), &path())
            .unwrap(); // 2, recovery
        // A viewer that cannot decode takes nothing but a key frame, so the recovery frame is of
        // no use to it. The key frame goes at once, and the recovery frame is reset.
        s.request_key_frame(Duration::from_millis(600), 1).unwrap();
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let sent = s
            .sent(Duration::from_millis(601), &frame(1000), &path())
            .unwrap();
        assert_eq!(sent.reset, vec![recovery.header.number]);
        assert_eq!(s.in_flight(), 1);
    }

    #[test]
    fn a_request_that_comes_while_a_recovery_frame_is_being_encoded_is_not_lost() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 1)); // 0, key, long-term reference
        s.acked(0);
        send(&mut s, MS, &frame(100)); // 1, late
        s.poll(Duration::from_millis(500));
        assert_eq!(s.begin(&path()), Some(FrameKind::Recovery));
        // The viewer's request arrives while the recovery frame is still being encoded. Sending
        // the recovery frame must not clear it.
        s.request_key_frame(Duration::from_millis(550), 1).unwrap();
        s.sent(Duration::from_millis(551), &frame(300), &path())
            .unwrap();
        assert!(s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
    }

    #[test]
    fn a_request_for_a_key_frame_that_is_already_wanted_changes_nothing() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0
        send(&mut s, LATER, &frame(100)); // 1
        send(&mut s, LATER + MS, &frame(100)); // 2
        s.request_key_frame(LATER + 2 * MS, 1).unwrap();
        // Another that names a newer frame is answered by the same key frame.
        s.request_key_frame(LATER + 3 * MS, 2).unwrap();
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        s.sent(LATER + 4 * MS, &frame(1000), &path()).unwrap();
        assert_eq!(s.stats().keys, 2);
        assert!(!s.recovery_pending());
    }

    #[test]
    fn a_request_that_names_a_frame_that_was_never_sent_is_refused() {
        let mut s = sender();
        assert_eq!(s.request_key_frame(MS, 0), Err(UnknownFrame(0)));
        start(&mut s, &frame(1000)); // 0
        assert_eq!(s.request_key_frame(LATER, 1), Err(UnknownFrame(1)));
        assert_eq!(
            s.request_key_frame(LATER, u64::MAX),
            Err(UnknownFrame(u64::MAX))
        );
        assert!(s.request_key_frame(LATER, 0).is_ok());
    }

    #[test]
    fn key_frames_made_on_request_are_at_least_the_interval_apart() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0, written at time 0
        send(&mut s, 50 * MS, &frame(100)); // 1
        s.acked(1);
        // Too soon after the first key frame: the request waits for its time.
        s.request_key_frame(100 * MS, 1).unwrap();
        assert!(!s.recovery_pending());
        assert_eq!(s.poll(249 * MS), [] as [u64; 0]);
        assert!(!s.recovery_pending());
        // At the end of the interval the key frame is wanted, and poll lets it go.
        s.poll(250 * MS);
        assert!(s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let sent = s.sent(251 * MS, &frame(1000), &path()).unwrap();
        // A request that is waiting is answered by the key frame, whichever way it was made.
        assert_eq!(sent.header.number, 2);
        assert!(!s.recovery_pending());
        // The interval counts from this one.
        s.acked(2);
        send(&mut s, 260 * MS, &frame(100)); // 3
        s.acked(3);
        s.request_key_frame(300 * MS, 3).unwrap();
        s.poll(500 * MS);
        assert!(!s.recovery_pending());
        s.poll(501 * MS);
        assert!(s.recovery_pending());
    }

    #[test]
    fn a_key_frame_for_a_new_size_does_not_wait_for_the_interval() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0
        assert_eq!(s.new_epoch(), [] as [u64; 0]);
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
    }

    #[test]
    fn a_key_frame_makes_the_frames_before_it_useless() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(1000, 1)); // 0
        s.acked(0);
        send(&mut s, MS, &frame(100)); // 1
        send(&mut s, 2 * MS, &frame(100)); // 2
        s.acked(2); // waits for 1
        s.request_key_frame(LATER, 2).unwrap();
        assert!(s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let sent = s.sent(LATER + MS, &frame(1000), &path()).unwrap();
        assert_eq!(sent.reset, vec![1]);
        assert_eq!(sent.header.chain, 1);
        assert_eq!(
            s.held_tip, None,
            "what is held before a key frame is not any more"
        );
        assert_eq!(s.held_ltrs, 0);
        // A late acknowledgement of a frame given up on is ignored.
        assert_eq!(s.acked(1), [] as [LtrToken; 0]);
        assert_eq!(s.held_tip, None);
    }

    #[test]
    fn a_frame_that_is_given_up_on_does_not_come_back() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0, key
        send(&mut s, MS, &frame(100)); // 1
        send(&mut s, 2 * MS, &droppable(100)); // 2
        assert_eq!(s.poll(Duration::from_secs(1)), vec![1, 2]);
        assert_eq!(s.acked(1), [] as [LtrToken; 0]);
        assert_eq!(s.acked(2), [] as [LtrToken; 0]);
        assert_eq!(s.in_flight(), 0);
        assert_eq!(s.poll(Duration::from_secs(5)), [] as [u64; 0]);
    }

    #[test]
    fn the_time_allowed_counts_round_trips_and_the_time_to_send() {
        let mut s = sender();
        let slow = Path {
            window: 1_000_000,
            rtt: 80 * MS,
            rate: 1_000_000, // a megabyte a second
        };
        // 100 ms of grace, two round trips of 80 ms, and 150 KB at a megabyte a second.
        assert_eq!(s.allowance(150_000, &slow), 410 * MS);
        assert_eq!(s.allowance(0, &slow), 260 * MS);
        // The frames in flight go first, so their bytes count too.
        s.begin(&slow).unwrap();
        s.sent(Duration::ZERO, &frame(50_000), &slow).unwrap();
        assert_eq!(s.allowance(150_000, &slow), 460 * MS);
        // An unknown rate is the window over the round trip, which is what Cubic delivers: a
        // megabyte over 80 ms takes 16 ms for the 200 KB.
        assert_eq!(s.allowance(150_000, &Path { rate: 0, ..slow }), 276 * MS);
        // Without a round trip or a window there is nothing to work it out from, and no time to
        // send is counted.
        let unknown = Path { rate: 0, ..slow };
        assert_eq!(
            s.allowance(
                150_000,
                &Path {
                    rtt: Duration::ZERO,
                    ..unknown
                }
            ),
            100 * MS
        );
        assert_eq!(
            s.allowance(
                150_000,
                &Path {
                    window: 0,
                    ..unknown
                }
            ),
            260 * MS
        );
        // A path that is hopeless does not make the wait longer than the limit.
        let crawl = Path { rate: 1, ..slow };
        assert_eq!(s.allowance(150_000, &crawl), Params::default().longest);
    }

    #[test]
    fn a_large_frame_on_a_slow_link_is_given_the_time_to_cross_it_when_no_rate_is_known() {
        // 150 KB at 2 Mbit/s, over a window of 7.5 KB and a round trip of 30 ms, which is what a
        // connection under Cubic can give: the frame needs 600 ms to cross, and is not late
        // after 160 ms, as it would be if the time to cross it were left out.
        let slow = Path {
            window: 7_500,
            rtt: 30 * MS,
            rate: 0,
        };
        let mut s = sender();
        s.begin(&slow).unwrap();
        s.sent(Duration::ZERO, &frame(150_000), &slow).unwrap(); // 0, key, never late
        s.acked(0);
        s.begin(&slow).unwrap();
        s.sent(Duration::ZERO, &frame(150_000), &slow).unwrap(); // 1
        assert_eq!(s.poll(160 * MS), [] as [u64; 0]);
        assert_eq!(s.poll(760 * MS), [] as [u64; 0]);
        assert_eq!(s.poll(761 * MS), vec![1]);
    }

    #[test]
    fn a_frame_is_judged_by_the_path_it_was_written_into() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &frame(1000)); // 0, key
        s.acked(0);
        let slow = Path {
            rtt: 200 * MS,
            ..path()
        };
        s.begin(&slow).unwrap();
        s.sent(Duration::ZERO, &frame(1000), &slow).unwrap();
        // 100 ms of grace and two round trips of 200 ms. That the path looks faster by now, or
        // that nobody asks what it looks like, changes nothing.
        assert_eq!(s.poll(500 * MS), [] as [u64; 0]);
        assert_eq!(s.poll(501 * MS), vec![1]);
    }

    #[test]
    fn a_frame_is_not_late_inside_its_deadline() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &frame(1000)); // 0, key
        s.acked(0);
        send(&mut s, Duration::ZERO, &frame(1000)); // 1
        // 100 ms of grace and two round trips of 20 ms.
        assert_eq!(s.poll(140 * MS), [] as [u64; 0]);
        assert_eq!(s.poll(141 * MS), vec![1]);
    }

    #[test]
    fn a_new_epoch_gives_up_on_everything_in_flight_and_asks_for_a_key_frame() {
        let mut s = sender();
        start(&mut s, &frame(1000));
        send(&mut s, MS, &frame(100));
        assert_eq!(s.new_epoch(), vec![1]);
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let h = s.sent(2 * MS, &frame(1000), &path()).unwrap().header;
        assert_eq!((h.epoch, h.chain, h.number), (1, 1, 2));
    }

    #[test]
    fn a_picture_that_was_being_encoded_when_the_size_changed_is_dropped() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0, key
        assert_eq!(s.begin(&path()), Some(FrameKind::Delta));
        // The window is resized while the picture of the old size is in the encoder.
        assert_eq!(s.new_epoch(), [] as [u64; 0]);
        assert!(s.sent(MS, &frame(100), &path()).is_none());
        assert_eq!(s.in_flight(), 0);
        // The next frame is the key frame of the new size, and nothing of the old size went out.
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let h = s.sent(2 * MS, &frame(1000), &path()).unwrap().header;
        assert_eq!((h.number, h.epoch, h.depends_on), (1, 1, 1));
    }

    #[test]
    fn a_key_frame_of_the_old_size_does_not_answer_the_wish_for_one_of_the_new() {
        let mut s = sender();
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        s.new_epoch();
        assert!(s.sent(MS, &frame(1000), &path()).is_none());
        assert!(s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let h = s.sent(2 * MS, &frame(1000), &path()).unwrap().header;
        assert_eq!((h.number, h.epoch), (0, 1));
    }

    #[test]
    fn a_delta_that_was_being_encoded_when_its_chain_broke_is_dropped() {
        let mut s = sender();
        start(&mut s, &ltr(1000, 1)); // 0, key, long-term reference, held
        send(&mut s, MS, &frame(100)); // 1
        assert_eq!(s.begin(&path()), Some(FrameKind::Delta));
        // Frame 1 is given up on while the next picture is in the encoder. A delta that follows
        // it could never be decoded.
        assert_eq!(s.poll(500 * MS), vec![1]);
        assert!(s.sent(501 * MS, &frame(100), &path()).is_none());
        assert_eq!(s.stats().sent, 2);
        // The frame after it begins the new chain.
        assert_eq!(s.begin(&path()), Some(FrameKind::Recovery));
    }

    #[test]
    fn a_key_frame_that_the_viewer_asked_for_goes_at_the_next_picture_whatever_is_in_flight() {
        let mut s = sender();
        start(&mut s, &frame(10)); // 0, key, held
        for i in 1..=6 {
            send(&mut s, LATER + i * MS, &frame(10));
        }
        assert_eq!(s.in_flight(), 6);
        assert_eq!(s.begin(&path()), None);
        // The viewer cannot decode what it has, and says so. The six frames are of no use to it
        // and the key frame resets them, so it does not wait for the acknowledgement of any.
        s.request_key_frame(LATER + 7 * MS, 1).unwrap();
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
        let sent = s.sent(LATER + 8 * MS, &frame(10), &path()).unwrap();
        assert_eq!(sent.reset, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(s.in_flight(), 1);
    }

    #[test]
    fn a_recovery_frame_does_not_wait_for_frames_in_flight_that_it_resets() {
        let mut s = sender();
        send(&mut s, Duration::ZERO, &ltr(10, 1)); // 0, key, long-term reference
        s.acked(0);
        send(&mut s, MS, &frame(10)); // 1
        send(&mut s, 2 * MS, &frame(10)); // 2
        send(&mut s, 3 * MS, &frame(10)); // 3
        let narrow = Path {
            window: 5,
            ..path()
        };
        assert_eq!(s.begin(&narrow), None);
        // The viewer stopped reading frame 3, which breaks the chain. Frames 1 and 2 are not
        // held, and the recovery frame resets them, so it goes at the next picture.
        assert_eq!(s.lost(3), vec![3]);
        assert_eq!(s.begin(&narrow), Some(FrameKind::Recovery));
        let sent = s.sent(4 * MS, &frame(10), &narrow).unwrap();
        assert_eq!(sent.reset, vec![1, 2]);
        assert_eq!(sent.header.depends_on, 0);
        assert_eq!(s.in_flight(), 1);
    }

    #[test]
    fn a_request_that_waits_is_let_go_by_the_request_that_follows_its_time() {
        let mut s = sender();
        start(&mut s, &frame(1000)); // 0, written at time 0
        send(&mut s, 50 * MS, &frame(100)); // 1
        s.acked(1);
        s.request_key_frame(100 * MS, 1).unwrap();
        assert!(!s.recovery_pending());
        // The viewer asks again every 500 ms while it waits. On a still screen nothing else
        // looks at the clock, and the request that comes after its time lets the key frame go.
        s.request_key_frame(600 * MS, 1).unwrap();
        assert!(s.recovery_pending());
        assert_eq!(s.begin(&path()), Some(FrameKind::Key));
    }

    #[test]
    fn a_long_run_of_frames_that_nothing_refers_to_is_forgotten() {
        let mut s = sender();
        start(&mut s, &frame(100)); // 0, key, held: the tip stays here
        let mut now = Duration::ZERO;
        for _ in 0..5000 {
            now += 16 * MS;
            let sent = send(&mut s, now, &droppable(100));
            s.acked(sent.header.number);
        }
        assert!(s.frames.len() <= KEPT + 1, "{} records", s.frames.len());
    }

    #[test]
    fn an_old_session_does_not_grow_without_limit() {
        let mut s = sender();
        let mut now = Duration::ZERO;
        for i in 0..5000_u64 {
            now += 16 * MS;
            let encoded = if i % 7 == 0 { ltr(100, i) } else { frame(100) };
            let sent = send(&mut s, now, &encoded);
            s.acked(sent.header.number);
            s.poll(now);
        }
        assert!(s.frames.len() <= KEPT + 1, "{} records", s.frames.len());
        assert_eq!(s.stats().sent, 5000);
    }

    /// What must hold of a frame that was just written.
    fn check(s: &Sender, sent: &Sent) -> Result<(), proptest::test_runner::TestCaseError> {
        sent.header.validate().unwrap();
        // A frame is never written with the epoch of a size it was not made for.
        proptest::prop_assert_eq!(sent.header.epoch, s.epoch);
        match sent.header.kind() {
            // A frame that begins a chain resets everything that is not held, so it is alone.
            FrameKind::Key | FrameKind::Recovery => proptest::prop_assert_eq!(s.in_flight(), 1),
            // A delta never follows a frame that was given up on.
            FrameKind::Delta => {
                if let Some(dependency) = s.frames.get(&sent.header.depends_on) {
                    proptest::prop_assert!(dependency.state != State::Abandoned);
                }
            }
        }
        Ok(())
    }

    proptest::proptest! {
        /// Whatever happens, headers are valid, the limits hold, and nothing is held that
        /// depends on something that is not. A picture can wait in the encoder while the size
        /// changes, or frames are given up on.
        #[test]
        fn any_run_of_events_keeps_the_rules(
            events in proptest::collection::vec((0_u8..10, 0_u64..40, 1_usize..3000), 1..300)
        ) {
            let mut s = sender();
            let mut now = Duration::ZERO;
            let mut headers: Vec<FrameHeader> = Vec::new();
            let mut encoding = false;
            for (event, n, size) in events {
                now += Duration::from_millis(n);
                let encoded = Encoded {
                    size,
                    referenced: n % 3 != 0,
                    ltr: (n % 5 == 0).then_some(LtrToken(n)),
                    capture_time: 0,
                };
                match event {
                    0 | 1 => {
                        if s.begin(&path()).is_some()
                            && let Some(sent) = s.sent(now, &encoded, &path())
                        {
                            check(&s, &sent)?;
                            headers.push(sent.header);
                        }
                    }
                    2 => encoding |= s.begin(&path()).is_some(),
                    3 => {
                        if encoding {
                            encoding = false;
                            if let Some(sent) = s.sent(now, &encoded, &path()) {
                                check(&s, &sent)?;
                                headers.push(sent.header);
                            }
                        }
                    }
                    4 | 5 => {
                        let pick = usize::try_from(n).unwrap() % headers.len().max(1);
                        if let Some(h) = headers.get(pick) {
                            s.acked(h.number);
                        }
                    }
                    6 => { s.poll(now); }
                    7 => {
                        if s.next_number() > 0 {
                            let tried = n % s.next_number();
                            s.request_key_frame(now, tried).unwrap();
                            // The request is answered, or a key frame is on its way or due.
                            let on_its_way = s.frames.range(tried + 1..).any(|(_, f)| {
                                f.kind == FrameKind::Key
                                    && matches!(f.state, State::InFlight | State::Held)
                            });
                            proptest::prop_assert!(
                                on_its_way || s.want_key || s.key_due.is_some()
                            );
                        }
                    }
                    _ => { s.new_epoch(); }
                }
                proptest::prop_assert!(s.in_flight() <= Params::default().max_in_flight);
                for (number, frame) in &s.frames {
                    if frame.state == State::Held
                        && frame.depends_on != *number
                        && let Some(dependency) = s.frames.get(&frame.depends_on)
                    {
                        proptest::prop_assert_eq!(dependency.state, State::Held);
                    }
                }
            }
        }
    }
}
