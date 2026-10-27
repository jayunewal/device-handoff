// SPDX-License-Identifier: Apache-2.0

//! Unlocking a Mac from a phone the way the screens drive it: two apps on this computer, paired,
//! with platforms that are played by the test. The Mac keeps a password in a box that only the
//! phone opens, and the phone asks for a face; the "cryptography" of the platforms is a stand-in
//! that is easy to read in a failure (the box holds its key and the password in plain bytes), since
//! what is under test is what the app does with the pieces and in which order.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicI64, AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use baton_app::{
    App, ErrorKind, Event, Events, Options, ScreenStarted, StartAccess, UnlockEntry, UnlockResult,
    UnlockSetUp, UnlockStatus, WindowAbilities, WindowEnd, WindowSession, WindowShared,
    WindowUnlock, Windows,
    unlock::{
        FaceUnlocker, Password, UnlockKeyError, UnlockQuestion, UnlockSetup, UnlockSetupError,
        UnlockSource,
    },
    windows::{
        PickedWindow, PlatformError, Privacy, Screen, ScreenError, ScreenLink, SharingFailure,
        ShowLimits, WindowSharer, WindowViewer,
    },
};
use baton_core::{
    NewUnlock,
    streaming::{
        CancelReason, DecodeError, Decoder, DecoderState, Picture, SourceEvents, SourceHandle,
        Takes, UnlockAttempt, UnlockOutcome, WindowOffer,
    },
    util::BoxFuture,
};
use baton_proto::{
    stream::Codec,
    unlock::{BoxKey, SealedKey, Sealing, SealingKey, UnlockKey},
};
use baton_stream::{
    control::{Config, InputBatch, PauseReason, Report, SourceMessage, Thermal, Want},
    header::FrameHeader,
    sender::LtrToken,
};
use tempfile::TempDir;
use tokio::sync::oneshot;

/// Long enough for a slow machine, short enough that a hang does not take the run with it.
const WAIT: Duration = Duration::from_secs(30);

/// Long enough for something that should not happen to have happened.
const QUIET: Duration = Duration::from_millis(500);

/// What a platform heard, in order.
#[derive(Default)]
struct Heard(Mutex<Vec<String>>);

impl Heard {
    fn push(&self, what: impl Into<String>) {
        self.0.lock().unwrap().push(what.into());
    }

    fn has(&self, what: &str) -> bool {
        self.0.lock().unwrap().iter().any(|heard| heard == what)
    }

    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn count(&self, prefix: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|heard| heard.starts_with(prefix))
            .count()
    }
}

/// Waits until `condition` holds.
async fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

// The stand-in for the cryptography of the platforms. The box is the key and then the password; the
// sealed key is a tag and then the key; a key is opened by reading it out.

const SEALED_TAG: &[u8] = b"sealed:";

fn sealed_key(key: &[u8; BoxKey::LEN]) -> SealedKey {
    SealedKey::new([SEALED_TAG, key.as_slice()].concat())
}

fn boxed_password(key: &[u8; BoxKey::LEN], password: &str) -> Vec<u8> {
    [key.as_slice(), password.as_bytes()].concat()
}

/// What the face of the person opens the sealed key to.
fn opened(sealed: &SealedKey) -> BoxKey {
    let key: [u8; BoxKey::LEN] = sealed.as_bytes()[SEALED_TAG.len()..].try_into().unwrap();
    BoxKey::from_bytes(key)
}

/// The key of the next box, which is the platform putting the password in a new box.
fn next_key(key: &[u8; BoxKey::LEN]) -> [u8; BoxKey::LEN] {
    let mut next = *key;
    next[0] = next[0].wrapping_add(1);
    next
}

/// A key that the phone made for a Mac: a point of P-256 in the uncompressed form, as far as the
/// core can tell.
fn phone_key(tag: u8) -> UnlockKey {
    let mut point = vec![tag; 65];
    point[0] = 0x04;
    UnlockKey {
        sealing: Sealing::EciesP256,
        public: SealingKey::new(point),
    }
}

// The platform of a phone, which shows screens and unlocks them with a face.

/// What asks for a face at the phone, and the questions that wait for it.
#[derive(Default)]
struct Faces {
    log: Heard,
    /// The questions that wait for the face of the person.
    waiting: Mutex<Vec<UnlockQuestion>>,
    /// Whether a question that comes is dropped at once, without a word.
    drops: AtomicBool,
}

impl Faces {
    /// Waits for a question to come, and takes it.
    async fn question(&self) -> UnlockQuestion {
        until("a question to come", || {
            !self.waiting.lock().unwrap().is_empty()
        })
        .await;
        self.waiting.lock().unwrap().remove(0)
    }
}

struct Face(Arc<Faces>);

impl FaceUnlocker for Face {
    fn open(&mut self, question: UnlockQuestion) {
        self.0.log.push(format!(
            "open for {} with {} bytes",
            question.source().name,
            question.sealed().as_bytes().len()
        ));
        if !self.0.drops.load(Ordering::Relaxed) {
            self.0.waiting.lock().unwrap().push(question);
        }
    }

    fn cancel(&mut self) {
        self.0.log.push("cancel");
    }

    fn ended(&mut self) {
        self.0.log.push("ended");
    }
}

struct Phone {
    next_screen: AtomicI64,
    /// What the platforms of both devices did, in order.
    journal: Arc<Heard>,
    /// Whether the platform unlocks screens with a face.
    unlocks: bool,
    faces: Arc<Faces>,
    /// What the Mac asked for a key with, in order: the device and whether to renew.
    asked: Mutex<Vec<(UnlockSource, bool)>>,
    /// What the next call for a key gives.
    key: Mutex<Result<UnlockKey, UnlockKeyError>>,
    /// Waited for by the next call for a key before it gives its answer, if set.
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    /// The decoders of the screens.
    pictures: Arc<Heard>,
}

impl Phone {
    fn new(unlocks: bool, journal: &Arc<Heard>) -> Arc<Self> {
        Arc::new(Self {
            next_screen: AtomicI64::new(100),
            journal: Arc::clone(journal),
            unlocks,
            faces: Arc::new(Faces::default()),
            asked: Mutex::new(Vec::new()),
            key: Mutex::new(Ok(phone_key(7))),
            gate: Mutex::new(None),
            pictures: Arc::new(Heard::default()),
        })
    }

    fn asked(&self) -> Vec<(UnlockSource, bool)> {
        self.asked.lock().unwrap().clone()
    }
}

impl WindowViewer for Phone {
    fn limits(&self) -> ShowLimits {
        ShowLimits {
            max_width: 1920,
            max_height: 1200,
            max_fps: 60,
            codecs: vec![Codec::H264],
        }
    }

    fn open(&self, _window: &WindowOffer, _link: ScreenLink) -> Result<Screen, PlatformError> {
        let screen = Screen::new(
            Box::new(Screening(Arc::clone(&self.pictures))),
            self.next_screen.fetch_add(1, Ordering::Relaxed),
        );
        Ok(if self.unlocks {
            screen.with_unlocker(Face(Arc::clone(&self.faces)))
        } else {
            screen
        })
    }

    fn can_unlock(&self) -> bool {
        self.unlocks
    }

    fn unlock_key(
        &self,
        source: UnlockSource,
        renew: bool,
    ) -> BoxFuture<'static, Result<UnlockKey, UnlockKeyError>> {
        self.journal
            .push(format!("key for {} renew {renew}", source.name));
        self.asked.lock().unwrap().push((source, renew));
        let answer = self.key.lock().unwrap().clone();
        let gate = self.gate.lock().unwrap().take();
        Box::pin(async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            answer
        })
    }
}

struct Screening(Arc<Heard>);

impl Decoder for Screening {
    fn decode(&mut self, frame: &FrameHeader, _picture: Vec<u8>) -> Result<(), DecodeError> {
        self.0.push(format!("decode {}", frame.number));
        Ok(())
    }

    fn show(&mut self, number: u64) {
        self.0.push(format!("show {number}"));
    }

    fn source(&mut self, _message: SourceMessage) {}

    fn state(&self) -> DecoderState {
        DecoderState {
            decode_us: 1000,
            thermal: Thermal::Nominal,
        }
    }

    fn ended(&mut self) {
        self.0.push("ended");
    }
}

// The platform of a Mac, which gives its screen, keeps a password in a box and types it.

/// What the platform was given to make a box.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Made {
    password: String,
    target: String,
    viewer: String,
    key: UnlockKey,
}

/// What a platform of a Mac can do, each of its own.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
struct Can {
    /// Post input.
    control: bool,
    /// Lift what keeps its screen private.
    lift: bool,
    /// Let a paired device unlock it.
    unlock: bool,
}

impl Can {
    const ALL: Self = Self {
        control: true,
        lift: true,
        unlock: true,
    };
}

struct Mac {
    /// The platform itself, for the windows that it makes.
    me: Weak<Mac>,
    can: Can,
    /// The password the system accepts.
    account: Mutex<String>,
    /// What the platforms of both devices did, in order: `check`, `setup`, `unlock`.
    journal: Arc<Heard>,
    /// What the boxes were made of, in order.
    made: Mutex<Vec<Made>>,
    /// If set, the platform fails to make the box with this.
    setup_fails: Mutex<Option<UnlockSetupError>>,
    /// The key of the box that the next set-up makes.
    next_box_key: AtomicU8,
    /// The passwords that were typed into the lock screen, in order.
    typed: Mutex<Vec<String>>,
    /// The session that runs, for the test to lock and unlock the screen.
    handle: Mutex<Option<Arc<SourceHandle>>>,
    /// Whether the screen is locked as soon as the session starts.
    locked_at_start: AtomicBool,
    /// Whether the box that the next set-up makes is empty, which no record can keep.
    makes_empty_box: AtomicBool,
    /// Run by the next set-up once it made the box, before it gives it back.
    during_setup: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Whether the person picks the whole screen when the app asks for a window to offer.
    picks_screen: AtomicBool,
}

impl Mac {
    /// A platform that can be unlocked, and whose system accepts `account`.
    fn new(account: &str, journal: &Arc<Heard>) -> Arc<Self> {
        Self::building(Can::ALL, account, journal)
    }

    fn building(can: Can, account: &str, journal: &Arc<Heard>) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            can,
            account: Mutex::new(account.to_owned()),
            journal: Arc::clone(journal),
            made: Mutex::new(Vec::new()),
            setup_fails: Mutex::new(None),
            next_box_key: AtomicU8::new(1),
            typed: Mutex::new(Vec::new()),
            handle: Mutex::new(None),
            locked_at_start: AtomicBool::new(false),
            makes_empty_box: AtomicBool::new(false),
            during_setup: Mutex::new(None),
            picks_screen: AtomicBool::new(false),
        })
    }

    fn made(&self) -> Vec<Made> {
        self.made.lock().unwrap().clone()
    }

    fn typed(&self) -> Vec<String> {
        self.typed.lock().unwrap().clone()
    }

    /// The system locks the screen.
    fn locks(&self) {
        self.handle().pause(PauseReason::Locked);
    }

    fn handle(&self) -> Arc<SourceHandle> {
        self.handle.lock().unwrap().clone().expect("no session")
    }

    /// The unlock of the platform: opens the box with the key, puts the password in a new box,
    /// types it, and says what came of it. It runs on a thread of its own.
    fn carry_out(&self, mut attempt: UnlockAttempt) {
        self.journal.push("unlock");
        let key = *attempt.key().expose();
        let boxed = attempt.boxed().to_vec();
        if boxed.len() < BoxKey::LEN || boxed[..BoxKey::LEN] != key {
            attempt.done(UnlockOutcome::Stale);
            return;
        }
        let password = String::from_utf8(boxed[BoxKey::LEN..].to_vec()).unwrap();
        let next = next_key(&key);
        if attempt
            .keep(boxed_password(&next, &password), sealed_key(&next))
            .is_err()
        {
            attempt.done(UnlockOutcome::Failed);
            return;
        }
        self.typed.lock().unwrap().push(password.clone());
        if *self.account.lock().unwrap() == password {
            self.handle().resume();
            attempt.done(UnlockOutcome::Unlocked);
        } else {
            attempt.done(UnlockOutcome::Rejected);
        }
    }
}

impl WindowSharer for Mac {
    fn control(&self) -> bool {
        self.can.control
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        let picked = self.picks_screen.load(Ordering::Relaxed).then(|| {
            Box::new(Picked {
                mac: self.me.upgrade().expect("the platform is gone"),
                private: self.can.lift,
            }) as Box<dyn PickedWindow>
        });
        Box::pin(std::future::ready(Ok(picked)))
    }

    fn can_give_screen(&self) -> bool {
        true
    }

    fn can_lift(&self) -> bool {
        self.can.lift
    }

    fn can_be_unlocked(&self) -> bool {
        self.can.unlock
    }

    fn screen(
        &self,
        privacy: Privacy,
    ) -> BoxFuture<'static, Result<Box<dyn PickedWindow>, ScreenError>> {
        let picked = Picked {
            mac: self.me.upgrade().expect("the platform is gone"),
            private: privacy == Privacy::Private && self.can.lift,
        };
        Box::pin(std::future::ready(Ok(
            Box::new(picked) as Box<dyn PickedWindow>
        )))
    }

    fn check_password(&self, password: &Password) -> Result<(), UnlockSetupError> {
        self.journal.push(format!("check {}", password.expose()));
        if *self.account.lock().unwrap() == password.expose() {
            Ok(())
        } else {
            Err(UnlockSetupError::WrongPassword)
        }
    }

    fn unlock_setup(&self, setup: UnlockSetup) -> Result<NewUnlock, UnlockSetupError> {
        self.journal
            .push(format!("setup {}", setup.password.expose()));
        self.made.lock().unwrap().push(Made {
            password: setup.password.expose().to_owned(),
            target: setup.target.to_string(),
            viewer: setup.viewer.to_string(),
            key: setup.key.clone(),
        });
        if let Some(error) = self.setup_fails.lock().unwrap().clone() {
            return Err(error);
        }
        let key = [self.next_box_key.fetch_add(1, Ordering::Relaxed); BoxKey::LEN];
        let during = self.during_setup.lock().unwrap().take();
        if let Some(during) = during {
            during();
        }
        Ok(NewUnlock {
            boxed: if self.makes_empty_box.load(Ordering::Relaxed) {
                Vec::new()
            } else {
                boxed_password(&key, setup.password.expose())
            },
            sealed: sealed_key(&key),
            sealing: setup.key.sealing,
            public: setup.key.public,
        })
    }
}

struct Picked {
    mac: Arc<Mac>,
    private: bool,
}

impl PickedWindow for Picked {
    fn display(&self) -> bool {
        true
    }

    fn private(&self) -> bool {
        self.private
    }

    fn title(&self) -> String {
        String::new()
    }

    fn app(&self) -> String {
        String::new()
    }

    fn size(&self) -> (u32, u32) {
        (3024, 1964)
    }

    fn events(&mut self) -> Box<dyn SourceEvents> {
        Box::new(Hearing(Arc::clone(&self.mac)))
    }

    fn prepare(&mut self, takes: &Takes) -> Option<Config> {
        takes.codecs.contains(&Codec::H264).then(|| Config {
            epoch: 0,
            codec: Codec::H264,
            width: takes.max_width.min(3024),
            height: takes.max_height.min(1964),
            fps: takes.max_fps.min(30),
        })
    }

    fn start(self: Box<Self>, handle: Arc<SourceHandle>, _failure: SharingFailure) {
        *self.mac.handle.lock().unwrap() = Some(Arc::clone(&handle));
        if self.mac.locked_at_start.load(Ordering::Relaxed) {
            handle.pause(PauseReason::Locked);
        }
        // The platform keeps the handle in `handle` above, and so the session runs until it ends.
        std::thread::spawn(move || {
            let deadline = Instant::now() + WAIT;
            let mut sent = 0;
            while sent < 3 && Instant::now() < deadline {
                if handle.begin().is_some() {
                    let picture = Picture {
                        bytes: vec![0x42; 2000],
                        referenced: true,
                        ltr: None,
                        capture_time: 16_000 * sent,
                    };
                    match handle.sent(picture) {
                        Ok(_) => sent += 1,
                        Err(_) => return,
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
    }
}

struct Hearing(Arc<Mac>);

impl SourceEvents for Hearing {
    fn report(&mut self, _report: Report) {}

    fn want(&mut self, _want: Want) {}

    fn input(&mut self, _batch: InputBatch, _arrived: Duration) {}

    fn held(&mut self, _tokens: Vec<LtrToken>) {}

    fn recovery(&mut self) {}

    fn private_in_place(&mut self) -> bool {
        true
    }

    fn unlock(&mut self, attempt: UnlockAttempt) {
        let mac = Arc::clone(&self.0);
        std::thread::spawn(move || mac.carry_out(attempt));
    }

    fn ended_leaving(&mut self) {
        self.0.journal.push("ended leaving");
    }

    fn ended(&mut self) {
        self.0.journal.push("ended");
    }
}

// Two devices.

struct Dev {
    app: App,
    dir: TempDir,
}

impl Dev {
    async fn start(name: &str, windows: Windows) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), name);
        let app = App::start_with(Options::new(dir.path()), windows)
            .await
            .unwrap();
        Self { app, dir }
    }

    fn id(&self) -> String {
        self.app.me().id
    }

    /// The app is stopped and started again over the same folder, as when a program is closed and
    /// opened again.
    async fn restarted(self, windows: Windows) -> Self {
        self.app.stop().await;
        let app = App::start_with(Options::new(self.dir.path()), windows)
            .await
            .unwrap();
        Self { app, ..self }
    }
}

/// Settings that keep the device to this computer.
fn write_settings(dir: &std::path::Path, name: &str) {
    std::fs::write(
        dir.join("config.toml"),
        format!(
            "device_name = \"{name}\"\n\n[network]\nrelay = \"disabled\"\nlan_discovery = false\n\
             port_mapping = false\n"
        ),
    )
    .unwrap();
}

/// Waits for an event `pick` likes, and returns what it made of it.
async fn next_matching<T>(events: &mut Events, mut pick: impl FnMut(Event) -> Option<T>) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            let event = events.next().await.expect("the app stopped while waiting");
            if let Some(found) = pick(event) {
                return found;
            }
        }
    })
    .await
    .expect("the event never came")
}

/// Pairs the two the way two people would: the offerer shows a ticket, the joiner enters it, and
/// the offerer says yes.
async fn pair(offerer: &Dev, joiner: &Dev) {
    let mut events = offerer.app.events();
    let offer = offerer.app.offer_pairing().await.unwrap();
    let app = joiner.app.clone();
    let ticket = offer.ticket.clone();
    let task = tokio::spawn(async move { app.join_pairing(&ticket, |_| {}).await });
    let request = next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;
    assert!(offerer.app.answer_pairing(request.id, true));
    task.await.unwrap().unwrap();
    next_matching(&mut events, |event| match event {
        Event::OfferEnded(_) => Some(()),
        _ => None,
    })
    .await;
}

/// What the screens at a device are told of an unlock of the window `id`, in order, up to the end
/// of the attempt.
async fn told_until_the_end(events: &mut Events, id: u64) -> Vec<Event> {
    let mut told = Vec::new();
    loop {
        let event = next_matching(events, |event| match event {
            Event::WindowLocked { id: of, .. } | Event::WindowUnlock { id: of, .. } if of == id => {
                Some(event)
            }
            _ => None,
        })
        .await;
        let over = matches!(
            event,
            Event::WindowUnlock {
                step: WindowUnlock::Ended(_),
                ..
            }
        );
        told.push(event);
        if over {
            return told;
        }
    }
}

/// Only the steps of the attempt, without the lock.
fn steps(told: &[Event]) -> Vec<WindowUnlock> {
    told.iter()
        .filter_map(|event| match event {
            Event::WindowUnlock { step, .. } => Some(*step),
            _ => None,
        })
        .collect()
}

fn ended(result: UnlockResult) -> WindowUnlock {
    WindowUnlock::Ended(result)
}

/// The next time the screens are told that the window `id` is locked or is not.
async fn locked_told(events: &mut Events, id: u64) -> bool {
    next_matching(events, |event| match event {
        Event::WindowLocked { id: of, locked } if of == id => Some(locked),
        _ => None,
    })
    .await
}

/// The next attempt to unlock the Mac that ended.
async fn attempt_told(events: &mut Events) -> UnlockEntry {
    next_matching(events, |event| match event {
        Event::UnlockAttempted(entry) => Some(entry),
        _ => None,
    })
    .await
}

/// The next end of a window.
async fn window_ended(events: &mut Events, id: u64) -> WindowEnd {
    next_matching(events, |event| match event {
        Event::WindowEnded { id: of, end } if of == id => Some(end),
        _ => None,
    })
    .await
}

/// Whether nothing of the kinds `interesting` likes is told in a quiet moment.
async fn nothing_told(events: &mut Events, interesting: impl Fn(&Event) -> bool) -> bool {
    let quiet = tokio::time::timeout(QUIET, async {
        loop {
            let event = events.next().await.expect("the app stopped while waiting");
            if interesting(&event) {
                return event;
            }
        }
    })
    .await;
    quiet.is_err()
}

/// A Mac and a phone, paired, and the platforms they were given.
struct Rig {
    mac: Dev,
    phone: Dev,
    mac_platform: Arc<Mac>,
    phone_platform: Arc<Phone>,
    /// What the platforms of both did, in order.
    journal: Arc<Heard>,
    /// The screens that [`until_shown`](Self::until_shown) has waited for.
    shown: AtomicUsize,
}

const PASSWORD: &str = "correct horse";

impl Rig {
    /// A Mac that can be unlocked, whose system accepts [`PASSWORD`], and a phone that unlocks.
    async fn new() -> Self {
        let journal = Arc::new(Heard::default());
        let mac = Mac::new(PASSWORD, &journal);
        let phone = Phone::new(true, &journal);
        Self::with(mac, phone, journal).await
    }

    async fn with(mac: Arc<Mac>, phone: Arc<Phone>, journal: Arc<Heard>) -> Self {
        let mac_dev = Dev::start("Studio Mac", Windows::none().sharing(mac.clone())).await;
        let phone_dev = Dev::start("Pocket Phone", Windows::none().showing(phone.clone())).await;
        pair(&mac_dev, &phone_dev).await;
        // The person at the Mac lets the phone ask for its screen, and control it.
        mac_dev
            .app
            .set_start_access(&phone_dev.id(), StartAccess::Control)
            .unwrap();
        Self {
            mac: mac_dev,
            phone: phone_dev,
            mac_platform: mac,
            phone_platform: phone,
            journal,
            shown: AtomicUsize::new(0),
        }
    }

    /// The person at the Mac sets the unlock up for the phone with this password.
    async fn sets_up(&self, password: &str) -> UnlockSetUp {
        self.mac
            .app
            .set_up_unlock(&self.phone.id(), password.to_owned())
            .await
            .unwrap()
    }

    /// The person at the Mac sets the unlock up with [`PASSWORD`], and it works.
    async fn is_set_up(&self) {
        assert_eq!(self.sets_up(PASSWORD).await, UnlockSetUp::Done);
    }

    fn status(&self) -> UnlockStatus {
        self.mac.app.unlock_status(&self.phone.id()).unwrap()
    }

    /// The person at the phone asks for the screen of the Mac, to control it.
    async fn started(&self) -> WindowSession {
        match self
            .phone
            .app
            .start_screen(&self.mac.id(), true)
            .await
            .unwrap()
        {
            ScreenStarted::Started(session) => session,
            ScreenStarted::NotStarted(why) => panic!("the screen was not started: {why}"),
        }
    }

    /// Waits until a picture has been shown at the phone for the next screen that it has not waited
    /// for, which is when both control streams of the session are up. Every screen starts again at
    /// the picture 0.
    async fn until_shown(&self) {
        let screens = self.shown.fetch_add(1, Ordering::Relaxed) + 1;
        until("a picture to be shown at the phone", || {
            self.phone_platform.pictures.count("show 0") >= screens
        })
        .await;
    }

    /// The Mac is set up, the phone shows its screen, and the screen locks. The phone has been
    /// told.
    async fn locked_screen(&self, phone_events: &mut Events) -> WindowSession {
        self.is_set_up().await;
        let session = self.started().await;
        assert!(session.unlock, "the session takes no unlock: {session:?}");
        self.until_shown().await;
        self.mac_platform.locks();
        assert!(locked_told(phone_events, session.id).await);
        session
    }

    /// The person at the phone ends the screen, once a picture was shown. Both devices are done
    /// with it when this returns, so that another screen can be asked for: the Mac is busy until it
    /// has heard of the end. A screen that ends within a millisecond of its start, before the Mac
    /// has begun, is heard of at the Mac only when the silence runs out, after 30 seconds.
    async fn ends(
        &self,
        session: &WindowSession,
        phone_events: &mut Events,
        mac_events: &mut Events,
    ) {
        assert!(self.phone.app.end_window(session.id));
        window_ended(phone_events, session.id).await;
        next_matching(mac_events, |event| {
            matches!(event, Event::WindowEnded { .. }).then_some(())
        })
        .await;
    }

    /// The person at the phone taps Unlock, and the platform is asked for their face.
    async fn taps_unlock(&self, session: &WindowSession) -> UnlockQuestion {
        // A platform that declined a question told the person at once, and the library takes the
        // answer a moment later: a tap right after it is taken when it has.
        until("the tap to be taken", || {
            self.phone.app.unlock_window(session.id)
        })
        .await;
        self.phone_platform.faces.question().await
    }
}

// What a device can do.

#[tokio::test]
async fn what_a_device_can_do_to_unlock_depends_on_the_platform_it_was_given() {
    let journal = Arc::new(Heard::default());
    let dev = |name: &'static str, windows: Windows| Dev::start(name, windows);
    let plain = dev("Plain", Windows::none()).await;
    let phone = dev("Phone", Windows::none().showing(Phone::new(true, &journal))).await;
    let blind = dev(
        "Blind phone",
        Windows::none().showing(Phone::new(false, &journal)),
    )
    .await;
    let mac = dev("Mac", Windows::none().sharing(Mac::new("pw", &journal))).await;
    let sharers = [
        (
            "Cannot post input",
            Can {
                control: false,
                ..Can::ALL
            },
        ),
        (
            "Cannot lift",
            Can {
                lift: false,
                ..Can::ALL
            },
        ),
        (
            "Cannot keep a password",
            Can {
                unlock: false,
                ..Can::ALL
            },
        ),
    ];
    let both = dev(
        "Both",
        Windows::none()
            .showing(Phone::new(true, &journal))
            .sharing(Mac::new("pw", &journal)),
    )
    .await;
    let unlock = |dev: &Dev| {
        let WindowAbilities {
            can_be_unlocked,
            can_unlock,
            ..
        } = dev.app.window_abilities();
        (can_be_unlocked, can_unlock)
    };
    assert_eq!(unlock(&plain), (false, false));
    assert_eq!(unlock(&phone), (false, true));
    assert_eq!(unlock(&blind), (false, false));
    assert_eq!(unlock(&mac), (true, false));
    assert_eq!(unlock(&both), (true, true));
    // An unlock goes with the privacy of a screen that is controlled and that the other device can
    // lift, and with a platform that keeps a password: without any of these there is nothing to
    // set up.
    for (name, can) in sharers {
        let lacking = dev(
            name,
            Windows::none().sharing(Mac::building(can, "pw", &journal)),
        )
        .await;
        assert_eq!(unlock(&lacking), (false, false), "{name}");
    }
}

// Setting the unlock up at the Mac.

#[tokio::test]
async fn the_unlock_is_set_up_by_checking_the_password_then_asking_the_phone_then_making_the_box() {
    let rig = Rig::new().await;
    assert_eq!(rig.status(), UnlockStatus::Off);

    assert_eq!(rig.sets_up(PASSWORD).await, UnlockSetUp::Done);

    // In this order: the system accepted the password before anything was asked of the phone, and
    // the box was made once the phone had given its key.
    assert_eq!(
        rig.journal.all(),
        [
            "check correct horse",
            "key for Studio Mac renew true",
            "setup correct horse"
        ]
    );
    // The phone was asked as the Mac, with a new key, and the box was made for these two devices
    // with the key that the phone gave.
    assert_eq!(
        rig.phone_platform.asked(),
        [(
            UnlockSource {
                id: rig.mac.id().parse().unwrap(),
                name: "Studio Mac".to_owned()
            },
            true
        )]
    );
    assert_eq!(
        rig.mac_platform.made(),
        [Made {
            password: PASSWORD.to_owned(),
            target: rig.mac.id(),
            viewer: rig.phone.id(),
            key: phone_key(7),
        }]
    );
    assert!(matches!(rig.status(), UnlockStatus::On { since } if since > 1_700_000_000));
}

#[tokio::test]
async fn a_password_that_the_system_does_not_accept_asks_nothing_of_the_phone() {
    let rig = Rig::new().await;
    assert_eq!(rig.sets_up("wrong horse").await, UnlockSetUp::WrongPassword);
    assert_eq!(rig.journal.all(), ["check wrong horse"]);
    assert_eq!(
        rig.phone_platform.asked(),
        Vec::<(UnlockSource, bool)>::new()
    );
    assert_eq!(rig.status(), UnlockStatus::Off);
}

#[tokio::test]
async fn a_phone_that_will_not_give_a_key_leaves_the_box_unmade_and_says_why() {
    let rig = Rig::new().await;
    for (error, said) in [
        (UnlockKeyError::Refused, "Nobody at Pocket Phone agreed"),
        (
            UnlockKeyError::Blocked,
            "Pocket Phone does not let this device ask it for a key",
        ),
        (
            UnlockKeyError::Unsupported,
            "Pocket Phone cannot unlock this device",
        ),
        (
            // The phone keeps its reason to itself: the Mac is told only that it cannot.
            UnlockKeyError::Failed(PlatformError::new("no enclave")),
            "Pocket Phone cannot unlock this device",
        ),
    ] {
        *rig.phone_platform.key.lock().unwrap() = Err(error.clone());
        let UnlockSetUp::NotDone(why) = rig.sets_up(PASSWORD).await else {
            panic!("the unlock was set up: {error:?}");
        };
        assert!(why.starts_with(said), "{error:?}: {why}");
        assert!(
            !why.contains("no enclave"),
            "the sentence of the platform is for the log"
        );
        assert_eq!(rig.status(), UnlockStatus::Off, "{error:?}");
    }
    // The box was never made.
    assert_eq!(rig.mac_platform.made(), Vec::<Made>::new());
    assert_eq!(rig.journal.count("setup"), 0);
}

#[tokio::test]
async fn a_platform_that_cannot_make_the_box_leaves_the_unlock_as_it_was() {
    let rig = Rig::new().await;
    let failure = PlatformError::new("The keychain is locked.\nTry again later.");
    *rig.mac_platform.setup_fails.lock().unwrap() = Some(UnlockSetupError::Failed(failure));

    // Nothing was set up before, and nothing is now.
    let UnlockSetUp::NotDone(why) = rig.sets_up(PASSWORD).await else {
        panic!("the unlock was set up");
    };
    assert_eq!(why, "The keychain is locked. Try again later.");
    assert_eq!(rig.status(), UnlockStatus::Off);

    // What was set up before is as it was.
    *rig.mac_platform.setup_fails.lock().unwrap() = None;
    rig.is_set_up().await;
    let UnlockStatus::On { since } = rig.status() else {
        panic!("the unlock is not set up");
    };
    *rig.mac_platform.setup_fails.lock().unwrap() = Some(UnlockSetupError::Unsupported);
    let UnlockSetUp::NotDone(why) = rig.sets_up(PASSWORD).await else {
        panic!("the unlock was set up");
    };
    assert!(
        why.starts_with("This device cannot keep an unlock"),
        "{why}"
    );
    assert_eq!(rig.status(), UnlockStatus::On { since });
}

#[tokio::test]
async fn a_password_that_cannot_be_kept_is_not_given_to_the_platform() {
    let long = "p".repeat(Password::MAX_BYTES);
    let journal = Arc::new(Heard::default());
    let mac = Mac::new(&long, &journal);
    let rig = Rig::with(mac, Phone::new(true, &journal), journal).await;

    let UnlockSetUp::NotDone(why) = rig.sets_up("").await else {
        panic!("an empty password was taken");
    };
    assert_eq!(why, "Type the password of this device.");
    let too_long = "p".repeat(Password::MAX_BYTES + 1);
    let UnlockSetUp::NotDone(why) = rig.sets_up(&too_long).await else {
        panic!("a password that is too long was taken");
    };
    assert!(why.contains("127 bytes at most"), "{why}");
    // A letter that takes two bytes counts as two.
    let accented = format!("{}\u{e9}", "p".repeat(Password::MAX_BYTES - 1));
    assert_eq!(accented.len(), Password::MAX_BYTES + 1);
    assert!(matches!(
        rig.sets_up(&accented).await,
        UnlockSetUp::NotDone(_)
    ));
    assert!(rig.journal.all().is_empty(), "the platform was asked");

    // The longest password that fits is kept.
    assert_eq!(rig.sets_up(&long).await, UnlockSetUp::Done);
}

#[tokio::test]
async fn a_device_that_cannot_be_unlocked_or_cannot_unlock_says_so_when_asked_to_set_it_up() {
    // A Mac whose platform keeps no password.
    let journal = Arc::new(Heard::default());
    let mac = Mac::building(
        Can {
            unlock: false,
            ..Can::ALL
        },
        PASSWORD,
        &journal,
    );
    let rig = Rig::with(mac, Phone::new(true, &journal), journal).await;
    let error = rig
        .mac
        .app
        .set_up_unlock(&rig.phone.id(), PASSWORD.to_owned())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert_eq!(
        error.message(),
        "This device cannot be unlocked from another device."
    );
    assert_eq!(rig.journal.all(), Vec::<String>::new());

    // A phone that approves nothing: the Mac checks the password, and the phone is not asked.
    let journal = Arc::new(Heard::default());
    let rig = Rig::with(
        Mac::new(PASSWORD, &journal),
        Phone::new(false, &journal),
        journal,
    )
    .await;
    let error = rig
        .mac
        .app
        .set_up_unlock(&rig.phone.id(), PASSWORD.to_owned())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined);
    assert_eq!(
        error.message(),
        "Pocket Phone cannot unlock this device. It may need a newer version of Baton."
    );
    assert_eq!(
        rig.phone_platform.asked(),
        Vec::<(UnlockSource, bool)>::new()
    );
    assert_eq!(rig.status(), UnlockStatus::Off);
}

#[tokio::test]
async fn a_device_that_is_not_paired_has_no_unlock() {
    let rig = Rig::new().await;
    let stranger = "0".repeat(64);
    for id in [stranger.as_str(), "not an id", ""] {
        assert_eq!(
            rig.mac.app.unlock_status(id).unwrap_err().kind(),
            ErrorKind::NotPaired
        );
        assert_eq!(
            rig.mac.app.turn_off_unlock(id).unwrap_err().kind(),
            ErrorKind::NotPaired
        );
        let error = rig
            .mac
            .app
            .set_up_unlock(id, PASSWORD.to_owned())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotPaired);
    }
    assert_eq!(rig.journal.all(), Vec::<String>::new());
}

#[tokio::test]
async fn setting_the_unlock_up_again_replaces_it_and_turning_it_off_deletes_it() {
    let rig = Rig::new().await;
    assert!(!rig.mac.app.turn_off_unlock(&rig.phone.id()).unwrap());

    rig.is_set_up().await;
    rig.is_set_up().await;
    // Each time the phone is asked for a new key, so that what was sealed to the old one opens
    // nothing.
    let renewed: Vec<bool> = rig.phone_platform.asked().iter().map(|a| a.1).collect();
    assert_eq!(renewed, [true, true]);
    assert_eq!(rig.mac_platform.made().len(), 2);

    assert!(rig.mac.app.turn_off_unlock(&rig.phone.id()).unwrap());
    assert_eq!(rig.status(), UnlockStatus::Off);
    assert!(!rig.mac.app.turn_off_unlock(&rig.phone.id()).unwrap());
    // The phone is not asked for anything when it is turned off.
    assert_eq!(rig.phone_platform.asked().len(), 2);
}

#[tokio::test]
async fn two_set_ups_at_once_run_one_after_the_other() {
    let rig = Rig::new().await;
    // The first set-up waits for the phone to give its key.
    let (give, gate) = oneshot::channel();
    *rig.phone_platform.gate.lock().unwrap() = Some(gate);
    let (app, id) = (rig.mac.app.clone(), rig.phone.id());
    let first = tokio::spawn({
        let (app, id) = (app.clone(), id.clone());
        async move { app.set_up_unlock(&id, PASSWORD.to_owned()).await }
    });
    until("the first set-up to ask the phone", || {
        rig.journal.has("key for Studio Mac renew true")
    })
    .await;
    let second = tokio::spawn(async move { app.set_up_unlock(&id, PASSWORD.to_owned()).await });
    tokio::time::sleep(QUIET).await;
    // The second has not begun: it would ask the phone for a key that ends the first's.
    assert_eq!(rig.journal.count("check"), 1);
    assert_eq!(rig.phone_platform.asked().len(), 1);

    give.send(()).unwrap();
    assert_eq!(first.await.unwrap().unwrap(), UnlockSetUp::Done);
    assert_eq!(second.await.unwrap().unwrap(), UnlockSetUp::Done);
    assert_eq!(
        rig.journal.all(),
        [
            "check correct horse",
            "key for Studio Mac renew true",
            "setup correct horse",
            "check correct horse",
            "key for Studio Mac renew true",
            "setup correct horse",
        ]
    );
}

// Unlocking a locked screen.

#[tokio::test]
async fn an_unlock_is_offered_to_the_phone_it_was_set_up_for_in_a_private_session_only() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();

    // Nothing is set up, so the session takes no unlock, and a tap does nothing.
    let session = rig.started().await;
    rig.until_shown().await;
    assert!(session.private && !session.unlock);
    assert!(!rig.phone.app.unlock_window(session.id));
    rig.ends(&session, &mut phone_events, &mut mac_events).await;

    rig.is_set_up().await;
    let session = rig.started().await;
    assert!(session.private && session.unlock);
    rig.until_shown().await;
    rig.ends(&session, &mut phone_events, &mut mac_events).await;
}

#[tokio::test]
async fn a_screen_that_the_phone_only_looks_at_is_not_private_and_takes_no_unlock() {
    let rig = Rig::new().await;
    rig.is_set_up().await;
    let ScreenStarted::Started(session) = rig
        .phone
        .app
        .start_screen(&rig.mac.id(), false)
        .await
        .unwrap()
    else {
        panic!("the screen was not started");
    };
    assert!(!session.private && !session.unlock);
    assert!(!rig.phone.app.unlock_window(session.id));
}

#[tokio::test]
async fn a_phone_whose_screens_ask_for_no_face_is_offered_no_unlock() {
    let journal = Arc::new(Heard::default());
    let rig = Rig::with(
        Mac::new(PASSWORD, &journal),
        Phone::new(false, &journal),
        journal,
    )
    .await;
    let session = rig.started().await;
    assert!(!session.unlock);
    assert!(!rig.phone.app.unlock_window(session.id));
}

#[tokio::test]
async fn a_locked_mac_is_unlocked_with_the_face_of_the_person_at_the_phone() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;

    // The platform of the phone is asked for a face, in words that name the Mac, with the key that
    // the Mac sealed to it.
    let question = rig.taps_unlock(&session).await;
    assert_eq!(question.source().name, "Studio Mac");
    assert_eq!(question.source().id.to_string(), rig.mac.id());
    assert_eq!(question.sealed(), &sealed_key(&[1; BoxKey::LEN]));
    // A second tap while the first goes is not taken.
    assert!(!rig.phone.app.unlock_window(session.id));

    // The face is recognised, and the key goes to the Mac.
    let key = opened(question.sealed());
    question.opened(key);
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(
        steps(&told),
        [
            WindowUnlock::Asking,
            WindowUnlock::Typing,
            ended(UnlockResult::Unlocked)
        ]
    );
    // The Mac typed the password, and the screen is not locked any more: the pictures go on.
    assert_eq!(rig.mac_platform.typed(), [PASSWORD]);
    let unlocked = told
        .iter()
        .any(|event| matches!(event, Event::WindowLocked { locked: false, .. }));
    assert!(unlocked || !locked_told(&mut phone_events, session.id).await);

    // The person at the Mac is told, and the history holds the attempt without a word of the
    // secret.
    let attempt = attempt_told(&mut mac_events).await;
    assert_eq!(attempt.result, UnlockResult::Unlocked);
    assert!(attempt.result.notifies());
    assert_eq!(attempt.device_id, rig.phone.id());
    assert_eq!(attempt.device_name, "Pocket Phone");
    assert!(attempt.at > 1_700_000_000);
    assert_eq!(rig.mac.app.unlock_history(), std::slice::from_ref(&attempt));
    assert!(!format!("{attempt:?}").contains(PASSWORD));
    assert_eq!(
        rig.phone.app.unlock_history(),
        Vec::<UnlockEntry>::new(),
        "the phone is not unlocked by anyone"
    );

    // Unchanged devices, and a change that is not an attempt, tell nothing of one.
    rig.mac
        .app
        .set_start_access(&rig.phone.id(), StartAccess::Look)
        .unwrap();
    assert!(
        nothing_told(&mut mac_events, |event| matches!(
            event,
            Event::UnlockAttempted(_)
        ))
        .await
    );
}

#[tokio::test]
async fn each_unlock_uses_the_key_that_the_one_before_made_and_the_history_keeps_them_in_order() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;

    let mut key = [1; BoxKey::LEN];
    for time in 1..=2 {
        let question = rig.taps_unlock(&session).await;
        // The Mac put the password in a new box after the last unlock, and sealed its new key.
        assert_eq!(question.sealed(), &sealed_key(&key), "unlock {time}");
        let opened_key = opened(question.sealed());
        question.opened(opened_key);
        let told = told_until_the_end(&mut phone_events, session.id).await;
        assert_eq!(steps(&told).last(), Some(&ended(UnlockResult::Unlocked)));
        assert_eq!(
            attempt_told(&mut mac_events).await.result,
            UnlockResult::Unlocked
        );
        key = next_key(&key);
        // The screen locks again for the next one.
        if time == 1 {
            rig.mac_platform.locks();
            until("the screen to lock", || true).await;
            loop {
                if locked_told(&mut phone_events, session.id).await {
                    break;
                }
            }
        }
    }
    assert_eq!(rig.mac_platform.typed(), [PASSWORD, PASSWORD]);
    let history = rig.mac.app.unlock_history();
    assert_eq!(history.len(), 2);
    assert!(history[0].at >= history[1].at, "the latest comes first");
}

#[tokio::test]
async fn a_person_who_does_not_agree_ends_the_attempt_and_the_unlock_stays_set_up() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;

    let question = rig.taps_unlock(&session).await;
    question.declined(CancelReason::Declined);
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(
        steps(&told),
        [WindowUnlock::Asking, ended(UnlockResult::Declined)]
    );
    let attempt = attempt_told(&mut mac_events).await;
    assert_eq!(attempt.result, UnlockResult::Declined);
    assert!(!attempt.result.notifies());
    assert_eq!(rig.mac_platform.typed(), Vec::<String>::new());
    assert!(matches!(rig.status(), UnlockStatus::On { .. }));

    // The person can try again, and it is told in full.
    let question = rig.taps_unlock(&session).await;
    let key = opened(question.sealed());
    question.opened(key);
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(steps(&told).last(), Some(&ended(UnlockResult::Unlocked)));
    assert_eq!(steps(&told).len(), 3);
}

#[tokio::test]
async fn a_phone_that_lost_its_key_ends_the_unlock_at_the_mac_until_it_is_set_up_again() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;

    let question = rig.taps_unlock(&session).await;
    question.declined(CancelReason::KeyLost);
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(
        steps(&told),
        [WindowUnlock::Asking, ended(UnlockResult::KeyLost)]
    );
    let attempt = attempt_told(&mut mac_events).await;
    assert_eq!(attempt.result, UnlockResult::KeyLost);
    assert!(attempt.result.notifies());

    // The Mac knows the unlock ended, and the next session offers none.
    until("the unlock to end", || {
        matches!(rig.status(), UnlockStatus::Ended { .. })
    })
    .await;
    rig.ends(&session, &mut phone_events, &mut mac_events).await;
    let next = rig.started().await;
    assert!(!next.unlock);

    // Setting it up again brings it back.
    rig.is_set_up().await;
    assert!(matches!(rig.status(), UnlockStatus::On { .. }));
}

#[tokio::test]
async fn a_password_that_the_system_no_longer_accepts_ends_the_unlock() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;
    // The person changed their password after they set the unlock up.
    *rig.mac_platform.account.lock().unwrap() = "battery staple".to_owned();

    let question = rig.taps_unlock(&session).await;
    let key = opened(question.sealed());
    question.opened(key);
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(
        steps(&told),
        [
            WindowUnlock::Asking,
            WindowUnlock::Typing,
            ended(UnlockResult::Rejected)
        ]
    );
    let attempt = attempt_told(&mut mac_events).await;
    assert_eq!(attempt.result, UnlockResult::Rejected);
    assert!(attempt.result.notifies());
    until("the unlock to end", || {
        matches!(rig.status(), UnlockStatus::Ended { .. })
    })
    .await;
}

#[tokio::test]
async fn a_screen_that_is_not_locked_gets_no_question() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    rig.is_set_up().await;
    let session = rig.started().await;
    rig.until_shown().await;

    assert!(rig.phone.app.unlock_window(session.id));
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(steps(&told), [ended(UnlockResult::NotLocked)]);
    assert_eq!(rig.phone_platform.faces.log.count("open"), 0);
    let attempt = attempt_told(&mut mac_events).await;
    assert_eq!(attempt.result, UnlockResult::NotLocked);
    assert!(!attempt.result.notifies());
    assert_eq!(rig.mac_platform.typed(), Vec::<String>::new());
}

#[tokio::test]
async fn a_screen_that_is_locked_when_the_window_starts_is_told_after_the_window() {
    let journal = Arc::new(Heard::default());
    let mac = Mac::new(PASSWORD, &journal);
    mac.locked_at_start.store(true, Ordering::Relaxed);
    let rig = Rig::with(mac, Phone::new(true, &journal), journal).await;
    rig.is_set_up().await;
    let mut phone_events = rig.phone.app.events();

    let session = rig.started().await;
    let first = next_matching(&mut phone_events, |event| match event {
        Event::WindowStarted(_) | Event::WindowLocked { .. } => Some(event),
        _ => None,
    })
    .await;
    assert!(matches!(first, Event::WindowStarted(_)), "{first:?}");
    assert!(locked_told(&mut phone_events, session.id).await);
}

#[tokio::test]
async fn a_window_that_ends_while_the_face_is_asked_for_stops_the_asking_and_drops_a_late_answer() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;
    let question = rig.taps_unlock(&session).await;

    assert!(rig.phone.app.end_window(session.id));
    assert_eq!(
        window_ended(&mut phone_events, session.id).await,
        WindowEnd::ThisDevice
    );
    // The platform stops asking for the face and lets go of the screen.
    until("the platform to stop asking", || {
        rig.phone_platform.faces.log.has("cancel") && rig.phone_platform.faces.log.has("ended")
    })
    .await;

    // The face is recognised a moment too late: nothing is told and nothing is sent.
    let key = opened(question.sealed());
    question.opened(key);
    assert!(
        nothing_told(&mut phone_events, |event| matches!(
            event,
            Event::WindowUnlock { .. }
        ))
        .await
    );
    let attempt = attempt_told(&mut mac_events).await;
    assert_eq!(attempt.result, UnlockResult::SessionEnded);
    assert!(!attempt.result.notifies());
    assert_eq!(rig.mac_platform.typed(), Vec::<String>::new());
}

#[tokio::test]
async fn a_question_that_the_platform_drops_ends_the_attempt_as_given_up() {
    let rig = Rig::new().await;
    rig.phone_platform
        .faces
        .drops
        .store(true, Ordering::Relaxed);
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;

    assert!(rig.phone.app.unlock_window(session.id));
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(
        steps(&told),
        [WindowUnlock::Asking, ended(UnlockResult::GaveUp)]
    );
    let attempt = attempt_told(&mut mac_events).await;
    assert_eq!(attempt.result, UnlockResult::GaveUp);
    assert!(attempt.result.notifies());
}

#[tokio::test]
async fn an_unlock_of_a_window_that_is_not_shown_here_is_not_asked_for() {
    let rig = Rig::new().await;
    let mut mac_events = rig.mac.app.events();
    rig.is_set_up().await;
    let session = rig.started().await;
    rig.until_shown().await;
    // The window that the Mac shares is a window of the Mac, and one that never was is none.
    let shared = next_matching(&mut mac_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    assert!(!rig.mac.app.unlock_window(shared.id));
    assert!(!rig.phone.app.unlock_window(session.id + 1000));
}

#[tokio::test]
async fn the_history_lists_the_latest_attempt_first_and_each_attempt_is_told_once() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;

    // The person says no, then yes, and then the phone has lost its key. No two ended alike, so
    // that an order and a repeat can be told.
    let question = rig.taps_unlock(&session).await;
    question.declined(CancelReason::Declined);
    told_until_the_end(&mut phone_events, session.id).await;
    let question = rig.taps_unlock(&session).await;
    let key = opened(question.sealed());
    question.opened(key);
    told_until_the_end(&mut phone_events, session.id).await;
    // The screen locks again for the third.
    rig.mac_platform.locks();
    while !locked_told(&mut phone_events, session.id).await {}
    let question = rig.taps_unlock(&session).await;
    question.declined(CancelReason::KeyLost);
    told_until_the_end(&mut phone_events, session.id).await;

    // They are told in the order they ended, and once each.
    for result in [
        UnlockResult::Declined,
        UnlockResult::Unlocked,
        UnlockResult::KeyLost,
    ] {
        assert_eq!(attempt_told(&mut mac_events).await.result, result);
    }
    assert!(
        nothing_told(&mut mac_events, |event| matches!(
            event,
            Event::UnlockAttempted(_)
        ))
        .await
    );
    // The history holds the latest first.
    let results: Vec<_> = rig
        .mac
        .app
        .unlock_history()
        .iter()
        .map(|entry| entry.result)
        .collect();
    assert_eq!(
        results,
        [
            UnlockResult::KeyLost,
            UnlockResult::Unlocked,
            UnlockResult::Declined
        ]
    );
}

#[tokio::test]
async fn an_attempt_from_before_a_restart_is_not_told_again() {
    let rig = Rig::new().await;
    let mut phone_events = rig.phone.app.events();
    let mut mac_events = rig.mac.app.events();
    let session = rig.locked_screen(&mut phone_events).await;
    let question = rig.taps_unlock(&session).await;
    let key = opened(question.sealed());
    question.opened(key);
    told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(
        attempt_told(&mut mac_events).await.result,
        UnlockResult::Unlocked
    );

    // The Mac is closed and opened again, and its history is as it was.
    let Rig {
        mac,
        phone,
        mac_platform,
        ..
    } = rig;
    let mac = mac.restarted(Windows::none().sharing(mac_platform)).await;
    assert_eq!(mac.app.unlock_history().len(), 1);

    // Something else changes at the Mac, which makes it look at its history again.
    let mut events = mac.app.events();
    mac.app
        .set_start_access(&phone.id(), StartAccess::Look)
        .unwrap();
    next_matching(&mut events, |event| {
        matches!(event, Event::DevicesChanged).then_some(())
    })
    .await;
    assert!(
        nothing_told(&mut events, |event| matches!(
            event,
            Event::UnlockAttempted(_)
        ))
        .await
    );
}

#[tokio::test]
async fn a_device_that_is_unpaired_while_the_unlock_is_set_up_keeps_nothing() {
    let rig = Rig::new().await;
    // The phone gave its key, and is unpaired before the platform gives the box back.
    let (app, phone) = (rig.mac.app.clone(), rig.phone.id());
    *rig.mac_platform.during_setup.lock().unwrap() = Some(Box::new(move || {
        app.unpair(&phone).unwrap();
    }));

    let error = rig
        .mac
        .app
        .set_up_unlock(&rig.phone.id(), PASSWORD.to_owned())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotPaired);
    assert_eq!(
        rig.mac
            .app
            .unlock_status(&rig.phone.id())
            .unwrap_err()
            .kind(),
        ErrorKind::NotPaired
    );
}

#[tokio::test]
async fn a_box_that_no_record_can_keep_is_not_kept_and_the_unlock_stays_as_it_was() {
    let rig = Rig::new().await;
    rig.is_set_up().await;
    let UnlockStatus::On { since } = rig.status() else {
        panic!("the unlock is not set up");
    };

    rig.mac_platform
        .makes_empty_box
        .store(true, Ordering::Relaxed);
    let UnlockSetUp::NotDone(why) = rig.sets_up(PASSWORD).await else {
        panic!("the unlock was set up");
    };
    assert_eq!(why, "This device could not make the box for the unlock.");
    assert_eq!(rig.status(), UnlockStatus::On { since });
}

#[tokio::test]
async fn a_screen_that_the_mac_offers_and_the_phone_accepts_takes_an_unlock_too() {
    let rig = Rig::new().await;
    rig.is_set_up().await;
    rig.mac_platform.picks_screen.store(true, Ordering::Relaxed);
    let mut phone_events = rig.phone.app.events();

    let (app, to) = (rig.mac.app.clone(), rig.phone.id());
    let offer = tokio::spawn(async move { app.share_window(&to, true).await });
    let request = next_matching(&mut phone_events, |event| match event {
        Event::WindowRequested(request) => Some(request),
        _ => None,
    })
    .await;
    assert!(rig.phone.app.answer_window(request.id, true));
    let WindowShared::Started(shared) = offer.await.unwrap().unwrap() else {
        panic!("the screen was not shared");
    };
    let session = next_matching(&mut phone_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    // The phone is offered the unlock of a private screen, and the Mac that shares it is not.
    assert!(session.private && session.unlock, "{session:?}");
    assert!(shared.private && !shared.unlock, "{shared:?}");

    // It works as on a screen that the phone asked for.
    rig.until_shown().await;
    rig.mac_platform.locks();
    while !locked_told(&mut phone_events, session.id).await {}
    let question = rig.taps_unlock(&session).await;
    let key = opened(question.sealed());
    question.opened(key);
    let told = told_until_the_end(&mut phone_events, session.id).await;
    assert_eq!(steps(&told).last(), Some(&ended(UnlockResult::Unlocked)));
    assert_eq!(rig.mac_platform.typed(), [PASSWORD]);
}
