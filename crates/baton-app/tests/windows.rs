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
    App, ErrorKind, Event, Events, Options, WindowAbilities, WindowButton, WindowEnd, WindowInput,
    WindowModifiers, WindowNoticeReason, WindowPointer, WindowRequest, WindowRole, WindowSession,
    WindowShared, Windows,
    windows::{
        PickedWindow, PlatformError, Screen, ScreenLink, SharingFailure, ShowLimits, WindowSharer,
        WindowViewer,
    },
};
use baton_core::{
    streaming::{
        DecodeError, Decoder, DecoderState, Picture, SourceEvents, SourceHandle, Takes, WindowOffer,
    },
    util::BoxFuture,
};
use baton_proto::stream::Codec;
use baton_stream::{
    control::{Config, InputBatch, Report, SourceMessage, Thermal, Want},
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
}

impl Viewer {
    fn new() -> Arc<Self> {
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
        Ok(Screen {
            decoder: Box::new(Screening(Arc::clone(&self.heard))),
            id: self.next_screen.fetch_add(1, Ordering::Relaxed),
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

struct Sharer {
    control: bool,
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
}

impl Sharer {
    fn new(control: bool, next: Option<Chosen>) -> Arc<Self> {
        Arc::new(Self {
            control,
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
        })
    }

    /// What the platform was given of what the person at the other device did, in order.
    fn inputs(&self) -> Vec<InputEvent> {
        self.inputs.lock().unwrap().clone()
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
        self.control
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        if let Some(why) = self.fail.lock().unwrap().clone() {
            return Box::pin(async move { Err(PlatformError::new(why)) });
        }
        let chosen = self.next.lock().unwrap().take();
        let gate = self.gate.lock().unwrap().take();
        let shared = Picked {
            chosen: chosen.clone(),
            heard: Arc::clone(&self.heard),
            handle: Arc::clone(&self.handle),
            dropped: Arc::clone(&self.dropped),
            pictures: self.pictures,
            makes_pictures: self.makes_pictures.load(Ordering::Relaxed),
            lets_go_at_once: self.lets_go_at_once.load(Ordering::Relaxed),
            fails_at_once: self.fails_at_once.lock().unwrap().clone(),
            failure: Arc::clone(&self.failure),
            inputs: Arc::clone(&self.inputs),
        };
        Box::pin(async move {
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            Ok(chosen.map(|_| Box::new(shared) as Box<dyn PickedWindow>))
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
        Box::new(Hearing(Arc::clone(&self.heard), Arc::clone(&self.inputs)))
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

struct Hearing(Arc<Heard>, Arc<Mutex<Vec<InputEvent>>>);

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

    fn recovery(&mut self) {
        self.0.push("recovery");
    }

    fn ended(&mut self) {
        self.0.push("ended");
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
        let sharer = Sharer::new(control, Some(report_window()));
        let viewer = Viewer::new();
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
    let abilities = |dev: &Dev| dev.app.window_abilities();
    assert_eq!(
        abilities(&plain),
        WindowAbilities {
            can_show: false,
            can_share: false,
            can_control: false
        }
    );
    assert_eq!(
        abilities(&shows),
        WindowAbilities {
            can_show: true,
            can_share: false,
            can_control: false
        }
    );
    assert_eq!(
        abilities(&shares),
        WindowAbilities {
            can_show: false,
            can_share: true,
            can_control: false
        }
    );
    assert_eq!(
        abilities(&controls),
        WindowAbilities {
            can_show: true,
            can_share: true,
            can_control: true
        }
    );
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
    tokio::time::sleep(QUIET).await;
    assert_eq!(pair.sharer.heard.count("recovery"), recoveries);
    assert!(!pair.sharing.app.end_window(shared.id));

    // A link that no session was ever made for does nothing either.
    ScreenLink::default().decode_failed();
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
