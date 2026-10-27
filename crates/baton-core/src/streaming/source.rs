// SPDX-License-Identifier: Apache-2.0

//! The source's side of a window streaming session.
//!
//! The source sends the picture of one window as a run of frames, each on a stream of its own,
//! and keeps a control stream open to the viewer. The rules for the frames are those of
//! `baton_stream::sender::Sender`, which says whether the next picture is to be encoded at all,
//! what it has to be, and which frames to give up on and when. This module carries them out on
//! the streams of a connection. The platform captures and encodes: it asks
//! [`SourceHandle::begin`] before it encodes a picture, hands over the picture with
//! [`SourceHandle::sent`], and hears from the viewer through [`SourceEvents`]. Nothing tells it
//! when a refusal by `begin` has stopped holding, so it asks on a frame clock of its own for as
//! long as the window has changed since the last picture it handed over.
//!
//! A frame has a task of its own. It opens the stream, writes the header and the picture,
//! finishes the stream, and waits for the viewer's transport to acknowledge all of it, which is
//! what a frame being acknowledged means to the rules. A viewer that stops the stream has not
//! taken the frame, and a frame that the rules give up on is reset. A task lasts until the stream
//! has stopped counting at the viewer, which for a reset stream is when the reset is acknowledged,
//! and the source never has more than twelve of them, which is what the viewer allows it
//! (`docs/adr/0009-window-streaming.md`).
//!
//! The viewer reports a few times a second. A source that has had no report for a while stops
//! encoding, because no one may be watching, and one that has had none for longer ends the
//! session. A session ends once: the platform is told, the frames in flight are reset, and later
//! streams of the session are stopped with code 499.
//!
//! The sound of a display goes the same way, a packet on a stream of its own, and follows the rules
//! of `baton_stream::sound::send`. The viewer asks for it ([`SourceEvents::hear`]), the platform
//! answers with [`SourceHandle::begin_sound`] when it captures, or [`SourceHandle::silent`] when
//! it cannot, and hands over the packets with [`SourceHandle::sound`]. A session of a window has no
//! sound, and the source says so itself.

use std::{
    collections::HashMap,
    ops::ControlFlow,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use baton_proto::{
    DeviceId, close_code,
    id::SessionId,
    limits::{MAX_PICTURE_BYTES, MAX_SOUND_PACKET_BYTES},
    message::Message,
    stream::StopReason,
};
use baton_stream::{
    MAX_FRAME_STREAMS, MAX_SOUND_STREAMS,
    control::{
        Config, Hear, InputBatch, KeyFrame, Lift, Pause, PauseReason, Report, Silent, SilentReason,
        Sound, SourceMessage, ViewerMessage, Want,
    },
    gate::{Notices, Reason},
    header::{FrameHeader, FrameKind},
    sender::{Encoded, LtrToken, Params as Rules, Path, Sender, Sent, Stats},
    sound::{
        PacketHeader,
        send::{Params as SoundRules, Sent as SoundSent, Stats as SoundStats},
    },
    unlock::{Ask, Cancelled},
};
use iroh::endpoint::{Connection, SendStream, WriteError};
use tokio::{sync::mpsc, time::MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use super::{
    ControlEnd, End, MayHear, SoundPacket, SoundPlan, Streaming, Takes, UnlockAttempt, WriteEnd,
    read_control, sound, stop,
    unlock::{Handoff, Job, Step, UnlockSetup, Unlocking, Waiting},
    write_control,
};
use crate::{
    error::{Error, Result},
    gate::code,
    session::Hold,
    trust::PairedDevice,
    uni::Arrival,
    util::lock,
};

/// A frame goes after the requests and the control streams, which are at 0, and before the
/// lanes of a file transfer, which are at -2. See ADR 0009.
const FRAME_PRIORITY: i32 = -1;

/// A packet of sound goes at the priority of the requests and the control streams, so that it
/// never waits behind a frame or a lane. See ADR 0012.
const SOUND_PRIORITY: i32 = 0;

/// How many messages to the viewer wait to be written. A message that finds the queue full is
/// dropped: a viewer that reads nothing for this long is not going to read it.
const OUTBOX: usize = 64;

/// What the rules use for a connection that shows no path, which is one that has just begun.
const UNKNOWN_PATH: Path = Path {
    window: 64 * 1024,
    rtt: Duration::from_millis(100),
    rate: 0,
};

/// The error for a picture format that the viewer did not say it takes.
fn beyond_what_the_viewer_takes() -> Error {
    Error::Config("the picture is beyond what the viewer can take".to_owned())
}

/// What the platform hears from the viewer and from the session.
///
/// Every method is called with the session's lock held, so none of them waits and none of them
/// calls the [`SourceHandle`]: a platform that takes time, or that has something to ask, hands
/// the work to a thread of its own.
pub trait SourceEvents: Send + 'static {
    /// How the viewer is doing: the newest frame it showed, the time it takes to decode, the
    /// frames it dropped and how hot the device is.
    fn report(&mut self, report: Report);

    /// The largest picture and the highest frame rate that the viewer can take now.
    fn want(&mut self, want: Want);

    /// What the person did on the viewer, in the order in which it happened, and when the batch
    /// arrived, on the clock of the session: the time since it began, which is the clock of the
    /// capture times and the `now` of [`baton_stream::gate::Context`]. Only in a session whose
    /// offer granted control; the events of any other session are dropped here, and the viewer
    /// is told why.
    ///
    /// The platform runs each event through a [`baton_stream::gate::Gate`], which checks the
    /// rate, the pause, the lock and the geometry, and tells the session what it dropped with
    /// [`SourceHandle::dropped`]. So input still arrives while the session is paused: the gate
    /// has to deliver the release of a press that it delivered.
    fn input(&mut self, batch: InputBatch, arrived: Duration);

    /// The viewer holds these long-term references now, and the encoder may refer to them.
    fn held(&mut self, tokens: Vec<LtrToken>);

    /// The viewer says what the people here may do while the session runs: see the screen, and
    /// use the keys and the pointer of this device. Only in a session whose offer said that the
    /// screen is private; the messages of any other session are dropped before they get here. The
    /// viewer says the same thing again and again, so the platform sees what it already did, and
    /// it is not told whether it worked: the platform ends the session if it cannot take back what
    /// was lifted. A platform that keeps nothing private has nothing to lift, and leaves this as
    /// it is.
    fn lift(&mut self, lift: Lift) {
        let _ = lift;
    }

    /// The next frame has to begin a chain, so a picture is wanted for it even if the screen has
    /// not changed: without one a frame that was lost would never be replaced. The first frame of
    /// a session is one too, so a session that has sent nothing yet says so at once.
    ///
    /// This is said once each time the next frame comes to begin a chain, to wake a platform that
    /// waits for the screen to change. It is not said again when a refusal by
    /// [`SourceHandle::begin`] stops holding (the viewer had gone quiet, the session was paused,
    /// too many frames were in flight): for as long as
    /// [`recovery_pending`](SourceHandle::recovery_pending) is true the platform asks `begin`
    /// again on its own frame clock.
    fn recovery(&mut self);

    /// Whether the sound of the screen is wanted: the viewer asked to hear it and has not turned
    /// it off, and has not gone quiet. Only in a session of a whole display, on a device that was
    /// set up to send sound ([`NodeBuilder::share_sound`](crate::NodeBuilder::share_sound)).
    ///
    /// On `true` the platform captures the sound, and says what its packets are with
    /// [`SourceHandle::begin_sound`], or says why it cannot with [`SourceHandle::silent`]. This is
    /// said again, with nothing in between, when the viewer asks again to hear while no run of
    /// sound is going, as it does to try again after a `Silent`. On `false` it stops capturing and
    /// lets the screen's sound play again. A viewer that has gone quiet, with no report for ten
    /// seconds, does not hear, so the sound of the screen is not held back for a viewer that is not
    /// there.
    ///
    /// The default does nothing: a platform that cannot capture sound is not asked.
    fn hear(&mut self, on: bool) {
        let _ = on;
    }

    /// Whether the cover of the screens, the hold of the keys and the pointer, and the guard that
    /// locks the device if the app dies are all in place, now. The session asks when the viewer
    /// asks to unlock, and refuses with `NotPrivate` unless the answer is yes: an unlock lights a
    /// display and shows a lock screen, which only a device that keeps its screens dark may do
    /// (ADR 0013). The platform answers from what it keeps, without looking at the system, since
    /// the session's lock is held.
    ///
    /// The default is no: a platform that keeps nothing private cannot vouch for it.
    fn private_in_place(&mut self) -> bool {
        false
    }

    /// The viewer's person was recognised and sent the key of the box: the platform unlocks the
    /// screen. It is given the [`UnlockAttempt`] and carries it out on a thread of its own, since
    /// the session's lock is held, and says what came of it with
    /// [`UnlockAttempt::done`]. This is called only in a session whose offer said `unlock`, only
    /// while the screen is locked and [`private_in_place`](Self::private_in_place) was true, and
    /// for one attempt at a time.
    ///
    /// The default lets go of the attempt, which reports `Failed`: a platform that cannot type a
    /// password cannot unlock.
    fn unlock(&mut self, attempt: UnlockAttempt) {
        drop(attempt);
    }

    /// The session is over, and the person at the viewer chose, when they ended it, that the
    /// screen be left as it is. The platform lets go of everything it holds for the session, as it
    /// does for [`ended`](Self::ended), and does not lock the screen. A screen that is locked
    /// stays locked, and nothing is unlocked. This is called instead of `ended`, at the same
    /// point, and the platform is called no more.
    ///
    /// It is called only for a session whose offer said `leave`, which is a private session of a
    /// node that was set up to leave its screen ([`NodeBuilder::allow_leave`](crate::NodeBuilder::allow_leave)),
    /// and only when the viewer's last message was `Leave` and it then finished its control
    /// stream. Every other end, whatever its cause, calls `ended`: the viewer going away, the
    /// connection ending, a rule broken, the person at the source, the node stopping.
    ///
    /// The default calls `ended`: a platform that cannot leave the screen as it is locks it, as it
    /// does at every other end.
    fn ended_leaving(&mut self) {
        self.ended();
    }

    /// The session is over. The platform lets go of everything it holds for it, and is called no
    /// more. It is called before anything that waits for the end of the session, such as
    /// [`SourceHandle::ended`], is woken.
    fn ended(&mut self);
}

impl SourceEvents for Box<dyn SourceEvents> {
    fn report(&mut self, report: Report) {
        (**self).report(report);
    }

    fn want(&mut self, want: Want) {
        (**self).want(want);
    }

    fn input(&mut self, batch: InputBatch, arrived: Duration) {
        (**self).input(batch, arrived);
    }

    fn held(&mut self, tokens: Vec<LtrToken>) {
        (**self).held(tokens);
    }

    fn lift(&mut self, lift: Lift) {
        (**self).lift(lift);
    }

    fn recovery(&mut self) {
        (**self).recovery();
    }

    fn hear(&mut self, on: bool) {
        (**self).hear(on);
    }

    fn private_in_place(&mut self) -> bool {
        (**self).private_in_place()
    }

    fn unlock(&mut self, attempt: UnlockAttempt) {
        (**self).unlock(attempt);
    }

    fn ended_leaving(&mut self) {
        (**self).ended_leaving();
    }

    fn ended(&mut self) {
        (**self).ended();
    }
}

/// The time limits and sizes of the source's side of a session. The defaults are where ADR 0009
/// starts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceParams {
    /// The rules for the frames in flight.
    pub(crate) rules: Rules,
    /// The rules for the packets of sound in flight.
    pub(crate) sound: SoundRules,
    /// How often the source looks at the clock, to give up on frames that are late.
    pub(crate) tick_every: Duration,
    /// Without a report for this long nothing is encoded.
    pub(crate) quiet_after: Duration,
    /// Without a report for this long the session ends.
    pub(crate) silent_after: Duration,
    /// Once the length of a message of the viewer has arrived, the rest of it has this long.
    pub(crate) stall: Duration,
    /// The time an unlock waits for the face of the person at the viewer, once it has asked.
    pub(crate) unlock_wait: Duration,
    /// The time the platform has to unlock the screen, from the moment it is given the key. The
    /// viewer waits longer than this for the end.
    pub(crate) unlock_work: Duration,
    /// How soon after a password was rejected a screen that unlocks proves it right.
    pub(crate) unlock_grace: Duration,
}

impl Default for SourceParams {
    fn default() -> Self {
        Self {
            rules: Rules::default(),
            sound: SoundRules::default(),
            tick_every: Duration::from_millis(100),
            quiet_after: Duration::from_secs(10),
            silent_after: Duration::from_secs(30),
            stall: Duration::from_secs(10),
            unlock_wait: Duration::from_secs(60),
            unlock_work: Duration::from_secs(25),
            unlock_grace: Duration::from_secs(20),
        }
    }
}

/// What a session needs to begin, besides who it is with.
#[derive(Debug, Clone)]
pub(crate) struct SourceSetup {
    /// The picture the source starts with. Its `epoch` is ignored: the source numbers the epochs.
    pub(crate) config: Config,
    /// Whether the person at the source granted the viewer control of the window.
    pub(crate) control: bool,
    /// Whether the screen is kept private for the session, and the viewer may lift the screen
    /// cover and the hold of the keys and the pointer.
    pub(crate) private: bool,
    /// What the viewer said it can take. The first picture format and every later one stay
    /// within it.
    pub(crate) takes: Takes,
    /// What the session does about sound.
    pub(crate) sound: SoundPlan,
    /// Whether the person at this device lets the viewer hear, which a display asks each time the
    /// viewer asks to.
    pub(crate) may_hear: MayHear,
    /// What the session does about an unlock.
    pub(crate) unlock: UnlockSetup,
    /// Whether the offer said `leave`, so that a viewer which ends the session can ask for the
    /// screen to be left as it is, and not locked.
    pub(crate) leave: bool,
    /// The limits.
    pub(crate) params: SourceParams,
}

/// Who a session is with, and where.
pub(super) struct Whom {
    pub(super) session: SessionId,
    pub(super) peer: DeviceId,
    pub(super) connection: Connection,
    /// Keeps the connection in the pool for as long as the session runs.
    pub(super) hold: Option<Hold>,
}

/// A picture that the encoder made.
#[derive(Debug, Clone)]
pub struct Picture {
    /// The encoded frame, in the format that the session agreed on.
    pub bytes: Vec<u8>,
    /// Whether the encoder says that a later frame may refer to it.
    pub referenced: bool,
    /// The token of the frame, if the encoder marked it as a long-term reference.
    pub ltr: Option<LtrToken>,
    /// When the picture was captured, in microseconds since the session began.
    pub capture_time: u64,
}

/// Why a picture was not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NotSent {
    /// The picture, or the packet of sound, is larger than a viewer takes. Nothing was sent, and
    /// the next frame is made again as the same kind of frame; the encoder is to make smaller
    /// pictures.
    #[error("the picture or the packet is larger than the protocol allows")]
    TooLarge,
    /// The session is over.
    #[error("the session is over")]
    Over,
}

/// What a session has counted, for the person and for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceStats {
    /// What the rules for the frames have counted.
    pub frames: Stats,
    /// The frame streams that are open or being reset.
    pub streams: usize,
    /// What the rules for the packets of sound have counted.
    pub sound: SoundStats,
    /// The packet streams that are open or being reset.
    pub sound_streams: usize,
}

/// How a frame's stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The viewer's transport has all of it.
    Acked,
    /// The viewer stopped the stream: the frame did not arrive.
    Stopped,
    /// This side reset it, because the frame was late or of no use, or the session ended.
    Reset,
    /// The connection went.
    Gone,
}

struct Core {
    sender: Sender,
    sound: sound::Sounding,
    events: Box<dyn SourceEvents>,
    notices: Notices,
    /// The frame streams that are open or being reset, by the number of the frame. Cancelling a
    /// token resets its stream; the entry stays until the stream has stopped counting.
    frames: HashMap<u64, CancellationToken>,
    /// When the last report came, on the clock of the session.
    last_report: Duration,
    /// No report has come for [`SourceParams::quiet_after`].
    quiet: bool,
    /// The platform paused the session.
    paused: bool,
    /// The platform was told that the next frame begins a chain.
    recovery_told: bool,
    /// What the session does about an unlock.
    unlock: Plan,
}

/// What a session does about an unlock, with the rules if it takes one.
enum Plan {
    /// The messages of an unlock are a violation.
    Unsupported,
    /// The messages of an unlock are dropped.
    NotOffered,
    /// The session takes an unlock. Boxed, since the rules are large next to the other variants.
    Offered(Box<Unlocking>),
}

impl Core {
    /// Resets the streams of these frames.
    fn reset(&mut self, numbers: &[u64]) {
        for number in numbers {
            if let Some(token) = self.frames.get(number) {
                token.cancel();
            }
        }
    }

    /// Tells the platform when the next frame has become one that begins a chain, once.
    fn check_recovery(&mut self) {
        let pending = self.sender.recovery_pending();
        if pending && !self.recovery_told {
            self.events.recovery();
        }
        self.recovery_told = pending;
    }

    /// The viewer sent a `Hear`, in a session of a display.
    fn heard(&mut self, on: bool) {
        self.sound.hear = on;
        if !on {
            self.sound.running = false;
        }
        // A `Hear` that is on asks the platform to try again when no run goes, also when it was
        // told to listen already: the last try may have ended in a `Silent`.
        let again = on && self.sound.told && !self.sound.running && !self.quiet;
        self.check_listening();
        if again {
            self.events.hear(true);
        }
    }

    /// Tells the platform whether the sound of the screen is wanted, when that has changed: the
    /// viewer wants to hear and has not gone quiet. A viewer that has gone quiet does not keep the
    /// screen's sound from playing, and the run it had is over.
    fn check_listening(&mut self) {
        let listening = self.sound.hear && !self.quiet;
        if listening == self.sound.told {
            return;
        }
        self.sound.told = listening;
        if !listening {
            self.sound.running = false;
        }
        self.events.hear(listening);
    }

    /// The rules of the unlock, if the session takes one.
    fn unlocking(&mut self) -> Option<&mut Unlocking> {
        match &mut self.unlock {
            Plan::Offered(unlocking) => Some(unlocking.as_mut()),
            Plan::Unsupported | Plan::NotOffered => None,
        }
    }
}

/// The source's side of one session.
pub(super) struct Source {
    registry: Weak<Streaming>,
    session: SessionId,
    peer: DeviceId,
    connection: Connection,
    /// The platform calls from threads of its own, which are not in the runtime.
    runtime: tokio::runtime::Handle,
    params: SourceParams,
    /// Whether the viewer may control the window.
    control: bool,
    /// Whether the screen is kept private, which the viewer may lift in part.
    private: bool,
    /// Whether the offer said `leave`: the viewer can ask, with a last `Leave`, for the screen to
    /// be left as it is when the session ends.
    leave: bool,
    /// Whether the viewer has said `Leave`, which is the last thing it says. It counts when the
    /// viewer then finishes its control stream, and not when the stream is reset or the connection
    /// ends: the person chose, but the end that follows is one that locks.
    left: AtomicBool,
    /// What the viewer takes, which the pictures stay within.
    takes: Takes,
    /// What the session does about sound.
    sound: SoundPlan,
    /// Whether the person here lets the viewer hear.
    may_hear: MayHear,
    began: Instant,
    core: Mutex<Core>,
    /// What the source says to the viewer, in the order in which it says it.
    outbox: mpsc::Sender<SourceMessage>,
    /// Whether the viewer's control stream has been taken. A second one is a violation.
    viewer_control: AtomicBool,
    /// Cancelled when the session ends. The tokens of the frame streams are its children.
    ended: CancellationToken,
    /// How the session ended, set before the platform is told. The platform is told of
    /// everything with the lock of `core` held, so whatever is about to tell it something looks
    /// here once it has the lock, and a session that has ended is told nothing more.
    end: OnceLock<End>,
}

/// What the owner of a session holds. Dropping it ends the session.
pub struct SourceHandle {
    source: Arc<Source>,
}

impl std::fmt::Debug for SourceHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceHandle").finish_non_exhaustive()
    }
}

impl SourceHandle {
    /// Asks whether the next picture may be encoded, and what it has to be: a key frame, a
    /// recovery frame or a delta. `None` means no: too much is in flight, or a frame is still
    /// being made, or the session is paused or quiet or over. The platform then skips the
    /// picture.
    ///
    /// A `None` is not remembered, and nothing says when it stops holding. A window that has
    /// stopped changing delivers no newer picture, so a platform that waited for the next one
    /// would leave the viewer with an old one. It asks again on its own frame clock, with the
    /// newest picture that the capture API hands over when it is asked for one, for as long as
    /// the window has changed since the last picture it handed over to [`sent`](Self::sent) or
    /// [`recovery_pending`](Self::recovery_pending) is true.
    ///
    /// A `Some` is answered with [`sent`](Self::sent) or [`cancel`](Self::cancel).
    pub fn begin(&self) -> Option<FrameKind> {
        let source = &self.source;
        let path = source.path();
        let mut core = lock(&source.core);
        if source.end.get().is_some()
            || core.paused
            || core.quiet
            || core.frames.len() >= MAX_FRAME_STREAMS
        {
            return None;
        }
        core.sender.begin(&path)
    }

    /// The frame that [`begin`](Self::begin) announced could not be made. Nothing was sent.
    pub fn cancel(&self) {
        lock(&self.source.core).sender.cancel();
    }

    /// Whether the next frame has to be something other than a delta, so a picture is wanted even
    /// if the screen has not changed.
    pub fn recovery_pending(&self) -> bool {
        lock(&self.source.core).sender.recovery_pending()
    }

    /// The frame that [`begin`](Self::begin) announced was made: writes it on a stream of its
    /// own. Returns the number it got, or `None` if it was of no use any more because the size
    /// of the picture changed, or a frame that it was to follow was given up on, while it was
    /// being made. The platform drops the picture then.
    ///
    /// # Errors
    ///
    /// The picture is larger than a viewer takes, or the session is over. Nothing was sent.
    pub fn sent(&self, picture: Picture) -> std::result::Result<Option<u64>, NotSent> {
        let source = &self.source;
        let path = source.path();
        let now = source.now();
        let mut core = lock(&source.core);
        if source.end.get().is_some() {
            return Err(NotSent::Over);
        }
        if picture.bytes.len() > MAX_PICTURE_BYTES {
            core.sender.cancel();
            return Err(NotSent::TooLarge);
        }
        let encoded = Encoded {
            size: picture.bytes.len(),
            referenced: picture.referenced,
            ltr: picture.ltr,
            capture_time: picture.capture_time,
        };
        let sent = core.sender.sent(now, &encoded, &path);
        core.check_recovery();
        let Some(Sent { header, reset }) = sent else {
            return Ok(None);
        };
        core.reset(&reset);
        let cancel = source.ended.child_token();
        core.frames.insert(header.number, cancel.clone());
        drop(core);
        source
            .runtime
            .spawn(Arc::clone(source).send_frame(header, picture.bytes, cancel));
        Ok(Some(header.number))
    }

    /// The size of the picture changed. Starts a new epoch, which begins with a key frame, resets
    /// the frames in flight, which are of the old size, and tells the viewer. A picture that is
    /// being made is of the old size too, and [`sent`](Self::sent) drops it.
    ///
    /// Once the session is over this changes nothing: a platform cannot rule out that the end
    /// comes while it is resizing, and it is told of the end anyway.
    ///
    /// # Errors
    ///
    /// `config` is beyond a limit of the protocol. Nothing changed.
    pub fn new_epoch(&self, mut config: Config) -> Result<()> {
        let source = &self.source;
        SourceMessage::Config(config)
            .validate()
            .map_err(|error| Error::protocol(&error))?;
        if !source.takes.allows(&config) {
            return Err(beyond_what_the_viewer_takes());
        }
        let mut core = lock(&source.core);
        if source.end.get().is_some() {
            return Ok(());
        }
        let reset = core.sender.new_epoch();
        core.reset(&reset);
        core.check_recovery();
        config.epoch = core.sender.epoch();
        drop(core);
        source.say(SourceMessage::Config(config));
        Ok(())
    }

    /// Stops sending pictures for a while, and says why. The platform's gate drops the events
    /// of the viewer until [`resume`](Self::resume), except the release of a press that it
    /// delivered, so [`SourceEvents::input`] is still called.
    ///
    /// A pause because the screen is locked ([`PauseReason::Locked`]) is what lets the viewer unlock
    /// it, in a session that takes an unlock. The platform pauses and resumes the session as the
    /// screen locks and unlocks, also while it unlocks the screen itself.
    pub fn pause(&self, reason: PauseReason) {
        let source = &self.source;
        let now = source.now();
        let mut core = lock(&source.core);
        core.paused = true;
        if reason == PauseReason::Locked {
            let step = core
                .unlocking()
                .map(|unlocking| unlocking.locked(true, now))
                .unwrap_or_default();
            source.carry_out(&mut core, step);
        }
        drop(core);
        source.say(SourceMessage::Pause(Pause { reason }));
    }

    /// Goes on sending pictures. A screen that was locked is not any more, so an unlock that waits
    /// for the face of the person ends. A session that is over gives nothing back.
    pub fn resume(&self) {
        let source = &self.source;
        let now = source.now();
        let mut core = lock(&source.core);
        core.paused = false;
        if source.end.get().is_none() {
            let step = core
                .unlocking()
                .map(|unlocking| unlocking.locked(false, now))
                .unwrap_or_default();
            source.carry_out(&mut core, step);
        }
        drop(core);
        source.say(SourceMessage::Resume);
    }

    /// An event of the viewer was dropped for this reason. The viewer is told, at most once a
    /// second, which reason dropped the most.
    pub fn dropped(&self, reason: Reason) {
        lock(&self.source.core).notices.dropped(reason);
    }

    /// What has been counted so far.
    pub fn stats(&self) -> SourceStats {
        let core = lock(&self.source.core);
        SourceStats {
            frames: core.sender.stats(),
            streams: core.frames.len(),
            sound: core.sound.sender.stats(),
            sound_streams: core.sound.packets.len(),
        }
    }

    /// The platform captures the sound of the screen and has encoded the first of it: says what
    /// the packets that follow are, and begins a run of sound. The platform asks for this when
    /// [`SourceEvents::hear`] said `true`. Nothing happens if the viewer has turned sound off
    /// meanwhile (the platform is told that with `hear(false)`), or the session is over.
    ///
    /// A session has one form of sound: the first call that is acted on fixes it, and a later run
    /// has to be in the same form. A platform whose capture changes, because the person chose
    /// another output on the Mac, say, converts the sound.
    ///
    /// # Errors
    ///
    /// `sound` is beyond a limit of the protocol, or differs from the form of an earlier run of
    /// the session. Nothing changed.
    pub fn begin_sound(&self, sound: Sound) -> Result<()> {
        sound::check(sound)?;
        let source = &self.source;
        let mut core = lock(&source.core);
        if source.end.get().is_some() || !core.sound.hear || core.quiet {
            return Ok(());
        }
        if core.sound.format.is_some_and(|format| format != sound) {
            return Err(Error::Config(
                "the sound is not in the form that the session began with".to_owned(),
            ));
        }
        core.sound.format = Some(sound);
        core.sound.running = true;
        core.sound.sender.begin_run();
        drop(core);
        source.say(SourceMessage::Sound(sound));
        Ok(())
    }

    /// The platform cannot send the sound that the viewer asked for, or can no more: says why and
    /// ends the run, if one goes. The viewer can ask again with another `Hear`, which the platform
    /// is told ([`SourceEvents::hear`]). Nothing is said to a viewer that does not want to hear.
    pub fn silent(&self, reason: SilentReason) {
        let source = &self.source;
        let mut core = lock(&source.core);
        if source.end.get().is_some() || !core.sound.hear {
            return;
        }
        core.sound.running = false;
        drop(core);
        source.say(SourceMessage::Silent(Silent { reason }));
    }

    /// The platform encoded a packet of sound: writes it on a stream of its own. Returns the
    /// number it got, or `None` if it was not sent because no run of sound is going (the viewer
    /// turned sound off, or has not been told yet what the packets are), or because the viewer
    /// takes no more streams just now. A packet that is not sent because there is no room still
    /// takes a number, so that the viewer sees that it is missing.
    ///
    /// # Errors
    ///
    /// The packet is larger than a viewer takes, or the session is over. Nothing was sent.
    pub fn sound(&self, packet: SoundPacket) -> std::result::Result<Option<u64>, NotSent> {
        let source = &self.source;
        let rtt = source.path().rtt;
        let now = source.now();
        let mut core = lock(&source.core);
        if source.end.get().is_some() {
            return Err(NotSent::Over);
        }
        if packet.bytes.len() > MAX_SOUND_PACKET_BYTES {
            return Err(NotSent::TooLarge);
        }
        if !core.sound.running {
            return Ok(None);
        }
        if core.sound.packets.len() >= MAX_SOUND_STREAMS {
            core.sound.sender.skip();
            return Ok(None);
        }
        let SoundSent { header, reset } = core.sound.sender.sent(now, packet.capture_time, rtt);
        core.sound.reset(&reset);
        let cancel = source.ended.child_token();
        core.sound.packets.insert(header.number, cancel.clone());
        drop(core);
        source
            .runtime
            .spawn(Arc::clone(source).send_packet(header, packet.bytes, cancel));
        Ok(Some(header.number))
    }

    /// The time since the session began. It is the clock of the capture times of the pictures
    /// ([`Picture::capture_time`], in microseconds) and of the arrival times that
    /// [`SourceEvents::input`] is given, so a platform stamps what it captures with this and
    /// not with a clock of its own.
    pub fn elapsed(&self) -> Duration {
        self.source.now()
    }

    /// Ends the session.
    pub fn end(&self) {
        self.source.end(End::Local);
    }

    /// Waits until the session is over, and says why.
    pub async fn ended(&self) -> End {
        self.source.over().await
    }

    /// The wait of [`ended`](Self::ended) as a future of its own, for whoever has to watch a
    /// session whose handle someone else owns. It does not hold the handle, so it does not keep
    /// the session going: the session still ends when the last copy of the handle is dropped,
    /// and the future is ready then.
    pub fn on_end(&self) -> impl Future<Output = End> + Send + 'static {
        let source = Arc::clone(&self.source);
        async move { source.over().await }
    }
}

impl Drop for SourceHandle {
    fn drop(&mut self) {
        self.source.end(End::Local);
    }
}

impl Source {
    /// Makes the session and starts what runs for as long as it does.
    pub(super) fn start(
        registry: Weak<Streaming>,
        whom: Whom,
        setup: SourceSetup,
        events: Box<dyn SourceEvents>,
    ) -> Result<(Arc<Self>, SourceHandle)> {
        let SourceSetup {
            mut config,
            control,
            private,
            takes,
            sound,
            may_hear,
            unlock,
            leave,
            params,
        } = setup;
        // The offer says `leave` only of a private session (protocol.md section 9): a screen that
        // is not kept private is not locked when the session ends, so there is nothing to leave.
        if leave && !private {
            return Err(Error::Config(
                "leaving the screen as it is is offered only in a private session".to_owned(),
            ));
        }
        let sender = Sender::new(whom.session, params.rules);
        config.epoch = sender.epoch();
        let first = SourceMessage::Config(config);
        first.validate().map_err(|error| Error::protocol(&error))?;
        if !takes.allows(&config) {
            return Err(beyond_what_the_viewer_takes());
        }
        let unlock = match unlock {
            UnlockSetup::Unsupported => Plan::Unsupported,
            UnlockSetup::NotOffered => Plan::NotOffered,
            UnlockSetup::Offered(store) => {
                // The offer says `unlock` only of a private session (protocol.md section 9).
                if !private {
                    return Err(Error::Config(
                        "an unlock is offered only in a private session".to_owned(),
                    ));
                }
                Plan::Offered(Box::new(Unlocking::new(
                    store,
                    whom.peer,
                    params.unlock_wait,
                    params.unlock_grace,
                )))
            }
        };
        let (outbox, queue) = mpsc::channel(OUTBOX);
        // The first message waits for the stream to be opened.
        let _ = outbox.try_send(first);
        let source = Arc::new(Self {
            registry,
            session: whom.session,
            peer: whom.peer,
            connection: whom.connection,
            runtime: tokio::runtime::Handle::current(),
            params,
            control,
            private,
            leave,
            left: AtomicBool::new(false),
            takes,
            sound,
            may_hear,
            began: Instant::now(),
            core: Mutex::new(Core {
                sender,
                sound: sound::Sounding::new(whom.session, params.sound),
                events,
                notices: Notices::new(),
                frames: HashMap::new(),
                last_report: Duration::ZERO,
                quiet: false,
                paused: false,
                recovery_told: false,
                unlock,
            }),
            outbox,
            viewer_control: AtomicBool::new(false),
            ended: CancellationToken::new(),
            end: OnceLock::new(),
        });
        tokio::spawn(Arc::clone(&source).supervise(whom.hold));
        tokio::spawn(Arc::clone(&source).write_control(queue));
        tokio::spawn(Arc::clone(&source).keep_time());
        let handle = SourceHandle {
            source: Arc::clone(&source),
        };
        Ok((source, handle))
    }

    /// The device at the other end.
    pub(super) fn peer(&self) -> DeviceId {
        self.peer
    }

    /// Waits until the session is over, and says why.
    async fn over(&self) -> End {
        self.ended.cancelled().await;
        self.end.get().copied().unwrap_or(End::Internal)
    }

    /// Whether `peer` on `connection` is the other end of this session.
    pub(super) fn belongs_to(&self, peer: &PairedDevice, connection: &Connection) -> bool {
        self.peer == peer.id && self.connection.stable_id() == connection.stable_id()
    }

    fn now(&self) -> Duration {
        self.began.elapsed()
    }

    /// What the connection says about the path now.
    fn path(&self) -> Path {
        let paths = self.connection.paths();
        paths
            .iter()
            .find(iroh::endpoint::Path::is_selected)
            .map_or(UNKNOWN_PATH, |path| {
                let stats = path.stats();
                Path {
                    window: stats.cwnd,
                    rtt: stats.rtt,
                    rate: 0,
                }
            })
    }

    /// Ends the session, once. The caller does not hold the session's lock.
    pub(super) fn end(&self, how: End) {
        self.end_leaving_if(how, false);
    }

    /// Ends the session because the viewer, whose person chose that the screen be left as it is,
    /// said so with a `Leave` and finished its control stream. The platform is told to leave the
    /// screen if the offer said `leave`, and told of an ordinary end if it did not. Does nothing
    /// if the session has ended already: the first end of a session is the only one, and a session
    /// that ended some other way was ended as that way ends it.
    fn end_leaving(&self) {
        self.end_leaving_if(End::Peer, self.leave);
    }

    /// Ends the session, once, and tells the platform to leave the screen as it is if `leave`.
    fn end_leaving_if(&self, how: End, leave: bool) {
        if self.end.set(how).is_err() {
            return;
        }
        // The platform is told before the end is announced to what waits for it. The platform of
        // an app closes the link of the session when it hears the announcement, and a choice to
        // leave the screen as it is that it is told after that is not taken: the screen is locked.
        let job = {
            let mut core = lock(&self.core);
            if leave {
                core.events.ended_leaving();
            } else {
                core.events.ended();
            }
            let now = self.now();
            core.unlocking()
                .and_then(|unlocking| unlocking.ended(now).job)
        };
        // The frame streams are reset by their tasks, which watch their tokens.
        self.ended.cancel();
        if let Some(registry) = self.registry.upgrade() {
            registry.forget(&self.session);
            // The viewer learns of this from the end of the control stream too, but it lets go of
            // its place only when it reads that, and an offer made straight after would meet it
            // taken. A `Stop` is answered when the place is free, and the next offer waits for
            // that. Any other end is the viewer's or the connection's doing, or the viewer is not
            // answering anyway. The platform ends a session from a thread of its own, so the
            // runtime that sends the `Stop` is the one the session keeps.
            if how == End::Local {
                registry.stop_later(
                    &self.runtime,
                    self.peer,
                    &self.connection,
                    self.session,
                    StopReason::Ended,
                );
            }
        }
        if let Some(job) = job {
            self.run(job);
        }
        tracing::debug!(peer = %self.peer.short(), reason = ?how, "a window streaming session ended");
    }

    /// Queues a message for the viewer.
    fn say(&self, message: SourceMessage) {
        // A full queue drops the message, see OUTBOX.
        let _ = self.outbox.try_send(message);
    }

    /// Ends the session when the connection ends.
    ///
    /// The hold keeps the connection in the pool until then.
    async fn supervise(self: Arc<Self>, hold: Option<Hold>) {
        tokio::select! {
            () = self.ended.cancelled() => {}
            _ = self.connection.closed() => self.end(End::Connection),
        }
        drop(hold);
    }

    /// Writes the source's control stream: the 17 bytes that start every stream of a session, and
    /// then the messages that wait in the queue, the first of which is the picture format.
    async fn write_control(self: Arc<Self>, queue: mpsc::Receiver<SourceMessage>) {
        match write_control(
            &self.connection,
            self.session,
            queue,
            &self.ended,
            &self.end,
            None,
        )
        .await
        {
            Ok(()) => {}
            Err(WriteEnd::Connection) => self.end(End::Connection),
            Err(WriteEnd::Peer) => self.end(End::Peer),
        }
    }

    /// Looks at the clock a few times a second, until the session is over.
    async fn keep_time(self: Arc<Self>) {
        let mut tick = tokio::time::interval(self.params.tick_every);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = self.ended.cancelled() => return,
                _ = tick.tick() => {}
            }
            if self.on_tick().is_break() {
                return;
            }
        }
    }

    /// Looks at the clock: gives up on the frames and the packets that are late, says what the
    /// viewer's events were dropped for, and ends the session of a viewer that has gone quiet.
    /// `Break` once the session is over.
    fn on_tick(self: &Arc<Self>) -> ControlFlow<()> {
        let now = self.now();
        let mut core = lock(&self.core);
        // The end can land after the task woke up and before it had the lock.
        if self.end.get().is_some() {
            return ControlFlow::Break(());
        }
        let reset = core.sender.poll(now);
        core.reset(&reset);
        core.check_recovery();
        let reset = core.sound.sender.poll(now);
        core.sound.reset(&reset);
        let notice = core.notices.poll(now);
        let silence = now.saturating_sub(core.last_report);
        core.quiet = silence > self.params.quiet_after;
        core.check_listening();
        let step = core
            .unlocking()
            .map(|unlocking| unlocking.tick(now))
            .unwrap_or_default();
        self.carry_out(&mut core, step);
        drop(core);
        if let Some(notice) = notice {
            self.say(SourceMessage::Notice(notice));
        }
        if silence > self.params.silent_after {
            self.end(End::Silent);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    /// Reads the viewer's control stream until the session ends.
    pub(super) async fn read_viewer_control(self: Arc<Self>, arrival: Arrival) {
        let Arrival { mut recv, slot, .. } = arrival;
        // The place of the stream in the count is let go before the session is ended, so that a
        // session that follows can open its control stream (protocol.md section 9).
        if self.viewer_control.swap(true, Ordering::SeqCst) {
            // A session has one control stream from each side.
            stop(&mut recv, close_code::PROTOCOL_VIOLATION);
            drop(slot);
            self.end(End::Violation);
            return;
        }
        let ended = read_control(recv, self.params.stall, &self.ended, |message| {
            self.viewer_said(message)
        })
        .await;
        drop(slot);
        match ended {
            ControlEnd::Over => {}
            // The viewer finished its stream, having said everything it had to say. If its last
            // word was that the person chose to leave the screen as it is, that is how it ends.
            ControlEnd::Finished if self.left.load(Ordering::SeqCst) => self.end_leaving(),
            ControlEnd::Finished | ControlEnd::Peer => self.end(End::Peer),
            ControlEnd::Violation => self.end(End::Violation),
            ControlEnd::Stalled => self.end(End::Stalled),
        }
    }

    /// Carries out a message of the viewer. `Break` if it broke a rule.
    fn viewer_said(self: &Arc<Self>, message: ViewerMessage) -> ControlFlow<()> {
        let now = self.now();
        let mut core = lock(&self.core);
        if self.end.get().is_some() {
            return ControlFlow::Continue(());
        }
        // `Leave` is the last thing that a viewer says.
        if self.left.load(Ordering::SeqCst) {
            return ControlFlow::Break(());
        }
        match message {
            ViewerMessage::Report(report) => {
                core.last_report = now;
                core.quiet = false;
                core.check_listening();
                core.events.report(report);
            }
            ViewerMessage::Want(want) => core.events.want(want),
            ViewerMessage::KeyFrame(KeyFrame { tried }) => {
                // A viewer can only name a frame that the source made.
                if core.sender.request_key_frame(now, tried).is_err() {
                    return ControlFlow::Break(());
                }
                core.check_recovery();
            }
            ViewerMessage::Input(batch) => {
                if self.control {
                    core.events.input(batch, now);
                } else {
                    for _ in &batch.events {
                        core.notices.dropped(Reason::ControlOff);
                    }
                }
            }
            ViewerMessage::Hear(Hear { on }) => match self.sound {
                // A viewer sends a `Hear` only to a source that advertised `stream.sound`, and
                // only if it advertised `stream.hear` itself (protocol.md section 9).
                SoundPlan::None => return ControlFlow::Break(()),
                // Sound goes with a whole display and with nothing smaller.
                SoundPlan::Window => {
                    if on {
                        self.say(SourceMessage::Silent(Silent {
                            reason: SilentReason::Window,
                        }));
                    }
                }
                SoundPlan::Display => {
                    if on && !self.may_hear.now() {
                        // The person here does not let this viewer hear, or no longer does. The
                        // platform is told to stop if it was listening, so this device plays its
                        // own sound again, and the viewer is told why.
                        core.heard(false);
                        self.say(SourceMessage::Silent(Silent {
                            reason: SilentReason::NotAllowed,
                        }));
                    } else {
                        core.heard(on);
                    }
                }
            },
            // A viewer sends the messages of an unlock only to a source that advertised
            // `unlock.target` and offered the session with `unlock` (protocol.md section 9).
            message @ (ViewerMessage::Unlock(_)
            | ViewerMessage::UnlockOpened(_)
            | ViewerMessage::UnlockCancelled(_)) => {
                return self.unlock_said(&mut core, now, message);
            }
            // The person at the viewer ended the session and chose that the screen stay as it is.
            // The offer told the viewer whether it may say so, and a viewer that may not breaks a
            // rule, as one does that sends a `Hear` to a source with no sound. It counts when the
            // viewer finishes its stream next.
            ViewerMessage::Leave => {
                if !self.leave {
                    return ControlFlow::Break(());
                }
                self.left.store(true, Ordering::SeqCst);
            }
            // The offer told the viewer whether it may ask. A message from a viewer that may not is
            // dropped, and is no violation.
            ViewerMessage::Lift(lift) => {
                if self.private {
                    core.events.lift(lift);
                    let step = core
                        .unlocking()
                        .map(|unlocking| unlocking.lifted(lift, now))
                        .unwrap_or_default();
                    self.carry_out(&mut core, step);
                }
            }
        }
        ControlFlow::Continue(())
    }

    /// Carries out a message of an unlock. `Break` if the session takes none and the viewer was not
    /// to send it.
    fn unlock_said(
        self: &Arc<Self>,
        core: &mut Core,
        now: Duration,
        message: ViewerMessage,
    ) -> ControlFlow<()> {
        let step = match &mut core.unlock {
            // The viewer is told by the offer whether it may unlock, and a device that never
            // advertised an unlock was not to be sent a message of one.
            Plan::Unsupported => return ControlFlow::Break(()),
            // A message can cross the offer on its way, and an attempt for which the offer did not
            // say `unlock` is not one that the viewer was to make: it is dropped.
            Plan::NotOffered => return ControlFlow::Continue(()),
            Plan::Offered(unlocking) => match message {
                ViewerMessage::Unlock(Ask { number }) => {
                    let in_place = core.events.private_in_place();
                    unlocking.asked(number, now, in_place)
                }
                ViewerMessage::UnlockOpened(opened) => unlocking.opened(opened, now),
                ViewerMessage::UnlockCancelled(Cancelled { reason }) => {
                    unlocking.cancelled(reason, now)
                }
                _ => return ControlFlow::Continue(()),
            },
        };
        self.carry_out(core, step);
        ControlFlow::Continue(())
    }

    /// Does what an unlock asked for, with the lock of the session held: says what is to be said,
    /// starts what waits for the disk, and hands the platform an attempt.
    fn carry_out(self: &Arc<Self>, core: &mut Core, step: Step) {
        let Step { say, job, hand } = step;
        if let Some(message) = say {
            self.say(message);
        }
        if let Some(job) = job {
            self.run(job);
        }
        if let Some(Handoff { attempt, waiting }) = hand {
            core.events.unlock(attempt);
            self.runtime.spawn(Arc::clone(self).await_unlock(waiting));
        }
    }

    /// Runs a call to the store on a thread that may wait.
    fn run(&self, job: Job) {
        drop(self.runtime.spawn_blocking(move || job.run()));
    }

    /// Waits for the platform to say what came of an attempt, writes it down, and then tells the
    /// viewer, unless the viewer gave up. The record is written first so that what the viewer is
    /// told is what the next question will find: a setup that was rejected is over already, unless
    /// the screen proved the password right while the record was written.
    async fn await_unlock(self: Arc<Self>, waiting: Waiting) {
        let (number, report) = waiting.report(self.params.unlock_work).await;
        let job = lock(&self.core)
            .unlocking()
            .and_then(|unlocking| unlocking.reported(number, report));
        if let Some(job) = job {
            // A store that fails does not keep the viewer from hearing.
            let _ = tokio::task::spawn_blocking(move || job.run()).await;
        }
        let now = self.now();
        let Step { say, job, .. } = lock(&self.core)
            .unlocking()
            .map(|unlocking| unlocking.closed(number, now))
            .unwrap_or_default();
        if let Some(job) = job {
            let _ = tokio::task::spawn_blocking(move || job.run()).await;
        }
        if let Some(message) = say {
            self.say(message);
        }
    }

    /// Writes one frame on a stream of its own, and settles what became of it.
    async fn send_frame(
        self: Arc<Self>,
        header: FrameHeader,
        picture: Vec<u8>,
        cancel: CancellationToken,
    ) {
        let outcome = self.write_frame(&header, picture, &cancel).await;
        self.settle(header.number, outcome);
    }

    async fn write_frame(
        &self,
        header: &FrameHeader,
        picture: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Outcome {
        let Ok(start) = header.encode() else {
            // The rules made the header, and they make valid ones.
            self.end(End::Internal);
            return Outcome::Gone;
        };
        self.write_stream(&start, picture, FRAME_PRIORITY, cancel)
            .await
    }

    /// Writes one packet of sound on a stream of its own, and settles what became of it.
    async fn send_packet(
        self: Arc<Self>,
        header: PacketHeader,
        packet: Vec<u8>,
        cancel: CancellationToken,
    ) {
        let outcome = self
            .write_stream(&header.encode(), packet, SOUND_PRIORITY, &cancel)
            .await;
        self.settle_packet(header.number, outcome);
    }

    /// Opens a stream, writes `start` and then `body` on it, finishes it, and waits until the
    /// viewer's transport has all of it. Resets the stream when `cancel` says so, at any point.
    async fn write_stream(
        &self,
        start: &[u8],
        body: Vec<u8>,
        priority: i32,
        cancel: &CancellationToken,
    ) -> Outcome {
        let opened = tokio::select! {
            () = cancel.cancelled() => return Outcome::Reset,
            opened = self.connection.open_uni() => opened,
        };
        let Ok(mut send) = opened else {
            return Outcome::Gone;
        };
        // Nothing to be done if the stream is gone already: the writes say so.
        let _ = send.set_priority(priority);
        let written = tokio::select! {
            () = cancel.cancelled() => None,
            written = write_all_of(&mut send, start, &body) => Some(written),
        };
        // The transport has what it needs, and the body is not kept for the wait.
        drop(body);
        match written {
            None => return reset_and_wait(&self.connection, send, self.params.tick_every).await,
            Some(Ok(())) => {}
            Some(Err(WriteError::Stopped(_))) => return Outcome::Stopped,
            Some(Err(_)) => return Outcome::Gone,
        }
        // A stream that the viewer stopped cannot be finished, and `stopped` says so below.
        let _ = send.finish();
        tokio::select! {
            () = cancel.cancelled() => {
                reset_and_wait(&self.connection, send, self.params.tick_every).await
            }
            stopped = send.stopped() => match stopped {
                Ok(None) => Outcome::Acked,
                Ok(Some(_)) => Outcome::Stopped,
                Err(_) => Outcome::Gone,
            },
        }
    }

    /// The stream of a frame has stopped counting at the viewer.
    fn settle(&self, number: u64, outcome: Outcome) {
        let mut core = lock(&self.core);
        core.frames.remove(&number);
        if self.end.get().is_some() {
            return;
        }
        match outcome {
            Outcome::Acked => {
                let held = core.sender.acked(number);
                if !held.is_empty() {
                    core.events.held(held);
                }
            }
            Outcome::Stopped => {
                let reset = core.sender.lost(number);
                core.reset(&reset);
                core.check_recovery();
            }
            Outcome::Reset | Outcome::Gone => {}
        }
    }

    /// The stream of a packet of sound has stopped counting at the viewer.
    fn settle_packet(&self, number: u64, outcome: Outcome) {
        let mut core = lock(&self.core);
        core.sound.packets.remove(&number);
        if self.end.get().is_some() {
            return;
        }
        match outcome {
            Outcome::Acked => core.sound.sender.acked(number),
            // A packet that the viewer stopped did not arrive, and the next one is not held back
            // for it: the viewer plays what it has.
            Outcome::Stopped => core.sound.sender.stopped(number),
            Outcome::Reset | Outcome::Gone => {}
        }
    }
}

/// Writes the start and then the rest of the stream.
async fn write_all_of(
    send: &mut SendStream,
    start: &[u8],
    body: &[u8],
) -> std::result::Result<(), WriteError> {
    send.write_all(start).await?;
    send.write_all(body).await
}

/// Resets the stream, and waits until the viewer's transport has acknowledged the reset, which is
/// when the stream stops counting there.
///
/// The transport lets go of a stream when it has the acknowledgement, but it wakes no one: what
/// `stopped` waits for is a stop by the viewer or the acknowledgement of a finished stream, so a
/// reset that is acknowledged is never reported to it. A stream that has been let go of has no
/// priority to ask for, so this asks every `every`.
async fn reset_and_wait(connection: &Connection, mut send: SendStream, every: Duration) -> Outcome {
    let _ = send.reset(code(close_code::CANCELLED));
    while send.priority().is_ok() && connection.close_reason().is_none() {
        tokio::time::sleep(every).await;
    }
    Outcome::Reset
}

#[cfg(test)]
mod tests {
    use baton_proto::{
        stream::Codec,
        unlock::{BoxKey, Challenge},
    };
    use baton_stream::{
        KIND_CONTROL, KIND_VIDEO,
        control::{Hear, SoundCodec, Thermal},
        header::{Flags, PREFIX_LEN, encode_prefix},
        input::{Event, Modifiers, PointerAction},
        unlock::{Ask, CancelReason, Cancelled, Opened},
    };
    use iroh::endpoint::{ReadError, RecvStream};

    use super::*;
    use crate::{
        testing::{Device, PATIENCE, QUIET, connect, next, nothing_arrives, stopped_with},
        uni::{Kind, Sinks},
        wire::{read_next, write_message},
    };

    /// What the platform of a test heard.
    #[derive(Debug, Clone, PartialEq)]
    enum Heard {
        Report(Report),
        Want(Want),
        Input(InputBatch),
        Held(Vec<LtrToken>),
        Lift(Lift),
        Recovery,
        Ended,
        /// The session ended and the screen is to be left as it is.
        Left,
    }

    #[derive(Clone, Default)]
    struct Log {
        heard: Arc<Mutex<Vec<Heard>>>,
        /// When each batch of input arrived, by the clock of the session.
        arrivals: Arc<Mutex<Vec<Duration>>>,
        /// The end of the session, which the platform looks at when it is told of the end. The
        /// rig puts it here once the session is made.
        end: Arc<OnceLock<CancellationToken>>,
        /// For each time that the platform was told of the end: whether the end had been
        /// announced to what waits for it by then.
        announced: Arc<Mutex<Vec<bool>>>,
    }

    impl Log {
        fn announced(&self) -> Vec<bool> {
            lock(&self.announced).clone()
        }

        fn all(&self) -> Vec<Heard> {
            lock(&self.heard).clone()
        }

        fn has(&self, heard: &Heard) -> bool {
            self.all().contains(heard)
        }

        fn count(&self, heard: &Heard) -> usize {
            self.all().iter().filter(|each| *each == heard).count()
        }

        fn arrivals(&self) -> Vec<Duration> {
            lock(&self.arrivals).clone()
        }
    }

    struct Recording(Log);

    impl Recording {
        fn note(&self, heard: Heard) {
            lock(&self.0.heard).push(heard);
        }

        /// Looks at whether the end of the session has been announced, as the platform of an app
        /// can when it is told of the end.
        fn look_at_the_end(&self) {
            let announced = self
                .0
                .end
                .get()
                .is_some_and(CancellationToken::is_cancelled);
            lock(&self.0.announced).push(announced);
        }
    }

    impl SourceEvents for Recording {
        fn report(&mut self, report: Report) {
            self.note(Heard::Report(report));
        }

        fn want(&mut self, want: Want) {
            self.note(Heard::Want(want));
        }

        fn input(&mut self, batch: InputBatch, arrived: Duration) {
            lock(&self.0.arrivals).push(arrived);
            self.note(Heard::Input(batch));
        }

        fn held(&mut self, tokens: Vec<LtrToken>) {
            self.note(Heard::Held(tokens));
        }

        fn lift(&mut self, lift: Lift) {
            self.note(Heard::Lift(lift));
        }

        fn recovery(&mut self) {
            self.note(Heard::Recovery);
        }

        fn ended_leaving(&mut self) {
            self.look_at_the_end();
            self.note(Heard::Left);
        }

        fn ended(&mut self) {
            self.look_at_the_end();
            self.note(Heard::Ended);
        }
    }

    fn session_id() -> SessionId {
        SessionId::from_bytes([7; SessionId::LEN])
    }

    /// The picture format of the tests. Its epoch is not the sender's: the session replaces it.
    fn config() -> Config {
        Config {
            epoch: 9,
            codec: Codec::H264,
            width: 1440,
            height: 900,
            fps: 60,
        }
    }

    /// Parameters with the times shortened, so that a test waits for them.
    fn quick() -> SourceParams {
        SourceParams {
            tick_every: Duration::from_millis(20),
            ..SourceParams::default()
        }
    }

    /// What a viewer that can take a great deal says.
    fn takes() -> Takes {
        Takes {
            max_width: 3840,
            max_height: 2160,
            max_fps: 120,
            codecs: vec![Codec::H264],
        }
    }

    fn setup(control: bool, params: SourceParams) -> SourceSetup {
        SourceSetup {
            config: config(),
            control,
            private: false,
            takes: takes(),
            sound: SoundPlan::None,
            may_hear: MayHear::always(),
            unlock: UnlockSetup::Unsupported,
            leave: false,
            params,
        }
    }

    /// A session that controls the window and keeps the screen private.
    fn private_setup() -> SourceSetup {
        SourceSetup {
            private: true,
            ..setup(true, quick())
        }
    }

    /// A session that keeps the screen private, and whose offer said that the screen can be left
    /// as it is when the session ends.
    fn leaving_setup() -> SourceSetup {
        SourceSetup {
            leave: true,
            ..private_setup()
        }
    }

    /// A platform that cannot leave a screen as it is: it is told of the end and nothing else.
    struct Plain(Log);

    impl SourceEvents for Plain {
        fn report(&mut self, _: Report) {}

        fn want(&mut self, _: Want) {}

        fn input(&mut self, _: InputBatch, _: Duration) {}

        fn held(&mut self, _: Vec<LtrToken>) {}

        fn recovery(&mut self) {}

        fn ended(&mut self) {
            lock(&(self.0).heard).push(Heard::Ended);
        }
    }

    /// A viewer that does what a test tells it, in front of the real source.
    struct Rig {
        /// The source's device, with the connection it dialed.
        source: Device,
        dialed: Connection,
        /// The viewer's device, with the connection it accepted.
        viewer: Device,
        accepted: Connection,
        /// What the source opens towards the viewer: frames, and its control stream.
        frames: mpsc::UnboundedReceiver<Arrival>,
        control: mpsc::UnboundedReceiver<Arrival>,
        streaming: Arc<Streaming>,
        handle: SourceHandle,
        log: Log,
    }

    impl Rig {
        async fn new() -> Self {
            Self::with(setup(true, quick()), 1).await
        }

        /// A rig whose source takes `control_streams` control streams at once.
        async fn with(setup: SourceSetup, control_streams: u32) -> Self {
            Self::starting(setup, control_streams)
                .await
                .expect("the session did not start")
        }

        /// The same, for a setup that the session may refuse.
        async fn starting(setup: SourceSetup, control_streams: u32) -> Result<Self> {
            let streaming = Streaming::new();
            let sinks = Sinks::default()
                .install(Kind::Control, control_streams, streaming.clone())
                .install(Kind::Video, 1, streaming.clone());
            let source = Device::new(sinks, PATIENCE).await;
            let (viewer, mut receivers) =
                Device::taking(&[(Kind::Video, 12), (Kind::Control, 1)]).await;
            let control = receivers.remove(1);
            let frames = receivers.remove(0);
            let (dialed, accepted) = connect(&source, &viewer).await;
            let log = Log::default();
            let paired = source.trust.get(&viewer.id).unwrap();
            let handle = streaming.source(
                &paired,
                dialed.clone(),
                None,
                session_id(),
                setup,
                Recording(log.clone()),
            )?;
            let _ = log.end.set(handle.source.ended.clone());
            Ok(Self {
                source,
                dialed,
                viewer,
                accepted,
                frames,
                control,
                streaming,
                handle,
                log,
            })
        }

        /// The source's control stream, as the viewer reads it.
        async fn source_control(&mut self) -> RecvStream {
            let arrival = next(&mut self.control).await;
            assert!(matches!(arrival.kind, Kind::Control));
            assert_eq!(arrival.id, *session_id().as_bytes());
            arrival.recv
        }

        /// Opens the viewer's control stream, as a viewer does once it has taken the source's.
        async fn open_viewer_control(&self) -> SendStream {
            let mut send = self.accepted.open_uni().await.unwrap();
            send.write_all(&encode_prefix(KIND_CONTROL, session_id()))
                .await
                .unwrap();
            send
        }

        fn picture(size: usize) -> Picture {
            Picture {
                bytes: vec![0x5a; size],
                referenced: true,
                ltr: None,
                capture_time: 1000,
            }
        }

        /// Asks for the next picture and sends one of `size` bytes. Says what kind of frame it
        /// was and what number it got.
        fn frame(&self, size: usize) -> (FrameKind, u64) {
            let kind = self.handle.begin().expect("no picture was wanted");
            let number = self
                .handle
                .sent(Self::picture(size))
                .unwrap()
                .expect("the frame was dropped");
            (kind, number)
        }

        /// Whether the source has read a `Leave` of the viewer.
        fn left(&self) -> bool {
            self.handle.source.left.load(Ordering::SeqCst)
        }

        /// Whether a picture is wanted, asked without leaving the question open.
        fn picture_wanted(&self) -> bool {
            let wanted = self.handle.begin();
            if wanted.is_some() {
                self.handle.cancel();
            }
            wanted.is_some()
        }

        /// How many times the platform has been told that the next frame begins a chain.
        fn recoveries(&self) -> usize {
            self.log.count(&Heard::Recovery)
        }

        /// Waits until every frame stream has stopped counting.
        async fn settled(&self) {
            self.until("the frame streams to settle", |rig| {
                rig.handle.stats().streams == 0
            })
            .await;
        }

        /// Waits until the session is over, and says why.
        async fn ended(&self) -> End {
            tokio::time::timeout(PATIENCE, self.handle.ended())
                .await
                .expect("the session did not end")
        }

        /// Waits until `condition` holds.
        async fn until(&self, what: &str, condition: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + PATIENCE;
            while !condition(self) {
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        /// A rig whose source sends the sound of a display, for a viewer that has asked to hear
        /// it, and whose platform has begun a run. The runtime of a test does not run what the
        /// calls spawn before the test waits for something, so no stream of a packet is written or
        /// let go of between the calls that follow.
        async fn hearing(params: SourceParams) -> Self {
            let setup = SourceSetup {
                sound: SoundPlan::Display,
                ..setup(true, params)
            };
            let rig = Self::with(setup, 1).await;
            let said = rig
                .handle
                .source
                .viewer_said(ViewerMessage::Hear(Hear { on: true }));
            assert!(said.is_continue());
            rig.handle.begin_sound(aac()).unwrap();
            rig
        }

        /// The token that resets the stream of a packet of sound, if the session still counts it.
        fn packet_token(&self, number: u64) -> CancellationToken {
            lock(&self.handle.source.core).sound.packets[&number].clone()
        }
    }

    /// The form of sound that the first version sends.
    fn aac() -> Sound {
        Sound {
            codec: SoundCodec::Aac,
            rate: 48_000,
            channels: 2,
            samples: 1024,
        }
    }

    fn packet(byte: u8) -> SoundPacket {
        SoundPacket {
            bytes: vec![byte; 300],
            capture_time: 0,
        }
    }

    /// The next frame, as the viewer reads it: the header and the whole picture.
    async fn read_frame(frames: &mut mpsc::UnboundedReceiver<Arrival>) -> (FrameHeader, Vec<u8>) {
        let arrival = next(frames).await;
        assert!(matches!(arrival.kind, Kind::Video));
        let mut recv = arrival.recv;
        let mut head = encode_prefix(KIND_VIDEO, session_id()).to_vec();
        head.resize(FrameHeader::LEN, 0);
        recv.read_exact(head.split_at_mut(PREFIX_LEN).1)
            .await
            .unwrap();
        let (header, _) = FrameHeader::parse(&head).unwrap();
        let picture = recv.read_to_end(16 * 1024 * 1024).await.unwrap();
        (header, picture)
    }

    async fn read_source_message(recv: &mut RecvStream) -> Option<SourceMessage> {
        tokio::time::timeout(PATIENCE, read_next::<SourceMessage>(recv, PATIENCE))
            .await
            .expect("the source wrote no message")
            .expect("the source wrote a message that cannot be read")
    }

    /// The code that a stream was reset with, once its data has been read.
    async fn reset_with(recv: &mut RecvStream) -> u64 {
        loop {
            let mut buffer = [0_u8; 4096];
            match recv.read(&mut buffer).await {
                Err(ReadError::Reset(code)) => break code.into_inner(),
                Ok(Some(_)) => {}
                other => panic!("expected the stream to be reset, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn the_control_stream_is_opened_at_once_and_says_what_the_picture_is() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        // The epoch in the setup is not the sender's, and is replaced.
        assert_eq!(
            read_source_message(&mut control).await,
            Some(SourceMessage::Config(Config {
                epoch: 0,
                ..config()
            }))
        );
    }

    #[tokio::test]
    async fn a_key_frame_arrives_whole_and_the_next_picture_is_a_delta() {
        let mut rig = Rig::new().await;
        assert_eq!(rig.frame(300_007), (FrameKind::Key, 0));
        let (header, picture) = read_frame(&mut rig.frames).await;
        assert_eq!(header.session, session_id());
        assert_eq!((header.number, header.depends_on, header.epoch), (0, 0, 0));
        assert_eq!(header.capture_time, 1000);
        assert_eq!(header.flags, Flags::KEY | Flags::REFERENCED);
        assert_eq!(picture.len(), 300_007);
        assert!(picture.iter().all(|byte| *byte == 0x5a));
        // Once the viewer has all of it, the stream stops counting and the next picture goes.
        rig.settled().await;
        assert_eq!(rig.frame(5000), (FrameKind::Delta, 1));
        let (header, picture) = read_frame(&mut rig.frames).await;
        assert_eq!((header.number, header.depends_on), (1, 0));
        assert_eq!(header.flags, Flags::REFERENCED);
        assert_eq!(picture.len(), 5000);
    }

    #[tokio::test]
    async fn a_frame_that_the_viewer_stops_breaks_the_chain() {
        let mut rig = Rig::new().await;
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        // The second frame is large enough to still be on its way when the viewer stops it.
        assert_eq!(rig.frame(6 * 1024 * 1024), (FrameKind::Delta, 1));
        let before = rig.recoveries();
        let mut stopped = next(&mut rig.frames).await.recv;
        stopped.stop(code(close_code::CANCELLED)).unwrap();
        rig.until("the platform to be told", |rig| rig.recoveries() > before)
            .await;
        assert!(rig.handle.recovery_pending());
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
        assert_eq!(rig.handle.stats().frames.abandoned, 1);
    }

    #[tokio::test]
    async fn a_frame_that_the_viewer_stops_breaks_the_chain_whatever_the_clock_says() {
        // The clock gives up on nothing for a minute, so only the stop can break the chain.
        let rules = Rules {
            grace: Duration::from_secs(60),
            longest: Duration::from_secs(60),
            ..Rules::default()
        };
        let mut rig = Rig::with(setup(true, SourceParams { rules, ..quick() }), 1).await;
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        // A picture of the largest size does not fit in the window of its stream, so the write
        // is still going when the viewer stops the stream.
        assert_eq!(rig.frame(MAX_PICTURE_BYTES), (FrameKind::Delta, 1));
        let before = rig.recoveries();
        let mut stopped = next(&mut rig.frames).await.recv;
        stopped.stop(code(close_code::CANCELLED)).unwrap();
        rig.until("the platform to be told", |rig| rig.recoveries() > before)
            .await;
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
        assert_eq!(rig.handle.stats().frames.abandoned, 1);
    }

    #[tokio::test]
    async fn the_clock_resets_a_frame_that_is_late() {
        // Nothing is acknowledged within a millisecond, so a frame that does not begin a chain is
        // late by the next look at the clock.
        let rules = Rules {
            grace: Duration::ZERO,
            round_trips: 0,
            longest: Duration::from_millis(1),
            ..Rules::default()
        };
        let mut rig = Rig::with(setup(true, SourceParams { rules, ..quick() }), 1).await;
        // A frame that begins a chain is never given up on for being late, and nothing goes
        // behind it until the viewer has all of it. So this one is read to the end.
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        // The viewer takes the stream of the next frame and does not read it, so the frame is
        // still on its way when the clock looks.
        let before = rig.recoveries();
        assert_eq!(rig.frame(8 * 1024 * 1024), (FrameKind::Delta, 1));
        let mut late = next(&mut rig.frames).await.recv;
        rig.until("the frame to be given up on", |rig| {
            rig.handle.stats().frames.abandoned == 1
        })
        .await;
        assert_eq!(reset_with(&mut late).await, 499);
        // The frame was referenced, so the next one begins a chain, and the platform is told.
        rig.until("the platform to be told", |rig| rig.recoveries() > before)
            .await;
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
        rig.handle.cancel();
        // The stream stops counting when the viewer's transport has the reset.
        rig.settled().await;
        // The session goes on.
        assert!(!rig.log.has(&Heard::Ended));
    }

    #[tokio::test]
    async fn the_platform_can_read_the_clock_that_the_pictures_are_stamped_with() {
        let rig = Rig::new().await;
        let first = rig.handle.elapsed();
        tokio::time::sleep(Duration::from_millis(30)).await;
        let second = rig.handle.elapsed();

        // It runs, and it counts from the start of the session.
        assert!(
            second >= first + Duration::from_millis(30),
            "{first:?} {second:?}"
        );
        assert!(second < Duration::from_secs(30), "{second:?}");
    }

    #[tokio::test]
    async fn a_session_that_has_sent_nothing_yet_says_that_a_first_picture_is_wanted() {
        let rig = Rig::new().await;
        // The first frame begins a chain, so a picture is wanted even if the screen is still.
        rig.until("the platform to be told", |rig| rig.recoveries() == 1)
            .await;
        assert!(rig.handle.recovery_pending());
        // Once that frame is on its way the platform is not told again.
        rig.frame(1000);
        assert!(!rig.handle.recovery_pending());
        tokio::time::sleep(3 * quick().tick_every).await;
        assert_eq!(rig.recoveries(), 1);
    }

    #[tokio::test]
    async fn no_frame_is_started_while_twelve_streams_are_open_or_being_reset() {
        let rig = Rig::new().await;
        let source = rig.streaming.sourcing(&session_id()).unwrap();
        for number in 100..112 {
            lock(&source.core)
                .frames
                .insert(number, CancellationToken::new());
        }
        assert_eq!(rig.handle.begin(), None);
        assert_eq!(rig.handle.stats().streams, 12);
        lock(&source.core).frames.remove(&100);
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
    }

    #[tokio::test]
    async fn a_picture_beyond_what_a_viewer_takes_is_not_sent() {
        let mut rig = Rig::new().await;
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
        assert_eq!(
            rig.handle.sent(Rig::picture(MAX_PICTURE_BYTES + 1)),
            Err(NotSent::TooLarge)
        );
        // Nothing went, and the next frame is a key frame again.
        assert_eq!(rig.handle.stats().frames.sent, 0);
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
        rig.handle.cancel();
        nothing_arrives(&mut rig.frames).await;
    }

    #[tokio::test]
    async fn a_long_term_reference_that_the_viewer_holds_is_reported_to_the_platform() {
        let mut rig = Rig::new().await;
        rig.handle.begin().unwrap();
        let picture = Picture {
            ltr: Some(LtrToken(7)),
            ..Rig::picture(1000)
        };
        assert_eq!(rig.handle.sent(picture), Ok(Some(0)));
        read_frame(&mut rig.frames).await;
        rig.until("the frame to be held", |rig| {
            rig.log.has(&Heard::Held(vec![LtrToken(7)]))
        })
        .await;
    }

    #[tokio::test]
    async fn a_viewer_that_goes_quiet_stops_the_encoder_and_then_the_session() {
        let params = SourceParams {
            quiet_after: Duration::from_millis(150),
            silent_after: Duration::from_millis(900),
            ..quick()
        };
        let mut rig = Rig::with(setup(true, params), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        assert!(rig.picture_wanted());
        rig.until("the source to go quiet", |rig| !rig.picture_wanted())
            .await;
        // A report is heard, and the source encodes again.
        let mut send = rig.open_viewer_control().await;
        let report = Report {
            shown: None,
            decode_us: 0,
            dropped: 0,
            thermal: Thermal::Nominal,
        };
        write_message(&mut send, &ViewerMessage::Report(report))
            .await
            .unwrap();
        rig.until("the source to encode again", Rig::picture_wanted)
            .await;
        // Then nothing is heard, and the session ends.
        assert_eq!(rig.ended().await, End::Silent);
        assert!(rig.log.has(&Heard::Ended));
        assert!(rig.streaming.is_empty());
        // The control stream is finished and not reset.
        assert_eq!(read_source_message(&mut control).await, None);
    }

    #[tokio::test]
    async fn reports_keep_the_session_going() {
        let params = SourceParams {
            quiet_after: Duration::from_millis(150),
            silent_after: Duration::from_millis(400),
            ..quick()
        };
        let mut rig = Rig::with(setup(true, params), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        let report = Report {
            shown: None,
            decode_us: 0,
            dropped: 0,
            thermal: Thermal::Nominal,
        };
        // Twice the time that ends a silent session, with a report every 50 ms.
        for _ in 0..16 {
            write_message(&mut send, &ViewerMessage::Report(report))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!rig.log.has(&Heard::Ended));
        assert!(rig.picture_wanted());
    }

    #[tokio::test]
    async fn what_the_viewer_says_reaches_the_platform_in_order() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        // The platform is told that a first picture is wanted when the session first looks at the
        // clock, not when it starts. A viewer on a fast path can say its three things before
        // that, so the test waits for it and the order is the same on every machine.
        rig.until("the platform to be told", |rig| rig.recoveries() == 1)
            .await;
        let mut send = rig.open_viewer_control().await;
        let report = Report {
            shown: Some(3),
            decode_us: 2500,
            dropped: 1,
            thermal: Thermal::Fair,
        };
        let want = Want {
            width: 2796,
            height: 1290,
            fps: 120,
        };
        let batch = InputBatch {
            events: vec![Event::Pointer {
                frame: 0,
                x: 0.5,
                y: 0.25,
                modifiers: Modifiers::NONE,
                action: PointerAction::Move,
            }],
        };
        let source = Arc::clone(&rig.handle.source);
        let before = source.now();
        for message in [
            ViewerMessage::Report(report),
            ViewerMessage::Want(want),
            ViewerMessage::Input(batch.clone()),
        ] {
            write_message(&mut send, &message).await.unwrap();
        }
        rig.until("the three messages", |rig| {
            rig.log.all().contains(&Heard::Input(batch.clone()))
        })
        .await;
        // The batch comes with the time it arrived, on the clock of the session.
        let after = source.now();
        let [arrived] = rig.log.arrivals()[..] else {
            panic!("the batch should have arrived once");
        };
        assert!(before <= arrived && arrived <= after);
        // First what the session told the platform itself, then the three things in the order in
        // which the viewer said them, and nothing else.
        assert_eq!(
            rig.log.all(),
            [
                Heard::Recovery,
                Heard::Report(report),
                Heard::Want(want),
                Heard::Input(batch)
            ]
        );
    }

    #[tokio::test]
    async fn input_in_a_session_without_control_is_dropped_and_the_viewer_is_told() {
        let mut rig = Rig::with(setup(false, quick()), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        let event = Event::Pointer {
            frame: 0,
            x: 0.5,
            y: 0.5,
            modifiers: Modifiers::NONE,
            action: PointerAction::Move,
        };
        let batch = InputBatch {
            events: vec![event.clone(), event],
        };
        write_message(&mut send, &ViewerMessage::Input(batch))
            .await
            .unwrap();
        assert_eq!(
            read_source_message(&mut control).await,
            Some(SourceMessage::Notice(baton_stream::gate::Notice {
                reason: Reason::ControlOff,
                count: 2,
            }))
        );
        assert!(
            !rig.log
                .all()
                .iter()
                .any(|heard| matches!(heard, Heard::Input(_)))
        );
    }

    #[tokio::test]
    async fn what_the_viewer_lifts_in_a_private_session_reaches_the_platform_as_it_was_said() {
        let mut rig = Rig::with(private_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        // Each message says both things, and the same thing said twice is heard twice: the
        // platform is the one that sees it has done it. Putting each back is a message like any.
        let said = [
            Lift {
                screen: true,
                input: false,
            },
            Lift {
                screen: true,
                input: false,
            },
            Lift {
                screen: true,
                input: true,
            },
            Lift {
                screen: false,
                input: true,
            },
            Lift {
                screen: false,
                input: false,
            },
        ];
        for lift in said {
            write_message(&mut send, &ViewerMessage::Lift(lift))
                .await
                .unwrap();
        }
        rig.until("the platform to hear the last", |rig| {
            rig.log.count(&Heard::Lift(Lift::default())) == 1
        })
        .await;
        let heard: Vec<_> = rig
            .log
            .all()
            .into_iter()
            .filter(|heard| matches!(heard, Heard::Lift(_)))
            .collect();
        assert_eq!(heard, said.map(Heard::Lift));
        // None of it is input, and the session goes on.
        assert!(
            rig.log
                .all()
                .iter()
                .all(|heard| !matches!(heard, Heard::Input(_)))
        );
        assert!(!rig.log.has(&Heard::Ended));
    }

    #[tokio::test]
    async fn a_lift_in_a_session_that_is_not_private_is_not_heard_and_is_no_violation() {
        // The offer told the viewer that it may not, so this is a viewer that ignores it. The
        // screen of such a source is as the person left it, and nothing is lifted.
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        let want = Want {
            width: 2796,
            height: 1290,
            fps: 120,
        };
        for message in [
            ViewerMessage::Lift(Lift {
                screen: true,
                input: true,
            }),
            ViewerMessage::Want(want),
        ] {
            write_message(&mut send, &message).await.unwrap();
        }
        // The message after it is heard, so the lift was read and left alone.
        rig.until("the platform to hear the want", |rig| {
            rig.log.has(&Heard::Want(want))
        })
        .await;
        assert!(
            rig.log
                .all()
                .iter()
                .all(|heard| !matches!(heard, Heard::Lift(_)))
        );
        assert!(!rig.log.has(&Heard::Ended));
        assert!(rig.picture_wanted());
    }

    #[tokio::test]
    async fn the_platform_can_say_why_it_dropped_the_events_of_the_viewer() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        rig.handle.dropped(Reason::Locked);
        rig.handle.dropped(Reason::Locked);
        rig.handle.dropped(Reason::Covered);
        assert_eq!(
            read_source_message(&mut control).await,
            Some(SourceMessage::Notice(baton_stream::gate::Notice {
                reason: Reason::Locked,
                count: 2,
            }))
        );
    }

    #[tokio::test]
    async fn a_request_for_a_key_frame_is_answered_with_one() {
        let mut rig = Rig::new().await;
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        let before = rig.recoveries();
        let mut send = rig.open_viewer_control().await;
        let tried = KeyFrame { tried: 1 };
        write_message(&mut send, &ViewerMessage::KeyFrame(tried))
            .await
            .unwrap();
        // Key frames made on request keep their distance, so this waits for its time.
        rig.until("a key frame to be wanted", |rig| {
            rig.handle.recovery_pending()
        })
        .await;
        assert!(rig.recoveries() > before);
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
    }

    #[tokio::test]
    async fn a_key_frame_resets_the_frames_on_their_way() {
        // The clock gives up on nothing for a minute, so only the key frame can reset the delta.
        let rules = Rules {
            grace: Duration::from_secs(60),
            longest: Duration::from_secs(60),
            ..Rules::default()
        };
        let mut rig = Rig::with(setup(true, SourceParams { rules, ..quick() }), 1).await;
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        assert_eq!(rig.frame(8 * 1024 * 1024), (FrameKind::Delta, 1));
        let mut on_its_way = next(&mut rig.frames).await.recv;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::KeyFrame(KeyFrame { tried: 0 }))
            .await
            .unwrap();
        rig.until("a key frame to be wanted", |rig| {
            rig.handle.recovery_pending()
        })
        .await;
        assert_eq!(rig.frame(1000).0, FrameKind::Key);
        assert_eq!(reset_with(&mut on_its_way).await, 499);
    }

    #[tokio::test]
    async fn a_request_that_names_a_frame_never_made_is_a_violation() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        let tried = KeyFrame { tried: 99 };
        write_message(&mut send, &ViewerMessage::KeyFrame(tried))
            .await
            .unwrap();
        assert_eq!(rig.ended().await, End::Violation);
        assert_eq!(stopped_with(&send).await, 400);
        // The source says so by resetting its own control stream.
        assert_eq!(reset_with(&mut control).await, 400);
        assert!(rig.log.has(&Heard::Ended));
    }

    #[tokio::test]
    async fn a_hear_is_a_violation_while_the_source_does_not_send_sound() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::Hear(Hear { on: true }))
            .await
            .unwrap();
        assert_eq!(rig.ended().await, End::Violation);
        assert_eq!(stopped_with(&send).await, 400);
        // The source says so by resetting its own control stream.
        assert_eq!(reset_with(&mut control).await, 400);
        assert!(rig.log.has(&Heard::Ended));
    }

    #[tokio::test]
    async fn the_messages_of_an_unlock_are_a_violation_while_the_source_takes_no_unlock() {
        let opened = Opened {
            challenge: Challenge::from_bytes([1; Challenge::LEN]),
            key: BoxKey::from_bytes([2; BoxKey::LEN]),
        };
        for message in [
            ViewerMessage::Unlock(Ask { number: 1 }),
            ViewerMessage::UnlockOpened(opened),
            ViewerMessage::UnlockCancelled(Cancelled {
                reason: CancelReason::Declined,
            }),
        ] {
            let mut rig = Rig::new().await;
            let mut control = rig.source_control().await;
            read_source_message(&mut control).await;
            let mut send = rig.open_viewer_control().await;
            write_message(&mut send, &message).await.unwrap();
            assert_eq!(rig.ended().await, End::Violation, "{message:?}");
            assert_eq!(stopped_with(&send).await, 400, "{message:?}");
            // The source says so by resetting its own control stream.
            assert_eq!(reset_with(&mut control).await, 400, "{message:?}");
            assert!(rig.log.has(&Heard::Ended));
        }
    }

    #[tokio::test]
    async fn a_message_that_cannot_be_read_ends_the_session() {
        let rig = Rig::new().await;
        let mut send = rig.open_viewer_control().await;
        // A length of 5 and five bytes that are not a message.
        send.write_all(&[0, 0, 0, 5, 0xff, 0xff, 0xff, 0xff, 0xff])
            .await
            .unwrap();
        assert_eq!(rig.ended().await, End::Violation);
        assert_eq!(stopped_with(&send).await, 400);
    }

    #[tokio::test]
    async fn a_message_of_the_viewer_that_stalls_ends_the_session_and_blames_no_one() {
        let mut params = quick();
        params.stall = Duration::from_millis(200);
        let mut rig = Rig::with(setup(true, params), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        // The length of a message of 20 bytes, and then nothing.
        send.write_all(&[0, 0, 0, 20]).await.unwrap();
        assert_eq!(stopped_with(&send).await, 408);
        assert_eq!(rig.ended().await, End::Stalled);
        // The viewer broke no rule, so the source finishes its stream and does not reset it.
        assert_eq!(read_source_message(&mut control).await, None);
        assert!(rig.log.has(&Heard::Ended));
    }

    #[tokio::test]
    async fn a_second_control_stream_of_the_viewer_is_a_violation() {
        let rig = Rig::with(setup(true, quick()), 2).await;
        let first = rig.open_viewer_control().await;
        let second = rig.open_viewer_control().await;
        assert_eq!(rig.ended().await, End::Violation);
        assert_eq!(stopped_with(&second).await, 400);
        drop(first);
    }

    #[tokio::test]
    async fn a_viewer_that_finishes_its_control_stream_has_ended_the_session() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        send.finish().unwrap();
        assert_eq!(rig.ended().await, End::Peer);
        assert!(rig.log.has(&Heard::Ended));
        assert_eq!(read_source_message(&mut control).await, None);
    }

    #[tokio::test]
    async fn the_end_of_the_connection_is_the_end_of_the_session() {
        let rig = Rig::new().await;
        rig.accepted.close(code(close_code::NORMAL), b"done");
        assert_eq!(rig.ended().await, End::Connection);
        assert!(rig.streaming.is_empty());
    }

    #[tokio::test]
    async fn ending_the_session_resets_the_frames_in_flight_and_finishes_the_control_stream() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        // A frame that is still on its way when the session ends.
        rig.frame(8 * 1024 * 1024);
        let mut frame = next(&mut rig.frames).await.recv;
        rig.handle.end();
        assert_eq!(rig.ended().await, End::Local);
        assert_eq!(reset_with(&mut frame).await, 499);
        assert_eq!(read_source_message(&mut control).await, None);
        rig.settled().await;
        assert_eq!(rig.log.count(&Heard::Ended), 1);
        assert!(rig.streaming.is_empty());
        // Nothing more is wanted or sent.
        assert_eq!(rig.handle.begin(), None);
        assert_eq!(rig.handle.sent(Rig::picture(10)), Err(NotSent::Over));
    }

    #[tokio::test]
    async fn a_session_that_ends_before_its_control_stream_is_opened_still_opens_and_ends_it() {
        let mut rig = Rig::new().await;
        // The platform lets go of the session at once, before the stream has been opened.
        rig.handle.end();
        // The viewer, which waits for the stream, gets it, and its end.
        let mut control = rig.source_control().await;
        while read_source_message(&mut control).await.is_some() {}
        assert_eq!(rig.ended().await, End::Local);
    }

    #[tokio::test]
    async fn dropping_the_handle_ends_the_session() {
        let rig = Rig::new().await;
        let Rig {
            handle,
            log,
            streaming,
            ..
        } = rig;
        drop(handle);
        assert!(log.has(&Heard::Ended));
        assert!(streaming.is_empty());
    }

    /// How often the platform was told of an ordinary end, and of an end that leaves the screen.
    fn ends_heard(log: &Log) -> (usize, usize) {
        (log.count(&Heard::Ended), log.count(&Heard::Left))
    }

    #[tokio::test]
    async fn an_end_that_leaves_the_screen_as_it_is_tells_the_platform_so_and_nothing_else() {
        let mut rig = Rig::with(leaving_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        rig.handle.source.end_leaving();
        assert_eq!(rig.ended().await, End::Peer);
        // Once, and not as an ordinary end.
        assert_eq!(ends_heard(&rig.log), (0, 1));
        // The viewer is told as it is of any end: the control stream is finished, not reset.
        assert_eq!(read_source_message(&mut control).await, None);
        assert!(rig.streaming.is_empty());
    }

    #[tokio::test]
    async fn every_other_end_of_a_session_that_may_be_left_has_the_platform_lock_the_screen() {
        for what in ["the platform ends it", "the connection ends"] {
            let rig = Rig::with(leaving_setup(), 1).await;
            match what {
                "the platform ends it" => rig.handle.end(),
                _ => rig.accepted.close(code(close_code::NORMAL), b"done"),
            }
            rig.ended().await;
            assert_eq!(ends_heard(&rig.log), (1, 0), "{what}");
        }
    }

    #[tokio::test]
    async fn a_viewer_that_finishes_its_control_stream_has_the_platform_lock_a_screen_that_may_be_left()
     {
        let mut rig = Rig::with(leaving_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        send.finish().unwrap();
        assert_eq!(rig.ended().await, End::Peer);
        assert_eq!(ends_heard(&rig.log), (1, 0));
    }

    #[tokio::test]
    async fn a_viewer_that_says_leave_and_finishes_its_stream_has_the_platform_leave_the_screen() {
        let mut rig = Rig::with(leaving_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::Leave)
            .await
            .unwrap();
        // The choice counts with the end of the stream that follows, and the session goes on until.
        tokio::time::sleep(QUIET).await;
        assert!(rig.handle.source.end.get().is_none());
        assert_eq!(ends_heard(&rig.log), (0, 0));
        send.finish().unwrap();
        assert_eq!(rig.ended().await, End::Peer);
        // Once, and not as an ordinary end. The viewer is told as it is of any end.
        assert_eq!(ends_heard(&rig.log), (0, 1));
        assert_eq!(read_source_message(&mut control).await, None);
        assert!(rig.streaming.is_empty());
    }

    #[tokio::test]
    async fn the_platform_is_told_of_the_end_before_the_end_is_announced_to_what_waits_for_it() {
        // The platform of an app closes the link of the session when the announcement wakes it,
        // which a runtime with more than one thread does at once, and a choice to leave the
        // screen as it is that the platform is told after that is not taken. So it is told first,
        // whichever way the session ends.
        for (what, leaving) in [
            ("the platform ends it", false),
            ("the viewer chooses to leave", true),
        ] {
            let mut rig = Rig::with(leaving_setup(), 1).await;
            let mut control = rig.source_control().await;
            read_source_message(&mut control).await;
            if leaving {
                let mut send = rig.open_viewer_control().await;
                write_message(&mut send, &ViewerMessage::Leave)
                    .await
                    .unwrap();
                send.finish().unwrap();
            } else {
                rig.handle.end();
            }
            rig.ended().await;
            assert_eq!(rig.log.announced(), [false], "{what}");
        }
    }

    #[tokio::test]
    async fn a_viewer_that_says_leave_and_then_resets_its_stream_has_the_platform_lock_the_screen()
    {
        // The person chose, and the end that followed was not the viewer finishing its stream: it
        // may have crashed, or been cut off. Everything that is not the end that was asked for locks.
        let mut rig = Rig::with(leaving_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::Leave)
            .await
            .unwrap();
        // A stream that is reset before the Mac has read what was written is reset before it
        // began, as far as the Mac can tell, so the reset waits for the Leave to be read.
        rig.until("the source to read the Leave", Rig::left).await;
        send.reset(code(close_code::CANCELLED)).unwrap();
        assert_eq!(rig.ended().await, End::Peer);
        assert_eq!(ends_heard(&rig.log), (1, 0));
    }

    #[tokio::test]
    async fn a_viewer_that_says_leave_and_loses_the_connection_has_the_platform_lock_the_screen() {
        let mut rig = Rig::with(leaving_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::Leave)
            .await
            .unwrap();
        rig.until("the source to read the Leave", Rig::left).await;
        rig.accepted.close(code(close_code::NORMAL), b"gone");
        rig.ended().await;
        assert_eq!(ends_heard(&rig.log), (1, 0));
        drop(send);
    }

    #[tokio::test]
    async fn a_viewer_that_says_leave_and_then_stalls_in_a_message_has_the_platform_lock_the_screen()
     {
        // The person chose, and then the viewer began a message that never came: it may have
        // crashed, or been cut off. A stall is not the end that was asked for.
        let mut params = quick();
        params.stall = Duration::from_millis(200);
        let mut rig = Rig::with(
            SourceSetup {
                params,
                ..leaving_setup()
            },
            1,
        )
        .await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::Leave)
            .await
            .unwrap();
        // The length of a message of 20 bytes, and then nothing.
        send.write_all(&[0, 0, 0, 20]).await.unwrap();
        assert_eq!(rig.ended().await, End::Stalled);
        assert_eq!(ends_heard(&rig.log), (1, 0));
    }

    #[tokio::test]
    async fn a_viewer_that_says_leave_and_finishes_in_the_middle_of_a_message_has_the_platform_lock_the_screen()
     {
        // A stream that ends between two messages was finished, and one that ends inside a message
        // was cut off, which the viewer does not do when it has said what it had to say.
        let mut rig = Rig::with(leaving_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::Leave)
            .await
            .unwrap();
        // Two bytes of the length of a message, and then the end of the stream.
        send.write_all(&[0, 0]).await.unwrap();
        send.finish().unwrap();
        assert_eq!(rig.ended().await, End::Peer);
        assert_eq!(ends_heard(&rig.log), (1, 0));
    }

    #[tokio::test]
    async fn a_leave_for_a_session_whose_offer_did_not_say_leave_breaks_a_rule_and_locks() {
        let mut rig = Rig::with(private_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        write_message(&mut send, &ViewerMessage::Leave)
            .await
            .unwrap();
        assert_eq!(rig.ended().await, End::Violation);
        assert_eq!(stopped_with(&send).await, 400);
        assert_eq!(ends_heard(&rig.log), (1, 0));
    }

    #[tokio::test]
    async fn a_message_after_leave_breaks_a_rule_and_locks() {
        // `Leave` is the last thing a viewer says, so there is no knowing what the rest means.
        for (what, after) in [
            (
                "a report",
                ViewerMessage::Report(Report {
                    shown: None,
                    decode_us: 0,
                    dropped: 0,
                    thermal: Thermal::Nominal,
                }),
            ),
            ("another leave", ViewerMessage::Leave),
        ] {
            let mut rig = Rig::with(leaving_setup(), 1).await;
            let mut control = rig.source_control().await;
            read_source_message(&mut control).await;
            let mut send = rig.open_viewer_control().await;
            write_message(&mut send, &ViewerMessage::Leave)
                .await
                .unwrap();
            write_message(&mut send, &after).await.unwrap();
            assert_eq!(rig.ended().await, End::Violation, "{what}");
            assert_eq!(ends_heard(&rig.log), (1, 0), "{what}");
        }
    }

    #[tokio::test]
    async fn a_leave_that_comes_after_the_session_ended_changes_nothing() {
        let mut rig = Rig::with(leaving_setup(), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        let mut send = rig.open_viewer_control().await;
        rig.handle.end();
        assert_eq!(ends_heard(&rig.log), (1, 0));
        // The stream may still take what the viewer wrote before it saw the end, and the session,
        // which has ended, takes none of it.
        let _ = write_message(&mut send, &ViewerMessage::Leave).await;
        let _ = send.finish();
        tokio::time::sleep(QUIET).await;
        assert_eq!(ends_heard(&rig.log), (1, 0));
    }

    #[tokio::test]
    async fn dropping_the_handle_of_a_session_that_may_be_left_has_the_platform_lock_the_screen() {
        let rig = Rig::with(leaving_setup(), 1).await;
        let Rig { handle, log, .. } = rig;
        drop(handle);
        assert_eq!(ends_heard(&log), (1, 0));
    }

    #[tokio::test]
    async fn the_first_end_of_a_session_is_the_only_one_whether_it_leaves_the_screen_or_not() {
        let rig = Rig::with(leaving_setup(), 1).await;
        rig.handle.end();
        rig.handle.source.end_leaving();
        assert_eq!(ends_heard(&rig.log), (1, 0));

        let rig = Rig::with(leaving_setup(), 1).await;
        rig.handle.source.end_leaving();
        rig.handle.end();
        assert_eq!(ends_heard(&rig.log), (0, 1));
    }

    #[tokio::test]
    async fn a_session_whose_offer_did_not_say_leave_locks_the_screen_even_when_the_viewer_asks() {
        let rig = Rig::with(private_setup(), 1).await;
        rig.handle.source.end_leaving();
        assert_eq!(rig.ended().await, End::Peer);
        assert_eq!(ends_heard(&rig.log), (1, 0));
    }

    #[tokio::test]
    async fn a_screen_that_is_not_private_is_never_one_to_leave() {
        let setup = SourceSetup {
            leave: true,
            ..setup(true, quick())
        };
        let Err(error) = Rig::starting(setup, 1).await else {
            panic!("the session started");
        };
        assert!(matches!(error, Error::Config(_)), "{error:?}");
    }

    #[test]
    fn a_platform_that_cannot_leave_a_screen_as_it_is_is_told_of_an_ordinary_end() {
        let log = Log::default();
        Plain(log.clone()).ended_leaving();
        assert_eq!(log.all(), [Heard::Ended]);
    }

    #[test]
    fn a_platform_behind_a_box_is_asked_to_leave_the_screen_as_it_is() {
        let log = Log::default();
        let mut boxed: Box<dyn SourceEvents> = Box::new(Recording(log.clone()));
        boxed.ended_leaving();
        assert_eq!(log.all(), [Heard::Left]);
        let mut plain: Box<dyn SourceEvents> = Box::new(Plain(log.clone()));
        plain.ended_leaving();
        assert_eq!(log.all(), [Heard::Left, Heard::Ended]);
    }

    #[tokio::test]
    async fn a_wait_for_the_end_that_is_given_out_says_why_the_session_ended() {
        let rig = Rig::new().await;
        let waiting = tokio::spawn(rig.handle.on_end());
        rig.accepted.close(code(close_code::NORMAL), b"done");
        let end = tokio::time::timeout(PATIENCE, waiting)
            .await
            .expect("the wait did not end")
            .unwrap();
        assert_eq!(end, End::Connection);
        // One that is asked for after the end is ready at once, with the same answer.
        let late = tokio::time::timeout(PATIENCE, rig.handle.on_end()).await;
        assert_eq!(late, Ok(End::Connection));
    }

    #[tokio::test]
    async fn a_wait_for_the_end_that_is_given_out_does_not_keep_the_session_going() {
        let rig = Rig::new().await;
        let Rig {
            handle,
            log,
            streaming,
            ..
        } = rig;
        // The wait holds the session and not its handle, so the last copy of the handle is
        // dropped while it waits.
        let waiting = tokio::spawn(handle.on_end());
        drop(handle);
        let end = tokio::time::timeout(PATIENCE, waiting)
            .await
            .expect("the session did not end")
            .unwrap();
        assert_eq!(end, End::Local);
        assert!(log.has(&Heard::Ended));
        assert!(streaming.is_empty());
    }

    #[tokio::test]
    async fn a_session_that_has_ended_tells_the_platform_nothing_more() {
        let rig = Rig::new().await;
        let source = Arc::clone(&rig.handle.source);
        // A frame with a long-term reference is on its way.
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
        let picture = Picture {
            ltr: Some(LtrToken(7)),
            ..Rig::picture(1000)
        };
        assert_eq!(rig.handle.sent(picture), Ok(Some(0)));
        rig.handle.end();
        let told = rig.log.all();
        assert_eq!(told.last(), Some(&Heard::Ended));
        // What the tasks that had passed their checks of the session when it ended can still do:
        // carry out what the viewer said, settle a frame, and change the picture.
        let report = Report {
            shown: None,
            decode_us: 0,
            dropped: 0,
            thermal: Thermal::Nominal,
        };
        let want = Want {
            width: 2796,
            height: 1290,
            fps: 120,
        };
        let batch = InputBatch {
            events: vec![Event::Pointer {
                frame: 0,
                x: 0.5,
                y: 0.25,
                modifiers: Modifiers::NONE,
                action: PointerAction::Move,
            }],
        };
        for message in [
            ViewerMessage::Report(report),
            ViewerMessage::Want(want),
            ViewerMessage::Input(batch),
            ViewerMessage::KeyFrame(KeyFrame { tried: 0 }),
        ] {
            assert!(source.viewer_said(message).is_continue());
        }
        source.settle(0, Outcome::Acked);
        let bigger = Config {
            width: 2048,
            height: 1280,
            ..config()
        };
        rig.handle.new_epoch(bigger).unwrap();
        // And the tasks of the session, which run once the test waits.
        tokio::time::sleep(3 * quick().tick_every).await;
        assert_eq!(rig.log.all(), told);
    }

    #[tokio::test]
    async fn the_clock_tells_the_platform_nothing_after_the_end() {
        let rig = Rig::new().await;
        let source = Arc::clone(&rig.handle.source);
        // A first picture is still wanted, and the next look at the clock would say so.
        assert!(rig.handle.recovery_pending());
        lock(&source.core).recovery_told = false;
        rig.handle.end();
        let told = rig.log.all();
        assert_eq!(told.last(), Some(&Heard::Ended));
        assert!(source.on_tick().is_break());
        assert_eq!(rig.log.all(), told);
    }

    #[tokio::test]
    async fn a_picture_that_was_begun_when_the_session_ended_is_refused() {
        let mut rig = Rig::new().await;
        assert_eq!(rig.handle.begin(), Some(FrameKind::Key));
        rig.handle.end();
        assert_eq!(rig.handle.sent(Rig::picture(1000)), Err(NotSent::Over));
        // Nothing was counted, and no stream was opened for it.
        assert_eq!(rig.handle.stats().frames.sent, 0);
        nothing_arrives(&mut rig.frames).await;
        // With nothing in flight and nothing being made, the rules would want a picture: it is
        // the end that says no.
        rig.handle.cancel();
        assert_eq!(rig.handle.begin(), None);
    }

    #[tokio::test]
    async fn pausing_stops_the_encoder_and_the_viewer_is_told_in_order() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        rig.handle.pause(PauseReason::Locked);
        assert_eq!(rig.handle.begin(), None);
        rig.handle.resume();
        assert!(rig.picture_wanted());
        let mut said = Vec::new();
        for _ in 0..3 {
            said.push(read_source_message(&mut control).await.unwrap());
        }
        assert_eq!(
            said,
            [
                SourceMessage::Config(Config {
                    epoch: 0,
                    ..config()
                }),
                SourceMessage::Pause(Pause {
                    reason: PauseReason::Locked
                }),
                SourceMessage::Resume,
            ]
        );
    }

    #[tokio::test]
    async fn a_new_epoch_resets_the_frames_and_begins_with_a_key_frame() {
        let mut rig = Rig::new().await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        let bigger = Config {
            width: 2048,
            height: 1280,
            ..config()
        };
        rig.handle.new_epoch(bigger).unwrap();
        assert_eq!(
            read_source_message(&mut control).await,
            Some(SourceMessage::Config(Config { epoch: 1, ..bigger }))
        );
        assert_eq!(rig.frame(1000), (FrameKind::Key, 1));
        let (header, _) = read_frame(&mut rig.frames).await;
        assert_eq!(header.epoch, 1);
        // A format beyond the limits changes nothing.
        let invalid = Config { width: 0, ..bigger };
        assert!(rig.handle.new_epoch(invalid).is_err());
        rig.settled().await;
        assert_eq!(rig.frame(1000), (FrameKind::Delta, 2));
    }

    #[tokio::test]
    async fn a_new_epoch_resets_a_frame_that_is_on_its_way() {
        // The clock gives up on nothing for a minute, so only the new epoch can reset the frame.
        let rules = Rules {
            grace: Duration::from_secs(60),
            longest: Duration::from_secs(60),
            ..Rules::default()
        };
        let mut rig = Rig::with(setup(true, SourceParams { rules, ..quick() }), 1).await;
        let mut control = rig.source_control().await;
        read_source_message(&mut control).await;
        rig.frame(1000);
        read_frame(&mut rig.frames).await;
        rig.settled().await;
        // A delta that is still on its way: the viewer takes its stream and does not read it.
        assert_eq!(rig.frame(8 * 1024 * 1024), (FrameKind::Delta, 1));
        let mut on_its_way = next(&mut rig.frames).await.recv;
        let bigger = Config {
            width: 2048,
            height: 1280,
            ..config()
        };
        rig.handle.new_epoch(bigger).unwrap();
        assert_eq!(reset_with(&mut on_its_way).await, 499);
    }

    #[tokio::test]
    async fn a_stream_that_does_not_belong_to_the_session_is_stopped() {
        let rig = Rig::with(setup(true, quick()), 2).await;
        // A control stream for a session that is not running.
        let mut other = rig.accepted.open_uni().await.unwrap();
        other
            .write_all(&encode_prefix(
                KIND_CONTROL,
                SessionId::from_bytes([8; SessionId::LEN]),
            ))
            .await
            .unwrap();
        assert_eq!(stopped_with(&other).await, 499);
        // A frame goes from the source to the viewer and not back.
        let mut frame = rig.accepted.open_uni().await.unwrap();
        frame
            .write_all(&encode_prefix(KIND_VIDEO, session_id()))
            .await
            .unwrap();
        assert_eq!(stopped_with(&frame).await, 400);
        // Nothing of this ended the session.
        assert!(!rig.streaming.is_empty());
    }

    #[tokio::test]
    async fn a_control_stream_on_another_connection_of_the_viewer_is_not_taken() {
        let rig = Rig::new().await;
        // The same two devices, on a connection of their own.
        let (_second_dialed, second_accepted) = connect(&rig.source, &rig.viewer).await;
        let mut other = second_accepted.open_uni().await.unwrap();
        other
            .write_all(&encode_prefix(KIND_CONTROL, session_id()))
            .await
            .unwrap();
        assert_eq!(stopped_with(&other).await, 499);
        assert!(!rig.log.has(&Heard::Ended));
        assert!(rig.streaming.sourcing(&session_id()).is_some());
    }

    #[tokio::test]
    async fn a_session_id_that_is_in_use_is_refused_and_so_is_a_format_beyond_the_limits() {
        let rig = Rig::new().await;
        let paired = rig.source.trust.get(&rig.viewer.id).unwrap();
        let again = rig.streaming.source(
            &paired,
            rig.dialed.clone(),
            None,
            session_id(),
            setup(true, quick()),
            Recording(Log::default()),
        );
        assert!(again.is_err());
        let invalid = SourceSetup {
            config: Config {
                width: 0,
                ..config()
            },
            ..setup(true, quick())
        };
        let other = SessionId::from_bytes([9; SessionId::LEN]);
        let refused = rig.streaming.source(
            &paired,
            rig.dialed.clone(),
            None,
            other,
            invalid,
            Recording(Log::default()),
        );
        assert!(refused.is_err());
        // Neither touched the session that was there first.
        assert!(rig.streaming.sourcing(&session_id()).is_some());
        assert!(rig.streaming.sourcing(&other).is_none());
        assert!(rig.picture_wanted());
    }

    #[tokio::test]
    async fn the_source_knows_the_path_of_its_connection() {
        let rig = Rig::new().await;
        let source = rig.streaming.sourcing(&session_id()).unwrap();
        let path = source.path();
        assert!(path.window > 0);
    }

    // The sound of a session: what the source does with the answers of the rules.

    #[tokio::test]
    async fn the_ninth_packet_in_flight_resets_the_stream_of_the_oldest() {
        let rig = Rig::hearing(quick()).await;
        for byte in 0..9_u8 {
            assert_eq!(
                rig.handle.sound(packet(byte)).unwrap(),
                Some(u64::from(byte))
            );
        }
        // Eight may be in flight, so the oldest makes room for the ninth, and no other is reset.
        assert!(rig.packet_token(0).is_cancelled());
        for number in 1..9 {
            assert!(!rig.packet_token(number).is_cancelled(), "{number}");
        }
    }

    #[tokio::test]
    async fn a_packet_that_is_late_at_a_look_at_the_clock_has_its_stream_reset() {
        let params = SourceParams {
            sound: SoundRules {
                grace: Duration::ZERO,
                round_trips: 0,
                ..SoundRules::default()
            },
            ..quick()
        };
        let rig = Rig::hearing(params).await;
        rig.handle.sound(packet(1)).unwrap();
        assert!(!rig.packet_token(0).is_cancelled());
        // The clock goes on while nothing else runs, and the packet is late by the next look.
        std::thread::sleep(Duration::from_millis(20));
        assert!(rig.handle.source.on_tick().is_continue());
        assert!(rig.packet_token(0).is_cancelled());
    }

    #[tokio::test]
    async fn what_became_of_the_stream_of_a_packet_is_told_to_the_rules() {
        let rig = Rig::hearing(quick()).await;
        for byte in 0..3_u8 {
            rig.handle.sound(packet(byte)).unwrap();
        }
        let source = &rig.handle.source;
        source.settle_packet(0, Outcome::Acked);
        source.settle_packet(1, Outcome::Stopped);
        source.settle_packet(2, Outcome::Reset);
        let stats = rig.handle.stats();
        // A stream that was reset has nothing to tell the rules: they asked for the reset.
        assert_eq!(
            stats.sound,
            SoundStats {
                sent: 3,
                acked: 1,
                stopped: 1,
                ..SoundStats::default()
            }
        );
        assert_eq!(stats.sound_streams, 0);
    }
}
