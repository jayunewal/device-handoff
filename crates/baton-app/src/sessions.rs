// SPDX-License-Identifier: Apache-2.0

//! The windows of the app: the offers that arrive, the sessions that run, and the offers that
//! this device makes. [`Windows`](crate::Windows) says what the platform provides, and this is
//! what the app does with it.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use baton_core::{
    PairedDevice,
    streaming::{
        DecodeError, Decoder, DecoderState, End, Play, ShareOutcome, SourceHandle, Speaker,
        StartOutcome, StartSlot, Takes, ViewHandle, Viewing, Wanted, WindowHandler, WindowOffer,
        WindowShare, WindowVerdict,
    },
    util::BoxFuture,
};
use baton_proto::{DeviceId, message::DeclineReason};
use baton_stream::{
    control::{PauseReason, SilentReason, Sound, SourceMessage},
    gate::Notice,
    header::FrameHeader,
};
use tokio::sync::broadcast;

use crate::{
    Error, ErrorKind, Event, Result,
    app::{Questions, locked},
    model::{
        ScreenStarted, UnlockResult, WindowAbilities, WindowEnd, WindowInput, WindowRequest,
        WindowRole, WindowSession, WindowShared, WindowSound, WindowUnlock,
    },
    unlock::{Asked, UnlockSource},
    windows::{PickedWindow, ScreenLink, SharingFailure, WindowSharer, WindowViewer, noun},
};

/// A session that runs, and how to end it.
#[derive(Clone)]
enum Live {
    /// A window of another device that is shown here. `sound` is whether the person can ask to
    /// hear it ([`WindowSession::sound`]).
    Viewing {
        handle: Arc<ViewHandle>,
        sound: bool,
    },
    /// A window of this device that another one shows. The platform has the handle, and ends the
    /// session by letting go of it, so the app keeps no copy that would keep it going.
    Sharing(Weak<SourceHandle>),
}

/// A window that the person accepted, between the answer to the offer and the start of its
/// session. A viewer shows one window at a time, so there is at most one.
struct Accepted {
    request: WindowRequest,
    screen: i64,
    /// Reaches the session of the screen, once the session runs.
    link: ScreenLink,
    /// Tells the screens how large the picture is, once they know the window.
    pictures: Arc<Pictures>,
}

/// What a window that is to be shown here needs before the node can be asked to take it.
struct Prepared {
    /// The number that the user interface draws the screen with.
    screen: i64,
    /// Reaches the session of the screen, once the session runs.
    link: ScreenLink,
    /// Tells the screens how large the picture is, once they know the window.
    pictures: Arc<Pictures>,
    /// What this device takes, and the decoder that shows it.
    viewing: Viewing,
}

/// What the source of a window that is shown here says that the screens need: the size of its
/// picture, kept until the screens know the window, the events of the person that it dropped, how
/// its sound stands, whether its screen is locked, and how the unlock that the person asked for
/// goes.
///
/// The source says the size before its first picture, and the node tells the app that the
/// session runs at about the same moment, on tasks of their own, so the size can come first. It
/// waits here for the window and is told right after [`Event::WindowStarted`], so that a screen
/// always hears of a window before it hears of its picture. A notice answers what the person did
/// on the window, which they can do only once they know it, so it is told when the window is
/// known and dropped when it is not. The same goes for the sound, which the person asks for only
/// once they know the window, and for the steps of an unlock. Whether the screen is locked is kept
/// until the screens know the window, as the size is: a screen that is locked when the window
/// starts is told right after it.
pub(crate) struct Pictures {
    events: broadcast::Sender<Event>,
    state: Mutex<PictureState>,
}

#[derive(Default)]
struct PictureState {
    /// The window, once the screens have been told of it.
    session: Option<u64>,
    /// The size that the source said last.
    size: Option<(u32, u32)>,
    /// Whether the source paused because its screen is locked and has not resumed since.
    locked: bool,
    /// How the question of the source stands with the platform.
    prompt: Prompt,
    /// How many questions have come, which numbers them.
    asked: u64,
    /// Whether the end of the attempt that is settling was told already, by the platform that
    /// declined its question, and the next end that the session settles is the same one.
    told: bool,
}

/// How the question that the source sent to unlock its screen stands, as the platform and the
/// session see it. Each question is answered once, and an answer to one that was taken back is
/// dropped, so that no step follows the end of an attempt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Prompt {
    /// No question is with the platform.
    #[default]
    Shut,
    /// The question with this number is with the platform, which asks for the face.
    Open(u64),
    /// The platform declined it, and the end was told.
    Declined,
}

impl Pictures {
    pub(crate) fn new(events: broadcast::Sender<Event>) -> Arc<Self> {
        Arc::new(Self {
            events,
            state: Mutex::new(PictureState::default()),
        })
    }

    /// The source says that its pictures are this size from now on.
    fn said(&self, width: u32, height: u32) {
        let mut state = locked(&self.state);
        state.size = Some((width, height));
        if let Some(id) = state.session {
            // Told under the lock, so that it cannot overtake the size that `began` tells.
            self.tell(id, width, height);
        }
    }

    /// The screens have been told of the window `id`: whatever the source said before is told now.
    fn began(&self, id: u64) {
        let mut state = locked(&self.state);
        state.session = Some(id);
        if let Some((width, height)) = state.size {
            self.tell(id, width, height);
        }
        if state.locked {
            self.tell_locked(id, true);
        }
    }

    fn tell(&self, id: u64, width: u32, height: u32) {
        // Nobody listening is fine.
        let _ = self.events.send(Event::WindowPicture { id, width, height });
    }

    /// The source did not do some of what the person did, for the reason in the notice.
    fn noticed(&self, notice: Notice) {
        let state = locked(&self.state);
        // Under the lock, so that it cannot overtake the window or a size that `began` tells.
        if let Some(id) = state.session {
            // The sentence on the screen cannot tell every reason apart, and a person who reports
            // that nothing happened can read the log of this device. The other device sends at most
            // one notice a second, so this is a line a second at the most.
            tracing::info!(
                reason = ?notice.reason,
                count = notice.count,
                "the other device did not do some of what was done on the window"
            );
            // Nobody listening is fine.
            let _ = self.events.send(Event::WindowNotice {
                id,
                reason: notice.reason.into(),
                count: notice.count,
            });
        }
    }

    /// The sound of the window began, ended or is not sent.
    fn sounded(&self, sound: WindowSound) {
        let state = locked(&self.state);
        // Under the lock, so that it cannot overtake the window or a size that `began` tells.
        if let Some(id) = state.session {
            tracing::info!(?sound, "the sound of the window changed");
            // Nobody listening is fine.
            let _ = self.events.send(Event::WindowSound { id, sound });
        }
    }

    /// The source stopped sending pictures. Only a pause because its screen is locked says that
    /// it is.
    fn paused(&self, reason: PauseReason) {
        if reason == PauseReason::Locked {
            self.lock_changed(true);
        }
    }

    /// The source goes on sending pictures: its screen is not locked, if it was.
    fn resumed(&self) {
        self.lock_changed(false);
    }

    /// Takes note of whether the screen of the source is locked. Only a change is told.
    fn lock_changed(&self, is_locked: bool) {
        let mut state = locked(&self.state);
        if state.locked == is_locked {
            return;
        }
        state.locked = is_locked;
        // Under the lock, so that it cannot overtake the window that `began` tells.
        if let Some(id) = state.session {
            tracing::debug!(is_locked, "the screen of the other device changed");
            self.tell_locked(id, is_locked);
        }
    }

    fn tell_locked(&self, id: u64, is_locked: bool) {
        // Nobody listening is fine.
        let _ = self.events.send(Event::WindowLocked {
            id,
            locked: is_locked,
        });
    }

    /// Tells the screens how the unlock that the person asked for goes. Dropped while the window
    /// is not known: the person asks only once they know it.
    fn unlock_step(&self, state: &PictureState, step: WindowUnlock) {
        if let Some(id) = state.session {
            // Nobody listening is fine.
            let _ = self.events.send(Event::WindowUnlock { id, step });
        }
    }

    /// The source sent its question and the platform asks for the face. Returns the number that
    /// the answer of the platform carries.
    pub(crate) fn asking(&self) -> u64 {
        let mut state = locked(&self.state);
        state.asked += 1;
        let number = state.asked;
        state.prompt = Prompt::Open(number);
        state.told = false;
        self.unlock_step(&state, WindowUnlock::Asking);
        number
    }

    /// The platform answers the question `number`, and the screens are told `step`: the key goes
    /// to the source, or the platform gave up and that is the end of the attempt. Returns whether
    /// the answer is taken, which it is not when the question was taken back first because the
    /// attempt ended.
    pub(crate) fn answered(&self, number: u64, step: WindowUnlock) -> bool {
        let mut state = locked(&self.state);
        if state.prompt != Prompt::Open(number) {
            return false;
        }
        state.prompt = if matches!(step, WindowUnlock::Ended(_)) {
            Prompt::Declined
        } else {
            Prompt::Shut
        };
        self.unlock_step(&state, step);
        true
    }

    /// The attempt ended while the question was with the platform: an answer that comes later is
    /// dropped. A question that the platform had declined told the end already, and the end that
    /// the session settles right after is the same.
    pub(crate) fn taken_back(&self) {
        let mut state = locked(&self.state);
        state.told = state.prompt == Prompt::Declined;
        state.prompt = Prompt::Shut;
    }

    /// The attempt ended, and this is how.
    pub(crate) fn settled(&self, result: UnlockResult) {
        let mut state = locked(&self.state);
        state.prompt = Prompt::Shut;
        if std::mem::take(&mut state.told) {
            return;
        }
        self.unlock_step(&state, WindowUnlock::Ended(result));
    }
}

/// The speaker of the platform, with the app hearing when the sound begins or ends, and why it is
/// not there. It passes everything on as it is.
struct Listening {
    speaker: Box<dyn Speaker>,
    pictures: Arc<Pictures>,
}

impl Speaker for Listening {
    fn sound(&mut self, sound: Sound) {
        self.speaker.sound(sound);
        self.pictures.sounded(WindowSound::Playing);
    }

    fn play(&mut self, play: Play, packet: Vec<u8>) {
        self.speaker.play(play, packet);
    }

    fn stopped(&mut self, why: Option<SilentReason>) {
        self.speaker.stopped(why);
        self.pictures
            .sounded(why.map_or(WindowSound::Off, WindowSound::from));
    }

    fn could_not_play(&mut self) {
        self.speaker.could_not_play();
        self.pictures.sounded(WindowSound::CannotPlay);
    }

    fn ended(&mut self) {
        self.speaker.ended();
    }
}

/// The decoder of the platform, with the app hearing what the source says about its picture and
/// about the events it dropped. It passes everything on as it is.
struct Watching {
    decoder: Box<dyn Decoder>,
    pictures: Arc<Pictures>,
}

impl Decoder for Watching {
    fn decode(
        &mut self,
        frame: &FrameHeader,
        picture: Vec<u8>,
    ) -> std::result::Result<(), DecodeError> {
        self.decoder.decode(frame, picture)
    }

    fn show(&mut self, number: u64) {
        self.decoder.show(number);
    }

    fn source(&mut self, message: SourceMessage) {
        match &message {
            SourceMessage::Config(config) => self.pictures.said(config.width, config.height),
            SourceMessage::Notice(notice) => self.pictures.noticed(*notice),
            SourceMessage::Pause(pause) => self.pictures.paused(pause.reason),
            SourceMessage::Resume => self.pictures.resumed(),
            _ => {}
        }
        self.decoder.source(message);
    }

    fn state(&self) -> DecoderState {
        self.decoder.state()
    }

    fn ended(&mut self) {
        self.decoder.ended();
    }
}

/// What the app keeps for window streaming.
pub(crate) struct Sessions {
    viewer: Option<Arc<dyn WindowViewer>>,
    sharer: Option<Arc<dyn WindowSharer>>,
    questions: Arc<Questions>,
    events: broadcast::Sender<Event>,
    live: Mutex<HashMap<u64, Live>>,
    next: AtomicU64,
    accepted: Mutex<Option<Accepted>>,
}

impl Sessions {
    pub(crate) fn new(
        viewer: Option<Arc<dyn WindowViewer>>,
        sharer: Option<Arc<dyn WindowSharer>>,
        questions: Arc<Questions>,
        events: broadcast::Sender<Event>,
    ) -> Arc<Self> {
        Arc::new(Self {
            viewer,
            sharer,
            questions,
            events,
            live: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            accepted: Mutex::new(None),
        })
    }

    pub(crate) fn abilities(&self) -> WindowAbilities {
        WindowAbilities {
            can_show: self.viewer.is_some(),
            can_share: self.sharer.is_some(),
            can_control: self.sharer.as_ref().is_some_and(|sharer| sharer.control()),
            can_give_screen: self
                .sharer
                .as_ref()
                .is_some_and(|sharer| sharer.can_give_screen()),
            can_be_unlocked: self.sharer_unlocks().is_some(),
            can_unlock: self.viewer_unlocks().is_some(),
        }
    }

    /// The sharer, if it lets a paired device unlock this device's screen: the platform keeps the
    /// password and types it, and the screens it gives can be controlled and lifted, since an
    /// unlock goes with the privacy of such a screen (`docs/adr/0013-unlock-from-a-phone.md`).
    pub(crate) fn sharer_unlocks(&self) -> Option<&Arc<dyn WindowSharer>> {
        self.sharer
            .as_ref()
            .filter(|sharer| sharer.control() && sharer.can_lift() && sharer.can_be_unlocked())
    }

    /// The viewer, if it unlocks the screens of paired devices with the face of the person here.
    pub(crate) fn viewer_unlocks(&self) -> Option<&Arc<dyn WindowViewer>> {
        self.viewer.as_ref().filter(|viewer| viewer.can_unlock())
    }

    /// Checks what the viewer says it can show against what the protocol allows, so that a
    /// platform that is set up wrongly fails when the app starts and not at the first window.
    pub(crate) fn check_limits(&self) -> Result<()> {
        let Some(viewer) = &self.viewer else {
            return Ok(());
        };
        viewer.limits().check().map_err(|error| {
            Error::new(
                ErrorKind::Other,
                format!("This device is set up wrongly to show windows ({error})."),
            )
        })
    }

    pub(crate) fn sharer(&self) -> Option<&Arc<dyn WindowSharer>> {
        self.sharer.as_ref()
    }

    /// The window `id`, if it is shown here and runs.
    fn viewing(&self, id: u64) -> Option<Arc<ViewHandle>> {
        match locked(&self.live).get(&id) {
            Some(Live::Viewing { handle, .. }) => Some(Arc::clone(handle)),
            _ => None,
        }
    }

    /// Gives what the person did on the window `id`. Returns whether it was taken: a window that
    /// is not shown here, one that the other device did not let this one control, and an event that
    /// has no meaning (a touch outside the picture, a release of what is not down) take nothing.
    pub(crate) fn input(&self, id: u64, input: WindowInput) -> bool {
        self.viewing(id)
            .is_some_and(|handle| handle.input(input.into()).is_ok())
    }

    /// Tells the other device of the window `id` what the people there may do. Returns whether
    /// the message was queued: it is not for a window that is not shown here or not private.
    pub(crate) fn lift(&self, id: u64, screen: bool, input: bool) -> bool {
        self.viewing(id)
            .is_some_and(|handle| handle.lift(screen, input))
    }

    /// Lets go of every button and key that is down on the window `id`, if it is shown here.
    pub(crate) fn release_input(&self, id: u64) {
        if let Some(handle) = self.viewing(id) {
            handle.release_input();
        }
    }

    /// Asks the other device to send the sound of the screen `id`, or to stop. Returns whether the
    /// request was made: a window that is not shown here, a window and not a whole screen, a
    /// device that sends no sound and a session that is over take no request.
    pub(crate) fn hear(&self, id: u64, on: bool) -> bool {
        let handle = match locked(&self.live).get(&id) {
            Some(Live::Viewing {
                handle,
                sound: true,
            }) => Arc::clone(handle),
            _ => return false,
        };
        handle.hear(on)
    }

    /// Whether this device plays the sound of the screens it shows: the platform says so.
    pub(crate) fn plays_sound(&self) -> bool {
        self.viewer
            .as_ref()
            .is_some_and(|viewer| viewer.plays_sound())
    }

    /// Ends the session `id`, if it runs. Returns whether it did.
    pub(crate) fn end(&self, id: u64) -> bool {
        let live = locked(&self.live).get(&id).cloned();
        match live {
            Some(Live::Viewing { handle, .. }) => handle.end(),
            Some(Live::Sharing(handle)) => match handle.upgrade() {
                Some(handle) => handle.end(),
                // The platform let go of the handle, which ended the session. Its entry goes when
                // the task that waits for the end gets to it.
                None => return false,
            },
            None => return false,
        }
        true
    }

    /// Ends the session `id`, which is shown here, and tells the device that shares it that the
    /// person chose to leave it as it is. Returns whether the choice was made, which is whether this
    /// call ended the session and the device could be asked ([`ViewHandle::end_leaving`]). A window
    /// that is not shown here, or whose device cannot leave its screen, ends as `end` ends it, or
    /// not at all, and says no.
    pub(crate) fn end_leaving(&self, id: u64) -> bool {
        self.viewing(id).is_some_and(|handle| handle.end_leaving())
    }

    /// Asks the device that shares the screen `id`, which is shown here, to unlock it. Returns
    /// whether the request was made: a window that is not shown here, a session that takes no
    /// unlock, one whose attempt is going and a session that is over take no request.
    pub(crate) fn unlock(&self, id: u64) -> bool {
        self.viewing(id).is_some_and(|handle| handle.unlock())
    }

    fn emit(&self, event: Event) {
        // Nobody listening is fine.
        let _ = self.events.send(event);
    }

    /// Takes note of a session that started, tells the screens, and reports its end when it comes.
    /// `ended` waits for the end and says how it is told. It is given apart from `live` because a
    /// window that this device shares is waited for without holding its handle, which the
    /// platform owns. `pictures` is given for a window that is shown here: what its source said of
    /// the size of the picture is told right after the window, and never before it.
    fn begin(
        self: &Arc<Self>,
        session: WindowSession,
        live: Live,
        ended: BoxFuture<'static, WindowEnd>,
        pictures: Option<&Pictures>,
    ) {
        let id = session.id;
        locked(&self.live).insert(id, live);
        self.emit(Event::WindowStarted(session));
        if let Some(pictures) = pictures {
            pictures.began(id);
        }
        let sessions = Arc::clone(self);
        tokio::spawn(async move {
            let end = ended.await;
            locked(&sessions.live).remove(&id);
            tracing::debug!(session = id, ?end, "window session ended");
            sessions.emit(Event::WindowEnded { id, end });
        });
    }

    fn next_id(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    /// The person is asked whether to show the window, and if they say yes the platform makes the
    /// screen for it.
    async fn offered(&self, from: &PairedDevice, offer: &WindowOffer) -> WindowVerdict {
        let Some(viewer) = &self.viewer else {
            return WindowVerdict::Refuse;
        };
        let device_id = from.id.to_string();
        let device_name = from.display_name().to_owned();
        let (accepted, id) = self
            .questions
            .ask_window(|id| WindowRequest {
                id,
                device_id: device_id.clone(),
                device_name: device_name.clone(),
                display: offer.display,
                title: offer.title.clone(),
                app: offer.app.clone(),
                width: offer.width,
                height: offer.height,
                control: offer.control,
            })
            .await;
        if !accepted {
            return WindowVerdict::Refuse;
        }
        let failed = |message: &str| {
            self.emit(Event::WindowRequestEnded {
                id,
                failure: Some(Error::new(ErrorKind::Other, message).message().to_owned()),
            });
            WindowVerdict::Refuse
        };
        let Prepared {
            screen,
            link,
            pictures,
            viewing,
        } = match self.prepare(viewer, offer, UnlockSource::of(from)) {
            Ok(prepared) => prepared,
            Err(sentence) => return failed(&sentence),
        };
        *locked(&self.accepted) = Some(Accepted {
            request: WindowRequest {
                id,
                device_id,
                device_name,
                display: offer.display,
                title: offer.title.clone(),
                app: offer.app.clone(),
                width: offer.width,
                height: offer.height,
                control: offer.control,
            },
            screen,
            link,
            pictures,
        });
        WindowVerdict::Accept(viewing)
    }

    /// The platform makes the screen for `offer`, which the device `source` shares, and the node is
    /// told what this device takes and shows the picture with. The sentence is what the person is
    /// told if that cannot be done.
    fn prepare(
        &self,
        viewer: &Arc<dyn WindowViewer>,
        offer: &WindowOffer,
        source: UnlockSource,
    ) -> std::result::Result<Prepared, String> {
        let link = ScreenLink::default();
        let screen = viewer.open(offer, link.clone()).map_err(|error| {
            tracing::warn!(error = %error, "the platform could not show a window");
            error.message().to_owned()
        })?;
        let limits = viewer.limits();
        let pictures = Pictures::new(self.events.clone());
        let viewing = Viewing::new(
            Watching {
                decoder: screen.decoder,
                pictures: Arc::clone(&pictures),
            },
            limits.max_width,
            limits.max_height,
            limits.max_fps,
            limits.codecs,
        )
        .map_err(|error| {
            tracing::error!(error = %error, "the platform's limits are not within the protocol");
            "This device cannot show windows.".to_owned()
        })?;
        // A speaker the platform gave is used only on a device that was set up to play sound; the
        // session never calls it on another.
        let viewing = match screen.speaker {
            Some(speaker) => viewing.with_speaker(Listening {
                speaker,
                pictures: Arc::clone(&pictures),
            }),
            None => viewing,
        };
        // The same goes for what asks for the face: a session whose offer takes no unlock never
        // calls it.
        let viewing = match screen.unlocker {
            Some(unlocker) => viewing.with_unlocker(Asked {
                unlocker,
                pictures: Arc::clone(&pictures),
                source,
            }),
            None => viewing,
        };
        Ok(Prepared {
            screen: screen.id,
            link,
            pictures,
            viewing,
        })
    }

    /// The session of a window that was accepted runs. The node asks about one window at a time
    /// and starts it before it asks about the next, so what starts is what the person accepted.
    fn started(self: &Arc<Self>, handle: ViewHandle) {
        let Some(accepted) = locked(&self.accepted).take() else {
            // A window nobody accepted: the handle's drop ends it.
            tracing::warn!("a window started that the person had not accepted");
            return;
        };
        let WindowRequest {
            device_id,
            device_name,
            display,
            title,
            app,
            width,
            height,
            control,
            ..
        } = accepted.request;
        let session = WindowSession {
            id: self.next_id(),
            role: WindowRole::Viewing,
            device_id,
            device_name,
            display,
            title,
            app,
            width,
            height,
            control,
            private: handle.private(),
            // Sound goes with a whole screen only.
            sound: display && handle.can_hear(),
            leave: handle.can_leave(),
            unlock: handle.can_unlock(),
            screen: Some(accepted.screen),
        };
        self.begin_viewing(session, handle, &accepted.link, &accepted.pictures);
    }

    /// The session `session` of a window that is shown here runs with `handle`.
    fn begin_viewing(
        self: &Arc<Self>,
        session: WindowSession,
        handle: ViewHandle,
        link: &ScreenLink,
        pictures: &Pictures,
    ) {
        let handle = Arc::new(handle);
        link.connect(&handle);
        let ended = {
            let handle = Arc::clone(&handle);
            Box::pin(async move { window_end(handle.ended().await) })
        };
        let live = Live::Viewing {
            handle,
            sound: session.sound,
        };
        self.begin(session, live, ended, Some(pictures));
    }

    /// Asks `peer` to start sharing its screen with this device, and shows it if it does. No
    /// person at `peer` is asked: they allowed this device beforehand, in the settings for it.
    /// `control` is whether the person here wishes to control the screen; `peer` grants no more
    /// than the settings for this device allow.
    pub(crate) async fn start(
        self: &Arc<Self>,
        node: &baton_core::Node,
        peer: &DeviceId,
        name: &str,
        control: bool,
    ) -> Result<ScreenStarted> {
        let Some(viewer) = &self.viewer else {
            return Err(Error::new(
                ErrorKind::Other,
                "This device cannot show screens.",
            ));
        };
        // The node needs the decoder to ask, and the offer that answers comes after, so the
        // platform makes the screen for a display of no known size.
        let Prepared {
            screen,
            link,
            pictures,
            viewing,
        } = self
            .prepare(
                viewer,
                &WindowOffer::asked_screen(control),
                UnlockSource {
                    id: *peer,
                    name: name.to_owned(),
                },
            )
            .map_err(|sentence| Error::new(ErrorKind::Other, sentence))?;
        match node.start(peer, Wanted::Screen, control, viewing).await {
            Ok(StartOutcome::Started(handle)) => {
                let session = WindowSession {
                    id: self.next_id(),
                    role: WindowRole::Viewing,
                    device_id: peer.to_string(),
                    device_name: name.to_owned(),
                    display: true,
                    title: None,
                    app: None,
                    width: 0,
                    height: 0,
                    control: handle.control(),
                    private: handle.private(),
                    sound: handle.can_hear(),
                    leave: handle.can_leave(),
                    unlock: handle.can_unlock(),
                    screen: Some(screen),
                };
                self.begin_viewing(session.clone(), handle, &link, &pictures);
                Ok(ScreenStarted::Started(session))
            }
            Ok(StartOutcome::NotStarted(why)) => Ok(ScreenStarted::NotStarted(
                Error::not_started(name, why).message().to_owned(),
            )),
            // An outcome that this version does not know.
            Ok(_) => Ok(ScreenStarted::NotStarted(format!(
                "{name} did not start its screen."
            ))),
            Err(error) => Err(Error::asking(&error, name)),
        }
    }

    /// Offers the window that the person picked to `peer`, and starts the session if it says yes.
    /// `asked` is the place of the session when `peer` asked this device to start it: the offer is
    /// then made under the id that `peer` chose, and `control` is no more than the slot allows.
    pub(crate) async fn share(
        self: &Arc<Self>,
        node: &baton_core::Node,
        peer: &DeviceId,
        name: &str,
        mut picked: Box<dyn PickedWindow>,
        control: bool,
        asked: Option<StartSlot>,
    ) -> Result<WindowShared> {
        let (width, height) = picked.size();
        let (title, app) = (picked.title(), picked.app());
        let display = picked.display();
        // A screen is private for a session that is controlled, and the device that controls it
        // is told so that it can lift what keeps it private.
        let private = control && picked.private();
        let mut window = WindowShare::new(title.clone(), app.clone(), width, height);
        if display {
            window = window.whole_display();
        }
        if control {
            window = window.with_control();
        }
        if private {
            window = window.with_private();
        }
        let events = picked.events();
        // The platform's part is used twice, once to choose the picture and once to start the
        // capture, and the offer in between takes the first of these as a closure.
        let picked = Arc::new(Mutex::new(Some(picked)));
        let chooser = Arc::clone(&picked);
        let choose = move |takes: &Takes| {
            locked(&chooser)
                .as_mut()
                .and_then(|picked| picked.prepare(takes))
        };
        let outcome = match asked {
            None => node.share(peer, window, events, choose).await,
            Some(slot) => node.share_asked(slot, window, events, choose).await,
        };
        match outcome {
            Ok(ShareOutcome::Started(handle)) => {
                let Some(picked) = locked(&picked).take() else {
                    return Err(Error::new(ErrorKind::Other, "The window was lost."));
                };
                let handle = Arc::new(handle);
                let failure = SharingFailure::new(&handle);
                let session = WindowSession {
                    id: self.next_id(),
                    role: WindowRole::Sharing,
                    device_id: peer.to_string(),
                    device_name: name.to_owned(),
                    display,
                    title: non_empty(&title),
                    app: non_empty(&app),
                    width,
                    height,
                    control,
                    private,
                    sound: false,
                    leave: false,
                    unlock: false,
                    screen: None,
                };
                self.begin(
                    session.clone(),
                    Live::Sharing(Arc::downgrade(&handle)),
                    Box::pin(sharing_end(handle.on_end(), failure.clone())),
                    None,
                );
                picked.start(handle, failure);
                Ok(WindowShared::Started(session))
            }
            Ok(ShareOutcome::Declined(reason)) => Ok(WindowShared::Declined(declined_window(
                name, reason, display,
            ))),
            Ok(ShareOutcome::Unmatched) => Ok(WindowShared::Declined(format!(
                "{name} cannot show the picture that this device makes of the {}.",
                noun(display)
            ))),
            Ok(_) => Ok(WindowShared::Declined(declined_window(
                name,
                DeclineReason::Other,
                display,
            ))),
            Err(error) => Err(Error::sharing(&error, name, display)),
        }
    }
}

/// The text of a title or a name, or `None` if there is none.
fn non_empty(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// How a window that this device shares ended, as a screen is told, once its session is over.
async fn sharing_end(ended: impl Future<Output = End>, failure: SharingFailure) -> WindowEnd {
    sharing_outcome(ended.await, failure.message())
}

/// How a window that this device shares ended, as a screen is told: with the sentence that the
/// platform gave if it ended the session to say that something went wrong, made fit for a screen,
/// and as any other session otherwise. A session that the other device or the connection ended
/// first is told as that, whatever the platform says afterwards.
fn sharing_outcome(end: End, sentence: Option<String>) -> WindowEnd {
    if end == End::Local {
        let sentence =
            sentence.map(|message| Error::new(ErrorKind::Other, message).message().to_owned());
        if let Some(sentence) = sentence.filter(|sentence| !sentence.is_empty()) {
            return WindowEnd::Failed(sentence);
        }
    }
    window_end(end)
}

/// How a session ended, as a screen is told.
fn window_end(end: End) -> WindowEnd {
    match end {
        End::Local => WindowEnd::ThisDevice,
        End::Peer => WindowEnd::OtherDevice,
        End::Connection | End::Silent | End::Stalled => WindowEnd::Lost,
        End::NoControl => WindowEnd::Failed(
            "The other device did not start sending the window in time.".to_owned(),
        ),
        End::Violation => WindowEnd::Failed(
            "The other device sent something that this version of Baton does not understand."
                .to_owned(),
        ),
        _ => WindowEnd::Failed("Something went wrong inside Baton.".to_owned()),
    }
}

/// What to tell the person when the other device did not take the window, or the screen.
fn declined_window(device: &str, reason: DeclineReason, display: bool) -> String {
    let what = noun(display);
    match reason {
        DeclineReason::Refused => format!("{device} did not accept the {what}."),
        DeclineReason::Blocked => {
            format!(
                "{device} does not take {what}s from this device: its settings do not allow it."
            )
        }
        DeclineReason::Unsupported => {
            format!("{device} cannot show {what}s. It may need a newer version of Baton.")
        }
        // What the other device is busy with is a window or a screen, whichever was offered.
        DeclineReason::RateLimited => {
            format!("{device} is busy with another window or screen. Try again in a moment.")
        }
        DeclineReason::Invalid => format!("{device} could not read the offer of the {what}."),
        _ => format!("{device} did not take the {what}."),
    }
}

/// The handler that the node asks about windows.
pub(crate) struct Handler(pub(crate) Arc<Sessions>);

impl WindowHandler for Handler {
    fn offered<'a>(
        &'a self,
        from: &'a PairedDevice,
        offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        Box::pin(self.0.offered(from, offer))
    }

    fn started(&self, _from: &PairedDevice, handle: ViewHandle) {
        self.0.started(handle);
    }
}

#[cfg(test)]
mod tests {
    use baton_proto::{id::SessionId, stream::Codec};
    use baton_stream::{
        control::{Config, Pause, PauseReason, SoundCodec, Thermal},
        gate::Reason,
        header::Flags,
    };
    use tokio::sync::broadcast::error::TryRecvError;

    use super::*;
    use crate::model::{WindowButton, WindowModifiers, WindowNoticeReason, WindowPointer};

    fn picture(id: u64, width: u32, height: u32) -> Event {
        Event::WindowPicture { id, width, height }
    }

    fn config(width: u32, height: u32) -> SourceMessage {
        SourceMessage::Config(Config {
            epoch: 0,
            codec: Codec::H264,
            width,
            height,
            fps: 30,
        })
    }

    fn pictures() -> (Arc<Pictures>, broadcast::Receiver<Event>) {
        logging();
        let (events, heard) = broadcast::channel(8);
        (Pictures::new(events), heard)
    }

    #[test]
    fn a_size_that_comes_when_the_window_is_known_is_told_at_once() {
        let (pictures, mut heard) = pictures();
        pictures.began(3);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.said(1280, 800);
        assert_eq!(heard.try_recv(), Ok(picture(3, 1280, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn a_size_that_comes_before_the_window_is_known_is_told_when_it_is() {
        let (pictures, mut heard) = pictures();
        pictures.said(1280, 800);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.began(3);
        assert_eq!(heard.try_recv(), Ok(picture(3, 1280, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn of_the_sizes_that_came_before_the_window_is_known_the_last_is_told() {
        let (pictures, mut heard) = pictures();
        pictures.said(1280, 800);
        pictures.said(640, 400);
        pictures.began(3);
        assert_eq!(heard.try_recv(), Ok(picture(3, 640, 400)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn every_new_size_is_told_in_the_order_it_came() {
        let (pictures, mut heard) = pictures();
        pictures.began(3);
        pictures.said(1280, 800);
        pictures.said(800, 800);
        pictures.said(1280, 800);
        for expected in [(1280, 800), (800, 800), (1280, 800)] {
            assert_eq!(heard.try_recv(), Ok(picture(3, expected.0, expected.1)));
        }
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    fn notice(reason: Reason, count: u32) -> Notice {
        Notice { reason, count }
    }

    fn noticed(id: u64, reason: WindowNoticeReason, count: u32) -> Event {
        Event::WindowNotice { id, reason, count }
    }

    #[test]
    fn a_notice_that_comes_when_the_window_is_known_is_told_at_once() {
        let (pictures, mut heard) = pictures();
        pictures.began(3);
        pictures.noticed(notice(Reason::Covered, 4));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(3, WindowNoticeReason::Covered, 4))
        );
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    /// What the logger of the tests wrote, whichever test it was for.
    static LOGGED: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

    /// Where the logger of the tests writes: a place that a test can read.
    #[derive(Clone, Copy)]
    struct Captured;

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            LOGGED.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl tracing_subscriber::fmt::MakeWriter<'_> for Captured {
        type Writer = Self;

        fn make_writer(&self) -> Self {
            *self
        }
    }

    /// Puts the logger of the tests in place, once for all of them, and it is in place before
    /// any test runs the line that logs (`pictures` calls this). A logger that only the thread
    /// of one test has can miss the line of that test: the first thread that runs the line with
    /// no logger of its own makes the logging library decide, for every thread, that nobody
    /// listens to that line, and the test then waits for a line that is never written.
    fn logging() {
        static IN_PLACE: std::sync::Once = std::sync::Once::new();
        IN_PLACE.call_once(|| {
            tracing::subscriber::set_global_default(
                tracing_subscriber::fmt()
                    .with_writer(Captured)
                    .with_ansi(false)
                    .with_target(false)
                    .with_thread_names(true)
                    .finish(),
            )
            .expect("no other test puts a logger in place");
        });
    }

    /// The lines that the logger of the tests wrote for the test that calls this: the lines that
    /// were written on its thread, which is named after the test. The logger writes the name with
    /// a space on each side, so a test whose name is the start of another's does not read its
    /// lines.
    fn logged() -> Vec<String> {
        let thread = std::thread::current();
        let name = thread.name().expect("the thread of a test has its name");
        let text = String::from_utf8(LOGGED.lock().unwrap().clone()).unwrap();
        text.lines()
            .filter(|line| line.contains(&format!(" {name} ")))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_notice_is_logged_with_its_reason_and_how_many_events_it_is_about() {
        let (pictures, _heard) = pictures();
        pictures.began(3);
        pictures.noticed(notice(Reason::FrameUnknown, 4));
        let lines = logged();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("INFO"), "{lines:?}");
        assert!(lines[0].contains("reason=FrameUnknown"), "{lines:?}");
        assert!(lines[0].contains("count=4"), "{lines:?}");
    }

    #[test]
    fn a_notice_that_comes_before_the_window_is_known_is_not_logged() {
        let (pictures, _heard) = pictures();
        pictures.noticed(notice(Reason::Locked, 2));
        assert_eq!(logged(), Vec::<String>::new());
    }

    #[test]
    fn a_notice_that_comes_before_the_window_is_known_is_dropped_and_not_told_later() {
        let (pictures, mut heard) = pictures();
        pictures.noticed(notice(Reason::Locked, 2));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.began(3);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.noticed(notice(Reason::Locked, 1));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(3, WindowNoticeReason::Locked, 1))
        );
    }

    #[test]
    fn a_notice_is_told_in_its_order_among_the_sizes_of_the_picture() {
        let (pictures, mut heard) = pictures();
        pictures.said(1280, 800);
        pictures.began(3);
        pictures.noticed(notice(Reason::WindowResized, 5));
        pictures.said(800, 800);
        assert_eq!(heard.try_recv(), Ok(picture(3, 1280, 800)));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(3, WindowNoticeReason::Resized, 5))
        );
        assert_eq!(heard.try_recv(), Ok(picture(3, 800, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn every_reason_the_other_device_gives_has_a_reason_for_the_screens() {
        use WindowNoticeReason as Shown;

        for (reason, shown) in [
            (Reason::ControlOff, Shown::ControlOff),
            (Reason::Paused, Shown::Paused),
            (Reason::Locked, Shown::Locked),
            (Reason::NotOnScreen, Shown::NotOnScreen),
            (Reason::TooFast, Shown::TooFast),
            (Reason::FrameUnknown, Shown::PictureUnknown),
            (Reason::PictureOutOfDate, Shown::OutOfDate),
            (Reason::WindowResized, Shown::Resized),
            (Reason::OutsideWindow, Shown::OutsideWindow),
            (Reason::Covered, Shown::Covered),
            (Reason::NoKeyboard, Shown::NoKeyboard),
            (Reason::AppWindowInFront, Shown::AppWindowInFront),
            (Reason::TooManyHeld, Shown::TooManyKeys),
            (Reason::NotPressed, Shown::NotPressed),
            (Reason::Malformed, Shown::Other),
            (Reason::Other, Shown::Other),
        ] {
            assert_eq!(Shown::from(reason), shown, "{reason:?}");
        }
    }

    #[tokio::test]
    async fn input_for_a_window_that_is_shared_from_here_or_unknown_is_not_taken() {
        let (events, _) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events);
        locked(&sessions.live).insert(7, Live::Sharing(Weak::new()));
        let touch = WindowInput::Pointer {
            x: 0.5,
            y: 0.5,
            modifiers: WindowModifiers::default(),
            action: WindowPointer::Down(WindowButton::Left),
        };
        assert!(!sessions.input(7, touch.clone()));
        assert!(!sessions.input(8, touch));
        // Nothing to let go of, and nothing goes wrong.
        sessions.release_input(7);
        sessions.release_input(8);
    }

    fn window(id: u64, role: WindowRole) -> WindowSession {
        WindowSession {
            id,
            role,
            device_id: "ab".repeat(32),
            device_name: "Studio Mac".to_owned(),
            display: false,
            title: None,
            app: None,
            width: 1440,
            height: 900,
            control: false,
            private: false,
            sound: false,
            leave: false,
            unlock: false,
            screen: Some(100),
        }
    }

    #[tokio::test]
    async fn the_screens_hear_of_a_window_before_they_hear_of_its_picture() {
        let (events, mut heard) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events.clone());
        let pictures = Pictures::new(events);
        // The source has said the size already when the app makes the window known.
        pictures.said(1280, 800);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        let shown = window(6, WindowRole::Viewing);

        // What the entry holds does not matter here, so it is the one that needs no handle.
        sessions.begin(
            shown.clone(),
            Live::Sharing(Weak::new()),
            Box::pin(std::future::pending()),
            Some(&pictures),
        );

        assert_eq!(heard.try_recv(), Ok(Event::WindowStarted(shown)));
        assert_eq!(heard.try_recv(), Ok(picture(6, 1280, 800)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[tokio::test]
    async fn a_window_that_has_no_pictures_to_wait_for_is_told_alone() {
        let (events, mut heard) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events);
        let shared = window(2, WindowRole::Sharing);

        sessions.begin(
            shared.clone(),
            Live::Sharing(Weak::new()),
            Box::pin(std::future::pending()),
            None,
        );

        assert_eq!(heard.try_recv(), Ok(Event::WindowStarted(shared)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    /// A decoder that writes down what it is given.
    struct Recorder(Arc<Mutex<Vec<String>>>);

    impl Decoder for Recorder {
        fn decode(
            &mut self,
            frame: &FrameHeader,
            picture: Vec<u8>,
        ) -> std::result::Result<(), DecodeError> {
            locked(&self.0).push(format!(
                "decode {} of {} bytes",
                frame.number,
                picture.len()
            ));
            Ok(())
        }

        fn show(&mut self, number: u64) {
            locked(&self.0).push(format!("show {number}"));
        }

        fn source(&mut self, message: SourceMessage) {
            locked(&self.0).push(format!("source {message:?}"));
        }

        fn state(&self) -> DecoderState {
            DecoderState {
                decode_us: 1234,
                thermal: Thermal::Fair,
            }
        }

        fn ended(&mut self) {
            locked(&self.0).push("ended".to_owned());
        }
    }

    fn watching() -> (
        Watching,
        Arc<Mutex<Vec<String>>>,
        broadcast::Receiver<Event>,
    ) {
        let (pictures, heard) = pictures();
        pictures.began(5);
        let written = Arc::new(Mutex::new(Vec::new()));
        let decoder = Watching {
            decoder: Box::new(Recorder(Arc::clone(&written))),
            pictures,
        };
        (decoder, written, heard)
    }

    #[test]
    fn the_decoder_of_the_platform_is_given_everything_as_it_comes() {
        let (mut decoder, written, _) = watching();
        let header = FrameHeader {
            session: SessionId::from_bytes([7; SessionId::LEN]),
            number: 9,
            depends_on: 9,
            chain: 0,
            epoch: 0,
            capture_time: 0,
            flags: Flags::KEY,
        };
        assert!(decoder.decode(&header, vec![0; 12]).is_ok());
        decoder.show(9);
        decoder.source(SourceMessage::Resume);
        decoder.ended();
        assert_eq!(
            *locked(&written),
            ["decode 9 of 12 bytes", "show 9", "source Resume", "ended"]
        );
        let state = decoder.state();
        assert_eq!((state.decode_us, state.thermal), (1234, Thermal::Fair));
    }

    #[test]
    fn what_the_source_says_is_given_on_and_only_its_picture_and_its_lock_are_told_to_the_screens()
    {
        let (mut decoder, written, mut heard) = watching();
        decoder.source(config(1000, 500));
        assert_eq!(heard.try_recv(), Ok(picture(5, 1000, 500)));
        for message in [
            SourceMessage::Pause(Pause {
                reason: PauseReason::Hidden,
            }),
            SourceMessage::Resume,
        ] {
            decoder.source(message);
        }
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        let written = locked(&written);
        assert_eq!(written.len(), 3);
        assert!(written[0].starts_with("source Config("), "{written:?}");
        assert!(written[1].starts_with("source Pause("), "{written:?}");
        assert_eq!(written[2], "source Resume");
    }

    #[test]
    fn a_notice_of_the_source_is_given_on_to_the_decoder_and_told_to_the_screens() {
        let (mut decoder, written, mut heard) = watching();
        decoder.source(SourceMessage::Notice(notice(Reason::NoKeyboard, 2)));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(5, WindowNoticeReason::NoKeyboard, 2))
        );
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        let written = locked(&written);
        assert_eq!(written.len(), 1);
        assert!(written[0].starts_with("source Notice("), "{written:?}");
    }

    fn lock(id: u64, is_locked: bool) -> Event {
        Event::WindowLocked {
            id,
            locked: is_locked,
        }
    }

    fn step(id: u64, step: WindowUnlock) -> Event {
        Event::WindowUnlock { id, step }
    }

    fn nothing_more(heard: &mut broadcast::Receiver<Event>) {
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn a_screen_that_is_locked_when_the_window_starts_is_told_right_after_the_window() {
        let (pictures, mut heard) = pictures();
        pictures.paused(PauseReason::Locked);
        // The screens do not know the window yet, so they hear nothing.
        nothing_more(&mut heard);
        pictures.began(5);
        assert_eq!(heard.try_recv(), Ok(lock(5, true)));
        nothing_more(&mut heard);
    }

    #[test]
    fn a_screen_that_was_locked_and_unlocked_before_the_window_starts_is_told_nothing() {
        let (pictures, mut heard) = pictures();
        pictures.paused(PauseReason::Locked);
        pictures.resumed();
        pictures.began(5);
        nothing_more(&mut heard);
    }

    #[test]
    fn the_lock_comes_after_the_size_that_the_source_said_before_the_window_was_known() {
        let (pictures, mut heard) = pictures();
        pictures.said(1000, 500);
        pictures.paused(PauseReason::Locked);
        pictures.began(5);
        assert_eq!(heard.try_recv(), Ok(picture(5, 1000, 500)));
        assert_eq!(heard.try_recv(), Ok(lock(5, true)));
        nothing_more(&mut heard);
    }

    #[test]
    fn only_a_change_of_the_lock_is_told_and_only_a_pause_because_of_it_locks() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        // Not locked, so a resume changes nothing, and a window that is hidden is not a lock.
        pictures.resumed();
        pictures.paused(PauseReason::Hidden);
        pictures.paused(PauseReason::Other);
        nothing_more(&mut heard);

        pictures.paused(PauseReason::Locked);
        assert_eq!(heard.try_recv(), Ok(lock(5, true)));
        // Another reason for a pause does not unlock a screen that stays locked, and the same news
        // twice is told once.
        pictures.paused(PauseReason::Locked);
        pictures.paused(PauseReason::Hidden);
        nothing_more(&mut heard);

        pictures.resumed();
        assert_eq!(heard.try_recv(), Ok(lock(5, false)));
        pictures.resumed();
        nothing_more(&mut heard);
    }

    #[test]
    fn the_lock_of_the_source_is_told_to_the_screens_as_the_decoder_hears_it() {
        let (mut decoder, written, mut heard) = watching();
        decoder.source(SourceMessage::Pause(Pause {
            reason: PauseReason::Locked,
        }));
        assert_eq!(heard.try_recv(), Ok(lock(5, true)));
        decoder.source(SourceMessage::Resume);
        assert_eq!(heard.try_recv(), Ok(lock(5, false)));
        nothing_more(&mut heard);
        // The decoder of the platform heard both, as it hears all.
        assert_eq!(locked(&written).len(), 2);
    }

    #[test]
    fn the_steps_of_an_unlock_are_told_in_their_order_and_the_attempt_ends_once() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let number = pictures.asking();
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Asking)));
        assert!(pictures.answered(number, WindowUnlock::Typing));
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Typing)));
        pictures.settled(UnlockResult::Unlocked);
        assert_eq!(
            heard.try_recv(),
            Ok(step(5, WindowUnlock::Ended(UnlockResult::Unlocked)))
        );
        nothing_more(&mut heard);
    }

    #[test]
    fn a_question_is_answered_once() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let number = pictures.asking();
        assert!(pictures.answered(number, WindowUnlock::Typing));
        assert!(!pictures.answered(number, WindowUnlock::Typing));
        assert!(!pictures.answered(number, WindowUnlock::Ended(UnlockResult::Declined)));
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Asking)));
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Typing)));
        nothing_more(&mut heard);
    }

    #[test]
    fn an_answer_to_a_question_that_was_taken_back_is_dropped_and_no_step_follows_the_end() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let number = pictures.asking();
        // The source gave up on the question: the session takes it back and settles the attempt.
        pictures.taken_back();
        pictures.settled(UnlockResult::NotLocked);
        // The face is recognised a moment later.
        assert!(!pictures.answered(number, WindowUnlock::Typing));
        assert!(!pictures.answered(number, WindowUnlock::Ended(UnlockResult::Declined)));
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Asking)));
        assert_eq!(
            heard.try_recv(),
            Ok(step(5, WindowUnlock::Ended(UnlockResult::NotLocked)))
        );
        nothing_more(&mut heard);
    }

    #[test]
    fn an_answer_to_an_older_question_is_not_taken_for_the_newer_one() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let old = pictures.asking();
        pictures.taken_back();
        pictures.settled(UnlockResult::NotLocked);
        let new = pictures.asking();
        assert_ne!(old, new);
        assert!(!pictures.answered(old, WindowUnlock::Typing));
        assert!(pictures.answered(new, WindowUnlock::Typing));
        let steps: Vec<_> = std::iter::from_fn(|| heard.try_recv().ok()).collect();
        assert_eq!(
            steps,
            [
                step(5, WindowUnlock::Asking),
                step(5, WindowUnlock::Ended(UnlockResult::NotLocked)),
                step(5, WindowUnlock::Asking),
                step(5, WindowUnlock::Typing),
            ]
        );
    }

    #[test]
    fn an_attempt_that_the_platform_declined_ends_once_even_if_the_source_gave_up_at_the_same_time()
    {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let number = pictures.asking();
        let declined = WindowUnlock::Ended(UnlockResult::Declined);
        assert!(pictures.answered(number, declined));
        // The session read the end of the source before it read the answer of the platform: it
        // takes the question back and settles the attempt, and the person was told already.
        pictures.taken_back();
        pictures.settled(UnlockResult::NotLocked);
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Asking)));
        assert_eq!(heard.try_recv(), Ok(step(5, declined)));
        nothing_more(&mut heard);

        // An attempt that gets no question comes next, and asks for nothing: it is told as well.
        pictures.settled(UnlockResult::NoQuestion);
        assert_eq!(
            heard.try_recv(),
            Ok(step(5, WindowUnlock::Ended(UnlockResult::NoQuestion)))
        );
        nothing_more(&mut heard);

        // The next attempt is told in full.
        let next = pictures.asking();
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Asking)));
        assert!(pictures.answered(next, WindowUnlock::Typing));
        pictures.settled(UnlockResult::Unlocked);
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Typing)));
        assert_eq!(
            heard.try_recv(),
            Ok(step(5, WindowUnlock::Ended(UnlockResult::Unlocked)))
        );
        nothing_more(&mut heard);
    }

    #[test]
    fn an_attempt_that_the_platform_declined_and_nothing_settled_leaves_the_next_one_to_be_told() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let number = pictures.asking();
        let declined = WindowUnlock::Ended(UnlockResult::KeyLost);
        assert!(pictures.answered(number, declined));
        // The session took the decline and went idle: it calls nothing. The next attempt gets no
        // question at all, because the source says that the screen is not locked.
        pictures.settled(UnlockResult::NotLocked);
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Asking)));
        assert_eq!(heard.try_recv(), Ok(step(5, declined)));
        assert_eq!(
            heard.try_recv(),
            Ok(step(5, WindowUnlock::Ended(UnlockResult::NotLocked)))
        );
        nothing_more(&mut heard);
    }

    #[test]
    fn an_end_that_the_platform_told_and_nothing_settled_does_not_swallow_the_next_one() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let number = pictures.asking();
        let declined = WindowUnlock::Ended(UnlockResult::Declined);
        assert!(pictures.answered(number, declined));
        // The session took the question back, and never settled the attempt.
        pictures.taken_back();

        let next = pictures.asking();
        assert!(pictures.answered(next, WindowUnlock::Typing));
        pictures.settled(UnlockResult::Unlocked);
        let steps: Vec<_> = std::iter::from_fn(|| heard.try_recv().ok()).collect();
        assert_eq!(
            steps,
            [
                step(5, WindowUnlock::Asking),
                step(5, declined),
                step(5, WindowUnlock::Asking),
                step(5, WindowUnlock::Typing),
                step(5, WindowUnlock::Ended(UnlockResult::Unlocked)),
            ]
        );
    }

    #[test]
    fn an_answer_after_the_attempt_was_settled_is_dropped_whether_or_not_it_was_taken_back() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        let number = pictures.asking();
        pictures.settled(UnlockResult::NotLocked);
        assert!(!pictures.answered(number, WindowUnlock::Typing));
        assert!(!pictures.answered(number, WindowUnlock::Ended(UnlockResult::Declined)));
        assert_eq!(heard.try_recv(), Ok(step(5, WindowUnlock::Asking)));
        assert_eq!(
            heard.try_recv(),
            Ok(step(5, WindowUnlock::Ended(UnlockResult::NotLocked)))
        );
        nothing_more(&mut heard);
    }

    #[test]
    fn an_attempt_that_gets_no_question_ends_with_what_settled_it() {
        let (pictures, mut heard) = pictures();
        pictures.began(5);
        pictures.settled(UnlockResult::NoQuestion);
        assert_eq!(
            heard.try_recv(),
            Ok(step(5, WindowUnlock::Ended(UnlockResult::NoQuestion)))
        );
        nothing_more(&mut heard);
    }

    #[test]
    fn the_steps_of_an_unlock_are_dropped_while_the_window_is_not_known() {
        let (pictures, mut heard) = pictures();
        let number = pictures.asking();
        assert!(pictures.answered(number, WindowUnlock::Typing));
        pictures.settled(UnlockResult::Unlocked);
        pictures.began(5);
        nothing_more(&mut heard);
    }

    #[test]
    fn an_end_is_told_as_a_screen_needs_it() {
        assert_eq!(window_end(End::Local), WindowEnd::ThisDevice);
        assert_eq!(window_end(End::Peer), WindowEnd::OtherDevice);
        for lost in [End::Connection, End::Silent, End::Stalled] {
            assert_eq!(window_end(lost), WindowEnd::Lost, "{lost:?}");
        }
        for failed in [End::NoControl, End::Violation, End::Internal] {
            assert!(
                matches!(window_end(failed), WindowEnd::Failed(_)),
                "{failed:?}"
            );
        }
    }

    #[test]
    fn a_window_that_the_platform_ended_is_told_with_its_sentence_if_it_gave_one() {
        assert_eq!(
            sharing_outcome(End::Local, Some("The capture stopped.".to_owned())),
            WindowEnd::Failed("The capture stopped.".to_owned())
        );
        // The sentence is made fit for a screen.
        assert_eq!(
            sharing_outcome(
                End::Local,
                Some("The capture\nstopped.\u{202e}\u{7}".to_owned())
            ),
            WindowEnd::Failed("The capture stopped.".to_owned())
        );
        // No sentence, and a sentence with nothing in it, are the person closing the window.
        for nothing in [
            None,
            Some(String::new()),
            Some(" \n\t".to_owned()),
            Some("\u{200b}".to_owned()),
        ] {
            assert_eq!(
                sharing_outcome(End::Local, nothing.clone()),
                WindowEnd::ThisDevice,
                "{nothing:?}"
            );
        }
    }

    #[test]
    fn a_window_that_something_else_ended_is_told_as_that_whatever_the_platform_says() {
        for end in [
            End::Peer,
            End::Connection,
            End::Silent,
            End::Stalled,
            End::NoControl,
            End::Violation,
            End::Internal,
        ] {
            for sentence in [None, Some("The capture stopped.".to_owned())] {
                assert_eq!(
                    sharing_outcome(end, sentence.clone()),
                    window_end(end),
                    "{end:?}: {sentence:?}"
                );
            }
        }
    }

    #[test]
    fn ending_a_window_that_the_platform_let_go_of_does_nothing() {
        let (events, _) = broadcast::channel(8);
        let sessions = Sessions::new(None, None, Questions::new(events.clone()), events);
        // The entry of a session whose handle the platform dropped, before the task that waits
        // for the end has removed it: the drop ended the session, and nothing is left to end.
        locked(&sessions.live).insert(7, Live::Sharing(Weak::new()));
        assert!(!sessions.end(7));
        assert!(!sessions.end(8));
    }

    #[test]
    fn a_title_that_is_blank_is_no_title() {
        assert_eq!(non_empty("  "), None);
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty(" Report "), Some("Report".to_owned()));
    }

    #[test]
    fn what_the_person_is_told_of_a_window_that_was_not_taken_names_the_device_and_the_reason() {
        for (reason, why) in [
            (DeclineReason::Refused, "did not accept the window"),
            (DeclineReason::Blocked, "its settings do not allow it"),
            (DeclineReason::Unsupported, "may need a newer version"),
            (DeclineReason::Invalid, "could not read the offer"),
            (DeclineReason::Other, "did not take the window"),
        ] {
            let told = declined_window("Kitchen iPad", reason, false);
            assert!(told.contains("Kitchen iPad"), "{reason:?}: {told}");
            assert!(told.contains(why), "{reason:?}: {told}");
            assert!(!told.contains("screen"), "{reason:?}: {told}");
        }
    }

    #[test]
    fn what_the_person_is_told_of_a_screen_that_was_not_taken_calls_it_a_screen() {
        for (reason, why) in [
            (DeclineReason::Refused, "did not accept the screen"),
            (DeclineReason::Blocked, "does not take screens"),
            (DeclineReason::Unsupported, "cannot show screens"),
            (DeclineReason::Invalid, "offer of the screen"),
            (DeclineReason::Other, "did not take the screen"),
        ] {
            let told = declined_window("Kitchen iPad", reason, true);
            assert!(told.contains("Kitchen iPad"), "{reason:?}: {told}");
            assert!(told.contains(why), "{reason:?}: {told}");
            assert!(!told.contains("window"), "{reason:?}: {told}");
        }
    }

    #[test]
    fn a_device_that_is_busy_is_busy_with_a_window_or_a_screen_whichever_was_offered() {
        // What it is busy with is not known, and it need not be what was offered.
        for display in [false, true] {
            let told = declined_window("Kitchen iPad", DeclineReason::RateLimited, display);
            assert!(told.contains("Kitchen iPad"), "{display}: {told}");
            assert!(
                told.contains("busy with another window or screen"),
                "{display}: {told}"
            );
        }
    }

    fn sounded(id: u64, sound: WindowSound) -> Event {
        Event::WindowSound { id, sound }
    }

    #[test]
    fn the_sound_of_a_window_is_told_once_the_window_is_known_and_dropped_before() {
        let (pictures, mut heard) = pictures();
        pictures.sounded(WindowSound::Playing);
        pictures.began(3);
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
        pictures.sounded(WindowSound::Playing);
        assert_eq!(heard.try_recv(), Ok(sounded(3, WindowSound::Playing)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn the_sound_is_told_in_its_order_among_the_sizes_and_the_notices() {
        let (pictures, mut heard) = pictures();
        pictures.said(1280, 800);
        pictures.began(3);
        pictures.sounded(WindowSound::Playing);
        pictures.noticed(notice(Reason::Locked, 1));
        pictures.sounded(WindowSound::NoPermission);
        assert_eq!(heard.try_recv(), Ok(picture(3, 1280, 800)));
        assert_eq!(heard.try_recv(), Ok(sounded(3, WindowSound::Playing)));
        assert_eq!(
            heard.try_recv(),
            Ok(noticed(3, WindowNoticeReason::Locked, 1))
        );
        assert_eq!(heard.try_recv(), Ok(sounded(3, WindowSound::NoPermission)));
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn a_sound_that_stands_is_logged_with_how_it_stands() {
        let (pictures, _heard) = pictures();
        pictures.began(3);
        pictures.sounded(WindowSound::Failed);
        let lines = logged();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("INFO"), "{lines:?}");
        assert!(lines[0].contains("sound=Failed"), "{lines:?}");
    }

    /// A speaker that keeps what it was told, in order.
    #[derive(Clone, Default)]
    struct Told(Arc<Mutex<Vec<String>>>);

    impl Told {
        fn all(&self) -> Vec<String> {
            locked(&self.0).clone()
        }

        fn push(&self, what: String) {
            locked(&self.0).push(what);
        }
    }

    impl Speaker for Told {
        fn sound(&mut self, sound: Sound) {
            self.push(format!("sound {:?} {}", sound.codec, sound.samples));
        }

        fn play(&mut self, play: Play, packet: Vec<u8>) {
            self.push(format!(
                "play {} {} {}",
                play.number,
                play.lost_before,
                packet.len()
            ));
        }

        fn stopped(&mut self, why: Option<SilentReason>) {
            self.push(format!("stopped {why:?}"));
        }

        fn could_not_play(&mut self) {
            self.push("could not play".to_owned());
        }

        fn ended(&mut self) {
            self.push("ended".to_owned());
        }
    }

    const AAC: Sound = Sound {
        codec: SoundCodec::Aac,
        rate: 48_000,
        channels: 2,
        samples: 1024,
    };

    #[test]
    fn the_speaker_of_the_platform_is_told_all_of_it_and_the_screens_hear_how_the_sound_stands() {
        let (pictures, mut heard) = pictures();
        pictures.began(3);
        let platform = Told::default();
        let mut speaker = Listening {
            speaker: Box::new(platform.clone()),
            pictures,
        };
        let play = Play {
            number: 4,
            capture_time: 90_000,
            first: false,
            lost_before: 2,
        };
        speaker.sound(AAC);
        speaker.play(play, vec![1, 2, 3]);
        speaker.stopped(Some(SilentReason::NoPermission));
        speaker.sound(AAC);
        speaker.stopped(None);
        speaker.sound(AAC);
        speaker.could_not_play();
        speaker.ended();
        assert_eq!(
            platform.all(),
            [
                "sound Aac 1024",
                "play 4 2 3",
                "stopped Some(NoPermission)",
                "sound Aac 1024",
                "stopped None",
                "sound Aac 1024",
                "could not play",
                "ended",
            ]
        );
        // The packets are not events, and the end of the session tells the screens nothing: they
        // hear of that through the end of the window.
        for expected in [
            WindowSound::Playing,
            WindowSound::NoPermission,
            WindowSound::Playing,
            WindowSound::Off,
            WindowSound::Playing,
            WindowSound::CannotPlay,
        ] {
            assert_eq!(heard.try_recv(), Ok(sounded(3, expected)));
        }
        assert_eq!(heard.try_recv(), Err(TryRecvError::Empty));
    }

    #[test]
    fn each_reason_the_source_gives_for_sending_no_sound_is_told_as_itself() {
        for (reason, expected) in [
            (SilentReason::Window, WindowSound::Window),
            (SilentReason::NoPermission, WindowSound::NoPermission),
            (SilentReason::Failed, WindowSound::Failed),
            (SilentReason::Other, WindowSound::Other),
        ] {
            assert_eq!(WindowSound::from(reason), expected, "{reason:?}");
        }
    }
}
