// SPDX-License-Identifier: Apache-2.0

//! Whole sessions through the interface: two apps on this computer, paired. The one that shares
//! a window makes its pictures with the output of a real encoder; the one that shows it has a
//! platform behind the C interface, played by [`Fake`]. The fake checks every call against the
//! promises of the header, so these tests hold the Rust side to them while real sessions start,
//! run, fail and end.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use baton_app::{
    App, Event, Events, Options, WindowEnd, WindowRequest, WindowRole, WindowSession, WindowShared,
    Windows,
    windows::{PickedWindow, PlatformError, SharingFailure, WindowSharer},
};
use baton_core::{
    streaming::{Picture, SourceEvents, SourceHandle, Takes},
    util::BoxFuture,
};
use baton_proto::stream::Codec;
use baton_stream::{
    control::{Config, InputBatch, PauseReason, Report, Thermal, Want},
    gate::Reason,
    header::FrameKind,
    sender::LtrToken,
};
use tempfile::TempDir;

use crate::{
    Error, Registry,
    abi::{
        CODEC_H264, DROPPED_TOO_FAST, FRAME_KEY, FRAME_REFERENCED, PAUSE_LOCKED, SOURCE_CONFIG,
        SOURCE_NOTICE, SOURCE_PAUSE, SOURCE_RESUME, THERMAL_SERIOUS,
    },
    fake::{Call, Decoded, Fake, OpenPlan, Watchdog},
    fixture::{HEIGHT, IDR, PPS, SEI, SLICE, SPS, WIDTH, avcc_kinds, kind, pictures, units},
};

/// Long enough for a slow machine, short enough that a hang does not take the run with it.
const WAIT: Duration = Duration::from_secs(30);

/// Long enough for something that should not happen to have happened.
const QUIET: Duration = Duration::from_millis(400);

/// Waits until `condition` holds.
async fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits for an event that `pick` likes, and gives what it made of it.
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

/// Waits for a window to end, and gives how.
async fn window_ended(events: &mut Events) -> WindowEnd {
    next_matching(events, |event| match event {
        Event::WindowEnded { end, .. } => Some(end),
        _ => None,
    })
    .await
}

// The device that shares a window.

/// What the platform of the source heard from the viewer.
#[derive(Default)]
struct Heard {
    reports: Vec<Report>,
    /// How often the next frame was said to have to begin a chain.
    recoveries: usize,
    ended: bool,
}

struct Hearing(Arc<Mutex<Heard>>);

impl SourceEvents for Hearing {
    fn report(&mut self, report: Report) {
        self.0.lock().unwrap().reports.push(report);
    }

    fn want(&mut self, _want: Want) {}

    fn input(&mut self, _batch: InputBatch, _arrived: Duration) {}

    fn held(&mut self, _tokens: Vec<LtrToken>) {}

    fn recovery(&mut self) {
        self.0.lock().unwrap().recoveries += 1;
    }

    fn ended(&mut self) {
        self.0.lock().unwrap().ended = true;
    }
}

/// The platform of the device that shares. Its pictures are those of a real encoder, one at a
/// time when the test asks for one, so that a test knows what the other device is given.
struct Sharer {
    control: bool,
    heard: Arc<Mutex<Heard>>,
    /// The handle of the session, once it runs.
    handle: Arc<Mutex<Option<Arc<SourceHandle>>>>,
    /// The three pictures of the encoder: a key frame, a delta frame and a key frame.
    pictures: Vec<Vec<u8>>,
    /// How many pictures were made, which is also the time of the next in units of 16 ms.
    made: Mutex<u64>,
}

impl Sharer {
    fn new(control: bool) -> Arc<Self> {
        Arc::new(Self {
            control,
            heard: Arc::new(Mutex::new(Heard::default())),
            handle: Arc::new(Mutex::new(None)),
            pictures: pictures(),
            made: Mutex::new(0),
        })
    }

    fn handle(&self) -> Arc<SourceHandle> {
        self.handle
            .lock()
            .unwrap()
            .clone()
            .expect("the session of the source has not started")
    }

    fn recoveries(&self) -> usize {
        self.heard.lock().unwrap().recoveries
    }

    /// Makes the picture that the session asks for, once it lets one be made: the key frame of
    /// the encoder for a key frame, and its delta frame for the others. Gives the kind it was.
    async fn send(&self) -> FrameKind {
        let handle = self.handle();
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(kind) = handle.begin() {
                let bytes = match kind {
                    FrameKind::Key => self.pictures[0].clone(),
                    FrameKind::Recovery | FrameKind::Delta => self.pictures[1].clone(),
                };
                let mut made = self.made.lock().unwrap();
                let picture = Picture {
                    bytes,
                    referenced: kind != FrameKind::Delta,
                    ltr: None,
                    capture_time: 16_000 * *made,
                };
                *made += 1;
                drop(made);
                handle.sent(picture).unwrap();
                return kind;
            }
            assert!(
                Instant::now() < deadline,
                "the session never let a picture be made"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

impl WindowSharer for Sharer {
    fn control(&self) -> bool {
        self.control
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        let picked = Picked {
            heard: Arc::clone(&self.heard),
            handle: Arc::clone(&self.handle),
        };
        Box::pin(async move { Ok(Some(Box::new(picked) as Box<dyn PickedWindow>)) })
    }
}

struct Picked {
    heard: Arc<Mutex<Heard>>,
    handle: Arc<Mutex<Option<Arc<SourceHandle>>>>,
}

impl PickedWindow for Picked {
    fn title(&self) -> String {
        "Report.pdf".to_owned()
    }

    fn app(&self) -> String {
        "Preview".to_owned()
    }

    fn size(&self) -> (u32, u32) {
        (1440, 900)
    }

    fn events(&mut self) -> Box<dyn SourceEvents> {
        Box::new(Hearing(Arc::clone(&self.heard)))
    }

    fn prepare(&mut self, takes: &Takes) -> Option<Config> {
        takes.codecs.contains(&Codec::H264).then(|| Config {
            epoch: 0,
            codec: Codec::H264,
            // The pictures of the encoder are 64 by 48. A viewer that takes less is told less,
            // and a test that wants to is sent the pictures all the same.
            width: takes.max_width.min(WIDTH),
            height: takes.max_height.min(HEIGHT),
            fps: takes.max_fps.min(30),
        })
    }

    fn start(self: Box<Self>, handle: Arc<SourceHandle>, _failure: SharingFailure) {
        *self.handle.lock().unwrap() = Some(handle);
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
        // Settings that keep the device to this computer.
        std::fs::write(
            dir.path().join("config.toml"),
            format!(
                "device_name = \"{name}\"\n\n[network]\nrelay = \"disabled\"\nlan_discovery = \
                 false\nport_mapping = false\n"
            ),
        )
        .unwrap();
        let app = App::start_with(Options::new(dir.path()), windows)
            .await
            .unwrap();
        Self { app, _dir: dir }
    }

    fn id(&self) -> String {
        self.app.me().id
    }
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

/// A window that is shown: what each side got when the person said yes.
struct Shown {
    /// The session on the device that shows the window.
    viewing: WindowSession,
    /// The session on the device that shares it.
    sharing: WindowSession,
    /// What the person was asked.
    request: WindowRequest,
}

/// A sharing device, and a viewing device whose platform is behind the interface.
struct Rig {
    sharing: Dev,
    viewing: Dev,
    sharer: Arc<Sharer>,
    fake: Arc<Fake>,
    registry: Arc<Registry>,
    /// The events of the viewing device, from before the first window.
    viewing_events: Events,
    sharing_events: Events,
}

impl Rig {
    /// Starts both devices, with the fake behind the interface as it is made by `setup`.
    async fn with(control: bool, setup: impl FnOnce(&Arc<Fake>)) -> Self {
        let fake = Fake::new();
        setup(&fake);
        let registry = Arc::new(Registry::new());
        fake.register_in(&registry).unwrap();
        let sharer = Sharer::new(control);
        let sharing = Dev::start("Studio Mac", Windows::none().sharing(sharer.clone())).await;
        let viewing = Dev::start("Kitchen iPad", registry.windows()).await;
        pair(&sharing, &viewing).await;
        let viewing_events = viewing.app.events();
        let sharing_events = sharing.app.events();
        Self {
            sharing,
            viewing,
            sharer,
            fake,
            registry,
            viewing_events,
            sharing_events,
        }
    }

    async fn new(control: bool) -> Self {
        Self::with(control, |_| {}).await
    }

    /// Offers the window from the sharing device and says yes on the viewing device. Gives what
    /// came of the offer, and the sessions if it was taken.
    async fn offer(&mut self, control: bool, accept: bool) -> (WindowShared, WindowRequest) {
        let app = self.sharing.app.clone();
        let to = self.viewing.id();
        let offer = tokio::spawn(async move { app.share_window(&to, control).await });
        let request = next_matching(&mut self.viewing_events, |event| match event {
            Event::WindowRequested(request) => Some(request),
            _ => None,
        })
        .await;
        assert!(self.viewing.app.answer_window(request.id, accept));
        (offer.await.unwrap().unwrap(), request)
    }

    /// Shares the window and gives both sessions, once it is shown.
    async fn share(&mut self, control: bool) -> Shown {
        let (shared, request) = self.offer(control, true).await;
        let WindowShared::Started(sharing) = shared else {
            panic!("the window was not shared: {shared:?}");
        };
        let viewing = next_matching(&mut self.viewing_events, |event| match event {
            Event::WindowStarted(session) => Some(session),
            _ => None,
        })
        .await;
        assert_eq!(viewing.role, WindowRole::Viewing);
        assert_eq!(sharing.role, WindowRole::Sharing);
        // The source says at once that it wants the first picture of the session, so a test that
        // counts what it is asked for afterwards counts from here.
        until("the source to want its first picture", || {
            self.sharer.recoveries() >= 1
        })
        .await;
        Shown {
            viewing,
            sharing,
            request,
        }
    }

    /// The calls about a screen that are not a question about how the decoder is doing, which the
    /// session asks as it likes.
    fn calls(&self, screen: u64) -> Vec<Call> {
        self.fake
            .calls(screen)
            .into_iter()
            .filter(|call| *call != Call::State)
            .collect()
    }

    fn decoded(&self, screen: u64) -> Vec<Decoded> {
        self.calls(screen)
            .into_iter()
            .filter_map(|call| match call {
                Call::Decode(decoded) => Some(decoded),
                _ => None,
            })
            .collect()
    }

    /// The link that `open` gave for a screen.
    fn link(&self, screen: u64) -> u64 {
        self.fake
            .log()
            .into_iter()
            .filter_map(|(_, call)| match call {
                Call::Open { link, .. } => Some(link),
                _ => None,
            })
            .nth(usize::try_from(screen).unwrap() - 1)
            .expect("the screen was not opened")
    }

    /// Waits until the screens that were opened have been released, and checks that nothing is
    /// left behind and that the platform was never promised anything that was not kept.
    async fn nothing_is_left(&self) {
        until("every screen to be released", || {
            self.fake.live_screens() == 0 && self.registry.open_links() == 0
        })
        .await;
        for screen in 1..=self.fake.screens_made() as u64 {
            let calls = self.calls(screen);
            let ends: Vec<&Call> = calls
                .iter()
                .filter(|call| matches!(call, Call::Ended | Call::Release))
                .collect();
            assert_eq!(
                ends,
                [&Call::Ended, &Call::Release],
                "screen {screen}: the session is over once, and the screen is released once"
            );
            assert_eq!(
                calls.last(),
                Some(&Call::Release),
                "screen {screen}: nothing follows the release"
            );
        }
        assert!(
            self.fake.complaints().is_empty(),
            "{:?}",
            self.fake.complaints()
        );
    }
}

/// The sample that a decoder is to be given for a picture of the encoder: the units that are not
/// delimiters, parameter sets or filler, each behind its length in four bytes.
fn avcc(picture: &[u8]) -> Vec<u8> {
    const DELIMITER: u8 = 9;
    const FILLER: u8 = 12;
    units(picture)
        .into_iter()
        .filter(|unit| ![DELIMITER, SPS, PPS, FILLER].contains(&kind(unit)))
        .flat_map(|unit| {
            let mut out = u32::try_from(unit.len()).unwrap().to_be_bytes().to_vec();
            out.extend(unit);
            out
        })
        .collect()
}

/// The units of a picture of the encoder that are of this kind, without their start codes.
fn units_of(picture: &[u8], wanted: u8) -> Vec<Vec<u8>> {
    units(picture)
        .into_iter()
        .filter(|unit| kind(unit) == wanted)
        .collect()
}

// The tests.

#[tokio::test]
async fn a_window_is_offered_to_the_platform_with_what_the_person_was_told() {
    let mut rig = Rig::new(true).await;
    let shown = rig.share(true).await;

    // The person was asked, and told what the window is.
    assert_eq!(shown.request.title.as_deref(), Some("Report.pdf"));
    assert_eq!(shown.request.app.as_deref(), Some("Preview"));
    assert_eq!((shown.request.width, shown.request.height), (1440, 900));
    assert!(shown.request.control);

    // The platform was asked for a screen for the same window, and the id it gave for the
    // screen is the one the user interface is given to draw it with.
    let opens: Vec<Call> = rig
        .fake
        .calls(0)
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .collect();
    let link = rig.link(1);
    assert_ne!(link, 0);
    assert_eq!(
        opens,
        [Call::Open {
            title: "Report.pdf".to_owned(),
            app: "Preview".to_owned(),
            width: 1440,
            height: 900,
            control: 1,
            link,
        }]
    );
    assert_eq!(shown.viewing.screen, Some(100));
    assert_eq!(rig.registry.open_links(), 1);
    assert_eq!(rig.fake.live_screens(), 1);
}

#[tokio::test]
async fn a_picture_reaches_the_platform_as_the_apple_decoder_takes_it() {
    let mut rig = Rig::new(false).await;
    rig.share(false).await;
    let pictures = pictures();

    // A key frame, and then a frame that refers to it.
    assert_eq!(rig.sharer.send().await, FrameKind::Key);
    assert_eq!(rig.sharer.send().await, FrameKind::Delta);
    until("both pictures to be shown", || {
        rig.calls(1).contains(&Call::Show(1))
    })
    .await;

    let decoded = rig.decoded(1);
    assert_eq!(decoded.len(), 2);
    let (key, delta) = (&decoded[0], &decoded[1]);

    // The key frame has the parameter sets, which are taken out of the picture, and the rest of
    // its units in the form with lengths.
    assert_eq!(key.sample, avcc(&pictures[0]));
    assert_eq!(avcc_kinds(&key.sample), [SEI, IDR, IDR]);
    let mut sets = units_of(&pictures[0], SPS);
    sets.extend(units_of(&pictures[0], PPS));
    assert_eq!(key.sets, sets);
    assert_eq!(key.sequence_sets, 1);
    assert_eq!(key.flags, FRAME_KEY | FRAME_REFERENCED);
    assert_eq!((key.number, key.depends_on, key.capture_time), (0, 0, 0));

    // A delta frame has the units of its slices and no sets, and says what it refers to.
    assert_eq!(delta.sample, avcc(&pictures[1]));
    assert_eq!(avcc_kinds(&delta.sample), [SLICE, SLICE]);
    assert_eq!(delta.sets.len(), 0);
    assert_eq!(delta.sequence_sets, 0);
    assert_eq!(delta.flags, 0);
    assert_eq!(
        (delta.number, delta.depends_on, delta.capture_time),
        (1, 0, 16_000)
    );
    assert_eq!((delta.chain, delta.epoch), (key.chain, key.epoch));

    // The platform is told which frame to show after it was given it, in that order, and the
    // source's configuration came before the first picture.
    let calls = rig.calls(1);
    let position = |wanted: &Call| calls.iter().position(|call| call == wanted).unwrap();
    assert!(position(&Call::Decode(key.clone())) < position(&Call::Show(0)));
    assert!(position(&Call::Show(0)) < position(&Call::Decode(delta.clone())));
    assert!(position(&Call::Decode(delta.clone())) < position(&Call::Show(1)));
    let configured = calls
        .iter()
        .position(|call| matches!(call, Call::Source(message) if message.kind == SOURCE_CONFIG))
        .expect("the platform was not told what the pictures are");
    let Call::Source(config) = &calls[configured] else {
        unreachable!()
    };
    assert_eq!(
        (config.codec, config.width, config.height, config.fps),
        (CODEC_H264, WIDTH, HEIGHT, 30)
    );
    assert_eq!(config.epoch, key.epoch);
    assert!(configured < position(&Call::Decode(key.clone())));
}

/// How a window can end.
#[derive(Debug, Clone, Copy)]
enum Ending {
    ViewerEndsIt,
    SharerEndsIt,
    SharingAppStops,
    ViewingAppStops,
}

/// Shares a window, shows two pictures, ends it the way that is said, and checks that the
/// platform was told once and has nothing left.
async fn ends(how: Ending) {
    let mut rig = Rig::new(false).await;
    let shown = rig.share(false).await;
    rig.sharer.send().await;
    rig.sharer.send().await;
    until("the pictures to be shown", || {
        rig.calls(1).contains(&Call::Show(1))
    })
    .await;
    match how {
        Ending::ViewerEndsIt => assert!(rig.viewing.app.end_window(shown.viewing.id)),
        Ending::SharerEndsIt => assert!(rig.sharing.app.end_window(shown.sharing.id)),
        Ending::SharingAppStops => rig.sharing.app.stop().await,
        Ending::ViewingAppStops => rig.viewing.app.stop().await,
    }
    // Each device that is still running hears how the window ended. The one that stopped does
    // not, since its events end with it.
    if !matches!(how, Ending::ViewingAppStops) {
        let end = window_ended(&mut rig.viewing_events).await;
        match how {
            Ending::ViewerEndsIt => assert_eq!(end, WindowEnd::ThisDevice, "{how:?}"),
            Ending::SharerEndsIt => assert_eq!(end, WindowEnd::OtherDevice, "{how:?}"),
            _ => assert!(
                matches!(end, WindowEnd::OtherDevice | WindowEnd::Lost),
                "{how:?}: {end:?}"
            ),
        }
    }
    if !matches!(how, Ending::SharingAppStops) {
        let end = window_ended(&mut rig.sharing_events).await;
        match how {
            Ending::SharerEndsIt => assert_eq!(end, WindowEnd::ThisDevice, "{how:?}"),
            Ending::ViewerEndsIt => assert_eq!(end, WindowEnd::OtherDevice, "{how:?}"),
            _ => assert!(
                matches!(end, WindowEnd::OtherDevice | WindowEnd::Lost),
                "{how:?}: {end:?}"
            ),
        }
    }
    rig.nothing_is_left().await;

    // A number that stood for the screen is not one any more.
    let link = rig.link(1);
    assert_eq!(
        rig.registry.link_decode_failed(link),
        Err(Error::UnknownLink)
    );
    // And the platform has not been called again by anything since.
    let calls = rig.fake.log().len();
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.fake.log().len(), calls, "{how:?}");
}

#[tokio::test]
async fn a_window_that_the_viewer_ends_is_released_once() {
    ends(Ending::ViewerEndsIt).await;
}

#[tokio::test]
async fn a_window_that_the_sharer_ends_is_released_once() {
    ends(Ending::SharerEndsIt).await;
}

#[tokio::test]
async fn a_window_whose_sharing_app_stops_is_released_once() {
    ends(Ending::SharingAppStops).await;
}

#[tokio::test]
async fn a_window_whose_viewing_app_stops_is_released_once() {
    ends(Ending::ViewingAppStops).await;
}

#[tokio::test]
async fn a_frame_that_the_platform_refuses_has_the_viewer_ask_for_a_new_key_frame() {
    let mut rig = Rig::new(false).await;
    rig.share(false).await;
    // The first frame of a session is a recovery in itself, so the source has said so once.
    let before = rig.sharer.recoveries();

    rig.fake.fail_decode(-1);
    assert_eq!(rig.sharer.send().await, FrameKind::Key);
    until("the source to be asked for another key frame", || {
        rig.sharer.recoveries() > before
    })
    .await;
    // The platform was given the frame, and it said no.
    assert_eq!(rig.decoded(1).len(), 1);

    // The next key frame is decoded, and shown.
    rig.fake.fail_decode(0);
    assert_eq!(rig.sharer.send().await, FrameKind::Key);
    until("the second key frame to be shown", || {
        rig.calls(1).contains(&Call::Show(1))
    })
    .await;
    assert_eq!(rig.decoded(1).len(), 2);
    assert_eq!(rig.sharer.send().await, FrameKind::Delta);
    until("the delta frame to be shown", || {
        rig.calls(1).contains(&Call::Show(2))
    })
    .await;
}

#[tokio::test]
async fn a_decode_that_fails_later_on_a_thread_of_the_platform_has_the_viewer_ask_again() {
    let mut rig = Rig::new(false).await;
    rig.share(false).await;
    rig.sharer.send().await;
    until("the picture to be shown", || {
        rig.calls(1).contains(&Call::Show(0))
    })
    .await;
    // Nothing is lost, so the source is not asked for anything. How many times it was asked
    // while the first picture was on its way depends on the timing, so this counts from here.
    tokio::time::sleep(QUIET).await;
    let before = rig.sharer.recoveries();

    // The decoder fails after `decode` took the frame, on a thread of its own, with the number
    // that `open` gave for the screen.
    let registry = Arc::clone(&rig.registry);
    let link = rig.link(1);
    let platform = std::thread::spawn(move || registry.link_decode_failed(link));
    assert_eq!(platform.join().unwrap(), Ok(()));
    until("the source to be asked for a new key frame", || {
        rig.sharer.recoveries() > before
    })
    .await;
    assert_eq!(rig.sharer.send().await, FrameKind::Key);
    until("the new key frame to be shown", || {
        rig.calls(1).contains(&Call::Show(1))
    })
    .await;
}

#[tokio::test]
async fn a_picture_beyond_what_the_platform_said_it_can_show_never_reaches_it() {
    // The platform takes 32 by 32, so the source is told so and configures it. The pictures of
    // the encoder are 64 by 48: a source that does not keep to what it was told.
    let mut rig = Rig::with(false, |fake| {
        fake.set_limits(|limits| {
            limits.max_width = 32;
            limits.max_height = 32;
        });
    })
    .await;
    rig.share(false).await;
    let before = rig.sharer.recoveries();
    assert_eq!(rig.sharer.send().await, FrameKind::Key);
    until("the source to be asked for another key frame", || {
        rig.sharer.recoveries() > before
    })
    .await;
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.decoded(1).len(), 0, "the platform was given a frame");
    assert!(
        !rig.calls(1)
            .iter()
            .any(|call| matches!(call, Call::Show(_)))
    );

    // The session goes on, and is ended in the usual way.
    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_screen_that_cannot_be_made_is_not_left_behind() {
    for (plan, sentence) in [
        (
            OpenPlan::Fail {
                text: b"The decoder is busy.".to_vec(),
                claimed_len: 20,
            },
            "The decoder is busy.",
        ),
        (
            OpenPlan::NoPointer,
            "This device could not show the window.",
        ),
    ] {
        let mut rig = Rig::new(false).await;
        rig.fake.plan_open(plan);
        let (shared, request) = rig.offer(false, true).await;
        assert!(matches!(shared, WindowShared::Declined(_)), "{shared:?}");

        // The person who said yes is told why nothing is shown.
        let failure = next_matching(&mut rig.viewing_events, |event| match event {
            Event::WindowRequestEnded { id, failure } if id == request.id => Some(failure),
            _ => None,
        })
        .await;
        assert_eq!(failure.as_deref(), Some(sentence));
        // The link that was made for the screen is not kept, and the screen that was not made
        // is not released.
        assert_eq!(rig.registry.open_links(), 0);
        assert_eq!(rig.fake.live_screens(), 0);
        assert!(
            rig.fake.complaints().is_empty(),
            "{:?}",
            rig.fake.complaints()
        );
        assert_eq!(rig.fake.count(|call| *call == Call::Release), 0);
    }
}

#[tokio::test]
async fn what_the_source_says_about_the_pictures_reaches_the_platform() {
    let mut rig = Rig::new(false).await;
    rig.share(false).await;
    let handle = rig.sharer.handle();
    let messages = |rig: &Rig| -> Vec<(u32, u32, u32, u32)> {
        rig.calls(1)
            .into_iter()
            .filter_map(|call| match call {
                Call::Source(message) => {
                    Some((message.kind, message.reason, message.count, message.epoch))
                }
                _ => None,
            })
            .collect()
    };

    handle.pause(PauseReason::Locked);
    handle.resume();
    // The size of the picture changed: a new epoch, and the viewer is told.
    handle
        .new_epoch(Config {
            epoch: 0,
            codec: Codec::H264,
            width: WIDTH,
            height: HEIGHT,
            fps: 15,
        })
        .unwrap();
    // Events of the viewer were dropped, three of them for being too fast.
    for _ in 0..3 {
        handle.dropped(Reason::TooFast);
    }
    // At most one notice comes in a second, so the three may come as one or as more.
    let counted = |rig: &Rig| -> u32 {
        messages(rig)
            .iter()
            .filter(|(kind, ..)| *kind == SOURCE_NOTICE)
            .map(|(_, _, count, _)| *count)
            .sum()
    };
    until("the platform to be told of all of it", || {
        counted(&rig) == 3
    })
    .await;

    let heard = messages(&rig);
    let kinds: Vec<u32> = heard.iter().map(|(kind, ..)| *kind).collect();
    assert_eq!(
        kinds[..4],
        [SOURCE_CONFIG, SOURCE_PAUSE, SOURCE_RESUME, SOURCE_CONFIG]
    );
    assert!(kinds[4..].iter().all(|kind| *kind == SOURCE_NOTICE));
    assert_eq!(heard[1].1, PAUSE_LOCKED);
    assert!(
        heard[4..]
            .iter()
            .all(|(_, reason, ..)| *reason == DROPPED_TOO_FAST)
    );
    // The second configuration is of a new epoch.
    assert_eq!(heard[3].3, heard[0].3 + 1);
    let Some(Call::Source(config)) = rig
        .calls(1)
        .into_iter()
        .filter(|call| matches!(call, Call::Source(message) if message.kind == SOURCE_CONFIG))
        .nth(1)
    else {
        panic!("no second configuration");
    };
    assert_eq!(
        (config.width, config.height, config.fps),
        (WIDTH, HEIGHT, 15)
    );
    assert!(
        rig.fake.complaints().is_empty(),
        "{:?}",
        rig.fake.complaints()
    );
}

#[tokio::test]
async fn how_the_decoder_is_doing_reaches_the_source() {
    let mut rig = Rig::new(false).await;
    rig.share(false).await;
    rig.fake.set_state(2345, THERMAL_SERIOUS);
    rig.sharer.send().await;
    until("the source to hear how the decoder is doing", || {
        rig.sharer
            .heard
            .lock()
            .unwrap()
            .reports
            .iter()
            .any(|report| report.decode_us == 2345)
    })
    .await;
    let heard = rig.sharer.heard.lock().unwrap();
    let report = heard
        .reports
        .iter()
        .rfind(|report| report.decode_us == 2345)
        .unwrap();
    assert_eq!(report.thermal, Thermal::Serious);
}

#[tokio::test]
async fn the_platform_cannot_call_back_while_the_library_is_calling_it() {
    let mut rig = Rig::new(false).await;
    // Whichever call of a screen the library makes, the platform answers by reporting a failure
    // of the decoder, which would take the lock that the library holds for the call.
    let answers = Arc::new(Mutex::new(Vec::new()));
    {
        let registry = Arc::clone(&rig.registry);
        let answers = Arc::clone(&answers);
        let link = Arc::new(Mutex::new(None));
        rig.fake.on_call(move |call| {
            if let Call::Open { link: opened, .. } = call {
                *link.lock().unwrap() = Some(*opened);
            }
            let known = *link.lock().unwrap();
            if let Some(known) = known {
                // A call that was let in would wait for the lock that the library holds.
                let _watch = Watchdog::start("a call back into the library");
                answers
                    .lock()
                    .unwrap()
                    .push((call.clone(), registry.link_decode_failed(known)));
            }
        });
    }
    rig.share(false).await;
    rig.sharer.send().await;
    rig.sharer.send().await;
    until("the pictures to be shown", || {
        rig.calls(1).contains(&Call::Show(1))
    })
    .await;
    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;

    let answers = answers.lock().unwrap();
    // Every call about the screen was answered, and none was let in.
    assert!(answers.len() >= 6, "{} calls", answers.len());
    for (call, answer) in answers.iter() {
        assert_eq!(*answer, Err(Error::Reentrant), "{call:?}");
    }
    // None of them was taken for a decoder that failed.
    assert_eq!(rig.sharer.recoveries(), 1);
}

#[tokio::test]
async fn a_second_window_gets_a_screen_and_a_link_of_its_own() {
    let mut rig = Rig::new(false).await;
    let first = rig.share(false).await;
    rig.sharer.send().await;
    until("the first picture to be shown", || {
        rig.calls(1).contains(&Call::Show(0))
    })
    .await;
    assert!(rig.viewing.app.end_window(first.viewing.id));
    window_ended(&mut rig.viewing_events).await;
    window_ended(&mut rig.sharing_events).await;
    until("the first screen to be released", || {
        rig.fake.live_screens() == 0
    })
    .await;

    rig.fake.plan_open(OpenPlan::Screen { id: 200 });
    let second = rig.share(false).await;
    assert_eq!(second.viewing.screen, Some(200));
    rig.sharer.send().await;
    until("the first picture of the second window to be shown", || {
        rig.calls(2).contains(&Call::Show(0))
    })
    .await;
    // A number that stood for the first screen does not reach the second.
    let (one, two) = (rig.link(1), rig.link(2));
    assert_ne!(one, two);
    assert_eq!(
        rig.registry.link_decode_failed(one),
        Err(Error::UnknownLink)
    );
    assert_eq!(rig.registry.link_decode_failed(two), Ok(()));

    assert!(rig.sharing.app.end_window(second.sharing.id));
    rig.nothing_is_left().await;
    assert_eq!(rig.fake.screens_made(), 2);
}
