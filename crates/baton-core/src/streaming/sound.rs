// SPDX-License-Identifier: Apache-2.0

//! The sound of a screen in a window streaming session (`docs/adr/0012-sound.md`, and section 9 of
//! `docs/protocol.md`).
//!
//! When a source shares a whole display, the sound that it plays can go with the picture. The
//! viewer says when it wants to hear. The platform of the source captures and encodes the sound,
//! tells the session what the packets are and hands them over one by one, and the session writes
//! each on a stream of its own, keeping to the rules of `baton_stream::sound::send` for the packets
//! in flight. The session of the viewer reads the streams, puts the packets in order with the rules
//! of `baton_stream::sound::receive`, and gives them in that order to the [`Speaker`], which the
//! platform provides, with a count of what was lost.
//!
//! This module holds what the two sides keep for sound, and what the platform is given and
//! provides. The session code is in `source.rs` and `view.rs`, next to the code for the frames of
//! video that it resembles.

use std::{collections::HashMap, time::Duration};

use baton_proto::message::Message;
use baton_stream::{
    control::{SilentReason, Sound, SoundCodec, SourceMessage},
    sound::{
        PacketHeader,
        receive::{Action, Params as ReceiveRules, Receiver, StreamId, Violation},
        send::{Params as SendRules, Sender},
    },
};
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};

/// A packet of sound that the encoder made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundPacket {
    /// The packet, as the codec of the [`Sound`] that began the run made it. For `Aac` it is one
    /// access unit with no ADTS header and no other framing.
    pub bytes: Vec<u8>,
    /// When the first sample of the packet was captured, in microseconds since the session
    /// began: the clock of [`SourceHandle::elapsed`](super::SourceHandle::elapsed), which is the
    /// clock of the capture times of the pictures.
    pub capture_time: u64,
}

/// A packet of sound for the speaker to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Play {
    /// The number of the packet.
    pub number: u64,
    /// When the first sample of the packet was captured, in microseconds since the session began,
    /// on the clock of the source.
    pub capture_time: u64,
    /// Whether the decoder starts afresh at this packet, which begins a run of sound.
    pub first: bool,
    /// How many packets before this one are missing: they did not come in time, or the source
    /// reset them, and they are not played if they come later. The speaker covers the gap, which
    /// is that many packets long. It is 0 for the first packet of a run.
    pub lost_before: u64,
}

/// What plays the sound of a session, which the platform provides.
///
/// A speaker is given a run of sound: first what the packets are ([`sound`](Self::sound)), then
/// the packets in the order in which they are to be played ([`play`](Self::play)), until the run is
/// over ([`stopped`](Self::stopped)). The viewer asks for a run with
/// [`ViewHandle::hear`](super::ViewHandle::hear).
///
/// Every method is called with the session's lock held, so none of them waits: a speaker that
/// decodes or plays hands the work to a thread of its own.
pub trait Speaker: Send + 'static {
    /// The source sends sound from now on, in this form, which is one that the session plays
    /// ([`playable`]) and the same every time in a session. A packet that comes after this one
    /// belongs to it, until the next [`sound`](Self::sound) or [`stopped`](Self::stopped).
    fn sound(&mut self, sound: Sound);

    /// Decodes and plays a packet, which follows the one before it unless `play.lost_before` says
    /// that some are missing.
    fn play(&mut self, play: Play, packet: Vec<u8>);

    /// The run of sound is over, and the speaker lets go of what it holds for it: the viewer turned
    /// sound off (`None`), or the source says that it sends no sound, and why (`Some`). Sound goes
    /// on only after the next [`sound`](Self::sound).
    fn stopped(&mut self, why: Option<SilentReason>);

    /// The session is over. The speaker lets go of everything it holds, and is called no more.
    fn ended(&mut self);
}

impl Speaker for Box<dyn Speaker> {
    fn sound(&mut self, sound: Sound) {
        (**self).sound(sound);
    }

    fn play(&mut self, play: Play, packet: Vec<u8>) {
        (**self).play(play, packet);
    }

    fn stopped(&mut self, why: Option<SilentReason>) {
        (**self).stopped(why);
    }

    fn ended(&mut self) {
        (**self).ended();
    }
}

/// What a source does about sound in one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SoundPlan {
    /// It sends none: this device was not set up to send sound, or the viewer does not play it. A
    /// viewer sends a `Hear` only to a device that advertised `stream.sound`, and only if it
    /// advertised `stream.hear` itself, so a `Hear` is a violation.
    None,
    /// The session is of a window, and sound goes with a whole display only. A `Hear` that is on
    /// is answered with a `Silent` whose reason is `Window`.
    Window,
    /// The session is of a display. The platform is told what the viewer wants.
    Display,
}

impl SoundPlan {
    /// The plan of a session: `sends` is whether this device advertises `stream.sound`, `plays`
    /// whether the viewer advertised `stream.hear`, and `display` whether the offer is of a whole
    /// display.
    pub(crate) fn of(sends: bool, plays: bool, display: bool) -> Self {
        match (sends && plays, display) {
            (false, _) => Self::None,
            (true, false) => Self::Window,
            (true, true) => Self::Display,
        }
    }
}

/// What a source keeps about the sound of a session.
pub(super) struct Sounding {
    /// The rules for the packets in flight.
    pub(super) sender: Sender,
    /// The streams of the packets that are open or being reset, by the number of the packet.
    /// Cancelling a token resets its stream; the entry stays until the stream has stopped
    /// counting at the viewer.
    pub(super) packets: HashMap<u64, CancellationToken>,
    /// The latest `Hear` of the viewer was on. Only a session of a display keeps this.
    pub(super) hear: bool,
    /// What the platform was last told: whether the sound of the screen is wanted.
    pub(super) told: bool,
    /// A `Sound` was written, and no `Silent` since and no `Hear` that is off: packets are taken.
    pub(super) running: bool,
    /// The form of the sound of the session, which the first `Sound` that was written fixed: a
    /// session has one (protocol.md section 9).
    pub(super) format: Option<Sound>,
}

impl Sounding {
    pub(super) fn new(session: baton_stream::SessionId, rules: SendRules) -> Self {
        Self {
            sender: Sender::new(session, rules),
            packets: HashMap::new(),
            hear: false,
            told: false,
            running: false,
            format: None,
        }
    }

    /// Resets the streams of these packets.
    pub(super) fn reset(&self, numbers: &[u64]) {
        for number in numbers {
            if let Some(token) = self.packets.get(number) {
                token.cancel();
            }
        }
    }
}

/// What a `Sound` of the source made the viewer do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Said {
    /// It was taken, or dropped because the viewer does not want to hear.
    Done,
    /// The viewer cannot play it and turned sound off. The source is to be told with a `Hear`.
    Unplayable,
    /// It differs from the form that the source began the session with, which is a violation.
    Violation,
}

/// Whether this version plays sound in this form: AAC-LC, which has 1024 samples in a packet, or
/// 960 in the variant that some encoders make. A rate from 8000 to 96000 hertz and one or two
/// channels are within what the protocol allows already, and the system decoders take them.
pub fn playable(sound: Sound) -> bool {
    sound.codec == SoundCodec::Aac && matches!(sound.samples, 960 | 1024)
}

/// A packet that waits for the packet before it.
struct Held {
    capture_time: u64,
    bytes: Vec<u8>,
}

/// What a viewer keeps about the sound of a session, when its application gave the session a
/// [`Speaker`].
pub(super) struct Hearing {
    speaker: Box<dyn Speaker>,
    /// The latest `Hear` that the viewer sent was on. A `Sound` or a `Silent` that comes while it
    /// is off is dropped, as is a packet.
    on: bool,
    /// The form of the sound of the session, from the first `Sound` that was taken: every other
    /// is the same (protocol.md section 9).
    format: Option<Sound>,
    receiver: Receiver,
    /// The packets that the receiver holds, by number.
    packets: HashMap<u64, Held>,
}

impl Hearing {
    pub(super) fn new(speaker: Box<dyn Speaker>, rules: ReceiveRules) -> Self {
        Self {
            speaker,
            on: false,
            format: None,
            receiver: Receiver::new(rules),
            packets: HashMap::new(),
        }
    }

    /// The rules for putting the packets in order, for the streams that are read.
    pub(super) fn receiver(&mut self) -> &mut Receiver {
        &mut self.receiver
    }

    /// What the rules have counted, and the packet streams being read.
    pub(super) fn counted(&self) -> (baton_stream::sound::receive::Stats, usize) {
        (self.receiver.stats(), self.receiver.streams())
    }

    /// The time at which the packet that has waited longest has waited for the hold.
    pub(super) fn next_deadline(&self) -> Option<Duration> {
        self.receiver.next_deadline()
    }

    /// The viewer turns sound on or off. Turning it off ends the run. Says whether anything
    /// changed.
    pub(super) fn turn(&mut self, on: bool) -> bool {
        let was = std::mem::replace(&mut self.on, on);
        if was && !on {
            self.stop(None);
        }
        was != on
    }

    /// The source says what its packets are. Dropped while the viewer does not want to hear. A
    /// form that this viewer cannot play ends the run, as a `Silent` would, and turns sound off:
    /// the source is to be told, since it keeps the Mac quiet for a viewer that plays nothing.
    pub(super) fn sound(&mut self, sound: Sound) -> Said {
        if !self.on {
            return Said::Done;
        }
        if self.format.is_some_and(|format| format != sound) {
            return Said::Violation;
        }
        if !playable(sound) {
            self.on = false;
            self.stop(Some(SilentReason::Other));
            return Said::Unplayable;
        }
        self.format = Some(sound);
        self.receiver.begin_run();
        self.speaker.sound(sound);
        Said::Done
    }

    /// The source sends no sound, and says why. Dropped while the viewer does not want to hear.
    pub(super) fn silent(&mut self, reason: SilentReason) {
        if self.on {
            self.stop(Some(reason));
        }
    }

    /// Ends the run: what the receiver holds is forgotten, and the speaker is told.
    fn stop(&mut self, why: Option<SilentReason>) {
        for action in self.receiver.end_run() {
            if let Action::Discard(number) = action {
                self.packets.remove(&number);
            }
        }
        self.speaker.stopped(why);
    }

    /// The packet of a stream is whole at `now`. Returns whether the speaker and the rules agree
    /// on which packets there are, which they do unless there is a fault here.
    ///
    /// # Errors
    ///
    /// The source sent a packet whose number the viewer holds already.
    pub(super) fn arrived(
        &mut self,
        now: Duration,
        stream: StreamId,
        header: PacketHeader,
        bytes: Vec<u8>,
    ) -> std::result::Result<bool, Violation> {
        let actions = self.receiver.arrived(now, stream, header)?;
        let read = Held {
            capture_time: header.capture_time,
            bytes,
        };
        Ok(self.apply(actions, Some((header.number, read))))
    }

    /// Looks at the clock. Returns whether the speaker and the rules agree.
    pub(super) fn tick(&mut self, now: Duration) -> bool {
        let actions = self.receiver.tick(now);
        self.apply(actions, None)
    }

    /// The session is over.
    pub(super) fn ended(&mut self) {
        self.packets.clear();
        self.speaker.ended();
    }

    /// Carries out what the rules say, in order. `read` is the packet that was just read, which
    /// the rules may hold, play or drop: a drop of its number is a drop of it, also when a packet
    /// with the same number is held, which stays.
    fn apply(&mut self, actions: Vec<Action>, mut read: Option<(u64, Held)>) -> bool {
        let mut consistent = true;
        for action in actions {
            match action {
                Action::Play {
                    number,
                    first,
                    lost_before,
                } => {
                    let held = match read.take_if(|(read, _)| *read == number) {
                        Some((_, held)) => Some(held),
                        None => self.packets.remove(&number),
                    };
                    let Some(Held {
                        capture_time,
                        bytes,
                    }) = held
                    else {
                        consistent = false;
                        continue;
                    };
                    let play = Play {
                        number,
                        capture_time,
                        first,
                        lost_before,
                    };
                    self.speaker.play(play, bytes);
                }
                Action::Discard(number) => {
                    if read.as_ref().is_some_and(|(read, _)| *read == number) {
                        read = None;
                    } else {
                        self.packets.remove(&number);
                    }
                }
            }
        }
        // What the rules did not play or drop is held, and waits for the packets before it.
        if let Some((number, held)) = read {
            self.packets.insert(number, held);
        }
        consistent
    }
}

/// Checks that `sound` is within the limits of the protocol.
///
/// # Errors
///
/// A number is outside what the protocol allows.
pub(super) fn check(sound: Sound) -> Result<()> {
    SourceMessage::Sound(sound)
        .validate()
        .map_err(|error| Error::protocol(&error))
}
