// SPDX-License-Identifier: Apache-2.0

//! Windows shown and shared the way a screen drives them: two apps on this computer, paired,
//! with a platform that is played by the test. The one that shares picks a window and makes
//! "pictures" of it; the one that shows keeps a log of what its decoder was given.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use baton_app::{
    App, ErrorKind, Event, Events, Options, ScreenStarted, StartAccess, WindowAbilities,
    WindowButton, WindowEnd, WindowInput, WindowModifiers, WindowNoticeReason, WindowPointer,
    WindowRequest, WindowRole, WindowSession, WindowShared, WindowSound, Windows,
    windows::{
        PickedWindow, PlatformError, Privacy, Screen, ScreenError, ScreenLink, SharingFailure,
        ShowLimits, WindowSharer, WindowViewer,
    },
};
use baton_core::{
    Config as NodeConfig, Identity, Node, PairedDevice, TrustStore,
    streaming::{
        DecodeError, Decoder, DecoderState, Picture, Play, SoundPacket, SourceEvents, SourceHandle,
        Speaker, StartOutcome, Takes, ViewHandle, Viewing, Wanted, WhyNot, WindowHandler,
        WindowOffer, WindowVerdict,
    },
    util::BoxFuture,
};
use baton_proto::{DeviceId, stream::Codec, ticket::PairTicket};
use baton_stream::{
    control::{
        Config, InputBatch, Lift, Report, SilentReason, Sound, SoundCodec, SourceMessage, Thermal,
        Want,
    },
    gate::Reason,
    header::FrameHeader,
    input::{Button, Event as InputEvent, Modifiers, PointerAction},
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

// The platform of a device that shows windows.

/// What `open` was given: the title, the size and whether the window can be controlled.
#[derive(Debug, PartialEq)]
struct Opened {
    title: Option<String>,
    width: u32,
    height: u32,
    control: bool,
}

impl Opened {
    fn report(control: bool) -> Self {
        Self {
            title: Some("Report.pdf".to_owned()),
            width: 1440,
            height: 900,
            control,
        }
    }
}

struct Viewer {
    next_screen: AtomicI64,
    /// What the decoders of the screens heard.
    heard: Arc<Heard>,
    opened: Mutex<Vec<Opened>>,
    /// The links of the screens that were opened.
    links: Mutex<Vec<ScreenLink>>,
    /// If set, `open` fails with this.
    fail: Mutex<Option<String>>,
    limits: ShowLimits,
    /// Whether the platform says that the screens play sound.
    plays: bool,
    /// Whether the screens are given a speaker. A platform that plays sound does so, and one that
    /// does not has no speaker to give, unless it is set up wrongly.
    speaks: bool,
    /// What the speakers of the screens heard.
    sounds: Arc<Heard>,
}

impl Viewer {
    fn new() -> Arc<Self> {
        Self::building(false, false)
    }

    /// A platform whose screens play sound.
    fn playing() -> Arc<Self> {
        Self::building(true, true)
    }

    /// A platform that gives its screens a speaker and does not say that they play sound.
    fn muddled() -> Arc<Self> {
        Self::building(false, true)
    }

    fn building(plays: bool, speaks: bool) -> Arc<Self> {
        Arc::new(Self {
            next_screen: AtomicI64::new(100),
            heard: Arc::new(Heard::default()),
            opened: Mutex::new(Vec::new()),
            links: Mutex::new(Vec::new()),
            fail: Mutex::new(None),
            limits: ShowLimits {
                max_width: 1920,
                max_height: 1200,
                max_fps: 60,
                codecs: vec![Codec::H264],
            },
            plays,
            speaks,
            sounds: Arc::new(Heard::default()),
        })
    }
}

impl WindowViewer for Viewer {
    fn limits(&self) -> ShowLimits {
        self.limits.clone()
    }

    fn open(&self, window: &WindowOffer, link: ScreenLink) -> Result<Screen, PlatformError> {
        if let Some(why) = self.fail.lock().unwrap().clone() {
            return Err(PlatformError::new(why));
        }
        self.opened.lock().unwrap().push(Opened {
            title: window.title.clone(),
            width: window.width,
            height: window.height,
            control: window.control,
        });
        self.links.lock().unwrap().push(link);
        let screen = Screen::new(
            Box::new(Screening(Arc::clone(&self.heard))),
            self.next_screen.fetch_add(1, Ordering::Relaxed),
        );
        Ok(if self.speaks {
            screen.with_speaker(Speaking(Arc::clone(&self.sounds)))
        } else {
            screen
        })
    }

    fn plays_sound(&self) -> bool {
        self.plays
    }
}

/// The speaker of a screen: it keeps a line for each call.
struct Speaking(Arc<Heard>);

impl Speaker for Speaking {
    fn sound(&mut self, sound: Sound) {
        self.0
            .push(format!("sound {:?} {}", sound.codec, sound.samples));
    }

    fn play(&mut self, play: Play, packet: Vec<u8>) {
        self.0.push(format!(
            "play {} lost {} first {} bytes {}",
            play.number,
            play.lost_before,
            play.first,
            packet.len()
        ));
    }

    fn stopped(&mut self, why: Option<SilentReason>) {
        self.0.push(format!("stopped {why:?}"));
    }

    fn ended(&mut self) {
        self.0.push("ended");
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

    fn source(&mut self, message: SourceMessage) {
        if matches!(message, SourceMessage::Config(_)) {
            self.0.push("config");
        }
    }

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

// The platform of a device that shares windows.

/// A window that the next pick gives.
#[derive(Clone)]
struct Chosen {
    display: bool,
    title: &'static str,
    app: &'static str,
    size: (u32, u32),
}

#[allow(clippy::struct_excessive_bools)] // what the platform can do, each of its own
struct Sharer {
    /// Whether the platform can post input. It can change while the app runs, as it does when the
    /// person takes back the consent to control the computer.
    control: AtomicBool,
    /// What the next pick gives; `None` is a person who gave up.
    next: Mutex<Option<Chosen>>,
    /// If set, the pick fails with this.
    fail: Mutex<Option<String>>,
    /// Waited for by the pick, if set, before it gives its answer.
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    heard: Arc<Heard>,
    /// The handle of the session that started, for the test to end it or to send pictures.
    handle: Arc<Mutex<Option<Arc<SourceHandle>>>>,
    /// How many windows were dropped, which is the platform letting go of what it keeps for them.
    dropped: Arc<AtomicUsize>,
    /// How many pictures to send once the session runs.
    pictures: usize,
    /// Whether the platform can make a picture within what the viewer takes.
    makes_pictures: AtomicBool,
    /// Whether the platform gives up the session as soon as it is started, which is how it ends
    /// one quietly.
    lets_go_at_once: AtomicBool,
    /// If set, the platform ends the session as soon as it is started, with this sentence, which
    /// is how it says that the capture could not begin.
    fails_at_once: Mutex<Option<String>>,
    /// The means of the session that started to say that something went wrong, for the test to
    /// use at the time of its choosing.
    failure: Arc<Mutex<Option<SharingFailure>>>,
    /// What the person at the other device did, as it reached the platform.
    inputs: Arc<Mutex<Vec<InputEvent>>>,
    /// Whether the platform can give its screen without the person picking it.
    gives_screen: bool,
    /// What the next call for the screen does.
    screen_plan: Mutex<ScreenPlan>,
    /// Waited for by the call for the screen, if set, before it gives its answer.
    screen_gate: Mutex<Option<oneshot::Receiver<()>>>,
    /// How many times the screen was asked for.
    screens: AtomicUsize,
    /// What the screen was asked for as, each time.
    privacies: Mutex<Vec<Privacy>>,
    /// Whether the platform can lift what keeps a screen private that it gave as private.
    lifts: bool,
    /// Whether the platform can end a private session and leave the screen as it is, which is
    /// ending it without locking it.
    leaves: bool,
    /// Whether a window that the person picks is given as private too, as a platform that lets the
    /// person pick a whole display could, and not only the screen that the other device asks for.
    /// It needs `lifts`.
    picks_private: AtomicBool,
    /// What the people at the other device lifted of the privacy, as it reached the platform.
    lifted: Arc<Mutex<Vec<Lift>>>,
    /// How many times the person was given a picker.
    picks: AtomicUsize,
    /// Whether the platform can send the sound of a whole display.
    sounds: bool,
}

/// What the platform does when it is asked for its screen.
#[derive(Clone)]
enum ScreenPlan {
    /// Gives this.
    Gives(Chosen),
    /// Says why not.
    Refuses(ScreenError),
}

/// What a platform can do besides sharing the window that the person picked.
#[allow(clippy::struct_excessive_bools)] // what the platform can do, each of its own
#[derive(Clone, Copy, Default)]
struct Can {
    /// Give its screen without the person picking it.
    screen: bool,
    /// Lift what keeps that screen private.
    lift: bool,
    /// End a private session and leave that screen as it is.
    leave: bool,
    /// Send the sound of a whole display.
    sound: bool,
}

impl Sharer {
    fn new(control: bool, next: Option<Chosen>) -> Arc<Self> {
        Self::building(control, next, Can::default())
    }

    /// A platform that can give its screen without the person picking it.
    fn with_screen(control: bool) -> Arc<Self> {
        let can = Can {
            screen: true,
            ..Can::default()
        };
        Self::building(control, None, can)
    }

    /// A platform that can give its screen without the person picking it, and can lift what keeps
    /// that screen private when the person at the other device says so.
    fn with_lift(control: bool) -> Arc<Self> {
        let can = Can {
            screen: true,
            lift: true,
            ..Can::default()
        };
        Self::building(control, None, can)
    }

    /// A platform that can give its screen without the person picking it, lift what keeps that
    /// screen private, and end a private session without locking the screen.
    fn with_leave(control: bool) -> Arc<Self> {
        let can = Can {
            screen: true,
            lift: true,
            leave: true,
            ..Can::default()
        };
        Self::building(control, None, can)
    }

    /// A platform that can send the sound of a whole display.
    fn with_sound(next: Chosen) -> Arc<Self> {
        let can = Can {
            sound: true,
            ..Can::default()
        };
        Self::building(false, Some(next), can)
    }

    /// A platform that can give its screen without the person picking it, and send its sound.
    fn with_screen_and_sound(control: bool) -> Arc<Self> {
        let can = Can {
            screen: true,
            sound: true,
            ..Can::default()
        };
        Self::building(control, None, can)
    }

    fn building(control: bool, next: Option<Chosen>, can: Can) -> Arc<Self> {
        Arc::new(Self {
            control: AtomicBool::new(control),
            next: Mutex::new(next),
            fail: Mutex::new(None),
            gate: Mutex::new(None),
            heard: Arc::new(Heard::default()),
            handle: Arc::new(Mutex::new(None)),
            dropped: Arc::new(AtomicUsize::new(0)),
            pictures: 3,
            makes_pictures: AtomicBool::new(true),
            lets_go_at_once: AtomicBool::new(false),
            fails_at_once: Mutex::new(None),
            failure: Arc::new(Mutex::new(None)),
            inputs: Arc::new(Mutex::new(Vec::new())),
            gives_screen: can.screen,
            screen_plan: Mutex::new(ScreenPlan::Gives(whole_display())),
            screen_gate: Mutex::new(None),
            screens: AtomicUsize::new(0),
            privacies: Mutex::new(Vec::new()),
            lifts: can.lift,
            leaves: can.leave,
            picks_private: AtomicBool::new(false),
            lifted: Arc::new(Mutex::new(Vec::new())),
            picks: AtomicUsize::new(0),
            sounds: can.sound,
        })
    }

    /// What the screen was asked for as, in order.
    fn privacies(&self) -> Vec<Privacy> {
        self.privacies.lock().unwrap().clone()
    }

    /// What the platform was given of what the person at the other device did, in order.
    fn inputs(&self) -> Vec<InputEvent> {
        self.inputs.lock().unwrap().clone()
    }

    /// What the people at the other device lifted of the privacy of the screen, in order, as the
    /// platform heard it.
    fn lifted(&self) -> Vec<Lift> {
        self.lifted.lock().unwrap().clone()
    }

    /// The window that the platform keeps for `chosen`, as the session will use it.
    fn picked(&self, chosen: Option<Chosen>) -> Picked {
        Picked {
            chosen,
            heard: Arc::clone(&self.heard),
            handle: Arc::clone(&self.handle),
            dropped: Arc::clone(&self.dropped),
            pictures: self.pictures,
            makes_pictures: self.makes_pictures.load(Ordering::Relaxed),
            lets_go_at_once: self.lets_go_at_once.load(Ordering::Relaxed),
            fails_at_once: self.fails_at_once.lock().unwrap().clone(),
            failure: Arc::clone(&self.failure),
            inputs: Arc::clone(&self.inputs),
            private: self.picks_private.load(Ordering::Relaxed) && self.lifts,
            lifted: Arc::clone(&self.lifted),
        }
    }
}

fn untitled_window() -> Chosen {
    Chosen {
        display: false,
        title: "  ",
        app: "",
        size: (800, 600),
    }
}

fn report_window() -> Chosen {
    Chosen {
        display: false,
        title: "Report.pdf",
        app: "Preview",
        size: (1440, 900),
    }
}

/// A whole display, which has no title and no application.
fn whole_display() -> Chosen {
    Chosen {
        display: true,
        title: "",
        app: "",
        size: (3024, 1964),
    }
}

impl WindowSharer for Sharer {
    fn control(&self) -> bool {
        self.control.load(Ordering::Relaxed)
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        self.picks.fetch_add(1, Ordering::Relaxed);
        if let Some(why) = self.fail.lock().unwrap().clone() {
            return Box::pin(async move { Err(PlatformError::new(why)) });
        }
        let chosen = self.next.lock().unwrap().take();
        let gate = self.gate.lock().unwrap().take();
        let shared = self.picked(chosen.clone());
        Box::pin(async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            Ok(chosen.map(|_| Box::new(shared) as Box<dyn PickedWindow>))
        })
    }

    fn can_give_screen(&self) -> bool {
        self.gives_screen
    }

    fn can_lift(&self) -> bool {
        self.lifts
    }

    fn can_leave(&self) -> bool {
        self.leaves
    }

    fn sends_sound(&self) -> bool {
        self.sounds
    }

    fn screen(
        &self,
        privacy: Privacy,
    ) -> BoxFuture<'static, Result<Box<dyn PickedWindow>, ScreenError>> {
        self.screens.fetch_add(1, Ordering::Relaxed);
        self.privacies.lock().unwrap().push(privacy);
        let plan = self.screen_plan.lock().unwrap().clone();
        let gate = self.screen_gate.lock().unwrap().take();
        let picked = match plan {
            ScreenPlan::Gives(chosen) => {
                let mut picked = self.picked(Some(chosen));
                picked.private = privacy == Privacy::Private && self.lifts;
                Ok(picked)
            }
            ScreenPlan::Refuses(error) => Err(error),
        };
        Box::pin(async move {
            // The window is made before the wait, so that a call that is dropped while it waits
            // lets go of it.
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            picked.map(|picked| Box::new(picked) as Box<dyn PickedWindow>)
        })
    }
}

struct Picked {
    chosen: Option<Chosen>,
    heard: Arc<Heard>,
    handle: Arc<Mutex<Option<Arc<SourceHandle>>>>,
    dropped: Arc<AtomicUsize>,
    pictures: usize,
    makes_pictures: bool,
    lets_go_at_once: bool,
    fails_at_once: Option<String>,
    failure: Arc<Mutex<Option<SharingFailure>>>,
    inputs: Arc<Mutex<Vec<InputEvent>>>,
    /// Whether the screen was given as private by a platform that can lift what keeps it so.
    private: bool,
    lifted: Arc<Mutex<Vec<Lift>>>,
}

impl Drop for Picked {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

impl PickedWindow for Picked {
    fn display(&self) -> bool {
        self.chosen.as_ref().is_some_and(|chosen| chosen.display)
    }

    fn private(&self) -> bool {
        self.private
    }

    fn title(&self) -> String {
        self.chosen.as_ref().map(|c| c.title.to_owned()).unwrap()
    }

    fn app(&self) -> String {
        self.chosen.as_ref().map(|c| c.app.to_owned()).unwrap()
    }

    fn size(&self) -> (u32, u32) {
        self.chosen.as_ref().map(|c| c.size).unwrap()
    }

    fn events(&mut self) -> Box<dyn SourceEvents> {
        Box::new(Hearing(
            Arc::clone(&self.heard),
            Arc::clone(&self.inputs),
            Arc::clone(&self.lifted),
        ))
    }

    fn prepare(&mut self, takes: &Takes) -> Option<Config> {
        self.heard.push(format!(
            "takes {}x{} at {} fps, {:?}",
            takes.max_width, takes.max_height, takes.max_fps, takes.codecs
        ));
        (self.makes_pictures && takes.codecs.contains(&Codec::H264)).then(|| Config {
            epoch: 0,
            codec: Codec::H264,
            width: takes.max_width.min(self.size().0),
            height: takes.max_height.min(self.size().1),
            fps: takes.max_fps.min(30),
        })
    }

    fn start(self: Box<Self>, handle: Arc<SourceHandle>, failure: SharingFailure) {
        if self.lets_go_at_once {
            return;
        }
        if let Some(why) = &self.fails_at_once {
            failure.fail(why.clone());
            return;
        }
        *self.failure.lock().unwrap() = Some(failure);
        *self.handle.lock().unwrap() = Some(Arc::clone(&handle));
        let pictures = self.pictures;
        std::thread::spawn(move || {
            let deadline = Instant::now() + WAIT;
            let mut sent = 0;
            while sent < pictures && Instant::now() < deadline {
                if handle.begin().is_some() {
                    let picture = Picture {
                        bytes: vec![0x42; 2000],
                        referenced: true,
                        ltr: None,
                        capture_time: 16_000 * sent as u64,
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

struct Hearing(
    Arc<Heard>,
    Arc<Mutex<Vec<InputEvent>>>,
    Arc<Mutex<Vec<Lift>>>,
);

impl SourceEvents for Hearing {
    fn report(&mut self, _report: Report) {
        self.0.push("report");
    }

    fn want(&mut self, _want: Want) {
        self.0.push("want");
    }

    fn input(&mut self, batch: InputBatch, _arrived: Duration) {
        self.0.push("input");
        self.1.lock().unwrap().extend(batch.events);
    }

    fn held(&mut self, _tokens: Vec<LtrToken>) {}

    fn lift(&mut self, lift: Lift) {
        self.0.push("lift");
        self.2.lock().unwrap().push(lift);
    }

    fn hear(&mut self, on: bool) {
        self.0.push(format!("hear {}", u8::from(on)));
    }

    fn recovery(&mut self) {
        self.0.push("recovery");
    }

    fn ended(&mut self) {
        self.0.push("ended");
    }

    fn ended_leaving(&mut self) {
        self.0.push("ended leaving");
    }
}

// Two devices.

struct Dev {
    app: App,
    _dir: TempDir,
}

impl Dev {
    async fn start(name: &str, windows: Windows) -> Self {
        let dir = tempfile::tempdir().unwrap();
        write_settings(dir.path(), name);
        let app = App::start_with(Options::new(dir.path()), windows)
            .await
            .unwrap();
        Self { app, _dir: dir }
    }

    fn id(&self) -> String {
        self.app.me().id
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

/// A sharing device, a viewing device, and the platforms they were given.
struct Pair {
    sharing: Dev,
    viewing: Dev,
    sharer: Arc<Sharer>,
    viewer: Arc<Viewer>,
}

impl Pair {
    async fn new(control: bool) -> Self {
        Self::with_sharer(Sharer::new(control, Some(report_window()))).await
    }

    async fn with_sharer(sharer: Arc<Sharer>) -> Self {
        Self::with_platforms(sharer, Viewer::new()).await
    }

    async fn with_platforms(sharer: Arc<Sharer>, viewer: Arc<Viewer>) -> Self {
        let sharing = Dev::start("Studio Mac", Windows::none().sharing(sharer.clone())).await;
        let viewing = Dev::start("Kitchen iPad", Windows::none().showing(viewer.clone())).await;
        pair(&sharing, &viewing).await;
        Self {
            sharing,
            viewing,
            sharer,
            viewer,
        }
    }

    /// The window that the viewing device is asked about.
    async fn requested(&self, events: &mut Events) -> WindowRequest {
        next_matching(events, |event| match event {
            Event::WindowRequested(request) => Some(request),
            _ => None,
        })
        .await
    }

    /// Offers the picked window from the sharing device, answers it on the viewing device, and
    /// gives the result of the offer.
    async fn offered_and_answered(&self, accept: bool, control: bool) -> WindowShared {
        let mut events = self.viewing.app.events();
        let app = self.sharing.app.clone();
        let to = self.viewing.id();
        let offer = tokio::spawn(async move { app.share_window(&to, control).await });
        let request = self.requested(&mut events).await;
        assert!(self.viewing.app.answer_window(request.id, accept));
        offer.await.unwrap().unwrap()
    }
}

#[tokio::test]
async fn what_a_device_can_do_with_windows_depends_on_the_platform_it_was_given() {
    let plain = Dev::start("Plain", Windows::none()).await;
    let shows = Dev::start("Shows", Windows::none().showing(Viewer::new())).await;
    let shares = Dev::start("Shares", Windows::none().sharing(Sharer::new(false, None))).await;
    let controls = Dev::start(
        "Controls",
        Windows::none()
            .showing(Viewer::new())
            .sharing(Sharer::new(true, None)),
    )
    .await;
    let gives = Dev::start("Gives", Windows::none().sharing(Sharer::with_screen(false))).await;
    let gives_and_controls = Dev::start(
        "Gives and controls",
        Windows::none().sharing(Sharer::with_screen(true)),
    )
    .await;
    let abilities = |dev: &Dev| dev.app.window_abilities();
    let can = |can_show, can_share, can_control, can_give_screen| WindowAbilities {
        can_show,
        can_share,
        can_control,
        can_give_screen,
    };
    assert_eq!(abilities(&plain), can(false, false, false, false));
    assert_eq!(abilities(&shows), can(true, false, false, false));
    assert_eq!(abilities(&shares), can(false, true, false, false));
    assert_eq!(abilities(&controls), can(true, true, true, false));
    // Giving the screen is something a platform says it can do, and is a thing apart from taking
    // input.
    assert_eq!(abilities(&gives), can(false, true, false, true));
    assert_eq!(abilities(&gives_and_controls), can(false, true, true, true));
}

#[tokio::test]
async fn a_window_goes_from_one_device_to_the_other_and_the_viewer_ends_it() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let mut sharing_events = pair.sharing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });

    // The person at the viewer is asked, and is told what the window is and who offers it.
    let request = pair.requested(&mut viewing_events).await;
    assert_eq!(request.device_id, pair.sharing.id());
    assert_eq!(request.device_name, "Studio Mac");
    assert_eq!(request.title.as_deref(), Some("Report.pdf"));
    assert_eq!(request.app.as_deref(), Some("Preview"));
    assert_eq!((request.width, request.height), (1440, 900));
    assert!(!request.control);
    assert!(!request.display, "a window was offered as a display");
    assert!(pair.viewing.app.answer_window(request.id, true));

    // The window is shown, with the screen that the platform made for it.
    let shown = next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    assert_eq!(shown.role, WindowRole::Viewing);
    assert_eq!(shown.device_id, pair.sharing.id());
    assert_eq!(shown.title.as_deref(), Some("Report.pdf"));
    assert_eq!(shown.screen, Some(100));
    assert!(!shown.control);
    assert!(!shown.display);
    assert_eq!(
        pair.viewer.opened.lock().unwrap().as_slice(),
        [Opened::report(false)]
    );

    // The device that shares hears of it too, from the call and from the event.
    let WindowShared::Started(shared) = offer.await.unwrap().unwrap() else {
        panic!("the window was not shared");
    };
    assert_eq!(shared.role, WindowRole::Sharing);
    assert_eq!(shared.device_id, pair.viewing.id());
    assert_eq!(shared.device_name, "Kitchen iPad");
    assert_eq!(shared.screen, None);
    assert!(!shared.display);
    let event = next_matching(&mut sharing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    assert_eq!(event, shared);

    // Pictures go from the encoder of the one to the decoder of the other, and the viewer's
    // reports reach the platform of the source.
    until("the pictures to be decoded and shown", || {
        pair.viewer.heard.has("decode 2") && pair.viewer.heard.has("show 2")
    })
    .await;
    assert!(pair.viewer.heard.has("config"));
    until("the source to hear a report", || {
        pair.sharer.heard.count("report") > 0
    })
    .await;
    // What the viewer can take is what the platform of the source chooses the picture within.
    assert!(
        pair.sharer.heard.has("takes 1920x1200 at 60 fps, [H264]"),
        "{:?}",
        pair.sharer.heard.0.lock().unwrap()
    );

    // The person at the viewer closes the window: both ends hear how it ended, and both
    // platforms are told.
    assert!(pair.viewing.app.end_window(shown.id));
    let ended = next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shown.id, WindowEnd::ThisDevice));
    let ended = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shared.id, WindowEnd::OtherDevice));
    until("both platforms to be told", || {
        pair.viewer.heard.has("ended") && pair.sharer.heard.has("ended")
    })
    .await;
    // A window that ended is gone, and ending it again does nothing.
    assert!(!pair.viewing.app.end_window(shown.id));
    assert!(!pair.sharing.app.end_window(shared.id));
}

#[tokio::test]
async fn a_whole_display_is_offered_shown_and_shared_as_a_display() {
    let pair = Pair::new(false).await;
    *pair.sharer.next.lock().unwrap() = Some(whole_display());
    let mut viewing_events = pair.viewing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });

    // The person at the viewer is told that it is a display, which has no title.
    let request = pair.requested(&mut viewing_events).await;
    assert!(request.display);
    assert_eq!((request.title, request.app), (None, None));
    assert_eq!((request.width, request.height), (3024, 1964));
    assert!(!request.control);
    assert!(pair.viewing.app.answer_window(request.id, true));

    // It is a display where it is shown, and where it is shared.
    let shown = next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    assert_eq!(shown.role, WindowRole::Viewing);
    assert!(shown.display);
    let WindowShared::Started(shared) = offer.await.unwrap().unwrap() else {
        panic!("the display was not shared");
    };
    assert_eq!(shared.role, WindowRole::Sharing);
    assert!(shared.display);
    assert_eq!((shared.width, shared.height), (3024, 1964));
}

#[tokio::test]
async fn a_whole_display_is_offered_with_control_when_the_person_asked_for_it() {
    // The platform can grant control, and the person asked for it.
    let pair = Pair::new(true).await;
    *pair.sharer.next.lock().unwrap() = Some(whole_display());
    let mut viewing_events = pair.viewing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, true).await });

    let request = pair.requested(&mut viewing_events).await;
    assert!(request.display);
    assert!(request.control, "a display was not offered with control");
    assert!(pair.viewing.app.answer_window(request.id, true));
    let WindowShared::Started(shared) = offer.await.unwrap().unwrap() else {
        panic!("the display was not shared");
    };
    assert!(shared.control);
}

#[tokio::test]
async fn a_no_to_a_whole_display_is_told_as_a_no_to_a_screen() {
    let pair = Pair::new(false).await;
    *pair.sharer.next.lock().unwrap() = Some(whole_display());
    let WindowShared::Declined(why) = pair.offered_and_answered(false, false).await else {
        panic!("the display should have been declined");
    };
    assert!(why.contains("Kitchen iPad"), "{why}");
    assert!(why.contains("the screen"), "{why}");
    assert!(!why.contains("window"), "{why}");
}

#[tokio::test]
async fn a_viewer_that_takes_no_picture_of_a_display_is_told_the_screen_is_off() {
    let pair = Pair::new(false).await;
    *pair.sharer.next.lock().unwrap() = Some(whole_display());
    pair.sharer.makes_pictures.store(false, Ordering::Relaxed);
    let WindowShared::Declined(why) = pair.offered_and_answered(true, false).await else {
        panic!("the display should not have started");
    };
    assert!(why.contains("picture"), "{why}");
    assert!(why.contains("the screen"), "{why}");
    assert!(!why.contains("window"), "{why}");
}

/// A window larger than the viewer takes, so that the picture of it is not the size of the window.
fn large_window() -> Chosen {
    Chosen {
        display: false,
        title: "Plan.dwg",
        app: "Drafting",
        size: (3000, 2000),
    }
}

/// The next of the two events that tell a screen of a window and of its picture.
async fn next_window_or_picture(events: &mut Events) -> Event {
    next_matching(events, |event| match event {
        Event::WindowStarted(_) | Event::WindowPicture { .. } => Some(event),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_screen_is_told_the_size_of_the_picture_and_not_that_of_the_window() {
    let pair = Pair::new(false).await;
    *pair.sharer.next.lock().unwrap() = Some(large_window());
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };

    // The window comes first. The platform of the source made the picture fit what the viewer
    // takes, which is smaller than the window and not in the same shape.
    let Event::WindowStarted(shown) = next_window_or_picture(&mut viewing_events).await else {
        panic!("the size of the picture came before the window");
    };
    assert_eq!((shown.width, shown.height), (3000, 2000));
    assert_eq!(
        next_window_or_picture(&mut viewing_events).await,
        Event::WindowPicture {
            id: shown.id,
            width: 1920,
            height: 1200
        }
    );
    // The decoder of the platform heard what the source said all the same.
    until("the decoder to hear the source", || {
        pair.viewer.heard.has("config")
    })
    .await;
}

#[tokio::test]
async fn a_screen_is_told_again_when_the_source_changes_the_size_of_the_picture() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let Event::WindowStarted(shown) = next_window_or_picture(&mut viewing_events).await else {
        panic!("the size of the picture came before the window");
    };
    assert_eq!(
        next_window_or_picture(&mut viewing_events).await,
        Event::WindowPicture {
            id: shown.id,
            width: 1440,
            height: 900
        }
    );

    // The window was resized: the source starts a new size of picture, and says so.
    until("the platform of the source to be started", || {
        pair.sharer.handle.lock().unwrap().is_some()
    })
    .await;
    let handle = pair.sharer.handle.lock().unwrap().clone().unwrap();
    handle
        .new_epoch(Config {
            epoch: 1,
            codec: Codec::H264,
            width: 800,
            height: 800,
            fps: 30,
        })
        .unwrap();
    assert_eq!(
        next_window_or_picture(&mut viewing_events).await,
        Event::WindowPicture {
            id: shown.id,
            width: 800,
            height: 800
        }
    );
    until("the decoder to hear both", || {
        pair.viewer.heard.count("config") == 2
    })
    .await;
}

#[tokio::test]
async fn a_window_that_is_shared_from_here_has_no_size_of_picture_to_tell() {
    let pair = Pair::new(false).await;
    let mut sharing_events = pair.sharing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;
    let mut heard = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(QUIET, sharing_events.next()).await {
        heard.push(event);
    }
    assert!(
        heard
            .iter()
            .any(|event| matches!(event, Event::WindowStarted(_))),
        "{heard:?}"
    );
    assert!(
        !heard
            .iter()
            .any(|event| matches!(event, Event::WindowPicture { .. })),
        "{heard:?}"
    );
}

/// The window that the viewing device shows, as the screens hear of it.
async fn shown_window(events: &mut Events) -> WindowSession {
    next_matching(events, |event| match event {
        Event::WindowStarted(session) if session.role == WindowRole::Viewing => Some(session),
        _ => None,
    })
    .await
}

fn touch(action: WindowPointer) -> WindowInput {
    WindowInput::Pointer {
        x: 0.25,
        y: 0.75,
        modifiers: WindowModifiers::default(),
        action,
    }
}

fn key(code: u16, down: bool) -> WindowInput {
    WindowInput::Key {
        code,
        down,
        modifiers: WindowModifiers::default(),
        repeat: false,
        text: None,
    }
}

#[tokio::test]
async fn what_the_person_does_on_a_window_reaches_the_platform_of_the_device_that_shares_it() {
    let pair = Pair::new(true).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, true).await else {
        panic!("the window was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    assert!(shown.control);
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    // A touch is a position in the picture that is on the screen, a key is a key.
    assert!(
        pair.viewing
            .app
            .window_input(shown.id, touch(WindowPointer::Down(WindowButton::Left)))
    );
    assert!(
        pair.viewing
            .app
            .window_input(shown.id, touch(WindowPointer::Up(WindowButton::Left)))
    );
    assert!(pair.viewing.app.window_input(shown.id, key(4, true)));
    assert!(pair.viewing.app.window_input(shown.id, key(4, false)));
    until("the platform of the source to hear all four", || {
        pair.sharer.inputs().len() == 4
    })
    .await;
    let pointer = |action| InputEvent::Pointer {
        frame: 2,
        x: 0.25,
        y: 0.75,
        modifiers: Modifiers::NONE,
        action,
    };
    let key = |down| InputEvent::Key {
        code: 4,
        down,
        modifiers: Modifiers::NONE,
        repeat: false,
        text: None,
    };
    assert_eq!(
        pair.sharer.inputs(),
        [
            pointer(PointerAction::Down(Button::Left)),
            pointer(PointerAction::Up(Button::Left)),
            key(true),
            key(false),
        ]
    );
}

#[tokio::test]
async fn a_window_that_the_other_device_did_not_let_this_one_control_takes_no_input() {
    // The device that shares could grant control, and its person did not.
    let pair = Pair::new(true).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    assert!(!shown.control);
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    assert!(
        !pair
            .viewing
            .app
            .window_input(shown.id, touch(WindowPointer::Move))
    );
    assert!(!pair.viewing.app.window_input(shown.id, key(4, true)));
    // The release of what was never pressed is as nothing, and nothing is sent.
    pair.viewing.app.release_window_input(shown.id);
    tokio::time::sleep(QUIET).await;
    assert_eq!(pair.sharer.inputs(), []);
}

#[tokio::test]
async fn input_for_a_window_that_is_not_shown_here_is_not_taken() {
    let pair = Pair::new(true).await;
    let WindowShared::Started(shared) = pair.offered_and_answered(true, true).await else {
        panic!("the window was not shared");
    };
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    // A window that this device shares is looked at by the other one, and a number that no window
    // has is no window.
    for session in [shared.id, shared.id + 100] {
        assert!(
            !pair
                .sharing
                .app
                .window_input(session, touch(WindowPointer::Move))
        );
        pair.sharing.app.release_window_input(session);
    }
    tokio::time::sleep(QUIET).await;
    assert_eq!(pair.sharer.inputs(), []);
}

#[tokio::test]
async fn a_position_that_means_nothing_is_not_taken_and_the_window_goes_on() {
    let pair = Pair::new(true).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, true).await else {
        panic!("the window was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    let outside = WindowInput::Pointer {
        x: 1.5,
        y: 0.5,
        modifiers: WindowModifiers::default(),
        action: WindowPointer::Move,
    };
    let not_a_number = WindowInput::Pointer {
        x: f32::NAN,
        y: 0.5,
        modifiers: WindowModifiers::default(),
        action: WindowPointer::Move,
    };
    assert!(!pair.viewing.app.window_input(shown.id, outside));
    assert!(!pair.viewing.app.window_input(shown.id, not_a_number));
    assert!(
        !pair
            .viewing
            .app
            .window_input(shown.id, touch(WindowPointer::Up(WindowButton::Right)))
    );
    assert!(!pair.viewing.app.window_input(shown.id, key(4, false)));
    // What does mean something still goes through.
    assert!(
        pair.viewing
            .app
            .window_input(shown.id, touch(WindowPointer::Move))
    );
    until("the platform of the source to hear it", || {
        pair.sharer.inputs().len() == 1
    })
    .await;
    assert!(matches!(
        pair.sharer.inputs()[..],
        [InputEvent::Pointer {
            action: PointerAction::Move,
            ..
        }]
    ));
}

#[tokio::test]
async fn what_is_down_is_lifted_when_the_screen_says_the_person_has_left() {
    let pair = Pair::new(true).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, true).await else {
        panic!("the window was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    assert!(
        pair.viewing
            .app
            .window_input(shown.id, touch(WindowPointer::Down(WindowButton::Left)))
    );
    assert!(pair.viewing.app.window_input(shown.id, key(4, true)));
    until("the platform of the source to hear both", || {
        pair.sharer.inputs().len() == 2
    })
    .await;

    pair.viewing.app.release_window_input(shown.id);
    until("the platform of the source to hear them lifted", || {
        pair.sharer.inputs().len() == 4
    })
    .await;
    let inputs = pair.sharer.inputs();
    assert!(
        matches!(
            inputs[2],
            InputEvent::Pointer {
                frame: 2,
                x: 0.25,
                y: 0.75,
                action: PointerAction::Up(Button::Left),
                ..
            }
        ),
        "{inputs:?}"
    );
    assert!(
        matches!(
            inputs[3],
            InputEvent::Key {
                code: 4,
                down: false,
                ..
            }
        ),
        "{inputs:?}"
    );
}

#[tokio::test]
async fn what_the_other_device_dropped_is_told_to_the_screen_of_the_window() {
    let pair = Pair::new(true).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, true).await else {
        panic!("the window was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    until("the platform of the source to be started", || {
        pair.sharer.handle.lock().unwrap().is_some()
    })
    .await;
    let handle = pair.sharer.handle.lock().unwrap().clone().unwrap();

    // Two events were dropped because a window was in front, one because the Mac is locked: the
    // notice names the reason that dropped the most.
    handle.dropped(Reason::Covered);
    handle.dropped(Reason::Locked);
    handle.dropped(Reason::Covered);
    let notice = next_matching(&mut viewing_events, |event| match event {
        Event::WindowNotice { id, reason, count } => Some((id, reason, count)),
        _ => None,
    })
    .await;
    assert_eq!(notice, (shown.id, WindowNoticeReason::Covered, 2));
}

#[tokio::test]
async fn a_decoder_that_fails_after_it_took_a_frame_has_the_viewer_ask_for_a_new_key_frame() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(_) => Some(()),
        _ => None,
    })
    .await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;
    // Nothing is lost, so the source is not asked for anything more. How many times it was asked
    // while the first picture was on its way depends on the timing, so this counts from here.
    tokio::time::sleep(QUIET).await;
    let asked = pair.sharer.heard.count("recovery");

    // The decoder fails later, from a thread of its own, with the link that `open` gave it.
    let link = pair.viewer.links.lock().unwrap()[0].clone();
    link.decode_failed();
    until("the source to be asked for a new key frame", || {
        pair.sharer.heard.count("recovery") > asked
    })
    .await;
}

#[tokio::test]
async fn a_link_that_the_session_has_not_reached_yet_or_has_left_does_nothing() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let shown = next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    // The source has made its side of the session once its first picture is shown, so ending the
    // session now tells it at once.
    until("the first picture to be shown", || {
        pair.viewer.heard.has("show 0")
    })
    .await;
    let link = pair.viewer.links.lock().unwrap()[0].clone();
    assert!(pair.viewing.app.end_window(shown.id));
    until("both platforms to be told", || {
        pair.viewer.heard.has("ended") && pair.sharer.heard.has("ended")
    })
    .await;
    let recoveries = pair.sharer.heard.count("recovery");

    // Nothing is left of the session for the link to reach, and the link does not keep it.
    link.decode_failed();
    link.sound_failed();
    tokio::time::sleep(QUIET).await;
    assert_eq!(pair.sharer.heard.count("recovery"), recoveries);
    assert!(!pair.sharing.app.end_window(shared.id));

    // A link that no session was ever made for does nothing either.
    ScreenLink::default().decode_failed();
    ScreenLink::default().sound_failed();
}

#[tokio::test]
async fn the_device_that_shares_can_end_the_window_too() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(_) => Some(()),
        _ => None,
    })
    .await;
    assert!(pair.sharing.app.end_window(shared.id));
    let end = next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { end, .. } => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end, WindowEnd::OtherDevice);
}

#[tokio::test]
async fn a_platform_that_lets_go_of_the_window_ends_the_session() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let mut sharing_events = pair.sharing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let shown = next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;
    // The platform's thread drops its copy of the handle once it has sent its pictures. The test
    // holds the other, as a platform holds one for as long as it shares the window, and the
    // platform lets go of it from a thread of its own: whichever copy goes last ends the session.
    let handle = pair.sharer.handle.lock().unwrap().take().unwrap();
    std::thread::spawn(move || drop(handle)).join().unwrap();

    // Nobody ended the window by name: the platform let go of it, which is how it ends one.
    let ended = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shared.id, WindowEnd::ThisDevice));
    let ended = next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shown.id, WindowEnd::OtherDevice));
    until("both platforms to be told", || {
        pair.viewer.heard.has("ended") && pair.sharer.heard.has("ended")
    })
    .await;
    assert!(!pair.sharing.app.end_window(shared.id));
}

#[tokio::test]
async fn a_platform_that_lets_go_of_the_window_at_once_ends_it_without_a_word() {
    let pair = Pair::new(false).await;
    pair.sharer.lets_go_at_once.store(true, Ordering::Relaxed);
    let mut viewing_events = pair.viewing.app.events();
    let mut sharing_events = pair.sharing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let shown = next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;

    // The window was started, and it ended at once.
    let started = next_matching(&mut sharing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    assert_eq!(started, shared);
    let ended = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shared.id, WindowEnd::ThisDevice));
    let ended = next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shown.id, WindowEnd::OtherDevice));
    until("both platforms to be told", || {
        pair.viewer.heard.has("ended") && pair.sharer.heard.has("ended")
    })
    .await;
    assert!(pair.sharer.handle.lock().unwrap().is_none());
    assert!(!pair.sharing.app.end_window(shared.id));
}

/// A device that shares a window whose platform ends the session with `sentence` as soon as it
/// is started, and the events of both devices.
async fn failed_at_once(sentence: &str) -> (Pair, Events, Events, WindowSession, WindowSession) {
    let pair = Pair::new(false).await;
    *pair.sharer.fails_at_once.lock().unwrap() = Some(sentence.to_owned());
    let mut viewing_events = pair.viewing.app.events();
    let sharing_events = pair.sharing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let shown = next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    (pair, sharing_events, viewing_events, shared, shown)
}

#[tokio::test]
async fn a_platform_that_cannot_begin_to_capture_says_why_and_the_person_is_told() {
    let (pair, mut sharing_events, mut viewing_events, shared, shown) =
        failed_at_once("Screen recording is turned off for Baton.").await;

    let ended = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(
        ended,
        (
            shared.id,
            WindowEnd::Failed("Screen recording is turned off for Baton.".to_owned())
        )
    );
    // The other device is only told that the window ended.
    let ended = next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shown.id, WindowEnd::OtherDevice));
    until("both platforms to be told", || {
        pair.viewer.heard.has("ended") && pair.sharer.heard.has("ended")
    })
    .await;
    assert!(!pair.sharing.app.end_window(shared.id));
}

#[tokio::test]
async fn the_sentence_of_the_platform_is_made_fit_for_a_screen() {
    let (_pair, mut sharing_events, _viewing_events, _shared, _shown) =
        failed_at_once("Screen recording\nis off.\u{202e}\u{7}").await;

    let end = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { end, .. } => Some(end),
        _ => None,
    })
    .await;

    assert_eq!(
        end,
        WindowEnd::Failed("Screen recording is off.".to_owned())
    );
}

#[tokio::test]
async fn a_sentence_with_nothing_in_it_ends_the_window_without_one() {
    let (_pair, mut sharing_events, _viewing_events, _shared, _shown) =
        failed_at_once(" \n\u{200b}").await;

    let end = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { end, .. } => Some(end),
        _ => None,
    })
    .await;

    assert_eq!(end, WindowEnd::ThisDevice);
}

#[tokio::test]
async fn a_platform_that_has_to_stop_later_says_why_and_the_first_sentence_counts() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let mut sharing_events = pair.sharing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(_) => Some(()),
        _ => None,
    })
    .await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    let failure = pair.sharer.failure.lock().unwrap().clone().unwrap();
    failure.fail("The system stopped the capture.");
    failure.fail("Something else.");

    let ended = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(
        ended,
        (
            shared.id,
            WindowEnd::Failed("The system stopped the capture.".to_owned())
        )
    );
    let ended = next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { end, .. } => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(ended, WindowEnd::OtherDevice);
}

#[tokio::test]
async fn a_window_that_the_other_device_ended_first_is_told_as_that_whatever_the_platform_says_after()
 {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let mut sharing_events = pair.sharing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let shown = next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    // A viewer that ends the window before the pictures come is not heard until the source's
    // liveness ends it, so the test waits for a picture.
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;
    assert!(pair.viewing.app.end_window(shown.id));
    let end = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { end, .. } => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end, WindowEnd::OtherDevice);

    let failure = pair.sharer.failure.lock().unwrap().clone().unwrap();
    failure.fail("Too late.");

    // Nothing is told twice.
    let more = tokio::time::timeout(
        QUIET,
        next_matching(&mut sharing_events, |event| match event {
            Event::WindowEnded { .. } => Some(()),
            _ => None,
        }),
    )
    .await;
    assert!(more.is_err(), "the end of a window was told twice");
}

#[tokio::test]
async fn a_sentence_that_comes_after_the_platform_ended_the_session_quietly_changes_nothing() {
    let pair = Pair::new(false).await;
    let mut sharing_events = pair.sharing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let handle = pair.sharer.handle.lock().unwrap().clone().unwrap();
    let failure = pair.sharer.failure.lock().unwrap().clone().unwrap();

    // The person closed the window, and the capture of another thread of the platform stopped at
    // the same moment. Nothing runs between the two calls, so the first is the end.
    handle.end();
    failure.fail("The capture stopped.");

    let ended = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shared.id, WindowEnd::ThisDevice));
}

#[tokio::test]
async fn a_sentence_that_comes_after_the_platform_let_go_of_the_window_changes_nothing() {
    let pair = Pair::new(false).await;
    let mut sharing_events = pair.sharing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    let handle = pair.sharer.handle.lock().unwrap().take().unwrap();
    // The platform keeps the failure for as long as it likes, and that does not keep the session
    // going: it ends with the last copy of the handle.
    let failure = pair.sharer.failure.lock().unwrap().clone().unwrap();
    until(
        "the thread that sends the pictures to let go of its copy",
        || Arc::strong_count(&handle) == 1,
    )
    .await;

    // The platform lets go of the handle, which is how the person's closing the window is told,
    // and a thread of it that has not heard says why a moment later. Nothing runs between the two
    // calls, so the first is the end.
    drop(handle);
    failure.fail("The capture stopped.");

    let ended = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } => Some((id, end)),
        _ => None,
    })
    .await;
    assert_eq!(ended, (shared.id, WindowEnd::ThisDevice));
}

#[tokio::test]
async fn a_no_is_an_answer_and_not_an_error() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Declined(why) = pair.offered_and_answered(false, false).await else {
        panic!("the window should have been declined");
    };
    assert!(why.contains("Kitchen iPad"), "{why}");
    // The platform that shares let go of the window, and nothing was shown.
    until("the platform to let go of the window", || {
        pair.sharer.dropped.load(Ordering::Relaxed) == 1
    })
    .await;
    assert!(pair.viewer.opened.lock().unwrap().is_empty());
    let started = tokio::time::timeout(
        QUIET,
        next_matching(&mut viewing_events, |event| match event {
            Event::WindowStarted(session) => Some(session),
            _ => None,
        }),
    )
    .await;
    assert!(started.is_err(), "a window was shown that was declined");
}

#[tokio::test]
async fn a_window_and_a_display_that_are_beyond_what_can_be_offered_are_each_called_what_they_are()
{
    for (chosen, called, not_called) in [
        (
            Chosen {
                size: (100_000, 900),
                ..large_window()
            },
            "That window cannot be shared",
            "screen",
        ),
        (
            Chosen {
                size: (100_000, 1964),
                ..whole_display()
            },
            "That screen cannot be shared",
            "window",
        ),
    ] {
        let pair = Pair::new(false).await;
        *pair.sharer.next.lock().unwrap() = Some(chosen);
        let error = pair
            .sharing
            .app
            .share_window(&pair.viewing.id(), false)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BadItem);
        assert!(error.message().contains(called), "{error}");
        assert!(!error.message().contains(not_called), "{error}");
    }
}

#[tokio::test]
async fn a_picker_that_fails_is_an_error_and_not_a_person_who_gave_up() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    *pair.sharer.fail.lock().unwrap() =
        Some("Screen recording is off.\nTurn it on in Settings.".to_owned());
    let error = pair
        .sharing
        .app
        .share_window(&pair.viewing.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert_eq!(
        error.message(),
        "Screen recording is off. Turn it on in Settings."
    );
    // Nothing was offered, and the device is not left busy.
    let asked = tokio::time::timeout(QUIET, pair.requested(&mut viewing_events)).await;
    assert!(asked.is_err(), "a window was offered that was not picked");
    *pair.sharer.fail.lock().unwrap() = None;
    let WindowShared::Declined(_) = pair.offered_and_answered(false, false).await else {
        panic!("the window should have been declined");
    };
}

#[tokio::test]
async fn a_person_who_gives_up_choosing_offers_nothing() {
    let pair = Pair::new(false).await;
    *pair.sharer.next.lock().unwrap() = None;
    let mut viewing_events = pair.viewing.app.events();
    let outcome = pair
        .sharing
        .app
        .share_window(&pair.viewing.id(), false)
        .await
        .unwrap();
    assert_eq!(outcome, WindowShared::NothingChosen);
    let asked = tokio::time::timeout(
        QUIET,
        next_matching(&mut viewing_events, |event| match event {
            Event::WindowRequested(request) => Some(request),
            _ => None,
        }),
    )
    .await;
    assert!(asked.is_err(), "the other device was asked about nothing");
}

#[tokio::test]
async fn a_screen_that_cannot_be_made_refuses_the_window_and_says_why() {
    let pair = Pair::new(false).await;
    // The platform's sentence is shown as one line.
    *pair.viewer.fail.lock().unwrap() = Some("The decoder is busy.\nTry again later.".to_owned());
    let mut viewing_events = pair.viewing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });
    let request = pair.requested(&mut viewing_events).await;
    assert!(pair.viewing.app.answer_window(request.id, true));

    // The person said yes and is told why nothing is shown.
    let (id, failure) = next_matching(&mut viewing_events, |event| match event {
        Event::WindowRequestEnded { id, failure } => Some((id, failure)),
        _ => None,
    })
    .await;
    assert_eq!(id, request.id);
    assert_eq!(
        failure.as_deref(),
        Some("The decoder is busy. Try again later.")
    );
    // The device that offered hears that the window was not taken.
    assert!(matches!(
        offer.await.unwrap().unwrap(),
        WindowShared::Declined(_)
    ));
}

#[tokio::test]
async fn an_answer_counts_once_and_only_for_the_kind_of_question_it_is_for() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    assert!(!pair.viewing.app.answer_window(9999, true));

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });
    let request = pair.requested(&mut viewing_events).await;
    // An answer to a pairing request is not an answer to a window.
    assert!(!pair.viewing.app.answer_pairing(request.id, true));
    assert!(pair.viewing.app.answer_window(request.id, false));
    assert!(!pair.viewing.app.answer_window(request.id, true));
    assert!(matches!(
        offer.await.unwrap().unwrap(),
        WindowShared::Declined(_)
    ));
}

#[tokio::test]
async fn giving_up_while_the_person_decides_takes_the_question_back() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });
    let request = pair.requested(&mut viewing_events).await;
    // The person at the sharing device gives up: the call is dropped.
    offer.abort();
    let (id, failure) = next_matching(&mut viewing_events, |event| match event {
        Event::WindowRequestEnded { id, failure } => Some((id, failure)),
        _ => None,
    })
    .await;
    assert_eq!((id, failure), (request.id, None));
    assert!(!pair.viewing.app.answer_window(request.id, true));
    assert!(pair.viewer.opened.lock().unwrap().is_empty());
}

#[tokio::test]
async fn one_window_is_offered_or_shown_at_a_time() {
    let pair = Pair::new(false).await;
    let (open_picker, gate) = oneshot::channel();
    *pair.sharer.gate.lock().unwrap() = Some(gate);
    let mut viewing_events = pair.viewing.app.events();

    // The first call waits for the picker.
    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let first = tokio::spawn(async move { app.share_window(&to, false).await });
    // The picker answers, and the person at the other device is asked.
    open_picker.send(()).unwrap();
    let request = pair.requested(&mut viewing_events).await;

    // A second window, offered while the first waits for an answer, is refused at once.
    *pair.sharer.next.lock().unwrap() = Some(report_window());
    let error = pair
        .sharing
        .app
        .share_window(&pair.viewing.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert!(error.message().contains("already"), "{error}");

    assert!(pair.viewing.app.answer_window(request.id, true));
    assert!(matches!(
        first.await.unwrap().unwrap(),
        WindowShared::Started(_)
    ));
    // And while the window is shown.
    *pair.sharer.next.lock().unwrap() = Some(report_window());
    let error = pair
        .sharing
        .app
        .share_window(&pair.viewing.id(), false)
        .await
        .unwrap_err();
    assert!(error.message().contains("already"), "{error}");
}

#[tokio::test]
async fn control_is_offered_only_by_a_device_that_can_grant_it() {
    // A platform that cannot post input cannot offer control.
    let pair = Pair::new(false).await;
    let error = pair
        .sharing
        .app
        .share_window(&pair.viewing.id(), true)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert_eq!(
        pair.sharer.dropped.load(Ordering::Relaxed),
        0,
        "no window was picked for an offer that could not be made"
    );

    // One that can, offers it, and the person at the viewer is told.
    let pair = Pair::new(true).await;
    let mut viewing_events = pair.viewing.app.events();
    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, true).await });
    let request = pair.requested(&mut viewing_events).await;
    assert!(request.control);
    assert!(pair.viewing.app.answer_window(request.id, true));
    let WindowShared::Started(shared) = offer.await.unwrap().unwrap() else {
        panic!("the window was not shared");
    };
    assert!(shared.control);
    assert_eq!(
        pair.viewer.opened.lock().unwrap().as_slice(),
        [Opened::report(true)]
    );
}

#[tokio::test]
async fn a_device_that_shows_no_windows_is_not_offered_one() {
    let sharer = Sharer::new(false, Some(report_window()));
    let sharing = Dev::start("Studio Mac", Windows::none().sharing(sharer.clone())).await;
    let plain = Dev::start("Old phone", Windows::none()).await;
    pair(&sharing, &plain).await;
    let error = sharing
        .app
        .share_window(&plain.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined);
    assert!(error.message().contains("Old phone"), "{error}");

    // And a device that shares nothing cannot offer.
    let error = plain
        .app
        .share_window(&sharing.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    // A device that is not paired is not reached.
    let stranger = Dev::start("Stranger", Windows::none().showing(Viewer::new())).await;
    let error = sharing
        .app
        .share_window(&stranger.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotPaired);
}

#[tokio::test]
async fn forgetting_a_device_ends_its_window() {
    let pair = Pair::new(false).await;
    let mut sharing_events = pair.sharing.app.events();
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(_) => Some(()),
        _ => None,
    })
    .await;

    pair.sharing.app.unpair(&pair.viewing.id()).unwrap();
    let end = next_matching(&mut sharing_events, |event| match event {
        Event::WindowEnded { id, end } if id == shared.id => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end, WindowEnd::ThisDevice);
    next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { .. } => Some(()),
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn stopping_the_app_ends_its_windows() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };
    next_matching(&mut viewing_events, |event| match event {
        Event::WindowStarted(_) => Some(()),
        _ => None,
    })
    .await;

    pair.sharing.app.stop().await;
    next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { .. } => Some(()),
        _ => None,
    })
    .await;
    until("the viewer's platform to be told", || {
        pair.viewer.heard.has("ended")
    })
    .await;
}

#[tokio::test]
async fn stopping_the_app_turns_a_window_question_that_is_showing_into_a_no() {
    let pair = Pair::new(false).await;
    let mut viewing_events = pair.viewing.app.events();
    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });
    let request = pair.requested(&mut viewing_events).await;

    pair.viewing.app.stop().await;

    // A yes that comes late is not taken, and the platform makes no screen for it.
    assert!(!pair.viewing.app.answer_window(request.id, true));
    assert!(pair.viewer.opened.lock().unwrap().is_empty());
    // The person at the source is not left waiting, and gets no window.
    let outcome = tokio::time::timeout(WAIT, offer)
        .await
        .expect("the offer was left waiting")
        .unwrap();
    assert!(
        !matches!(outcome, Ok(WindowShared::Started(_))),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn a_platform_that_is_set_up_wrongly_fails_when_the_app_starts() {
    struct Wrong;
    impl WindowViewer for Wrong {
        fn limits(&self) -> ShowLimits {
            ShowLimits {
                max_width: 0,
                max_height: 1200,
                max_fps: 60,
                codecs: vec![Codec::H264],
            }
        }

        fn open(&self, _: &WindowOffer, _: ScreenLink) -> Result<Screen, PlatformError> {
            Err(PlatformError::new("never"))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    write_settings(dir.path(), "Wrong");
    let error = App::start_with(
        Options::new(dir.path()),
        Windows::none().showing(Arc::new(Wrong)),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    // The folder was not kept locked by the attempt.
    App::start(Options::new(dir.path())).await.unwrap();
}

#[test]
fn what_a_platform_gets_wrong_is_a_sentence() {
    let error = PlatformError::new("No window server.");
    assert_eq!(error.message(), "No window server.");
    assert_eq!(error.to_string(), "No window server.");
}

#[tokio::test]
async fn a_window_with_no_title_is_shown_and_shared_without_one() {
    let pair = Pair::new(false).await;
    *pair.sharer.next.lock().unwrap() = Some(untitled_window());
    let mut viewing_events = pair.viewing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });
    let request = pair.requested(&mut viewing_events).await;
    assert_eq!((request.title, request.app), (None, None));
    assert_eq!((request.width, request.height), (800, 600));
    assert!(pair.viewing.app.answer_window(request.id, true));
    let WindowShared::Started(shared) = offer.await.unwrap().unwrap() else {
        panic!("the window was not shared");
    };
    assert_eq!((shared.title, shared.app), (None, None));
}

#[tokio::test]
async fn a_viewer_that_takes_nothing_the_platform_can_make_is_told_the_window_is_off() {
    let pair = Pair::new(false).await;
    pair.sharer.makes_pictures.store(false, Ordering::Relaxed);
    let mut viewing_events = pair.viewing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });
    let request = pair.requested(&mut viewing_events).await;
    assert!(pair.viewing.app.answer_window(request.id, true));

    // The person who offered is told in a sentence, and the platform lets go of the window.
    let WindowShared::Declined(why) = offer.await.unwrap().unwrap() else {
        panic!("the window should not have started");
    };
    assert!(why.contains("Kitchen iPad"), "{why}");
    assert!(why.contains("picture"), "{why}");
    assert!(why.contains("the window"), "{why}");
    until("the platform to let go of the window", || {
        pair.sharer.dropped.load(Ordering::Relaxed) == 1
    })
    .await;
    // The viewer had made its side already, and the window ends there at once: the other device
    // stopped it.
    let ended = next_matching(&mut viewing_events, |event| match event {
        Event::WindowEnded { end, .. } => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(ended, WindowEnd::OtherDevice);
    // No picture was made.
    assert_eq!(pair.sharer.heard.count("report"), 0);
}

#[tokio::test]
async fn stopping_while_the_person_chooses_offers_nothing() {
    let pair = Pair::new(false).await;
    let (open_picker, gate) = oneshot::channel();
    *pair.sharer.gate.lock().unwrap() = Some(gate);
    let mut viewing_events = pair.viewing.app.events();

    let app = pair.sharing.app.clone();
    let to = pair.viewing.id();
    let offer = tokio::spawn(async move { app.share_window(&to, false).await });
    // The app stops while the picker is open, and then the person chooses.
    until("the picker to open", || {
        pair.sharer.gate.lock().unwrap().is_none()
    })
    .await;
    pair.sharing.app.stop().await;
    open_picker.send(()).unwrap();
    let error = offer.await.unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Stopped);
    until("the platform to let go of the window", || {
        pair.sharer.dropped.load(Ordering::Relaxed) == 1
    })
    .await;
    let asked = tokio::time::timeout(
        QUIET,
        next_matching(&mut viewing_events, |event| match event {
            Event::WindowRequested(request) => Some(request),
            _ => None,
        }),
    )
    .await;
    assert!(asked.is_err(), "a device that had stopped made an offer");
}

// Sessions that the viewer starts (ADR 0010).

/// Turns down every offer that reaches the person, and counts them: a session that the phone
/// started has no offer that asks.
struct Asked(Arc<AtomicUsize>);

impl WindowHandler for Asked {
    fn offered<'a>(
        &'a self,
        _from: &'a PairedDevice,
        _offer: &'a WindowOffer,
    ) -> BoxFuture<'a, WindowVerdict> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(std::future::ready(WindowVerdict::Refuse))
    }

    fn started(&self, _from: &PairedDevice, _handle: ViewHandle) {}
}

/// A phone, as far as these tests need one: a node that shows the screens of other devices and can
/// ask for one. The app has no call for the asking yet (the button comes with the pull request
/// after this one), so the asking is the node's own.
struct Phone {
    node: Arc<Node>,
    id: DeviceId,
    /// How many offers reached the person.
    asked: Arc<AtomicUsize>,
    /// What the decoders of the screens heard.
    heard: Arc<Heard>,
    _dir: TempDir,
}

impl Phone {
    async fn start(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
        let config = NodeConfig::from_toml(&format!(
            "device_name = \"{name}\"\n\n[network]\nrelay = \"disabled\"\nlan_discovery = false\n\
             port_mapping = false\n"
        ))
        .unwrap();
        let asked = Arc::new(AtomicUsize::new(0));
        let node = Node::builder(&identity, trust, config)
            .window_handler(Asked(Arc::clone(&asked)))
            .bind()
            .await
            .unwrap();
        Self {
            node: Arc::new(node),
            id: identity.device_id(),
            asked,
            heard: Arc::new(Heard::default()),
            _dir: dir,
        }
    }

    /// Asks `mac` to start sharing its screen, with the picture the phone can show.
    async fn asks(&self, mac: &Dev, control: bool) -> baton_core::Result<StartOutcome> {
        let mac: DeviceId = mac.id().parse().unwrap();
        let viewing = Viewing::new(
            Screening(Arc::clone(&self.heard)),
            1920,
            1200,
            60,
            vec![Codec::H264],
        )
        .unwrap();
        self.node
            .start(&mac, Wanted::Screen, control, viewing)
            .await
    }
}

/// Pairs the phone with the Mac the way two people would: the Mac shows a ticket, the phone
/// enters it, and the Mac says yes. What the Mac allows the phone is not set: it is off.
async fn pair_phone(mac: &Dev, phone: &Phone) {
    let mut events = mac.app.events();
    let offer = mac.app.offer_pairing().await.unwrap();
    let ticket: PairTicket = offer.ticket.parse().unwrap();
    let node = Arc::clone(&phone.node);
    let task = tokio::spawn(async move { node.join_pairing(&ticket, |_| {}).await });
    let request = next_matching(&mut events, |event| match event {
        Event::PairingRequested(request) => Some(request),
        _ => None,
    })
    .await;
    assert!(mac.app.answer_pairing(request.id, true));
    task.await.unwrap().unwrap();
    next_matching(&mut events, |event| match event {
        Event::OfferEnded(_) => Some(()),
        _ => None,
    })
    .await;
}

/// A Mac that can give its screen without a picker, a phone that it is paired with, and the
/// platform of the Mac.
struct Screens {
    mac: Dev,
    phone: Phone,
    sharer: Arc<Sharer>,
}

impl Screens {
    /// The platform of the Mac can post input if `control`.
    async fn new(control: bool) -> Self {
        Self::with(Sharer::with_screen(control)).await
    }

    async fn with(sharer: Arc<Sharer>) -> Self {
        let mac = Dev::start("Studio Mac", Windows::none().sharing(sharer.clone())).await;
        let phone = Phone::start("Pocket Phone").await;
        pair_phone(&mac, &phone).await;
        Self { mac, phone, sharer }
    }

    /// What the person at the Mac allows the phone, in the settings for it.
    fn allow(&self, access: StartAccess) {
        self.mac
            .app
            .set_start_access(&self.phone.id.to_string(), access)
            .unwrap();
    }

    /// The phone asks, and the session starts.
    async fn started(&self, control: bool) -> ViewHandle {
        match self.phone.asks(&self.mac, control).await.unwrap() {
            StartOutcome::Started(handle) => handle,
            other => panic!("the screen was not started: {other:?}"),
        }
    }

    /// The phone asks, and the Mac says why not.
    async fn refused(&self, control: bool) -> WhyNot {
        match self.phone.asks(&self.mac, control).await.unwrap() {
            StartOutcome::NotStarted(why) => why,
            other => panic!("the screen was started: {other:?}"),
        }
    }

    fn screens(&self) -> usize {
        self.sharer.screens.load(Ordering::Relaxed)
    }
}

/// The next session that the Mac started.
async fn started_at(events: &mut Events) -> WindowSession {
    next_matching(events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_phone_that_the_person_allowed_gets_the_screen_with_no_picker_and_no_question() {
    let screens = Screens::new(true).await;
    screens.allow(StartAccess::Look);
    let mut events = screens.mac.app.events();

    let handle = screens.started(false).await;

    // The platform was asked for the screen and the person at the Mac was given no picker.
    assert_eq!(screens.screens(), 1);
    assert_eq!(screens.sharer.picks.load(Ordering::Relaxed), 0);
    // The person at the phone was not asked about an offer either: the offer answered their asking.
    assert_eq!(screens.phone.asked.load(Ordering::Relaxed), 0);

    // The Mac hears of the session as of any it shares: a whole display, and no control.
    let shared = started_at(&mut events).await;
    assert_eq!(shared.role, WindowRole::Sharing);
    assert_eq!(shared.device_id, screens.phone.id.to_string());
    assert_eq!(shared.device_name, "Pocket Phone");
    assert!(shared.display);
    assert_eq!((shared.title, shared.app), (None, None));
    assert_eq!((shared.width, shared.height), (3024, 1964));
    assert!(!shared.control);
    assert!(!handle.control());

    // Pictures go from the Mac to the decoder of the phone.
    until("the pictures to be decoded and shown", || {
        screens.phone.heard.has("decode 2") && screens.phone.heard.has("show 2")
    })
    .await;
    assert!(screens.phone.heard.has("config"));

    // The person at the phone closes it, and the platform of the Mac is told.
    handle.end();
    let end = next_matching(&mut events, |event| match event {
        Event::WindowEnded { id, end } if id == shared.id => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end, WindowEnd::OtherDevice);
    until("the platform of the Mac to be told", || {
        screens.sharer.heard.has("ended")
    })
    .await;
}

#[tokio::test]
async fn a_phone_that_the_person_did_not_allow_is_told_so_and_the_screen_is_left_alone() {
    let screens = Screens::new(true).await;
    let mut events = screens.mac.app.events();
    let phone = screens.phone.id.to_string();
    assert_eq!(
        screens.mac.app.start_access(&phone).unwrap(),
        StartAccess::Off,
        "a device that was just paired is allowed nothing"
    );

    assert_eq!(screens.refused(false).await, WhyNot::NotAllowed);
    assert_eq!(
        screens.screens(),
        0,
        "the platform was asked for the screen"
    );
    let started = tokio::time::timeout(QUIET, started_at(&mut events)).await;
    assert!(started.is_err(), "a session started that was not allowed");

    // The refusal left nothing behind: when the person allows the phone, it starts.
    screens.allow(StartAccess::Look);
    let _handle = screens.started(false).await;
    assert_eq!(screens.screens(), 1);
}

/// What the two ends were granted when the phone wished for `wish`, the person at the Mac allowed
/// `access`, and the platform of the Mac can post input if `platform`.
async fn control_granted(wish: bool, access: StartAccess, platform: bool) -> (bool, bool) {
    let screens = Screens::new(platform).await;
    screens.allow(access);
    let mut events = screens.mac.app.events();
    let handle = screens.started(wish).await;
    let shared = started_at(&mut events).await;
    (handle.control(), shared.control)
}

#[tokio::test]
async fn control_is_the_least_of_what_the_phone_wished_the_person_allowed_and_the_platform_can_do()
{
    for (wish, access, platform, granted) in [
        (true, StartAccess::Control, true, true),
        (false, StartAccess::Control, true, false),
        (true, StartAccess::Look, true, false),
        (true, StartAccess::Control, false, false),
    ] {
        assert_eq!(
            control_granted(wish, access, platform).await,
            (granted, granted),
            "the phone wished for control: {wish}, the person allowed {access:?}, the platform \
             can post input: {platform}"
        );
    }
}

#[tokio::test]
async fn control_is_not_granted_when_the_platform_can_no_longer_post_input() {
    // The app was started when the platform could post input, so the node takes it. The person
    // takes back the consent to control the computer while the app runs, and the platform says so
    // from then on: a phone that asks afterwards, and was allowed, gets no control.
    let screens = Screens::new(true).await;
    screens.allow(StartAccess::Control);
    screens.sharer.control.store(false, Ordering::Relaxed);
    let mut events = screens.mac.app.events();

    let handle = screens.started(true).await;
    let shared = started_at(&mut events).await;

    assert_eq!((handle.control(), shared.control), (false, false));
    // The phone only looks, so the screen is not asked for as private either.
    assert_eq!(screens.sharer.privacies(), [Privacy::Open]);
}

#[tokio::test]
async fn a_screen_that_a_phone_controls_is_asked_for_as_private_and_one_it_only_looks_at_is_not() {
    // What the platform is asked for follows what the phone is granted, not what it wished for.
    for (wish, access, platform, privacy) in [
        (true, StartAccess::Control, true, Privacy::Private),
        (false, StartAccess::Control, true, Privacy::Open),
        (true, StartAccess::Look, true, Privacy::Open),
        (true, StartAccess::Control, false, Privacy::Open),
    ] {
        let screens = Screens::new(platform).await;
        screens.allow(access);
        let _handle = screens.started(wish).await;
        assert_eq!(
            screens.sharer.privacies(),
            [privacy],
            "the phone wished for control: {wish}, the person allowed {access:?}, the platform \
             can post input: {platform}"
        );
    }
}

#[tokio::test]
async fn a_screen_that_cannot_be_private_is_not_given_open_to_a_phone_that_controls_it() {
    // The platform has not been allowed to hold the keyboard and the pointer, which a controlled
    // screen needs, so the phone is told that, and the Mac shares nothing.
    let screens = Screens::new(true).await;
    screens.allow(StartAccess::Control);
    *screens.sharer.screen_plan.lock().unwrap() =
        ScreenPlan::Refuses(ScreenError::NeedsInputPermission);
    let mut events = screens.mac.app.events();

    assert_eq!(screens.refused(true).await, WhyNot::NeedsInputPermission);
    let started = tokio::time::timeout(QUIET, started_at(&mut events)).await;
    assert!(started.is_err(), "a session started with no privacy");

    // The phone can still look, which asks for nothing of the keyboard or the pointer.
    *screens.sharer.screen_plan.lock().unwrap() = ScreenPlan::Gives(whole_display());
    let _handle = screens.started(false).await;
    assert_eq!(
        screens.sharer.privacies(),
        [Privacy::Private, Privacy::Open]
    );
}

/// A platform written before the screen could be asked for: it picks windows and says nothing
/// about its screen.
struct Silent;

impl WindowSharer for Silent {
    fn control(&self) -> bool {
        false
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        Box::pin(std::future::ready(Ok(None)))
    }
}

#[tokio::test]
async fn a_platform_that_says_nothing_of_the_screen_cannot_give_it() {
    assert!(!Silent.can_give_screen());
    assert!(matches!(
        Silent.screen(Privacy::Open).await,
        Err(ScreenError::Unsupported)
    ));

    // Its device does not tell the screens that the setting is worth offering.
    let dev = Dev::start("Old Mac", Windows::none().sharing(Arc::new(Silent))).await;
    assert!(dev.app.window_abilities().can_share);
    assert!(!dev.app.window_abilities().can_give_screen);
}

#[tokio::test]
async fn why_the_platform_cannot_give_the_screen_is_what_the_phone_is_told() {
    let screens = Screens::new(false).await;
    screens.allow(StartAccess::Look);
    let reasons = [
        (ScreenError::Unsupported, WhyNot::Unsupported),
        (ScreenError::NeedsPermission, WhyNot::NeedsPermission),
        (
            ScreenError::NeedsInputPermission,
            WhyNot::NeedsInputPermission,
        ),
        (ScreenError::Unavailable, WhyNot::Unavailable),
        (
            ScreenError::Failed(PlatformError::new("No window server.")),
            WhyNot::Failed,
        ),
    ];
    for (error, why) in reasons {
        *screens.sharer.screen_plan.lock().unwrap() = ScreenPlan::Refuses(error.clone());
        assert_eq!(screens.refused(false).await, why, "{error}");
    }
    assert_eq!(screens.screens(), 5);

    // No asking left the Mac busy: once the platform can give the screen, the phone gets it.
    *screens.sharer.screen_plan.lock().unwrap() = ScreenPlan::Gives(whole_display());
    let _handle = screens.started(false).await;
}

#[tokio::test]
async fn a_platform_that_gives_a_window_for_the_screen_is_not_believed() {
    let screens = Screens::new(false).await;
    screens.allow(StartAccess::Look);
    *screens.sharer.screen_plan.lock().unwrap() = ScreenPlan::Gives(report_window());
    let mut events = screens.mac.app.events();

    assert_eq!(screens.refused(false).await, WhyNot::Failed);
    // The window is let go of and not offered in place of the screen.
    until("the platform to let go of the window", || {
        screens.sharer.dropped.load(Ordering::Relaxed) == 1
    })
    .await;
    assert_eq!(screens.phone.asked.load(Ordering::Relaxed), 0);
    let started = tokio::time::timeout(QUIET, started_at(&mut events)).await;
    assert!(started.is_err(), "a window was shared for the screen");
}

#[tokio::test]
async fn a_device_whose_platform_cannot_give_the_screen_is_not_asked_for_it() {
    // The person allowed the phone, and the platform shows windows only through the picker.
    let screens = Screens::with(Sharer::new(true, None)).await;
    screens.allow(StartAccess::Control);

    let error = screens.phone.asks(&screens.mac, false).await.unwrap_err();
    assert!(
        matches!(error, baton_core::Error::Unsupported(_)),
        "{error}"
    );
    assert_eq!(screens.screens(), 0);
}

#[tokio::test]
async fn a_phone_that_gives_up_while_the_mac_gets_ready_leaves_no_session_behind() {
    let screens = Screens::new(false).await;
    screens.allow(StartAccess::Look);
    let (open, gate) = oneshot::channel();
    *screens.sharer.screen_gate.lock().unwrap() = Some(gate);
    let mut events = screens.mac.app.events();

    // The phone asks, the platform of the Mac takes its time to give the screen, and the person at
    // the phone gives up: the asking is dropped.
    tokio::select! {
        outcome = screens.phone.asks(&screens.mac, false) => {
            panic!("the asking was answered: {outcome:?}");
        }
        () = until("the Mac to ask its platform for the screen", || screens.screens() == 1) => {}
    }
    open.send(()).unwrap();

    // The platform lets go of the screen it made, and nothing is shared.
    until("the platform to let go of the screen", || {
        screens.sharer.dropped.load(Ordering::Relaxed) == 1
    })
    .await;
    let started = tokio::time::timeout(QUIET, started_at(&mut events)).await;
    assert!(started.is_err(), "a session started that nobody wanted");

    // The Mac is free for the next asking.
    let _handle = screens.started(false).await;
}

#[tokio::test]
async fn stopping_the_mac_ends_the_session_that_the_phone_started() {
    let screens = Screens::new(false).await;
    screens.allow(StartAccess::Look);
    let handle = screens.started(false).await;

    screens.mac.app.stop().await;
    tokio::time::timeout(WAIT, handle.ended())
        .await
        .expect("the session was left running");
}

#[tokio::test]
async fn turning_the_setting_off_ends_nothing_that_runs_and_stops_the_next_asking() {
    let screens = Screens::new(false).await;
    screens.allow(StartAccess::Look);
    let mut events = screens.mac.app.events();
    let handle = screens.started(false).await;
    let shared = started_at(&mut events).await;

    // The person at the Mac takes the yes back. The session that runs goes on.
    screens.allow(StartAccess::Off);
    let ended = tokio::time::timeout(QUIET, handle.ended()).await;
    assert!(ended.is_err(), "the session ended when the setting did");

    // The next asking is told that the settings do not let the phone.
    handle.end();
    next_matching(&mut events, |event| match event {
        Event::WindowEnded { id, .. } if id == shared.id => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(screens.refused(false).await, WhyNot::NotAllowed);
    assert_eq!(screens.screens(), 1);
}

#[tokio::test]
async fn a_mac_that_shares_a_window_is_busy_and_its_screen_is_not_asked_for() {
    let sharer = Sharer::with_screen(false);
    *sharer.next.lock().unwrap() = Some(report_window());
    let pair = Pair::with_sharer(Arc::clone(&sharer)).await;
    let phone = Phone::start("Pocket Phone").await;
    pair_phone(&pair.sharing, &phone).await;
    pair.sharing
        .app
        .set_start_access(&phone.id.to_string(), StartAccess::Look)
        .unwrap();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the window was not shared");
    };

    let StartOutcome::NotStarted(why) = phone.asks(&pair.sharing, false).await.unwrap() else {
        panic!("the screen was started on a Mac that shares a window");
    };
    assert_eq!(why, WhyNot::Busy);
    assert_eq!(sharer.screens.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn the_person_at_the_mac_allows_a_paired_device_and_only_a_paired_device() {
    let pair = Pair::new(false).await;
    let app = &pair.sharing.app;
    let viewing = pair.viewing.id();
    assert_eq!(app.start_access(&viewing).unwrap(), StartAccess::Off);
    for access in [StartAccess::Control, StartAccess::Look, StartAccess::Off] {
        app.set_start_access(&viewing, access).unwrap();
        assert_eq!(app.start_access(&viewing).unwrap(), access);
    }

    // A device that is not paired is allowed nothing, and cannot be.
    let stranger = Identity::generate().device_id().to_string();
    assert_eq!(
        app.start_access(&stranger).unwrap_err().kind(),
        ErrorKind::NotPaired
    );
    assert_eq!(
        app.set_start_access(&stranger, StartAccess::Look)
            .unwrap_err()
            .kind(),
        ErrorKind::NotPaired
    );
    assert_eq!(
        app.set_start_access("not an id", StartAccess::Look)
            .unwrap_err()
            .kind(),
        ErrorKind::NotPaired
    );
}

// Sessions that the app of the phone starts (ADR 0010): the call that the button makes.

/// A Mac that can give its screen with no picker and a phone that shows screens: two apps, paired.
struct Asking {
    mac: Dev,
    phone: Dev,
    sharer: Arc<Sharer>,
    viewer: Arc<Viewer>,
}

impl Asking {
    /// The platform of the Mac can post input if `control`.
    async fn new(control: bool) -> Self {
        Self::with(Sharer::with_screen(control)).await
    }

    async fn with(sharer: Arc<Sharer>) -> Self {
        Self::with_platforms(sharer, Viewer::new()).await
    }

    async fn with_platforms(sharer: Arc<Sharer>, viewer: Arc<Viewer>) -> Self {
        let mac = Dev::start("Studio Mac", Windows::none().sharing(sharer.clone())).await;
        let phone = Dev::start("Pocket Phone", Windows::none().showing(viewer.clone())).await;
        pair(&mac, &phone).await;
        Self {
            mac,
            phone,
            sharer,
            viewer,
        }
    }

    /// What the person at the Mac allows the phone, in the settings for it.
    fn allow(&self, access: StartAccess) {
        self.mac
            .app
            .set_start_access(&self.phone.id(), access)
            .unwrap();
    }

    /// The person at the phone asks for the screen of the Mac.
    async fn asks(&self, control: bool) -> ScreenStarted {
        self.phone
            .app
            .start_screen(&self.mac.id(), control)
            .await
            .unwrap()
    }

    /// The person at the phone asks, and the screen is shown.
    async fn started(&self, control: bool) -> WindowSession {
        match self.asks(control).await {
            ScreenStarted::Started(session) => session,
            ScreenStarted::NotStarted(why) => panic!("the screen was not started: {why}"),
        }
    }

    /// The person at the phone asks, and the Mac says why not, in words.
    async fn refused(&self, control: bool) -> String {
        match self.asks(control).await {
            ScreenStarted::NotStarted(why) => why,
            ScreenStarted::Started(session) => panic!("the screen was started: {session:?}"),
        }
    }

    /// How many times the platform of the Mac was asked for the screen.
    fn screens(&self) -> usize {
        self.sharer.screens.load(Ordering::Relaxed)
    }

    /// Waits until a picture has been shown at the phone, which is when both control streams of
    /// the session are up. A phone that ends the session before that is noticed by the Mac only
    /// when it gives up waiting.
    async fn until_shown(&self) {
        until("a picture to be shown at the phone", || {
            self.viewer.heard.has("show 0")
        })
        .await;
    }
}

#[tokio::test]
async fn a_screen_that_the_person_at_the_phone_asks_for_is_shown_with_no_question_at_either_end() {
    let asking = Asking::new(false).await;
    asking.allow(StartAccess::Look);
    let mut phone_events = asking.phone.app.events();
    let mut mac_events = asking.mac.app.events();

    let session = asking.started(false).await;

    // A screen is shown here that has no title and no application, and the size of which is not
    // known before the first picture.
    assert_eq!(session.role, WindowRole::Viewing);
    assert_eq!(session.device_id, asking.mac.id());
    assert_eq!(session.device_name, "Studio Mac");
    assert!(session.display);
    assert_eq!(
        (session.title.as_deref(), session.app.as_deref()),
        (None, None)
    );
    assert_eq!((session.width, session.height), (0, 0));
    assert!(!session.control);
    assert_eq!(session.screen, Some(100));
    // The platform of the phone made the screen for that, before the Mac was asked.
    assert_eq!(
        asking.viewer.opened.lock().unwrap().as_slice(),
        [Opened {
            title: None,
            width: 0,
            height: 0,
            control: false
        }]
    );
    // The event says the same as the call.
    assert_eq!(started_at(&mut phone_events).await, session);

    // The Mac gave the screen without a picker, and shares it as any other.
    let shared = started_at(&mut mac_events).await;
    assert_eq!(shared.role, WindowRole::Sharing);
    assert_eq!(shared.device_id, asking.phone.id());
    assert_eq!(shared.device_name, "Pocket Phone");
    assert!(shared.display);
    assert_eq!((shared.width, shared.height), (3024, 1964));
    assert!(!shared.control);
    assert_eq!(asking.screens(), 1);
    assert_eq!(asking.sharer.picks.load(Ordering::Relaxed), 0);

    // Pictures reach the decoder, and the phone is told the size of the picture, which is the
    // shape of the screen within what the phone takes.
    until("the pictures to be decoded and shown", || {
        asking.viewer.heard.has("decode 2") && asking.viewer.heard.has("show 2")
    })
    .await;
    assert!(asking.viewer.heard.has("config"));
    let mut heard = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(QUIET, phone_events.next()).await {
        heard.push(event);
    }
    assert!(
        heard.contains(&Event::WindowPicture {
            id: session.id,
            width: 1920,
            height: 1200
        }),
        "{heard:?}"
    );
    // Nobody was asked about an offer, because the offer answered the phone.
    assert!(
        !heard
            .iter()
            .any(|event| matches!(event, Event::WindowRequested(_))),
        "{heard:?}"
    );

    // The person at the phone ends it: both ends hear how it ended, and both platforms are told.
    assert!(asking.phone.app.end_window(session.id));
    let end = next_matching(&mut phone_events, |event| match event {
        Event::WindowEnded { id, end } if id == session.id => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end, WindowEnd::ThisDevice);
    let end = next_matching(&mut mac_events, |event| match event {
        Event::WindowEnded { id, end } if id == shared.id => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end, WindowEnd::OtherDevice);
    until("both platforms to be told", || {
        asking.viewer.heard.has("ended") && asking.sharer.heard.has("ended")
    })
    .await;
    assert!(!asking.phone.app.end_window(session.id));
}

/// What the two ends were granted when the person at the phone wished for `wish`, the person at the
/// Mac allowed `access`, and the platform of the Mac can post input if `platform`.
async fn screen_control(wish: bool, access: StartAccess, platform: bool) -> (bool, bool) {
    let asking = Asking::new(platform).await;
    asking.allow(access);
    let mut mac_events = asking.mac.app.events();
    let session = asking.started(wish).await;
    let shared = started_at(&mut mac_events).await;
    // The platform of the phone is told what was wished for. What was granted is in the session.
    assert_eq!(
        asking
            .viewer
            .opened
            .lock()
            .unwrap()
            .iter()
            .map(|opened| opened.control)
            .collect::<Vec<_>>(),
        [wish]
    );
    (session.control, shared.control)
}

/// Waits until the platform of the Mac has heard `count` words about what is lifted.
async fn lifts_heard(sharer: &Sharer, count: usize) {
    until("the platform to hear what was lifted", || {
        sharer.lifted().len() >= count
    })
    .await;
}

#[tokio::test]
async fn a_screen_that_the_platform_keeps_private_is_private_at_both_ends_and_the_phone_lifts_it_in_parts()
 {
    let asking = Asking::with(Sharer::with_lift(true)).await;
    asking.allow(StartAccess::Control);
    let mut mac_events = asking.mac.app.events();

    let session = asking.started(true).await;
    let shared = started_at(&mut mac_events).await;

    // The screen was asked for as private, both ends say so, and nothing is lifted to begin with.
    assert_eq!(asking.sharer.privacies(), [Privacy::Private]);
    assert!(session.control && session.private);
    assert!(shared.control && shared.private);
    assert_eq!(asking.sharer.lifted(), Vec::<Lift>::new());

    // The person at the phone lifts either part and puts it back, as often as they like, and the
    // platform hears every word, whole and in the order it was said, the same word twice included.
    let said = [
        (true, false),
        (true, false),
        (true, true),
        (false, true),
        (false, false),
        (true, false),
        (false, false),
    ];
    for (n, (screen, input)) in said.into_iter().enumerate() {
        assert!(asking.phone.app.window_lift(session.id, screen, input));
        lifts_heard(&asking.sharer, n + 1).await;
    }
    assert_eq!(
        asking.sharer.lifted(),
        said.map(|(screen, input)| Lift { screen, input })
    );

    // It is the phone that lifts: a window that is shared from here is not one to lift.
    assert!(!asking.mac.app.window_lift(shared.id, true, true));
    tokio::time::sleep(QUIET).await;
    assert_eq!(asking.sharer.lifted().len(), said.len());

    // Nothing is taken once the session is over.
    assert!(asking.phone.app.end_window(session.id));
    until("the platform of the Mac to be told", || {
        asking.sharer.heard.has("ended")
    })
    .await;
    assert!(!asking.phone.app.window_lift(session.id, true, true));
}

#[tokio::test]
async fn a_screen_of_a_platform_that_cannot_lift_is_kept_private_and_the_phone_is_not_told_it_may()
{
    let asking = Asking::new(true).await;
    asking.allow(StartAccess::Control);
    let mut mac_events = asking.mac.app.events();

    let session = asking.started(true).await;
    let shared = started_at(&mut mac_events).await;

    // The platform is asked to keep the screen private, as for any screen that is controlled, and
    // the session is controlled, but the phone has nothing to lift.
    assert_eq!(asking.sharer.privacies(), [Privacy::Private]);
    assert!(session.control && shared.control);
    assert!(!session.private && !shared.private);
    assert!(!asking.phone.app.window_lift(session.id, true, true));
    tokio::time::sleep(QUIET).await;
    assert_eq!(asking.sharer.lifted(), Vec::<Lift>::new());
}

#[tokio::test]
async fn a_screen_that_the_phone_only_looks_at_is_not_private_and_has_nothing_to_lift() {
    let asking = Asking::with(Sharer::with_lift(true)).await;
    asking.allow(StartAccess::Control);
    let mut mac_events = asking.mac.app.events();

    let session = asking.started(false).await;
    let shared = started_at(&mut mac_events).await;

    assert_eq!(asking.sharer.privacies(), [Privacy::Open]);
    assert!(!session.control && !session.private);
    assert!(!shared.control && !shared.private);
    assert!(!asking.phone.app.window_lift(session.id, true, true));
    tokio::time::sleep(QUIET).await;
    assert_eq!(asking.sharer.lifted(), Vec::<Lift>::new());
}

#[tokio::test]
async fn a_lift_for_a_window_that_is_not_shown_here_is_not_taken() {
    let asking = Asking::with(Sharer::with_lift(true)).await;
    assert!(!asking.phone.app.window_lift(0, true, true));
    assert!(!asking.phone.app.window_lift(7, false, true));
    assert!(!asking.mac.app.window_lift(u64::MAX, true, false));
}

/// Waits until the end of the window `id` is told, and says how it ended.
async fn ended_at(events: &mut Events, id: u64) -> WindowEnd {
    next_matching(events, |event| match event {
        Event::WindowEnded { id: ended, end } if ended == id => Some(end),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_person_who_chooses_to_leave_the_mac_as_it_is_has_the_platform_of_the_mac_told_so() {
    let asking = Asking::with(Sharer::with_leave(true)).await;
    asking.allow(StartAccess::Control);
    let mut phone_events = asking.phone.app.events();
    let mut mac_events = asking.mac.app.events();

    let session = asking.started(true).await;
    let shared = started_at(&mut mac_events).await;

    // The Mac offered the choice with its private screen, to the phone that shows it. A window
    // that the Mac shares is not one to leave, and asking for it ends nothing.
    assert!(session.private && session.leave);
    assert!(shared.private && !shared.leave);
    asking.until_shown().await;
    assert!(!asking.mac.app.end_window_leaving(shared.id));
    tokio::time::sleep(QUIET).await;
    assert!(
        asking
            .sharer
            .heard
            .all()
            .iter()
            .all(|heard| !heard.starts_with("ended"))
    );

    // The person at the phone chooses to leave it. Both ends hear that it ended, and the platform
    // of the Mac is told that the choice was made, in place of the ordinary end, once.
    assert!(asking.phone.app.end_window_leaving(session.id));
    assert_eq!(
        ended_at(&mut phone_events, session.id).await,
        WindowEnd::ThisDevice
    );
    assert_eq!(
        ended_at(&mut mac_events, shared.id).await,
        WindowEnd::OtherDevice
    );
    until("the platform of the Mac to be told", || {
        asking.sharer.heard.has("ended leaving")
    })
    .await;
    tokio::time::sleep(QUIET).await;
    assert!(!asking.sharer.heard.has("ended"));
    assert_eq!(asking.sharer.heard.count("ended"), 1);

    // It is over: a second call finds nothing to end.
    assert!(!asking.phone.app.end_window_leaving(session.id));
    assert!(!asking.phone.app.end_window(session.id));
}

#[tokio::test]
async fn every_end_of_a_session_that_could_be_left_but_the_choice_to_leave_is_an_ordinary_end() {
    // The person ends it at the phone with the ordinary call, the person ends it at the Mac, and
    // the phone goes away without a word. The Mac locks at each of them, which its platform hears
    // as the ordinary end and never as the choice.
    for how in ["phone", "mac", "gone"] {
        let asking = Asking::with(Sharer::with_leave(true)).await;
        asking.allow(StartAccess::Control);
        let mut mac_events = asking.mac.app.events();
        let session = asking.started(true).await;
        let shared = started_at(&mut mac_events).await;
        assert!(session.leave, "{how}");
        asking.until_shown().await;
        match how {
            "phone" => assert!(asking.phone.app.end_window(session.id)),
            "mac" => assert!(asking.mac.app.end_window(shared.id)),
            _ => asking.phone.app.stop().await,
        }
        until("the platform of the Mac to be told", || {
            asking.sharer.heard.has("ended")
        })
        .await;
        tokio::time::sleep(QUIET).await;
        assert!(!asking.sharer.heard.has("ended leaving"), "{how}");
        assert_eq!(asking.sharer.heard.count("ended"), 1, "{how}");
    }
}

#[tokio::test]
async fn a_screen_that_the_phone_only_looks_at_has_nothing_to_leave_and_ends_as_any_other() {
    let asking = Asking::with(Sharer::with_leave(true)).await;
    asking.allow(StartAccess::Control);
    let mut phone_events = asking.phone.app.events();

    // Nothing is kept private for a screen that is only looked at, so nothing is offered.
    let session = asking.started(false).await;
    assert!(!session.private && !session.leave);
    asking.until_shown().await;

    // A phone that asks all the same ends the session as it ends every other, and is told that the
    // choice was not made.
    assert!(!asking.phone.app.end_window_leaving(session.id));
    assert_eq!(
        ended_at(&mut phone_events, session.id).await,
        WindowEnd::ThisDevice
    );
    until("the platform of the Mac to be told", || {
        asking.sharer.heard.has("ended")
    })
    .await;
    tokio::time::sleep(QUIET).await;
    assert!(!asking.sharer.heard.has("ended leaving"));
}

#[tokio::test]
async fn a_screen_of_a_platform_that_cannot_leave_is_not_offered_to_leave_and_ends_as_any_other() {
    let asking = Asking::with(Sharer::with_lift(true)).await;
    asking.allow(StartAccess::Control);
    let mut phone_events = asking.phone.app.events();

    // The screen is private, with everything that the platform can do about it, and the phone is
    // not offered the choice.
    let session = asking.started(true).await;
    assert!(session.control && session.private && !session.leave);
    asking.until_shown().await;

    // A phone that asks all the same ends the session as it ends every other, and is told that the
    // choice was not made.
    assert!(!asking.phone.app.end_window_leaving(session.id));
    assert_eq!(
        ended_at(&mut phone_events, session.id).await,
        WindowEnd::ThisDevice
    );
    until("the platform of the Mac to be told", || {
        asking.sharer.heard.has("ended")
    })
    .await;
    tokio::time::sleep(QUIET).await;
    assert!(!asking.sharer.heard.has("ended leaving"));
}

#[tokio::test]
async fn a_choice_to_leave_a_window_that_is_not_shown_here_is_not_taken() {
    let asking = Asking::with(Sharer::with_leave(true)).await;
    assert!(!asking.phone.app.end_window_leaving(0));
    assert!(!asking.phone.app.end_window_leaving(7));
    assert!(!asking.mac.app.end_window_leaving(u64::MAX));
}

#[tokio::test]
async fn a_screen_that_the_person_picks_and_the_platform_keeps_private_is_left_as_it_is_only_if_it_can_be()
 {
    // The phone does not always ask for the screen. A platform could keep a display that the person
    // picked private as well, and the phone that says yes to it is offered the choice, if the
    // platform can end such a session without locking, and only then.
    for can_leave in [true, false] {
        let platform = if can_leave {
            Sharer::with_leave(true)
        } else {
            Sharer::with_lift(true)
        };
        let pair = Pair::with_sharer(platform).await;
        *pair.sharer.next.lock().unwrap() = Some(whole_display());
        pair.sharer.picks_private.store(true, Ordering::Relaxed);
        let mut viewing_events = pair.viewing.app.events();

        let app = pair.sharing.app.clone();
        let to = pair.viewing.id();
        let offer = tokio::spawn(async move { app.share_window(&to, true).await });
        let request = pair.requested(&mut viewing_events).await;
        assert!(pair.viewing.app.answer_window(request.id, true));
        let WindowShared::Started(shared) = offer.await.unwrap().unwrap() else {
            panic!("the display was not shared");
        };
        let session = started_at(&mut viewing_events).await;
        assert!(shared.private && !shared.leave, "{can_leave}");
        assert!(session.private, "{can_leave}");
        assert_eq!(session.leave, can_leave);
        until("a picture to be shown at the phone", || {
            pair.viewer.heard.has("show 0")
        })
        .await;

        // The choice is taken only if it was offered, and the platform hears which end it was.
        assert_eq!(pair.viewing.app.end_window_leaving(session.id), can_leave);
        let (told, not_told) = if can_leave {
            ("ended leaving", "ended")
        } else {
            ("ended", "ended leaving")
        };
        until("the platform of the Mac to be told", || {
            pair.sharer.heard.has(told)
        })
        .await;
        tokio::time::sleep(QUIET).await;
        assert!(!pair.sharer.heard.has(not_told), "{can_leave}");
    }
}

#[tokio::test]
async fn a_screen_is_controlled_as_far_as_the_phone_wished_the_person_allowed_and_the_platform_can()
{
    for (wish, access, platform, granted) in [
        (true, StartAccess::Control, true, true),
        (false, StartAccess::Control, true, false),
        (true, StartAccess::Look, true, false),
        (true, StartAccess::Control, false, false),
    ] {
        assert_eq!(
            screen_control(wish, access, platform).await,
            (granted, granted),
            "the phone wished for control: {wish}, the person allowed {access:?}, the platform \
             can post input: {platform}"
        );
    }
}

#[tokio::test]
async fn a_phone_that_the_person_did_not_allow_is_told_so_in_words_and_may_ask_again() {
    let asking = Asking::new(true).await;
    let mut mac_events = asking.mac.app.events();

    let why = asking.refused(false).await;
    assert!(why.contains("Studio Mac"), "{why}");
    assert!(why.contains("has not allowed this device"), "{why}");
    assert_eq!(asking.screens(), 0);
    let started = tokio::time::timeout(QUIET, started_at(&mut mac_events)).await;
    assert!(started.is_err(), "a screen was shared that was not allowed");

    // Nothing is left behind: when the person at the Mac allows the phone, the next asking starts.
    asking.allow(StartAccess::Look);
    let session = asking.started(false).await;
    assert!(session.display);
    assert_eq!(asking.screens(), 1);
}

#[tokio::test]
async fn why_the_mac_has_no_screen_to_give_is_told_in_words_that_name_it() {
    let asking = Asking::new(false).await;
    asking.allow(StartAccess::Look);
    let reasons = [
        (ScreenError::Unsupported, "cannot start sharing its screen"),
        (
            ScreenError::NeedsPermission,
            "Screen & System Audio Recording",
        ),
        (
            ScreenError::NeedsInputPermission,
            "Accessibility and Input Monitoring",
        ),
        (ScreenError::Unavailable, "no screen to give right now"),
        (
            ScreenError::Failed(PlatformError::new("No window server.")),
            "could not give its screen",
        ),
    ];
    for (error, words) in reasons {
        *asking.sharer.screen_plan.lock().unwrap() = ScreenPlan::Refuses(error.clone());
        let why = asking.refused(false).await;
        assert!(why.contains("Studio Mac"), "{error}: {why}");
        assert!(why.contains(words), "{error}: {why}");
    }
    assert_eq!(asking.screens(), 5);

    // No asking left the Mac or the phone busy: once the Mac has a screen, the phone shows it.
    *asking.sharer.screen_plan.lock().unwrap() = ScreenPlan::Gives(whole_display());
    asking.started(false).await;
}

#[tokio::test]
async fn a_mac_that_cannot_start_sharing_when_asked_is_an_error_that_names_it() {
    let asking = Asking::with(Sharer::new(true, None)).await;
    asking.allow(StartAccess::Control);

    let error = asking
        .phone
        .app
        .start_screen(&asking.mac.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Declined);
    assert!(
        error.message().contains("Studio Mac"),
        "{}",
        error.message()
    );
    assert!(
        error.message().contains("cannot start sharing its screen"),
        "{}",
        error.message()
    );
    assert_eq!(asking.screens(), 0);
}

#[tokio::test]
async fn a_device_that_shows_no_screens_cannot_ask_for_one() {
    let mac = Dev::start(
        "Studio Mac",
        Windows::none().sharing(Sharer::with_screen(true)),
    )
    .await;
    let plain = Dev::start("Plain", Windows::none()).await;
    pair(&mac, &plain).await;
    mac.app
        .set_start_access(&plain.id(), StartAccess::Look)
        .unwrap();

    let error = plain.app.start_screen(&mac.id(), false).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert!(
        error.message().contains("cannot show screens"),
        "{}",
        error.message()
    );
}

#[tokio::test]
async fn only_a_paired_device_can_be_asked_for_its_screen() {
    let asking = Asking::new(false).await;
    for id in [
        Identity::generate().device_id().to_string(),
        "not an id".to_owned(),
    ] {
        let error = asking.phone.app.start_screen(&id, false).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotPaired, "{id}");
    }
    assert_eq!(asking.screens(), 0);
}

#[tokio::test]
async fn a_platform_that_cannot_make_the_screen_says_so_and_the_mac_is_not_asked() {
    let asking = Asking::new(false).await;
    asking.allow(StartAccess::Look);
    *asking.viewer.fail.lock().unwrap() = Some("The screen could not be made.".to_owned());

    let error = asking
        .phone
        .app
        .start_screen(&asking.mac.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert!(
        error.message().contains("could not be made"),
        "{}",
        error.message()
    );
    assert_eq!(asking.screens(), 0);

    // It was nothing that the Mac needs to know: once the platform can, the screen is shown.
    *asking.viewer.fail.lock().unwrap() = None;
    asking.started(false).await;
}

#[tokio::test]
async fn one_screen_at_a_time_and_the_next_asking_waits_until_the_first_is_ended() {
    let asking = Asking::new(false).await;
    asking.allow(StartAccess::Look);
    let mut mac_events = asking.mac.app.events();
    let session = asking.started(false).await;
    let shared = started_at(&mut mac_events).await;
    until("the first picture to be shown", || {
        asking.viewer.heard.has("show 0")
    })
    .await;

    let error = asking
        .phone
        .app
        .start_screen(&asking.mac.id(), false)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Other);
    assert!(
        error.message().contains("End it first"),
        "{}",
        error.message()
    );
    assert_eq!(asking.screens(), 1, "the Mac was asked a second time");

    // The Mac is busy until it has heard that the screen ended, and not for long.
    assert!(asking.phone.app.end_window(session.id));
    next_matching(&mut mac_events, |event| match event {
        Event::WindowEnded { id, .. } if id == shared.id => Some(()),
        _ => None,
    })
    .await;
    let again = asking.started(false).await;
    assert_ne!(again.id, session.id);
    assert_eq!(asking.screens(), 2);
}

#[tokio::test]
async fn the_mac_ends_the_screen_it_gave_and_the_phone_hears_it() {
    let asking = Asking::new(false).await;
    asking.allow(StartAccess::Look);
    let mut mac_events = asking.mac.app.events();
    let mut phone_events = asking.phone.app.events();
    let session = asking.started(false).await;
    let shared = started_at(&mut mac_events).await;

    assert!(asking.mac.app.end_window(shared.id));
    let end = next_matching(&mut phone_events, |event| match event {
        Event::WindowEnded { id, end } if id == session.id => Some(end),
        _ => None,
    })
    .await;
    assert_eq!(end, WindowEnd::OtherDevice);
    until("the platform of the phone to be told", || {
        asking.viewer.heard.has("ended")
    })
    .await;
}

#[tokio::test]
async fn a_person_who_gives_up_while_the_mac_gets_ready_leaves_nothing_shown_and_may_ask_again() {
    let asking = Asking::new(false).await;
    asking.allow(StartAccess::Look);
    let (open, gate) = oneshot::channel();
    *asking.sharer.screen_gate.lock().unwrap() = Some(gate);
    let mut phone_events = asking.phone.app.events();

    // The Mac takes its time to give the screen, and the asking is dropped.
    tokio::select! {
        outcome = asking.asks(false) => panic!("the asking was answered: {outcome:?}"),
        () = until("the Mac to ask its platform for the screen", || asking.screens() == 1) => {}
    }
    open.send(()).unwrap();
    until("the platform of the Mac to let go of the screen", || {
        asking.sharer.dropped.load(Ordering::Relaxed) == 1
    })
    .await;
    let started = tokio::time::timeout(QUIET, started_at(&mut phone_events)).await;
    assert!(
        started.is_err(),
        "a screen was shown that nobody waited for"
    );

    // The Mac and the phone are free for the next asking.
    asking.started(false).await;
    assert_eq!(asking.screens(), 2);
}

// The sound of a screen (ADR 0012): the person asks to hear it, the platform of the source records
// it, and the speaker of the viewer plays it.

/// The form of the sound that the platform of the source sends.
const AAC: Sound = Sound {
    codec: SoundCodec::Aac,
    rate: 48_000,
    channels: 2,
    samples: 1024,
};

/// The packet of sound with this number, as the encoder of the platform makes it.
fn packet(number: u64) -> SoundPacket {
    SoundPacket {
        bytes: vec![0x2a; 300],
        capture_time: 21_333 * number,
    }
}

/// A Mac that sends the sound of a whole display, an iPad that plays it, and the two paired.
async fn sounding() -> Pair {
    Pair::with_platforms(Sharer::with_sound(whole_display()), Viewer::playing()).await
}

/// The next time the screens hear how the sound of the window `id` stands.
async fn next_sound(events: &mut Events, id: u64) -> WindowSound {
    next_matching(events, |event| match event {
        Event::WindowSound { id: of, sound } if of == id => Some(sound),
        _ => None,
    })
    .await
}

/// The handle of the session that the platform of the source holds, once it has it.
async fn source_handle(sharer: &Sharer) -> Arc<SourceHandle> {
    until("the platform of the source to be started", || {
        sharer.handle.lock().unwrap().is_some()
    })
    .await;
    sharer.handle.lock().unwrap().clone().unwrap()
}

#[tokio::test]
async fn a_screen_can_be_heard_only_when_both_devices_and_the_screen_allow_it() {
    let cases = [
        (
            "a whole screen between two devices that carry sound",
            Sharer::with_sound(whole_display()),
            Viewer::playing(),
            true,
        ),
        (
            "a window",
            Sharer::with_sound(report_window()),
            Viewer::playing(),
            false,
        ),
        (
            "a source that sends no sound",
            Sharer::new(false, Some(whole_display())),
            Viewer::playing(),
            false,
        ),
        (
            "a viewer that plays no sound",
            Sharer::with_sound(whole_display()),
            Viewer::new(),
            false,
        ),
    ];
    for (what, sharer, viewer, heard) in cases {
        let pair = Pair::with_platforms(sharer, viewer).await;
        let mut viewing_events = pair.viewing.app.events();
        let WindowShared::Started(shared) = pair.offered_and_answered(true, false).await else {
            panic!("{what}: the window was not shared");
        };
        let shown = shown_window(&mut viewing_events).await;
        assert_eq!(shown.sound, heard, "{what}");
        // The request is taken exactly when the session says that the sound can be heard.
        assert_eq!(
            pair.viewing.app.hear_window(shown.id, false),
            heard,
            "{what}"
        );
        // The device that shares the screen sends its sound, and is not asked for it.
        assert!(!shared.sound, "{what}");
        assert!(!pair.sharing.app.hear_window(shared.id, true), "{what}");
    }
    // Nor is a session that does not run, or one that never was.
    let pair = sounding().await;
    assert!(!pair.viewing.app.hear_window(1, true));
    assert!(!pair.viewing.app.hear_window(u64::MAX, false));
}

#[tokio::test]
async fn a_device_is_set_up_to_play_sound_only_when_its_platform_says_it_does() {
    // The platform gives its screens a speaker and does not say that they play sound, as one that
    // was changed and not finished would. The device is not set up to play sound, which is also
    // what it tells the devices it is paired with, so the screen cannot be heard here and the
    // speaker is not used.
    let pair = Pair::with_platforms(Sharer::with_sound(whole_display()), Viewer::muddled()).await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the screen was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    assert!(!shown.sound);
    assert!(!pair.viewing.app.hear_window(shown.id, true));
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;
    // The Mac was not asked to record, and the speaker was not called, not even at the end.
    assert_eq!(pair.sharer.heard.count("hear"), 0);
    assert!(pair.viewing.app.end_window(shown.id));
    until("the screen to be told that the session is over", || {
        pair.viewer.heard.has("ended")
    })
    .await;
    assert_eq!(pair.viewer.sounds.all(), Vec::<String>::new());
}

#[tokio::test]
async fn the_sound_of_a_screen_goes_from_the_platform_of_the_source_to_the_speaker_of_the_viewer() {
    let pair = sounding().await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the screen was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    assert!(shown.sound);
    let handle = source_handle(&pair.sharer).await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    // Sound is off until the person asks, and the Mac is not asked to record before.
    assert_eq!(pair.sharer.heard.count("hear"), 0);
    assert!(pair.viewing.app.hear_window(shown.id, true));
    until(
        "the platform of the source to be asked for the sound",
        || pair.sharer.heard.has("hear 1"),
    )
    .await;
    // The speaker is told nothing until the platform says what its packets are.
    assert_eq!(pair.viewer.sounds.all(), Vec::<String>::new());
    handle.begin_sound(AAC).unwrap();
    assert_eq!(
        next_sound(&mut viewing_events, shown.id).await,
        WindowSound::Playing
    );

    // The packets reach the speaker in order, whole.
    for number in 0..3 {
        assert_eq!(handle.sound(packet(number)).unwrap(), Some(number));
    }
    until("the speaker to play all three", || {
        pair.viewer.sounds.count("play") == 3
    })
    .await;
    assert_eq!(
        pair.viewer.sounds.all(),
        [
            "sound Aac 1024",
            "play 0 lost 0 first true bytes 300",
            "play 1 lost 0 first false bytes 300",
            "play 2 lost 0 first false bytes 300",
        ]
    );

    // The person turns it off: the speaker stops, the screens hear it, and the Mac is told.
    assert!(pair.viewing.app.hear_window(shown.id, false));
    assert_eq!(
        next_sound(&mut viewing_events, shown.id).await,
        WindowSound::Off
    );
    until("the platform of the source to be told", || {
        pair.sharer.heard.has("hear 0")
    })
    .await;
    assert_eq!(pair.viewer.sounds.all().last().unwrap(), "stopped None");

    // The session ends: the speaker lets go, and nothing more is taken.
    assert!(pair.viewing.app.end_window(shown.id));
    until("the speaker to be told that the session is over", || {
        pair.viewer.sounds.has("ended")
    })
    .await;
    assert!(!pair.viewing.app.hear_window(shown.id, true));
}

#[tokio::test]
async fn a_source_that_cannot_record_says_why_and_tries_again_when_the_person_asks_again() {
    let pair = sounding().await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the screen was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    let handle = source_handle(&pair.sharer).await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;

    // The Mac has not let the program record its sound: the person is told why.
    assert!(pair.viewing.app.hear_window(shown.id, true));
    until("the platform of the source to be asked", || {
        pair.sharer.heard.count("hear 1") == 1
    })
    .await;
    handle.silent(SilentReason::NoPermission);
    assert_eq!(
        next_sound(&mut viewing_events, shown.id).await,
        WindowSound::NoPermission
    );
    assert_eq!(
        pair.viewer.sounds.all(),
        ["stopped Some(NoPermission)"],
        "the speaker was told why there is no sound"
    );

    // The person at the Mac allowed it, and the person here asks again: the platform of the source
    // is asked again, with nothing in between, and this time it can.
    assert!(pair.viewing.app.hear_window(shown.id, true));
    until("the platform of the source to be asked again", || {
        pair.sharer.heard.count("hear 1") == 2
    })
    .await;
    assert_eq!(pair.sharer.heard.count("hear 0"), 0);
    handle.begin_sound(AAC).unwrap();
    assert_eq!(
        next_sound(&mut viewing_events, shown.id).await,
        WindowSound::Playing
    );
    assert_eq!(
        pair.viewer.sounds.all(),
        ["stopped Some(NoPermission)", "sound Aac 1024"]
    );
}

#[tokio::test]
async fn a_device_that_cannot_play_the_sound_says_so_and_the_source_stops_sending_it() {
    let pair = sounding().await;
    let mut viewing_events = pair.viewing.app.events();
    let WindowShared::Started(_) = pair.offered_and_answered(true, false).await else {
        panic!("the screen was not shared");
    };
    let shown = shown_window(&mut viewing_events).await;
    let handle = source_handle(&pair.sharer).await;
    until("the pictures to be shown", || {
        pair.viewer.heard.has("show 2")
    })
    .await;
    assert!(pair.viewing.app.hear_window(shown.id, true));
    until("the platform of the source to be asked", || {
        pair.sharer.heard.count("hear 1") == 1
    })
    .await;
    handle.begin_sound(AAC).unwrap();
    assert_eq!(
        next_sound(&mut viewing_events, shown.id).await,
        WindowSound::Playing
    );

    // The speaker of the platform finds that it cannot play the run, from a thread of its own,
    // with the link that `open` gave it. The person hears that it was this device, the speaker is
    // stopped, and the Mac is told, so that it plays again.
    let link = pair.viewer.links.lock().unwrap()[0].clone();
    link.sound_failed();
    assert_eq!(
        next_sound(&mut viewing_events, shown.id).await,
        WindowSound::CannotPlay
    );
    until("the platform of the source to be told", || {
        pair.sharer.heard.has("hear 0")
    })
    .await;
    assert_eq!(pair.viewer.sounds.all().last().unwrap(), "stopped None");

    // A second report is about a run that is over: nothing more is told to anyone.
    link.sound_failed();
    tokio::time::sleep(QUIET).await;
    assert_eq!(pair.sharer.heard.count("hear 0"), 1);
    assert_eq!(pair.viewer.sounds.count("stopped"), 1);

    // The person asks again. The Mac is asked, begins a new run, and this time it plays.
    assert!(pair.viewing.app.hear_window(shown.id, true));
    until("the platform of the source to be asked again", || {
        pair.sharer.heard.count("hear 1") == 2
    })
    .await;
    handle.begin_sound(AAC).unwrap();
    assert_eq!(
        next_sound(&mut viewing_events, shown.id).await,
        WindowSound::Playing
    );
    assert_eq!(pair.viewer.sounds.count("sound"), 2);
}

#[tokio::test]
async fn the_sound_of_a_screen_that_a_phone_asked_for_can_be_heard_too() {
    let asking =
        Asking::with_platforms(Sharer::with_screen_and_sound(false), Viewer::playing()).await;
    asking.allow(StartAccess::Look);
    let session = asking.started(false).await;
    assert!(session.display);
    assert!(session.sound);
    assert!(asking.phone.app.hear_window(session.id, true));
    until("the platform of the Mac to be asked for the sound", || {
        asking.sharer.heard.has("hear 1")
    })
    .await;
}

#[tokio::test]
async fn a_screen_that_a_phone_asked_for_cannot_be_heard_unless_both_devices_carry_sound() {
    let cases = [
        (
            "a Mac that sends no sound",
            Sharer::with_screen(false),
            Viewer::playing(),
        ),
        (
            "a phone that plays no sound",
            Sharer::with_screen_and_sound(false),
            Viewer::new(),
        ),
        (
            "a phone whose platform says that it plays no sound",
            Sharer::with_screen_and_sound(false),
            Viewer::muddled(),
        ),
    ];
    for (what, sharer, viewer) in cases {
        let asking = Asking::with_platforms(sharer, viewer).await;
        asking.allow(StartAccess::Look);
        let session = asking.started(false).await;
        assert!(session.display, "{what}");
        assert!(!session.sound, "{what}");
        // The request is not taken, and the Mac is never asked to record.
        assert!(!asking.phone.app.hear_window(session.id, true), "{what}");
        assert_eq!(asking.sharer.heard.count("hear"), 0, "{what}");
    }
}
