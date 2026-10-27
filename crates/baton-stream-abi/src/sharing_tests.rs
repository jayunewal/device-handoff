// SPDX-License-Identifier: Apache-2.0

//! Whole sessions through the interface, with the platform of the device that SHARES the window
//! behind it: two apps on this computer, paired. The device that shares has a platform played by
//! [`FakeSharer`], which checks every call against the promises of the header and stands in for
//! the capture and the encoder (the pictures are those of a real encoder, in the form that the
//! Apple encoder makes them). The device that shows the window has a platform played by [`Fake`],
//! so a picture goes the whole way: from the functions that the sharing platform calls, over the
//! network, to the functions that the showing platform gets.

#![allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    collections::HashSet,
    mem::{self, offset_of},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use baton_app::{
    Event, Events, StartAccess, WindowButton, WindowEnd, WindowInput, WindowModifiers,
    WindowNoticeReason, WindowPointer, WindowRequest, WindowRole, WindowSession, WindowShared,
    WindowSound, Windows,
    windows::{
        PickedWindow, PlatformError, Privacy, ScreenError, ScreenLink, SharingFailure, WindowSharer,
    },
};
use baton_core::{
    Config as NodeConfig, Identity, Node, TrustStore,
    streaming::{
        DecodeError, Decoder, DecoderState, SourceEvents, SourceHandle, StartOutcome, Takes,
        Viewing, Wanted,
    },
    util::BoxFuture,
};
use baton_proto::{
    DeviceId,
    limits::{MAX_PICTURE_BYTES, MAX_SOUND_PACKET_BYTES, MAX_WINDOW_EDGE},
    stream::Codec,
    ticket::PairTicket,
};
use baton_stream::{
    control::{Config as Format, Lift, SourceMessage, Thermal},
    gate::MAX_STALE,
    header::FrameHeader,
    input::{NO_KEY, PointerAction},
};
use tempfile::TempDir;
use tokio::task::JoinHandle;

use crate::{
    Error, Registry,
    abi::{
        BEGIN_DELTA, BEGIN_KEY, BEGIN_NOT_NOW, BatonBytes, BatonFormat, BatonFront, BatonGeometry,
        BatonPicked, BatonPicture, BatonRect, BatonSharerVTable, BatonSound, BatonSoundPacket,
        CODEC_H264, E_PLATFORM, FRAME_KEY, FRAME_RECOVERY, FRAME_REFERENCED, KEYBOARD_ELSEWHERE,
        LIFT_INPUT, LIFT_SCREEN, OK, PAUSE_HIDDEN, PAUSE_LOCKED, PAUSE_OTHER, PICKED_DISPLAY,
        PICKED_WINDOW, PLAY_FIRST, SCREEN_NEEDS_INPUT_PERMISSION, SCREEN_NEEDS_PERMISSION,
        SCREEN_PRIVATE, SCREEN_UNAVAILABLE, SENT, SENT_OVER, SENT_STALE, SENT_TOO_LARGE,
        SILENT_FAILED, SILENT_NO_PERMISSION, SILENT_OFF, SILENT_OTHER, SILENT_WINDOW,
        SOUND_DROPPED, SOUND_OVER, SOUND_SENT, SOUND_TOO_LARGE, SOURCE_CONFIG, SOURCE_PAUSE,
        SOURCE_RESUME, size_of_u32,
    },
    exports,
    fake::{Call, Decoded, Fake, Played, Watchdog},
    fake_sharer::{
        self, Around, FakeSharer, Pic, PickPlan, PreparePlan, RECORD, STOP, ScreenPlan, SharerCall,
        StartPlan, aac, begin, cancel, changed, clock, end, key_post, left_down, left_up,
        new_epoch, new_epoch_raw, pause, pointer_post, recovery_pending, resume, sent, sent_sized,
        sent_with, sound, sound_begin, sound_begin_raw, sound_raw, sound_silent,
    },
    fixture::{HEIGHT, PPS, SPS, WIDTH, pictures, sps},
    input::{Forward, Timing},
    registry::standing_in,
    session_tests::{Dev, QUIET, WAIT, avcc, next_matching, pair, units_of, until, window_ended},
};

/// How long everything has to be let go after the end of a window. It is far less than the 30 s
/// of silence after which a session ends by itself, so that a session that nobody ended does not
/// pass for one that was.
const LET_GO: Duration = Duration::from_secs(10);

/// A window that is shared: what each side got when the person said yes.
struct Shown {
    /// The session on the device that shows the window.
    viewing: WindowSession,
    /// The session on the device that shares it.
    sharing: WindowSession,
    /// What the person at the other device was asked.
    request: WindowRequest,
    /// The number of the window at the sharing platform: the first window it gave is 1.
    window: u64,
    /// The number that `start` gave the platform for the session.
    link: u64,
}

/// A device that shares windows, with a platform behind the interface, and a device that shows
/// them, with another.
struct Rig {
    sharing: Dev,
    viewing: Dev,
    /// The platform of the device that shares.
    platform: Arc<FakeSharer>,
    /// The platform of the device that shows.
    viewer: Arc<Fake>,
    /// What the device that shares registered.
    registry: Arc<Registry>,
    /// What the device that shows registered.
    viewing_registry: Arc<Registry>,
    sharing_events: Events,
    viewing_events: Events,
    /// How many picks the platform was asked for, and how many windows it gave.
    picks: usize,
    windows: u64,
}

/// What happens to a window instead of the start of its session: it is given as the app handed it
/// over, with the handle and the failure of the session.
type Starting = Arc<dyn Fn(Box<dyn PickedWindow>, Arc<SourceHandle>, SharingFailure) + Send + Sync>;

/// The sharer of the platform, with `starting` between the app and every window that it picks: a
/// way to start a session in a state that the app does not start one in, such as one that is over
/// already or one that is started where nothing can wait for its end.
struct Meddling {
    sharer: Arc<dyn WindowSharer>,
    starting: Starting,
}

impl WindowSharer for Meddling {
    fn control(&self) -> bool {
        self.sharer.control()
    }

    fn pick(&self) -> BoxFuture<'static, Result<Option<Box<dyn PickedWindow>>, PlatformError>> {
        let (pick, starting) = (self.sharer.pick(), Arc::clone(&self.starting));
        Box::pin(async move {
            Ok(pick.await?.map(|window| {
                Box::new(MeddledWindow {
                    window,
                    starting: Arc::clone(&starting),
                }) as Box<dyn PickedWindow>
            }))
        })
    }
}

struct MeddledWindow {
    window: Box<dyn PickedWindow>,
    starting: Starting,
}

impl PickedWindow for MeddledWindow {
    fn display(&self) -> bool {
        self.window.display()
    }

    fn title(&self) -> String {
        self.window.title()
    }

    fn app(&self) -> String {
        self.window.app()
    }

    fn size(&self) -> (u32, u32) {
        self.window.size()
    }

    fn events(&mut self) -> Box<dyn SourceEvents> {
        self.window.events()
    }

    fn prepare(&mut self, takes: &Takes) -> Option<Format> {
        self.window.prepare(takes)
    }

    fn start(self: Box<Self>, handle: Arc<SourceHandle>, failure: SharingFailure) {
        let Self { window, starting } = *self;
        starting(window, handle, failure);
    }
}

impl Rig {
    /// Starts both devices, with the platforms as `setup` leaves them.
    async fn with(setup: impl FnOnce(&Arc<FakeSharer>, &Arc<Fake>)) -> Self {
        Self::meddling(setup, None).await
    }

    /// The same, with `starting` between the app and the windows that the platform picks, if there
    /// is one.
    async fn meddling(
        setup: impl FnOnce(&Arc<FakeSharer>, &Arc<Fake>),
        starting: Option<Starting>,
    ) -> Self {
        Self::built(setup, starting, Timing::default()).await
    }

    /// The same, with the threads that serve the windows keeping time as `timing` says.
    async fn built(
        setup: impl FnOnce(&Arc<FakeSharer>, &Arc<Fake>),
        starting: Option<Starting>,
        timing: Timing,
    ) -> Self {
        let (platform, viewer) = (FakeSharer::new(), Fake::new());
        setup(&platform, &viewer);
        let mut sharing_registry = Registry::new();
        sharing_registry.input_timing = timing;
        let (registry, viewing_registry) = (Arc::new(sharing_registry), Arc::new(Registry::new()));
        platform.register_in(&registry).unwrap();
        viewer.register_in(&viewing_registry).unwrap();
        let windows = match starting {
            None => registry.windows(),
            Some(starting) => Windows::none().sharing(Arc::new(Meddling {
                sharer: registry.sharer().unwrap(),
                starting,
            })),
        };
        let sharing = Dev::start("Studio Mac", windows).await;
        let viewing = Dev::start("Kitchen iPad", viewing_registry.windows()).await;
        pair(&sharing, &viewing).await;
        // The person at the Mac lets the iPad hear, as the tests of sound assume. The one about
        // that setting takes it back.
        sharing
            .app
            .set_hear_setting(&viewing.id(), Some(true))
            .unwrap();
        let (sharing_events, viewing_events) = (sharing.app.events(), viewing.app.events());
        Self {
            sharing,
            viewing,
            platform,
            viewer,
            registry,
            viewing_registry,
            sharing_events,
            viewing_events,
            picks: 0,
            windows: 0,
        }
    }

    async fn new() -> Self {
        Self::with(|_, _| {}).await
    }

    /// The person at the sharing device wants to share a window with the other device.
    fn begin_share(&self, control: bool) -> JoinHandle<baton_app::Result<WindowShared>> {
        let app = self.sharing.app.clone();
        let to = self.viewing.id();
        tokio::spawn(async move { app.share_window(&to, control).await })
    }

    /// Waits until the platform is asked to show its picker, and gives the request.
    async fn next_pick(&mut self) -> u64 {
        self.picks += 1;
        let wanted = self.picks;
        let asked = |call: &SharerCall| matches!(call, SharerCall::Pick { .. });
        until("the platform to be asked to pick a window", || {
            self.platform.count(asked) >= wanted
        })
        .await;
        self.platform
            .log()
            .into_iter()
            .filter_map(|(_, call)| match call {
                SharerCall::Pick { request } => Some(request),
                _ => None,
            })
            .nth(wanted - 1)
            .unwrap()
    }

    /// The person picks the window "Report.pdf" of "Preview", which is 1440 by 900.
    async fn pick_report(&mut self) {
        let request = self.next_pick().await;
        self.windows += 1;
        self.platform
            .choose(
                &self.registry,
                request,
                b"Report.pdf",
                b"Preview",
                (1440, 900),
            )
            .unwrap();
    }

    /// The person picks a whole display, which is 3024 by 1964 and has no title.
    async fn pick_display(&mut self) {
        let request = self.next_pick().await;
        self.windows += 1;
        self.platform
            .choose_display(&self.registry, request, b"", (3024, 1964))
            .unwrap();
    }

    /// Offers the window and has the person at the other device answer.
    async fn offer(
        &mut self,
        control: bool,
        accept: bool,
    ) -> (baton_app::Result<WindowShared>, WindowRequest) {
        self.offer_of(false, control, accept).await
    }

    /// As `offer`, for a whole display if `display` and for a window if not.
    async fn offer_of(
        &mut self,
        display: bool,
        control: bool,
        accept: bool,
    ) -> (baton_app::Result<WindowShared>, WindowRequest) {
        let share = self.begin_share(control);
        if display {
            self.pick_display().await;
        } else {
            self.pick_report().await;
        }
        let request = next_matching(&mut self.viewing_events, |event| match event {
            Event::WindowRequested(request) => Some(request),
            _ => None,
        })
        .await;
        assert!(self.viewing.app.answer_window(request.id, accept));
        (share.await.unwrap(), request)
    }

    /// Shares the window, and gives both sessions once the platform has been started.
    async fn share(&mut self, control: bool) -> Shown {
        self.share_of(false, control).await
    }

    /// As `share`, for a whole display if `display` and for a window if not.
    async fn share_of(&mut self, display: bool, control: bool) -> Shown {
        let (shared, request) = self.offer_of(display, control, true).await;
        let Ok(WindowShared::Started(sharing)) = shared else {
            panic!("the window was not shared: {shared:?}");
        };
        let viewing = next_matching(&mut self.viewing_events, |event| match event {
            Event::WindowStarted(session) => Some(session),
            _ => None,
        })
        .await;
        assert_eq!(viewing.role, WindowRole::Viewing);
        assert_eq!(sharing.role, WindowRole::Sharing);
        let window = self.windows;
        until("the platform to be started", || {
            self.platform
                .calls(window)
                .iter()
                .any(|call| matches!(call, SharerCall::Start { .. }))
        })
        .await;
        let link = self.platform.link(window);
        Shown {
            viewing,
            sharing,
            request,
            window,
            link,
        }
    }

    /// Asks `begin` until a picture is wanted, and gives the kind that it asks for. The picture
    /// is announced then, and the platform answers with `sent` or `cancel`.
    async fn wanted(&self, link: u64) -> u32 {
        let deadline = Instant::now() + WAIT;
        loop {
            let kind = begin(&self.registry, link).unwrap();
            if kind != BEGIN_NOT_NOW {
                return kind;
            }
            assert!(
                Instant::now() < deadline,
                "the session never let a picture be made"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Makes the kind of picture that `begin` asks for and hands it over. Gives the kind, what
    /// became of the picture and the number of its frame.
    async fn send(&self, link: u64, at: u64) -> (u32, u32, u64) {
        let kind = self.wanted(link).await;
        let (outcome, number) = sent(&self.registry, link, &pic(kind, at)).unwrap();
        (kind, outcome, number)
    }

    /// As `send`, with a key frame that has a sequence parameter set for a picture of `size`.
    async fn send_sized(&self, link: u64, at: u64, size: (u32, u32)) -> (u32, u32, u64) {
        let kind = self.wanted(link).await;
        let (outcome, number) = sent(&self.registry, link, &pic_sized(kind, at, size)).unwrap();
        (kind, outcome, number)
    }

    /// The calls of the viewing platform about a screen that are not a question about how the
    /// decoder is doing, which the session asks as it likes.
    fn screen_calls(&self, screen: u64) -> Vec<Call> {
        self.viewer
            .calls(screen)
            .into_iter()
            .filter(|call| *call != Call::State)
            .collect()
    }

    fn decoded(&self, screen: u64) -> Vec<Decoded> {
        self.screen_calls(screen)
            .into_iter()
            .filter_map(|call| match call {
                Call::Decode(decoded) => Some(decoded),
                _ => None,
            })
            .collect()
    }

    /// What the viewing platform was told the pictures are: the codec, the width, the height and
    /// the rate, and the epoch, for each time.
    fn configs(&self, screen: u64) -> Vec<(u32, u32, u32, u32, u32)> {
        self.screen_calls(screen)
            .into_iter()
            .filter_map(|call| match call {
                Call::Source(message) if message.kind == SOURCE_CONFIG => Some((
                    message.codec,
                    message.width,
                    message.height,
                    message.fps,
                    message.epoch,
                )),
                _ => None,
            })
            .collect()
    }

    /// The calls of the viewing platform about the sound of a screen, in order.
    fn sound_calls(&self, screen: u64) -> Vec<Call> {
        self.screen_calls(screen)
            .into_iter()
            .filter(|call| matches!(call, Call::Sound(_) | Call::Play(_) | Call::Stopped(_)))
            .collect()
    }

    /// The link that `open` gave the viewing platform for a screen.
    fn screen_link(&self, screen: u64) -> u64 {
        self.viewer
            .log()
            .into_iter()
            .filter_map(|(_, call)| match call {
                Call::Open { link, .. } => Some(link),
                _ => None,
            })
            .nth(usize::try_from(screen).unwrap() - 1)
            .expect("the screen was not opened")
    }

    /// The packets of sound that the viewing platform was given to play, in order.
    fn played(&self, screen: u64) -> Vec<Played> {
        self.screen_calls(screen)
            .into_iter()
            .filter_map(|call| match call {
                Call::Play(played) => Some(played),
                _ => None,
            })
            .collect()
    }

    async fn sharing_end(&mut self) -> WindowEnd {
        window_ended(&mut self.sharing_events).await
    }

    async fn viewing_end(&mut self) -> WindowEnd {
        window_ended(&mut self.viewing_events).await
    }

    /// Waits until every window and screen has been let go, and checks that the platforms were
    /// never promised anything that was not kept: each window is released once, last, and the
    /// session of a window that ran was ended once, just before.
    async fn nothing_is_left(&self) {
        let deadline = Instant::now() + LET_GO;
        loop {
            // The windows and the links of the device that shares, the picks that wait, and the
            // screens and the links of the device that shows.
            let left = (
                self.platform.live_windows(),
                self.registry.open_links(),
                self.registry.waiting_picks(),
                self.viewer.live_screens(),
                self.viewing_registry.open_links(),
            );
            if left == (0, 0, 0, 0, 0) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for everything to be let go: {left:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        for window in 1..=self.platform.windows_made() as u64 {
            let calls = self.platform.calls(window);
            // A window that the library refused is the platform's still, and is never called.
            if self.platform.is_kept(window) {
                assert_eq!(calls, [], "window {window}");
                continue;
            }
            let count = |wanted: &SharerCall| calls.iter().filter(|call| *call == wanted).count();
            assert_eq!(count(&SharerCall::Release), 1, "window {window}: {calls:?}");
            assert_eq!(calls.last(), Some(&SharerCall::Release), "window {window}");
            assert!(count(&SharerCall::Ended) <= 1, "window {window}: {calls:?}");
            if count(&SharerCall::Ended) == 1 {
                assert_eq!(
                    calls[calls.len() - 2],
                    SharerCall::Ended,
                    "window {window}: nothing but the release follows the end: {calls:?}"
                );
            }
        }
        let (platform, viewer) = (self.platform.complaints(), self.viewer.complaints());
        assert!(platform.is_empty(), "{platform:?}");
        assert!(viewer.is_empty(), "{viewer:?}");
    }
}

/// What the encoder made, for the kind of picture that `begin` asked for: its key frame with the
/// parameter sets for a key frame, and its delta frame for the others.
fn pic(kind: u32, at: u64) -> Pic {
    let pictures = pictures();
    if kind == BEGIN_KEY {
        let mut sets = units_of(&pictures[0], SPS);
        sets.extend(units_of(&pictures[0], PPS));
        Pic {
            flags: FRAME_REFERENCED,
            capture_time: 16_000 * at,
            length_size: 4,
            sample: avcc(&pictures[0]),
            sets,
        }
    } else {
        Pic {
            flags: 0,
            capture_time: 16_000 * at,
            length_size: 4,
            sample: avcc(&pictures[1]),
            sets: Vec::new(),
        }
    }
}

/// The sequence parameter set (a unit, without a start code) of a picture of this size: whole
/// blocks of 16 pixels, with the rest cropped off, as an encoder writes it. Both sides are even.
fn sps_of(width: u32, height: u32) -> Vec<u8> {
    let (across, down) = (width.div_ceil(16), height.div_ceil(16));
    sps(
        (across, down),
        ((across * 16 - width) / 2, (down * 16 - height) / 2),
    )
}

/// A key frame of the encoder with these sequence parameter sets in place of its own, and its
/// picture parameter set after them.
fn key_with(sequence: Vec<Vec<u8>>, at: u64) -> Pic {
    let key = pic(BEGIN_KEY, at);
    let mut sets = sequence;
    sets.extend(units_of(&pictures()[0], PPS));
    Pic { sets, ..key }
}

/// What `pic` makes, with the sequence parameter set of a picture of `size` in a key frame: the
/// slices are the encoder's, and the library does not look at them.
fn pic_sized(kind: u32, at: u64, (width, height): (u32, u32)) -> Pic {
    if kind == BEGIN_KEY {
        key_with(vec![sps_of(width, height)], at)
    } else {
        pic(kind, at)
    }
}

// The tests.

#[tokio::test]
async fn a_window_is_offered_as_the_platform_described_it_and_prepared_within_what_the_other_device_takes()
 {
    let mut rig = Rig::with(|platform, _| platform.set_control(true)).await;
    let shown = rig.share(true).await;

    // The person at the other device was asked about the window that the platform described.
    assert_eq!(shown.request.title.as_deref(), Some("Report.pdf"));
    assert_eq!(shown.request.app.as_deref(), Some("Preview"));
    assert_eq!((shown.request.width, shown.request.height), (1440, 900));
    assert!(shown.request.control);
    assert!(!shown.request.display, "a window was offered as a display");
    assert!(!shown.sharing.display);
    assert_eq!(shown.sharing.title.as_deref(), Some("Report.pdf"));

    // The platform was asked for the picture within what the other device said it takes (the
    // limits of its own platform), and then started with the number of the session.
    let takes = Fake::default_limits();
    assert_ne!(shown.link, 0);
    assert_eq!(
        rig.platform.calls(shown.window),
        [
            SharerCall::Prepare {
                takes: (
                    takes.max_width,
                    takes.max_height,
                    takes.max_fps,
                    takes.codecs
                )
            },
            SharerCall::Start { link: shown.link },
        ]
    );
    // The other device was told which picture to expect: the one the platform chose, which is
    // that of the encoder, at no more than 30 frames a second.
    until("the viewing platform to be told the pictures", || {
        !rig.configs(1).is_empty()
    })
    .await;
    assert_eq!(
        rig.configs(1)[0],
        (CODEC_H264, WIDTH, HEIGHT, 30, rig.configs(1)[0].4)
    );
    assert_eq!(rig.registry.open_links(), 1);
    assert_eq!(rig.platform.live_windows(), 1);
    assert!(
        rig.platform.complaints().is_empty(),
        "{:?}",
        rig.platform.complaints()
    );
    assert!(
        rig.viewer.complaints().is_empty(),
        "{:?}",
        rig.viewer.complaints()
    );
}

#[tokio::test]
async fn a_whole_display_is_offered_and_shown_as_a_display_and_the_platform_is_started_for_it() {
    let mut rig = Rig::new().await;
    let share = rig.begin_share(false);
    let request = rig.next_pick().await;
    rig.windows += 1;
    rig.platform
        .choose_display(&rig.registry, request, b"", (3024, 1964))
        .unwrap();

    // The person at the other device was asked about a display, which has no title.
    let asked = next_matching(&mut rig.viewing_events, |event| match event {
        Event::WindowRequested(request) => Some(request),
        _ => None,
    })
    .await;
    assert!(asked.display);
    assert_eq!((asked.title, asked.app), (None, None));
    assert_eq!((asked.width, asked.height), (3024, 1964));
    assert!(!asked.control);
    assert!(rig.viewing.app.answer_window(asked.id, true));

    // Both ends have a display, and the platform captures it like it would a window.
    let Ok(WindowShared::Started(sharing)) = share.await.unwrap() else {
        panic!("the display was not shared");
    };
    assert!(sharing.display);
    let viewing = next_matching(&mut rig.viewing_events, |event| match event {
        Event::WindowStarted(session) => Some(session),
        _ => None,
    })
    .await;
    assert!(viewing.display);
    let window = rig.windows;
    until("the platform to be started", || {
        rig.platform
            .calls(window)
            .iter()
            .any(|call| matches!(call, SharerCall::Start { .. }))
    })
    .await;
    assert!(
        rig.platform.complaints().is_empty(),
        "{:?}",
        rig.platform.complaints()
    );
}

#[tokio::test]
async fn a_window_cannot_be_offered_with_control_by_a_platform_that_cannot_grant_it() {
    let mut rig = Rig::new().await;
    let to = rig.viewing.id();
    // A refusal is at once. An offer that goes on waits for the person to choose a window.
    let Ok(Err(refused)) = tokio::time::timeout(
        Duration::from_secs(5),
        rig.sharing.app.share_window(&to, true),
    )
    .await
    else {
        panic!("a window was offered with control that the platform cannot grant");
    };
    assert!(
        refused.to_string().contains("control"),
        "{refused}: the person is told what cannot be done"
    );
    // The platform was asked what it can do, and not to show its picker.
    assert!(rig.platform.count(|call| *call == SharerCall::Control) >= 1);
    assert_eq!(rig.platform.count(|call| *call != SharerCall::Control), 0);
    assert_eq!(rig.registry.waiting_picks(), 0);

    // Without control the same offer goes on to the picker.
    let share = rig.begin_share(false);
    rig.pick_report().await;
    share.abort();
    let _ = share.await;
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn pictures_go_from_the_functions_the_platform_calls_to_the_functions_the_other_platform_gets()
 {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    let link = shown.link;
    let pictures = pictures();

    // The first picture of a session is wanted at once, and is a key frame; the platform can
    // ask what is wanted without making anything.
    assert!(recovery_pending(&rig.registry, link).unwrap());
    let first = clock(&rig.registry, link).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let later = clock(&rig.registry, link).unwrap();
    assert!(
        later >= first + 20_000 && later < first + 60_000_000,
        "the clock counts microseconds: {first} then {later}"
    );

    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    assert!(!recovery_pending(&rig.registry, link).unwrap());
    assert_eq!(rig.send(link, 1).await, (BEGIN_DELTA, SENT, 1));
    assert_eq!(rig.send(link, 2).await, (BEGIN_DELTA, SENT, 2));
    until("the pictures to be shown", || {
        rig.screen_calls(1).contains(&Call::Show(2))
    })
    .await;

    // The key frame reaches the other platform with its parameter sets taken out of the sample,
    // as that platform takes it, and the others with none.
    let decoded = rig.decoded(1);
    assert_eq!(decoded.len(), 3);
    let (key, delta) = (&decoded[0], &decoded[1]);
    let mut sets = units_of(&pictures[0], SPS);
    sets.extend(units_of(&pictures[0], PPS));
    assert_eq!(key.sample, avcc(&pictures[0]));
    assert_eq!(key.sets, sets);
    assert_eq!(key.sequence_sets, 1);
    assert_eq!(key.flags, FRAME_KEY | FRAME_REFERENCED);
    assert_eq!((key.number, key.depends_on, key.capture_time), (0, 0, 0));
    assert_eq!(delta.sample, avcc(&pictures[1]));
    assert_eq!(delta.sets.len(), 0);
    assert_eq!(delta.flags, 0);
    assert_eq!(
        (delta.number, delta.depends_on, delta.capture_time),
        (1, 0, 16_000)
    );
    assert_eq!(decoded[2].capture_time, 32_000);
    assert!(
        rig.platform.complaints().is_empty(),
        "the platform: {:?}",
        rig.platform.complaints()
    );
    assert!(
        rig.viewer.complaints().is_empty(),
        "the viewer: {:?}",
        rig.viewer.complaints()
    );

    assert!(rig.viewing.app.end_window(shown.viewing.id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn parameter_sets_given_with_a_picture_that_is_not_a_key_frame_are_not_looked_at() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));

    // The platform may give them with every picture, and they may be anything then.
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    let mut delta = pic(BEGIN_DELTA, 1);
    delta.sets = vec![b"not a parameter set".to_vec(), Vec::new()];
    assert_eq!(sent(&rig.registry, link, &delta), Ok((SENT, 1)));
    until("the picture to be decoded", || rig.decoded(1).len() == 2).await;
    assert_eq!(rig.decoded(1)[1].sets.len(), 0);
    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn of_the_flags_of_a_picture_only_the_one_for_a_later_frame_to_refer_to_counts() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));

    // `begin` says what kind of frame it is, so a platform that sets the other flags does not
    // make a key frame of a delta, and a bit that this version does not know is nothing to it.
    // What the other device gets: the flags it was sent.
    let flags = [
        (FRAME_REFERENCED, FRAME_REFERENCED),
        (FRAME_KEY, 0),
        (FRAME_RECOVERY, 0),
        (0, 0),
        (u32::MAX, FRAME_REFERENCED),
        (!FRAME_REFERENCED, 0),
    ];
    for (number, (given, _)) in (1..).zip(flags) {
        assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
        let delta = Pic {
            flags: given,
            ..pic(BEGIN_DELTA, number)
        };
        assert_eq!(sent(&rig.registry, link, &delta), Ok((SENT, number)));
    }
    until("the pictures to be decoded", || {
        rig.decoded(1).len() == flags.len() + 1
    })
    .await;
    let got: Vec<u32> = rig.decoded(1).iter().map(|decoded| decoded.flags).collect();
    let mut expected = vec![FRAME_KEY | FRAME_REFERENCED];
    expected.extend(flags.iter().map(|(_, told)| *told));
    assert_eq!(got, expected);

    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_picture_that_is_given_up_on_is_asked_for_again() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;

    // The first picture that is wanted is a key frame, and it stays wanted until one is sent.
    for _ in 0..3 {
        assert_eq!(rig.wanted(link).await, BEGIN_KEY);
        assert_eq!(cancel(&rig.registry, link), Ok(()));
        // The frame was given up on, so there is nothing to hand over for it.
        assert_eq!(
            sent(&rig.registry, link, &pic(BEGIN_KEY, 0)),
            Err(Error::Argument)
        );
    }
    // A cancel with nothing announced does no harm.
    assert_eq!(cancel(&rig.registry, link), Ok(()));
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));

    // Nothing was announced, so there is nothing to hand over.
    assert_eq!(
        sent(&rig.registry, link, &pic(BEGIN_DELTA, 1)),
        Err(Error::Argument)
    );
    // And a picture that is handed over twice goes out once.
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    assert_eq!(
        sent(&rig.registry, link, &pic(BEGIN_DELTA, 1)),
        Ok((SENT, 1))
    );
    assert_eq!(
        sent(&rig.registry, link, &pic(BEGIN_DELTA, 1)),
        Err(Error::Argument)
    );
    assert!(rig.sharing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn pausing_stops_the_pictures_and_tells_the_other_device_why() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));

    assert_eq!(pause(&rig.registry, link, PAUSE_LOCKED), Ok(()));
    assert_eq!(begin(&rig.registry, link), Ok(BEGIN_NOT_NOW));
    assert_eq!(resume(&rig.registry, link), Ok(()));
    assert_eq!(rig.send(link, 1).await.0, BEGIN_DELTA);
    assert_eq!(pause(&rig.registry, link, PAUSE_HIDDEN), Ok(()));
    assert_eq!(begin(&rig.registry, link), Ok(BEGIN_NOT_NOW));
    assert_eq!(resume(&rig.registry, link), Ok(()));
    assert_eq!(pause(&rig.registry, link, PAUSE_OTHER), Ok(()));
    assert_eq!(begin(&rig.registry, link), Ok(BEGIN_NOT_NOW));
    assert_eq!(resume(&rig.registry, link), Ok(()));

    // A reason that is not known is refused, and the pictures go on as before.
    assert_eq!(pause(&rig.registry, link, 99), Err(Error::Argument));
    rig.wanted(link).await;
    assert_eq!(cancel(&rig.registry, link), Ok(()));

    let heard = |rig: &Rig| -> Vec<(u32, u32)> {
        rig.screen_calls(1)
            .into_iter()
            .filter_map(|call| match call {
                Call::Source(message)
                    if message.kind == SOURCE_PAUSE || message.kind == SOURCE_RESUME =>
                {
                    Some((message.kind, message.reason))
                }
                _ => None,
            })
            .collect()
    };
    until("the other platform to be told", || heard(&rig).len() == 6).await;
    assert_eq!(
        heard(&rig),
        [
            (SOURCE_PAUSE, PAUSE_LOCKED),
            (SOURCE_RESUME, 0),
            (SOURCE_PAUSE, PAUSE_HIDDEN),
            (SOURCE_RESUME, 0),
            (SOURCE_PAUSE, PAUSE_OTHER),
            (SOURCE_RESUME, 0),
        ]
    );
    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_new_size_starts_a_new_epoch_and_a_picture_of_the_old_size_is_stale() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));

    // A picture is being made when the window is resized: it is of the old size.
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    assert_eq!(
        new_epoch(&rig.registry, link, (CODEC_H264, 32, 24, 15)),
        Ok(())
    );
    assert_eq!(
        sent(&rig.registry, link, &pic(BEGIN_DELTA, 1)),
        Ok((SENT_STALE, 0))
    );
    // The next picture is a key frame of the new epoch.
    assert!(recovery_pending(&rig.registry, link).unwrap());
    assert_eq!(
        rig.send_sized(link, 2, (32, 24)).await,
        (BEGIN_KEY, SENT, 1)
    );

    until("the other platform to be told the new size", || {
        rig.configs(1).len() == 2
    })
    .await;
    let configs = rig.configs(1);
    assert_eq!(configs[0], (CODEC_H264, WIDTH, HEIGHT, 30, configs[0].4));
    assert_eq!(configs[1], (CODEC_H264, 32, 24, 15, configs[0].4 + 1));

    // What is beyond the protocol, or beyond what the other device takes, is refused and
    // changes nothing: the picture that is being made can still be sent.
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    let takes = Fake::default_limits();
    let wrong = [
        ("no codec", (0, 32, 24, 15)),
        ("two codecs", (3, 32, 24, 15)),
        ("a codec that is not known", (2, 32, 24, 15)),
        ("no width", (CODEC_H264, 0, 24, 15)),
        (
            "a width beyond the largest picture",
            (CODEC_H264, 3841, 24, 15),
        ),
        ("no height", (CODEC_H264, 32, 0, 15)),
        (
            "a height beyond the largest picture",
            (CODEC_H264, 32, 3841, 15),
        ),
        ("no rate", (CODEC_H264, 32, 24, 0)),
        ("a rate beyond the highest", (CODEC_H264, 32, 24, 121)),
        ("a rate beyond 16 bits", (CODEC_H264, 32, 24, 65_536 + 15)),
        (
            "a width beyond what the other device takes",
            (CODEC_H264, takes.max_width + 1, 24, 15),
        ),
        (
            "a height beyond what the other device takes",
            (CODEC_H264, 32, takes.max_height + 1, 15),
        ),
        (
            "a rate beyond what the other device takes",
            (CODEC_H264, 32, 24, takes.max_fps + 1),
        ),
    ];
    for (what, format) in wrong {
        assert_eq!(
            new_epoch(&rig.registry, link, format),
            Err(Error::Argument),
            "{what}"
        );
    }
    assert_eq!(
        sent(&rig.registry, link, &pic(BEGIN_DELTA, 3)),
        Ok((SENT, 2))
    );
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.configs(1).len(), 2, "a size that was refused was told");
    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;
}

/// A unit of the kinds that an encoder makes, behind a length of four bytes.
fn behind(unit: &[u8]) -> Vec<u8> {
    let mut out = u32::try_from(unit.len()).unwrap().to_be_bytes().to_vec();
    out.extend_from_slice(unit);
    out
}

/// Pictures with a sample that cannot go on the wire, and what is wrong with each.
fn wrong_samples(key: &Pic) -> Vec<(&'static str, Pic)> {
    let many_units: Vec<u8> = [behind(&[0x65, 0x88])]
        .into_iter()
        .chain((0..256).map(|_| behind(&[0x41, 0x9a])))
        .flatten()
        .collect();
    vec![
        (
            "a length that reaches beyond the end",
            Pic {
                sample: vec![0, 0, 0, 200, 0x65, 1, 2],
                ..key.clone()
            },
        ),
        (
            "a sample with nothing in it",
            Pic {
                sample: Vec::new(),
                ..key.clone()
            },
        ),
        (
            "a sample of nothing but a delimiter",
            Pic {
                sample: behind(&[0x09, 0xf0]),
                ..key.clone()
            },
        ),
        (
            "a unit with nothing in it",
            Pic {
                sample: vec![0, 0, 0, 0],
                ..key.clone()
            },
        ),
        (
            "a unit with its forbidden bit set",
            Pic {
                sample: behind(&[0xe5, 1]),
                ..key.clone()
            },
        ),
        (
            "a unit that could be taken for two",
            Pic {
                sample: behind(&[0x65, 0, 0, 1, 7]),
                ..key.clone()
            },
        ),
        (
            "more units than a picture may have",
            Pic {
                sample: many_units,
                ..key.clone()
            },
        ),
    ]
}

/// Pictures with a length or parameter sets that cannot go on the wire, and what is wrong with
/// each.
fn wrong_sets(key: &Pic, delta: &Pic) -> Vec<(&'static str, Pic)> {
    let (sequence, picture_set) = (key.sets[0].clone(), key.sets[1].clone());
    vec![
        (
            "a length of three bytes",
            Pic {
                length_size: 3,
                ..key.clone()
            },
        ),
        (
            "a length of no bytes",
            Pic {
                length_size: 0,
                ..key.clone()
            },
        ),
        (
            "a length of eight bytes",
            Pic {
                length_size: 8,
                ..key.clone()
            },
        ),
        (
            "a key frame with no parameter sets",
            Pic {
                sets: Vec::new(),
                ..key.clone()
            },
        ),
        (
            "a key frame with a sequence set and no picture set",
            Pic {
                sets: vec![sequence.clone()],
                ..key.clone()
            },
        ),
        (
            "a key frame with a picture set and no sequence set",
            Pic {
                sets: vec![picture_set.clone()],
                ..key.clone()
            },
        ),
        (
            "a parameter set that is not one",
            Pic {
                sets: vec![sequence.clone(), picture_set.clone(), vec![0x65, 1, 2, 3]],
                ..key.clone()
            },
        ),
        (
            "a key frame that holds no slice of an IDR picture",
            Pic {
                sample: delta.sample.clone(),
                ..key.clone()
            },
        ),
        (
            "more parameter sets than the header allows",
            Pic {
                sets: vec![sequence.clone(); 257],
                ..key.clone()
            },
        ),
        (
            "a parameter set that is longer than the header allows",
            Pic {
                sets: vec![sequence.clone(), picture_set.clone(), vec![0x67; 4097]],
                ..key.clone()
            },
        ),
    ]
}

/// Pictures that cannot go on the wire, and what is wrong with each.
fn pictures_that_cannot_go_on_the_wire() -> Vec<(&'static str, Pic)> {
    let (key, delta) = (pic(BEGIN_KEY, 0), pic(BEGIN_DELTA, 0));
    let mut wrong = wrong_samples(&key);
    wrong.extend(wrong_sets(&key, &delta));
    wrong
}

#[tokio::test]
async fn a_picture_that_cannot_go_on_the_wire_is_refused_and_the_frame_is_asked_for_again() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    let wrong = pictures_that_cannot_go_on_the_wire();
    for (what, bad) in wrong {
        assert_eq!(rig.wanted(link).await, BEGIN_KEY, "{what}");
        assert_eq!(
            sent(&rig.registry, link, &bad),
            Err(Error::Argument),
            "{what}"
        );
    }
    // Nothing was sent: the frame that goes out first is the first of the session.
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    until("the picture to be decoded", || rig.decoded(1).len() == 1).await;
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        rig.decoded(1).len(),
        1,
        "a picture that was refused was sent"
    );

    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_key_frame_that_describes_a_larger_picture_than_the_one_announced_is_refused() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    // The picture that was announced is the encoder's: 64 by 48 pixels. The other device refuses
    // a key frame that describes more than it takes, and nothing here would say why.
    let too_large = [
        ("wider", vec![sps_of(80, 48)]),
        ("higher", vec![sps_of(64, 64)]),
        ("a window at its own size", vec![sps_of(2880, 1800)]),
        // A set can crop a large picture down to the right size, and a decoder allocates for the
        // large one.
        (
            "a large picture cropped to the size",
            vec![sps((8, 6), (32, 24))],
        ),
        (
            "a set that fits and one that does not",
            vec![sps_of(64, 48), sps_of(80, 48)],
        ),
        (
            "a set that does not fit and one that does",
            vec![sps_of(80, 48), sps_of(64, 48)],
        ),
    ];
    for (what, sequence) in too_large {
        // Each refusal gives the frame up, and no key frame went out yet: this is asked for again.
        assert_eq!(rig.wanted(link).await, BEGIN_KEY, "{what}");
        assert_eq!(
            sent(&rig.registry, link, &key_with(sequence, 0)),
            Err(Error::Argument),
            "{what}"
        );
    }

    // Nothing went out, and a key frame of the size that was announced is the first of the session.
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);
    assert_eq!(
        sent(&rig.registry, link, &key_with(vec![sps_of(64, 48)], 0)),
        Ok((SENT, 0))
    );
    // The others are the first key frames of epochs of the same size.
    let fits = [
        ("a smaller picture", vec![sps_of(64, 40)]),
        ("two sets that fit", vec![sps_of(64, 48), sps_of(32, 32)]),
    ];
    for (number, (what, sequence)) in (1..).zip(fits) {
        assert_eq!(
            new_epoch(&rig.registry, link, (CODEC_H264, 64, 48, 30)),
            Ok(())
        );
        assert_eq!(rig.wanted(link).await, BEGIN_KEY, "{what}");
        assert_eq!(
            sent(&rig.registry, link, &key_with(sequence, number)),
            Ok((SENT, number)),
            "{what}"
        );
    }

    // The sets of a frame that is not a key frame are not sent, and are not looked at.
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    let delta = Pic {
        sets: key_with(vec![sps_of(80, 48)], 0).sets,
        ..pic(BEGIN_DELTA, 3)
    };
    assert_eq!(sent(&rig.registry, link, &delta), Ok((SENT, 3)));

    // What decides is the picture and not the kind that was announced: one that holds an IDR
    // slice goes out with its sets, and the other device refuses them when they describe more
    // than it takes, so a frame announced as a delta is held to the size too.
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    assert_eq!(
        sent(&rig.registry, link, &key_with(vec![sps_of(80, 48)], 4)),
        Err(Error::Argument)
    );
    assert!(rig.sharing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_new_epoch_changes_the_size_that_a_key_frame_is_held_to() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    assert_eq!(
        rig.send_sized(link, 0, (64, 48)).await,
        (BEGIN_KEY, SENT, 0)
    );

    // Smaller: a key frame of the old size describes more than the other device was told.
    assert_eq!(
        new_epoch(&rig.registry, link, (CODEC_H264, 32, 24, 15)),
        Ok(())
    );
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);
    assert_eq!(
        sent(&rig.registry, link, &pic_sized(BEGIN_KEY, 1, (64, 48))),
        Err(Error::Argument)
    );
    assert_eq!(
        rig.send_sized(link, 1, (32, 24)).await,
        (BEGIN_KEY, SENT, 1)
    );

    // Larger: one that was too large before is not now, and the new size is the limit.
    assert_eq!(
        new_epoch(&rig.registry, link, (CODEC_H264, 128, 96, 15)),
        Ok(())
    );
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);
    assert_eq!(
        sent(&rig.registry, link, &pic_sized(BEGIN_KEY, 2, (160, 96))),
        Err(Error::Argument)
    );

    // A format that is refused changes nothing, the size that pictures are held to included. The
    // library refuses what the protocol does not allow, and the session refuses what the protocol
    // allows and the other device does not take.
    let takes = Fake::default_limits();
    for (what, format) in [
        ("an edge beyond the protocol", (CODEC_H264, 3841, 24, 15)),
        ("no rate", (CODEC_H264, 16, 12, 0)),
        ("a codec that is not known", (2, 16, 12, 15)),
        (
            "a width beyond what the other device takes",
            (CODEC_H264, takes.max_width + 1, 24, 15),
        ),
        (
            "a height beyond what the other device takes",
            (CODEC_H264, 16, takes.max_height + 1, 15),
        ),
        (
            "a rate beyond what the other device takes",
            (CODEC_H264, 16, 12, takes.max_fps + 1),
        ),
    ] {
        assert_eq!(
            new_epoch(&rig.registry, link, format),
            Err(Error::Argument),
            "{what}"
        );
    }
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);
    assert_eq!(
        sent(&rig.registry, link, &pic_sized(BEGIN_KEY, 2, (160, 96))),
        Err(Error::Argument)
    );
    assert_eq!(
        rig.send_sized(link, 2, (128, 96)).await,
        (BEGIN_KEY, SENT, 2)
    );
    assert!(rig.sharing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_format_that_is_not_whole_is_refused_and_starts_no_epoch() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    until("the other platform to be told the pictures", || {
        rig.configs(1).len() == 1
    })
    .await;

    let whole = size_of_u32::<BatonFormat>();
    let format = |size: u32| BatonFormat {
        size,
        codec: CODEC_H264,
        width: 32,
        height: 24,
        fps: 15,
        reserved: 0,
    };
    for (what, bad) in [
        ("no format", None),
        ("a struct with no size", Some(format(0))),
        ("a struct that is too short", Some(format(whole - 1))),
    ] {
        // SAFETY: the struct is whole, and lives through the call.
        let result = unsafe { new_epoch_raw(&rig.registry, link, bad.as_ref()) };
        assert_eq!(result, Err(Error::Argument), "{what}");
    }
    // No epoch began: the other device heard nothing, and the next picture is a delta.
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        rig.configs(1).len(),
        1,
        "a format that was refused was told"
    );
    assert_eq!(rig.send(link, 1).await, (BEGIN_DELTA, SENT, 1));

    // A struct that says it is longer than this version knows is read as far as this version
    // knows it, and no further.
    // SAFETY: the struct is whole, and lives through the call.
    let result = unsafe { new_epoch_raw(&rig.registry, link, Some(&format(whole + 8))) };
    assert_eq!(result, Ok(()));
    until("the other platform to be told the new size", || {
        rig.configs(1).len() == 2
    })
    .await;
    assert!(rig.viewing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn pictures_at_the_edge_of_what_the_protocol_allows_are_handled_with_care() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    let key = pic(BEGIN_KEY, 0);

    // A length that is more than a picture may be is not read at all: the pointer leads nowhere.
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);
    let nowhere = BatonPicture {
        size: size_of_u32::<BatonPicture>(),
        flags: 0,
        capture_time: 0,
        length_size: 4,
        reserved: 0,
        sample: BatonBytes {
            ptr: 8 as *const u8,
            len: MAX_PICTURE_BYTES + 1,
        },
        sets: ptr::null(),
        set_count: 0,
        geometry: BatonGeometry::default(),
    };
    let (mut outcome, mut number) = (u32::MAX, u64::MAX);
    // SAFETY: the sample is described but is not read, as the header says of a length this large.
    let result = unsafe {
        fake_sharer::sent_raw(
            &rig.registry,
            link,
            Some(&nowhere),
            Some(&mut outcome),
            Some(&mut number),
        )
    };
    assert_eq!(result, Ok(()));
    assert_eq!((outcome, number), (SENT_TOO_LARGE, 0));
    // The frame was given up on, and the next picture that is asked for is a key frame again.
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);

    // A sample of the largest size that is read is a picture that is too large once it has the
    // start codes and the parameter sets in front of it on the wire.
    let mut unit = vec![0xaa_u8; MAX_PICTURE_BYTES - 4];
    unit[0] = 0x65;
    let largest = Pic {
        sample: behind(&unit),
        ..key.clone()
    };
    assert_eq!(largest.sample.len(), MAX_PICTURE_BYTES);
    assert_eq!(sent(&rig.registry, link, &largest), Ok((SENT_TOO_LARGE, 0)));
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);

    // And the session goes on: nothing was sent, and the next picture is the first of it.
    assert_eq!(sent(&rig.registry, link, &key), Ok((SENT, 0)));

    // A delta of the largest size that is read has no parameter sets in front of it, and goes out
    // as the largest picture there is.
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    let mut unit = vec![0xaa_u8; MAX_PICTURE_BYTES - 4];
    unit[0] = 0x41;
    let delta = Pic {
        sample: behind(&unit),
        ..pic(BEGIN_DELTA, 1)
    };
    assert_eq!(delta.sample.len(), MAX_PICTURE_BYTES);
    assert_eq!(sent(&rig.registry, link, &delta), Ok((SENT, 1)));
    assert!(rig.sharing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn pointers_that_are_not_there_are_refused_and_nothing_is_written_through_them() {
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;

    // A pointer for an answer that is missing is refused; the others are not written.
    assert_eq!(
        fake_sharer::clock_to(&rig.registry, link, false),
        Err(Error::Argument)
    );
    assert_eq!(
        fake_sharer::begin_to(&rig.registry, link, false),
        Err(Error::Argument)
    );
    assert_eq!(
        fake_sharer::recovery_pending_to(&rig.registry, link, false),
        Err(Error::Argument)
    );
    // That did not announce a picture: this one is the first that is asked for.
    assert_eq!(rig.wanted(link).await, BEGIN_KEY);
    assert_eq!(cancel(&rig.registry, link), Ok(()));

    let key = pic(BEGIN_KEY, 0);
    let sets: Vec<BatonBytes> = key.sets.iter().map(|set| BatonBytes::of(set)).collect();
    let sample = key.sample.clone();
    let picture = |size: u32, length_size: u32| BatonPicture {
        size,
        flags: FRAME_REFERENCED,
        capture_time: 0,
        length_size,
        reserved: 0,
        sample: BatonBytes::of(&sample),
        sets: sets.as_ptr(),
        set_count: sets.len(),
        geometry: BatonGeometry::default(),
    };
    let whole = size_of_u32::<BatonPicture>();
    // The struct as it was before input was carried ends where the geometry begins.
    let before_input = u32::try_from(offset_of!(BatonPicture, geometry)).unwrap();
    let (before_outcome, before_number) = (0xaaaa_aaaa_u32, 0xbbbb_bbbb_bbbb_bbbb_u64);
    let (mut outcome, mut number) = (before_outcome, before_number);
    let wrong: Vec<(&str, Option<BatonPicture>)> = vec![
        ("no picture", None),
        ("a struct with no size", Some(picture(0, 4))),
        (
            "a struct that is too short",
            Some(picture(before_input - 1, 4)),
        ),
        ("a length of three bytes", Some(picture(whole, 3))),
    ];
    for (what, bad) in wrong {
        // A frame is announced each time, and each refusal gives it up.
        assert_eq!(rig.wanted(link).await, BEGIN_KEY, "{what}");
        // SAFETY: the sample and the sets are there, and live through the call.
        let result = unsafe {
            fake_sharer::sent_raw(
                &rig.registry,
                link,
                bad.as_ref(),
                Some(&mut outcome),
                Some(&mut number),
            )
        };
        assert_eq!(result, Err(Error::Argument), "{what}");
        assert_eq!((outcome, number), (before_outcome, before_number), "{what}");
    }
    // A missing pointer for what became of the picture is refused too, before it is sent.
    for (what, outcome_there, number_there) in [
        ("no place for the outcome", false, true),
        ("no place for the number", true, false),
    ] {
        assert_eq!(rig.wanted(link).await, BEGIN_KEY, "{what}");
        let (mut outcome, mut number) = (before_outcome, before_number);
        let good = picture(whole, 4);
        // SAFETY: as above.
        let result = unsafe {
            fake_sharer::sent_raw(
                &rig.registry,
                link,
                Some(&good),
                outcome_there.then_some(&mut outcome),
                number_there.then_some(&mut number),
            )
        };
        assert_eq!(result, Err(Error::Argument), "{what}");
        assert_eq!((outcome, number), (before_outcome, before_number), "{what}");
    }
    // None of that sent anything.
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    assert!(rig.sharing.app.end_window(1));
    rig.nothing_is_left().await;
}

// Windows that do not start, and sessions that end.

/// What the platform is asked when it is to choose the picture for the device of the tests.
fn asked_to_prepare() -> SharerCall {
    let takes = Fake::default_limits();
    SharerCall::Prepare {
        takes: (
            takes.max_width,
            takes.max_height,
            takes.max_fps,
            takes.codecs,
        ),
    }
}

/// The call that asks the platform to prepare for a viewer that takes pictures of up to this size
/// and rate, in H.264.
fn asked_to_prepare_within(width: u32, height: u32, fps: u32) -> SharerCall {
    SharerCall::Prepare {
        takes: (width, height, fps, CODEC_H264),
    }
}

/// A start that fails with this text, which it claims to be this long.
fn start_fails(text: &[u8], claimed_len: u32) -> StartPlan {
    StartPlan::Fail {
        code: -1,
        text: text.to_vec(),
        claimed_len,
    }
}

impl Rig {
    /// The person at the showing device closes the window that was shared last. The viewer can
    /// tell the other device only once the first message of the session has reached it: until
    /// then it has no stream of its own, and the sharing device hears nothing until it has been
    /// silent for 30 s.
    async fn close_at_viewer(&self, shown: &Shown) {
        let screen = self.viewer.screens_made() as u64;
        until("the showing platform to be told the pictures", || {
            !self.configs(screen).is_empty()
        })
        .await;
        assert!(self.viewing.app.end_window(shown.viewing.id));
    }
}

/// Every function that the platform calls about a window, with this link, and what each answered.
/// The ones that change something are the ones that a platform that is let in would change it with.
fn every_call(registry: &Registry, link: u64) -> Vec<(&'static str, Result<(), Error>)> {
    vec![
        ("clock", clock(registry, link).map(drop)),
        ("changed", changed(registry, link, 0)),
        ("begin", begin(registry, link).map(drop)),
        ("cancel", cancel(registry, link)),
        (
            "recovery_pending",
            recovery_pending(registry, link).map(drop),
        ),
        ("sent", sent(registry, link, &pic(BEGIN_KEY, 0)).map(drop)),
        (
            "new_epoch",
            new_epoch(registry, link, (CODEC_H264, 32, 24, 15)),
        ),
        ("pause", pause(registry, link, PAUSE_HIDDEN)),
        ("resume", resume(registry, link)),
        (
            "sound_begin",
            sound_begin(registry, link, &aac(48_000, 2, 1024)),
        ),
        ("sound_silent", sound_silent(registry, link, SILENT_FAILED)),
        ("sound", sound(registry, link, 0, b"packet").map(drop)),
        ("end", end(registry, link, None)),
        (
            "end with a sentence",
            end(registry, link, Some(b"It went wrong.")),
        ),
    ]
}

#[tokio::test]
async fn a_window_that_the_other_person_says_no_to_is_released_and_never_started() {
    let mut rig = Rig::new().await;
    let (shared, request) = rig.offer(false, false).await;
    let Ok(WindowShared::Declined(sentence)) = shared else {
        panic!("the window was not declined: {shared:?}");
    };
    assert!(sentence.contains("Kitchen iPad"), "{sentence}");
    assert_eq!(request.title.as_deref(), Some("Report.pdf"));

    // The picture is chosen once the other device has said yes, so it was never asked for, and
    // the window was let go when it was not taken. It was never started, so the platform was not
    // told that it ended.
    rig.nothing_is_left().await;
    assert_eq!(rig.platform.calls(1), [SharerCall::Release]);
    assert_eq!(rig.viewer.screens_made(), 0);
}

#[tokio::test]
async fn a_picture_that_the_platform_cannot_make_within_what_the_other_device_takes_is_not_offered()
{
    let takes = Fake::default_limits();
    let plans = [
        ("a platform that cannot choose", PreparePlan::Fail(-1)),
        (
            "a platform that chooses and answers with a positive number",
            PreparePlan::Answers(1),
        ),
        ("no codec", PreparePlan::Format(0, 64, 48, 30)),
        (
            "a codec that is not known",
            PreparePlan::Format(2, 64, 48, 30),
        ),
        ("two codecs", PreparePlan::Format(3, 64, 48, 30)),
        ("no width", PreparePlan::Format(CODEC_H264, 0, 48, 30)),
        ("no height", PreparePlan::Format(CODEC_H264, 64, 0, 30)),
        ("no rate", PreparePlan::Format(CODEC_H264, 64, 48, 0)),
        (
            "a rate beyond the protocol",
            PreparePlan::Format(CODEC_H264, 64, 48, 121),
        ),
        (
            "a width beyond what the other device takes",
            PreparePlan::Format(CODEC_H264, takes.max_width + 1, 48, 30),
        ),
        (
            "a height beyond what the other device takes",
            PreparePlan::Format(CODEC_H264, 64, takes.max_height + 1, 30),
        ),
        (
            "a rate beyond what the other device takes",
            PreparePlan::Format(CODEC_H264, 64, 48, takes.max_fps + 1),
        ),
    ];
    for (what, plan) in plans {
        let mut rig = Rig::with(|platform, _| platform.plan_prepare(plan)).await;
        // The person at the other device says yes, and the platform is then asked for the picture.
        let (shared, _) = rig.offer(false, true).await;
        let Ok(WindowShared::Declined(sentence)) = shared else {
            panic!("{what}: the window was not declined: {shared:?}");
        };
        assert!(
            sentence.contains("Kitchen iPad") && sentence.contains("picture"),
            "{what}: {sentence}"
        );
        // Nothing was started, and the screen that was made for the window was let go.
        rig.nothing_is_left().await;
        assert_eq!(
            rig.platform.calls(1),
            [asked_to_prepare(), SharerCall::Release],
            "{what}"
        );
    }
}

#[tokio::test]
async fn a_capture_that_cannot_begin_ends_the_session_and_the_person_is_told_what_the_platform_said()
 {
    let said = "Screen recording is switched off for Baton. Switch it on in System Settings.";
    let cases = [
        ("a sentence", start_fails(said.as_bytes(), 76), said),
        (
            "a start that answers with a positive number",
            StartPlan::Fail {
                code: 1,
                text: said.as_bytes().to_vec(),
                claimed_len: 76,
            },
            said,
        ),
        (
            "a sentence that is claimed to be longer than there was room for",
            start_fails(said.as_bytes(), 100_000),
            said,
        ),
        (
            "a sentence with a zero in it, as a C string",
            start_fails(b"Screen recording is off.\0And then some.", 40),
            "Screen recording is off.",
        ),
        (
            "no sentence",
            start_fails(b"", 0),
            "This device could not start sharing the window.",
        ),
        (
            "a sentence of nothing but blanks",
            start_fails(b"  \n ", 4),
            "This device could not start sharing the window.",
        ),
    ];
    for (what, plan, told) in cases {
        let mut rig = Rig::with(|platform, _| platform.plan_start(plan)).await;
        let shown = rig.share(false).await;

        // The person at the sharing device is told what went wrong, and the other one that the
        // window has ended, which is all that it has to know.
        assert_eq!(
            rig.sharing_end().await,
            WindowEnd::Failed(told.to_owned()),
            "{what}"
        );
        assert_eq!(rig.viewing_end().await, WindowEnd::OtherDevice, "{what}");
        rig.nothing_is_left().await;
        // The capture never began, so the platform is not told that it ended.
        assert_eq!(
            rig.platform.calls(shown.window),
            [
                asked_to_prepare(),
                SharerCall::Start { link: shown.link },
                SharerCall::Release
            ],
            "{what}"
        );
        // The link the platform was given closed with the session.
        for (call, answer) in every_call(&rig.registry, shown.link) {
            assert_eq!(answer, Err(Error::UnknownLink), "{what}: {call}");
        }
    }
}

#[tokio::test]
async fn a_session_that_is_over_before_the_capture_starts_does_not_get_one() {
    // The session ends between the yes of the other device and the start, which a thread of the
    // platform or the other device can do: the window goes back to the platform as it came.
    let starting: Starting = Arc::new(|window, handle, failure| {
        handle.end();
        window.start(handle, failure);
    });
    let mut rig = Rig::meddling(|_, _| {}, Some(starting)).await;
    let (shared, _) = rig.offer(false, true).await;
    let Ok(WindowShared::Started(_)) = shared else {
        panic!("the window was not shared: {shared:?}");
    };

    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;
    // The capture never began, so the platform is not told that it ended either.
    assert_eq!(
        rig.platform.calls(1),
        [asked_to_prepare(), SharerCall::Release]
    );
}

#[tokio::test]
async fn a_window_that_is_started_where_nothing_can_wait_for_its_end_is_not_started() {
    // A thread of the platform's own, which is not in a runtime.
    let starting: Starting = Arc::new(|window, handle, failure| {
        thread::spawn(move || window.start(handle, failure))
            .join()
            .unwrap();
    });
    let mut rig = Rig::meddling(|_, _| {}, Some(starting)).await;
    let (shared, _) = rig.offer(false, true).await;
    let Ok(WindowShared::Started(_)) = shared else {
        panic!("the window was not shared: {shared:?}");
    };

    // The person is told, and the other device only that the window ended.
    assert_eq!(
        rig.sharing_end().await,
        WindowEnd::Failed("This device could not start sharing the window.".to_owned())
    );
    assert_eq!(rig.viewing_end().await, WindowEnd::OtherDevice);
    rig.nothing_is_left().await;
    assert_eq!(
        rig.platform.calls(1),
        [asked_to_prepare(), SharerCall::Release]
    );
}

#[tokio::test]
async fn a_session_that_ends_is_told_to_the_platform_once_and_its_link_closes() {
    // The person at the sharing device ends it.
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    assert!(rig.sharing.app.end_window(shown.sharing.id));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    assert_eq!(rig.viewing_end().await, WindowEnd::OtherDevice);
    rig.nothing_is_left().await;
    assert_eq!(
        rig.platform.calls(1),
        [
            asked_to_prepare(),
            SharerCall::Start { link: shown.link },
            SharerCall::Ended,
            SharerCall::Release
        ]
    );
    // Whatever the platform says about the window after that is answered with the news that
    // there is no such link, and changes nothing.
    for (call, answer) in every_call(&rig.registry, shown.link) {
        assert_eq!(answer, Err(Error::UnknownLink), "{call}");
    }
    assert_eq!(rig.platform.calls(1).len(), 4);

    // The person at the showing device ends it.
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    rig.close_at_viewer(&shown).await;
    assert_eq!(rig.viewing_end().await, WindowEnd::ThisDevice);
    assert_eq!(rig.sharing_end().await, WindowEnd::OtherDevice);
    rig.nothing_is_left().await;
    assert_eq!(rig.platform.calls(1).len(), 4);

    // The device is unpaired.
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    rig.sharing.app.unpair(&rig.viewing.id()).unwrap();
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    drop(shown);
    rig.nothing_is_left().await;
    assert_eq!(rig.platform.calls(1).len(), 4);
}

#[tokio::test]
async fn a_platform_that_ends_the_session_says_whether_something_went_wrong() {
    let cases: [(&str, Option<&[u8]>, WindowEnd); 4] = [
        (
            "a sentence",
            Some(b"Screen recording was switched off."),
            WindowEnd::Failed("Screen recording was switched off.".to_owned()),
        ),
        (
            "a sentence on several lines",
            Some(b"The window\nwas closed.\t"),
            WindowEnd::Failed("The window was closed.".to_owned()),
        ),
        // The person closed the window: they know, and nothing is shown.
        ("no sentence", None, WindowEnd::ThisDevice),
        (
            "a sentence with nothing in it",
            Some(b""),
            WindowEnd::ThisDevice,
        ),
    ];
    for (what, sentence, ending) in cases {
        let mut rig = Rig::new().await;
        let shown = rig.share(false).await;
        assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
        assert_eq!(end(&rig.registry, shown.link, sentence), Ok(()), "{what}");
        assert_eq!(rig.sharing_end().await, ending, "{what}");
        assert_eq!(rig.viewing_end().await, WindowEnd::OtherDevice, "{what}");
        rig.nothing_is_left().await;
        assert_eq!(rig.platform.calls(1).len(), 4, "{what}");
    }
}

#[tokio::test]
async fn a_sentence_that_comes_after_a_quiet_end_changes_nothing() {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    // The person closed the window, and the capture of another thread of the platform stopped at
    // the same moment. Nothing runs between the two calls, so the first is the end.
    assert_eq!(end(&rig.registry, shown.link, None), Ok(()));
    let late = end(&rig.registry, shown.link, Some(b"The capture stopped."));
    assert_eq!(late, Ok(()));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_picture_that_is_handed_over_as_the_session_ends_is_told_that_it_is_over() {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    assert_eq!(rig.wanted(shown.link).await, BEGIN_KEY);
    // The end is not told to the platform at once: a task of the library does that. Until it
    // has, the link is open, and a thread of the platform that is making a picture is answered.
    assert_eq!(end(&rig.registry, shown.link, None), Ok(()));
    assert_eq!(end(&rig.registry, shown.link, None), Ok(()));
    assert_eq!(begin(&rig.registry, shown.link), Ok(BEGIN_NOT_NOW));
    assert_eq!(
        sent(&rig.registry, shown.link, &pic(BEGIN_KEY, 0)),
        Ok((SENT_OVER, 0))
    );
    // The size changing, and the rest, change nothing once the session is over.
    assert_eq!(
        new_epoch(&rig.registry, shown.link, (CODEC_H264, 32, 24, 15)),
        Ok(())
    );
    assert_eq!(pause(&rig.registry, shown.link, PAUSE_HIDDEN), Ok(()));
    assert_eq!(resume(&rig.registry, shown.link), Ok(()));
    assert_eq!(cancel(&rig.registry, shown.link), Ok(()));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_session_that_the_other_device_ended_is_not_told_as_a_failure_of_the_platform() {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    rig.close_at_viewer(&shown).await;
    // The platform gives a sentence a moment later: the end was the other device's already, and
    // that is how the person is told.
    assert_eq!(rig.sharing_end().await, WindowEnd::OtherDevice);
    rig.nothing_is_left().await;
    let late = end(&rig.registry, shown.link, Some(b"Too late."));
    assert_eq!(late, Err(Error::UnknownLink));
}

#[tokio::test]
async fn a_person_who_gives_up_choosing_offers_nothing_and_a_picker_that_fails_tells_why() {
    let mut rig = Rig::new().await;

    // The person closes the picker without choosing.
    let share = rig.begin_share(false);
    let request = rig.next_pick().await;
    assert_eq!(rig.registry.waiting_picks(), 1);
    assert_eq!(rig.platform.give_up(&rig.registry, request), Ok(()));
    assert!(matches!(
        share.await.unwrap(),
        Ok(WindowShared::NothingChosen)
    ));
    assert_eq!(rig.registry.waiting_picks(), 0);
    // An answer that comes again, or for a pick that was never asked for, is refused.
    assert_eq!(
        rig.platform.give_up(&rig.registry, request),
        Err(Error::UnknownLink)
    );
    assert_eq!(
        rig.platform.give_up(&rig.registry, request + 100),
        Err(Error::UnknownLink)
    );

    // The picker fails after it was shown, and the person is told what the platform said.
    let share = rig.begin_share(false);
    let request = rig.next_pick().await;
    assert_eq!(
        rig.platform.fail_pick(
            &rig.registry,
            request,
            b"Screen recording is switched off for Baton."
        ),
        Ok(())
    );
    let Err(error) = share.await.unwrap() else {
        panic!("a pick that failed gave a window");
    };
    assert_eq!(
        error.message(),
        "Screen recording is switched off for Baton."
    );

    // An answer that cannot be read still ends the wait, with a sentence of the library's, and
    // the platform is told what was wrong with it.
    let share = rig.begin_share(false);
    let request = rig.next_pick().await;
    let unreadable = BatonPicked {
        size: 0,
        width: 1440,
        height: 900,
        kind: PICKED_WINDOW,
        context: ptr::null_mut(),
        title: BatonBytes::of(b"Report.pdf"),
        app: BatonBytes::of(b"Preview"),
    };
    assert_eq!(
        rig.platform
            .answer_raw(&rig.registry, request, Some(&unreadable), None),
        Err(Error::Argument)
    );
    let Err(error) = share.await.unwrap() else {
        panic!("an answer that could not be read gave a window");
    };
    assert_eq!(
        error.message(),
        "This device could not tell which window was chosen."
    );

    // A picker that cannot be shown fails at once.
    let to = rig.viewing.id();
    rig.platform.plan_pick(PickPlan::Fail {
        code: -1,
        text: b"The list of windows is not available.".to_vec(),
        claimed_len: 37,
    });
    let Err(error) = rig.sharing.app.share_window(&to, false).await else {
        panic!("a picker that could not be shown gave a window");
    };
    assert_eq!(error.message(), "The list of windows is not available.");
    rig.platform.plan_pick(PickPlan::Fail {
        code: -1,
        text: Vec::new(),
        claimed_len: 0,
    });
    let Err(error) = rig.sharing.app.share_window(&to, false).await else {
        panic!("a picker that could not be shown gave a window");
    };
    assert_eq!(
        error.message(),
        "This device could not show the list of windows to choose from."
    );
    assert_eq!(rig.registry.waiting_picks(), 0);
    assert_eq!(rig.platform.waiting(), Vec::<u64>::new());
    // No window was made, so none is left.
    assert_eq!(rig.platform.windows_made(), 0);
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn an_offer_that_is_given_up_before_a_window_is_chosen_takes_the_picker_down() {
    let mut rig = Rig::new().await;
    let share = rig.begin_share(false);
    let request = rig.next_pick().await;
    share.abort();
    let _ = share.await;

    until("the platform to be told that nobody waits", || {
        rig.platform
            .count(|call| matches!(call, SharerCall::CancelPick { .. }))
            == 1
    })
    .await;
    assert!(
        rig.platform
            .log()
            .contains(&(0, SharerCall::CancelPick { request }))
    );
    assert_eq!(rig.registry.waiting_picks(), 0);
    assert_eq!(rig.platform.waiting(), Vec::<u64>::new());

    // The picker was on its way: its answer is refused, and the window stays with the platform.
    assert_eq!(
        rig.platform.choose(
            &rig.registry,
            request,
            b"Report.pdf",
            b"Preview",
            (1440, 900)
        ),
        Err(Error::UnknownLink)
    );
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.platform.count(|call| *call == SharerCall::Release), 0);
    assert_eq!(
        rig.platform
            .count(|call| matches!(call, SharerCall::CancelPick { .. })),
        1
    );
    rig.nothing_is_left().await;
}

// The platform and the library calling each other.

/// What a capture thread does: asks, makes and hands over a picture, for as long as the link is
/// open. Returns how many pictures it handed over.
fn capture_until_closed(registry: &Registry, link: u64, made: &AtomicU64) -> u64 {
    let (key, delta) = (pic(BEGIN_KEY, 0), pic(BEGIN_DELTA, 0));
    let mut count = 0;
    loop {
        match begin(registry, link) {
            Ok(BEGIN_NOT_NOW) => thread::sleep(Duration::from_micros(200)),
            Ok(kind) => match sent(
                registry,
                link,
                if kind == BEGIN_KEY { &key } else { &delta },
            ) {
                Ok(_) => {
                    count += 1;
                    made.fetch_add(1, Ordering::SeqCst);
                }
                Err(Error::UnknownLink) => return count,
                Err(other) => panic!("a picture was refused: {other:?}"),
            },
            Err(Error::UnknownLink) => return count,
            Err(other) => panic!("the library answered {other:?}"),
        }
    }
}

/// A capture thread that is not careful: it calls the functions that the library has about a
/// window in an order of its own, from the moment it is started until the link is closed. One
/// that is `ender` also ends the session, once, after a while.
fn hammer(registry: &Registry, link: u64, seed: u64, ender: bool, calls: &AtomicU64) {
    let (key, delta) = (pic(BEGIN_KEY, 0), pic(BEGIN_DELTA, 0));
    let mut state = seed;
    let mut done = false;
    loop {
        // A small generator that needs nothing else.
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let step = (state >> 33) % 9;
        let answer = match step {
            0 => clock(registry, link).map(drop),
            1 => begin(registry, link).map(drop),
            2 => cancel(registry, link),
            3 => recovery_pending(registry, link).map(drop),
            4 => sent(registry, link, &key).map(drop),
            5 => sent(registry, link, &delta).map(drop),
            6 => pause(registry, link, PAUSE_HIDDEN).and_then(|()| resume(registry, link)),
            7 => new_epoch(registry, link, (CODEC_H264, 32, 24, 15)),
            _ if ender && !done && calls.load(Ordering::SeqCst) > 1000 => {
                done = true;
                end(registry, link, None)
            }
            _ => Ok(()),
        };
        calls.fetch_add(1, Ordering::SeqCst);
        match answer {
            // Nothing was announced, or it was another kind of picture.
            Ok(()) | Err(Error::Argument) => {}
            Err(Error::UnknownLink) => return,
            Err(other) => panic!("step {step}: the library answered {other:?}"),
        }
    }
}

#[tokio::test]
async fn the_platform_cannot_call_back_while_the_library_is_calling_it() {
    type Answer = (SharerCall, &'static str, Result<(), Error>);
    let mut rig = Rig::with(|platform, _| platform.set_control(true)).await;
    let answers: Arc<Mutex<Vec<Answer>>> = Arc::default();
    {
        let (registry, answers) = (Arc::clone(&rig.registry), Arc::clone(&answers));
        rig.platform.on_call(move |_, call| {
            // A call that was let in could wait for a lock that the library holds for this one.
            let _watch = Watchdog::start("a call back into the library");
            let mut heard = every_call(&registry, 1);
            // The answer to a pick would end the wait of the person for a window.
            // SAFETY: both pointers are null, which the function checks for.
            heard.push(("pick_done", unsafe {
                registry.answer_pick(1, ptr::null(), ptr::null())
            }));
            answers.lock().unwrap().extend(
                heard
                    .into_iter()
                    .map(|(name, answer)| (call.clone(), name, answer)),
            );
        });
    }
    // A picker that is shown and taken down again: Control, Pick and CancelPick.
    let share = rig.begin_share(true);
    rig.next_pick().await;
    share.abort();
    let _ = share.await;
    until("the platform to be told that nobody waits", || {
        rig.platform
            .count(|call| matches!(call, SharerCall::CancelPick { .. }))
            == 1
    })
    .await;

    // A window that is shared with control, touched and ended: Prepare, Start, Ended and Release
    // as well, and the two calls that put what the person does to the window.
    let shown = rig.shown_with_control().await;
    assert!(
        rig.viewing
            .app
            .window_input(shown.viewing.id, touch(0.5, 0.5, WindowPointer::Move))
    );
    until("the window to be asked and told", || {
        rig.platform.posts(shown.window).len() == 1
    })
    .await;
    assert!(rig.sharing.app.end_window(shown.sharing.id));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;

    // A screen that is given without a picker, and let go of: Screen, on the thread that waits for
    // the system, and the Release of the display that follows. It is the second window.
    drop(
        rig.registry
            .sharer()
            .unwrap()
            .screen(Privacy::Open)
            .await
            .unwrap(),
    );
    rig.nothing_is_left().await;
    assert_eq!(rig.platform.calls(2), [SharerCall::Release]);

    // A display that is shared with sound, which the person at the other device wants to hear:
    // Hear as well, on the thread that serves the window. It is the third window.
    rig.windows += 1;
    let heard = rig.share_of(true, false).await;
    assert!(rig.viewing.app.hear_window(heard.viewing.id, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(heard.window) == [RECORD]
    })
    .await;
    assert!(rig.sharing.app.end_window(heard.sharing.id));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;

    let answers = answers.lock().unwrap();
    let kinds: HashSet<_> = answers
        .iter()
        .map(|(call, ..)| mem::discriminant(call))
        .collect();
    assert_eq!(kinds.len(), 11, "every kind of call of the library");
    // Every function was tried inside every call, and none was let in.
    assert!(answers.len() >= 11 * 15, "{} answers", answers.len());
    for (call, name, answer) in answers.iter() {
        assert_eq!(*answer, Err(Error::Reentrant), "{name} inside {call:?}");
    }
    // None of them was taken for a call from elsewhere: the window ran as it would have.
    assert_eq!(
        rig.platform.calls(1),
        [
            asked_to_prepare(),
            SharerCall::Start { link: shown.link },
            SharerCall::Surroundings,
            SharerCall::Post(pointer_post(300.0, 200.0, PointerAction::Move)),
            SharerCall::Ended,
            SharerCall::Release
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_platform_that_waits_in_ended_for_its_capture_thread_does_not_hold_up_the_session() {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    let made = Arc::new(AtomicU64::new(0));
    let capture = {
        let (registry, made) = (Arc::clone(&rig.registry), Arc::clone(&made));
        thread::spawn(move || capture_until_closed(&registry, shown.link, &made))
    };
    // A platform stops the capture in `ended` and waits for the thread that does it: it may be
    // in the middle of a call into the library, which must not be held up by the end.
    let capture = Arc::new(Mutex::new(Some(capture)));
    let stopped = Arc::new(AtomicBool::new(false));
    rig.platform.on_call({
        let (capture, stopped) = (Arc::clone(&capture), Arc::clone(&stopped));
        move |_, call| {
            if *call == SharerCall::Ended {
                let _watch = Watchdog::start("the end of a window");
                let thread = capture.lock().unwrap().take().unwrap();
                assert!(thread.join().unwrap() > 0);
                stopped.store(true, Ordering::SeqCst);
            }
        }
    });
    until("pictures to be made", || made.load(Ordering::SeqCst) >= 20).await;
    assert!(rig.sharing.app.end_window(shown.sharing.id));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;
    assert!(stopped.load(Ordering::SeqCst));
    assert_eq!(
        rig.platform.calls(1).last(),
        Some(&SharerCall::Release),
        "the window is released after the thread is gone"
    );
}

/// The runtime of the app is shut down before the session is over, which drops the task that
/// waits for the end. The window is told all the same, and the link is closed by then, as it is
/// when the session ends in any other way.
#[test]
fn a_window_is_told_of_the_end_with_its_link_closed_also_when_the_runtime_is_shut_down() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (rig, link) = runtime.block_on(async {
        let mut rig = Rig::new().await;
        let link = rig.share(false).await.link;
        (rig, link)
    });
    let (platform, registry) = (Arc::clone(&rig.platform), Arc::clone(&rig.registry));
    let answers = Arc::new(Mutex::new(Vec::new()));
    platform.on_call({
        let (registry, answers) = (Arc::clone(&registry), Arc::clone(&answers));
        move |_, call| {
            if *call == SharerCall::Ended {
                // From a thread of the platform's own: a call made inside a call of the library
                // is refused for another reason.
                let registry = Arc::clone(&registry);
                let answer = thread::spawn(move || begin(&registry, link))
                    .join()
                    .unwrap();
                answers.lock().unwrap().push(answer);
            }
        }
    });
    drop(runtime);

    let deadline = Instant::now() + LET_GO;
    while platform.calls(1).last() != Some(&SharerCall::Release) {
        assert!(
            Instant::now() < deadline,
            "the window was not let go: {:?}",
            platform.calls(1)
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(*answers.lock().unwrap(), [Err(Error::UnknownLink)]);
    assert_eq!(
        platform.calls(1),
        [
            asked_to_prepare(),
            SharerCall::Start { link },
            SharerCall::Ended,
            SharerCall::Release
        ]
    );
    platform.assert_no_complaints();
    assert_eq!(registry.open_links(), 0);
    // The two devices belong to the runtime that is gone, and are not dropped.
    mem::forget(rig);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn threads_of_the_platform_that_are_in_the_library_when_a_session_ends_are_all_told_so() {
    // The session is ended by the person at the other device, by the platform itself from one of
    // its threads, and by the person at this one.
    for how in [
        "the other device",
        "a thread of the platform",
        "this device",
    ] {
        let mut rig = Rig::new().await;
        let shown = rig.share(false).await;
        let calls = Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..4)
            .map(|number| {
                let (registry, calls) = (Arc::clone(&rig.registry), Arc::clone(&calls));
                let (link, ender) = (shown.link, number == 0 && how == "a thread of the platform");
                thread::spawn(move || hammer(&registry, link, 77 + number, ender, &calls))
            })
            .collect();
        until("the threads to be at work", || {
            calls.load(Ordering::SeqCst) >= 500
        })
        .await;
        match how {
            "the other device" => rig.close_at_viewer(&shown).await,
            "this device" => assert!(rig.sharing.app.end_window(shown.sharing.id)),
            _ => {}
        }
        // Each thread was told at some time that the link is closed, and stopped.
        let _watch = Watchdog::start("a thread of the platform");
        for thread in threads {
            thread.join().unwrap();
        }
        rig.nothing_is_left().await;
        let end = rig.sharing_end().await;
        assert_eq!(
            end,
            match how {
                "the other device" => WindowEnd::OtherDevice,
                _ => WindowEnd::ThisDevice,
            },
            "{how}"
        );
        assert_eq!(rig.platform.calls(1).len(), 4, "{how}");
    }
}

#[tokio::test]
async fn a_second_window_gets_a_window_and_a_link_of_its_own() {
    let mut rig = Rig::new().await;
    let first = rig.share(false).await;
    assert_eq!(rig.send(first.link, 0).await, (BEGIN_KEY, SENT, 0));
    assert!(rig.sharing.app.end_window(first.sharing.id));
    rig.sharing_end().await;
    rig.viewing_end().await;
    rig.nothing_is_left().await;

    let second = rig.share(false).await;
    assert_eq!(second.window, 2);
    assert_ne!(second.link, first.link);
    // The number that stood for the first window does not reach the second.
    for (call, answer) in every_call(&rig.registry, first.link) {
        assert_eq!(answer, Err(Error::UnknownLink), "{call}");
    }
    // The second session begins as any other: with a key frame, of number 0.
    assert_eq!(rig.send(second.link, 0).await, (BEGIN_KEY, SENT, 0));
    rig.close_at_viewer(&second).await;
    rig.sharing_end().await;
    rig.viewing_end().await;
    rig.nothing_is_left().await;
    assert_eq!(rig.platform.windows_made(), 2);
    assert_eq!(rig.viewer.screens_made(), 2);
}

#[tokio::test]
async fn the_limits_of_the_other_device_reach_the_platform_and_hold_it_to_them() {
    let mut rig = Rig::with(|_, viewer| {
        viewer.set_limits(|limits| {
            limits.max_width = 32;
            limits.max_height = 24;
            limits.max_fps = 15;
        });
    })
    .await;
    let shown = rig.share(false).await;
    assert_eq!(
        rig.platform.calls(1)[0],
        SharerCall::Prepare {
            takes: (32, 24, 15, CODEC_H264)
        }
    );
    until("the showing platform to be told the pictures", || {
        !rig.configs(1).is_empty()
    })
    .await;
    let first = rig.configs(1)[0];
    assert_eq!(
        (first.0, first.1, first.2, first.3),
        (CODEC_H264, 32, 24, 15)
    );
    // The pictures stay within them for the whole session.
    for (what, format) in [
        ("a width", (CODEC_H264, 33, 24, 15)),
        ("a height", (CODEC_H264, 32, 25, 15)),
        ("a rate", (CODEC_H264, 32, 24, 16)),
    ] {
        assert_eq!(
            new_epoch(&rig.registry, shown.link, format),
            Err(Error::Argument),
            "{what} beyond what the other device takes"
        );
    }
    assert_eq!(
        new_epoch(&rig.registry, shown.link, (CODEC_H264, 16, 12, 5)),
        Ok(())
    );
    rig.close_at_viewer(&shown).await;
    rig.sharing_end().await;
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_number_that_stands_for_a_screen_is_not_one_for_a_window_that_is_shared() {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    // A device that shows windows too has links of both kinds, numbered from one count.
    let screen = rig.registry.links.add(ScreenLink::default());
    assert_ne!(screen, shown.link);
    assert_eq!(rig.registry.open_links(), 2);
    assert_eq!(
        rig.registry.link_decode_failed(shown.link),
        Err(Error::UnknownLink)
    );
    assert_eq!(begin(&rig.registry, screen), Err(Error::UnknownLink));
    assert_eq!(cancel(&rig.registry, screen), Err(Error::UnknownLink));
    assert_eq!(end(&rig.registry, screen, None), Err(Error::UnknownLink));
    assert_eq!(rig.registry.link_decode_failed(screen), Ok(()));
    // Nothing of that touched the window that is shared.
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    assert_eq!(
        rig.platform.calls(1),
        [asked_to_prepare(), SharerCall::Start { link: shown.link }]
    );
    rig.registry.links.remove(screen);
    assert!(rig.sharing.app.end_window(shown.sharing.id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_device_that_goes_away_ends_the_session_and_the_platform_is_told() {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    rig.viewing.app.stop().await;
    let end = rig.sharing_end().await;
    assert!(
        matches!(end, WindowEnd::Lost | WindowEnd::OtherDevice),
        "{end:?}"
    );
    rig.nothing_is_left().await;
    assert_eq!(rig.platform.calls(1).len(), 4);
}

// What the person at the other device does to a window that is shared with control.

/// How the window is laid out in the pictures of the encoder: 400 by 300 points, whose content
/// fills the picture.
fn window_layout() -> BatonGeometry {
    BatonGeometry {
        window_width: 400.0,
        window_height: 300.0,
        picture_width: WIDTH,
        picture_height: HEIGHT,
        content: BatonRect {
            x: 0.0,
            y: 0.0,
            width: f64::from(WIDTH),
            height: f64::from(HEIGHT),
        },
    }
}

/// A finger or a pointer at this place on the picture, as the app is given it.
fn touch(x: f32, y: f32, action: WindowPointer) -> WindowInput {
    WindowInput::Pointer {
        x,
        y,
        modifiers: WindowModifiers::default(),
        action,
    }
}

/// A window of another application lies over the middle of the picture, and another window has
/// the keyboard.
fn covered_without_the_keyboard() -> Around {
    Around {
        fronts: vec![BatonFront {
            rect: BatonRect {
                x: 250.0,
                y: 150.0,
                width: 100.0,
                height: 100.0,
            },
            flags: 0,
            reserved: 0,
        }],
        keyboard: KEYBOARD_ELSEWHERE,
        ..Around::default()
    }
}

/// A key, or text with no key, as the app is given it.
fn typed(code: u16, down: bool, text: Option<&str>) -> WindowInput {
    WindowInput::Key {
        code,
        down,
        modifiers: WindowModifiers::default(),
        repeat: false,
        text: text.map(str::to_owned),
    }
}

impl Rig {
    /// Starts both devices, with a platform that can let the other device control a window.
    async fn controlled() -> Self {
        Self::with(|platform, _| platform.set_control(true)).await
    }

    /// As `controlled`, with the threads that serve the windows keeping time as `timing` says.
    async fn timed(timing: Timing) -> Self {
        Self::built(|platform, _| platform.set_control(true), None, timing).await
    }

    /// As `send`, with the layout of the window in the picture.
    async fn send_laid_out(&self, link: u64, at: u64, layout: BatonGeometry) -> (u32, u32, u64) {
        let kind = self.wanted(link).await;
        let (outcome, number) = sent_with(&self.registry, link, &pic(kind, at), layout).unwrap();
        (kind, outcome, number)
    }

    /// Shares a window with control, hands over its key frame with the layout of `window_layout`,
    /// and waits until the other device shows it. From here on the person can touch it.
    async fn shown_with_control(&mut self) -> Shown {
        self.shown_with_control_of(false).await
    }

    /// As `shown_with_control`, for a whole display if `display` and for a window if not.
    async fn shown_with_control_of(&mut self, display: bool) -> Shown {
        let shown = self.share_of(display, true).await;
        assert_eq!(
            self.send_laid_out(shown.link, 0, window_layout()).await,
            (BEGIN_KEY, SENT, 0)
        );
        self.until_shown(0).await;
        shown
    }

    /// Waits until the showing platform is told to show this frame of the window shared last.
    async fn until_shown(&self, number: u64) {
        until("the picture to be shown", || {
            self.screen_calls(self.viewer.screens_made() as u64)
                .contains(&Call::Show(number))
        })
        .await;
    }

    /// Waits for the news that the other device did not do some of what the person did, and gives
    /// the window, the reason and how many events.
    async fn notice(&mut self) -> (u64, WindowNoticeReason, u32) {
        next_matching(&mut self.viewing_events, |event| match event {
            Event::WindowNotice { id, reason, count } => Some((id, reason, count)),
            _ => None,
        })
        .await
    }
}

#[tokio::test]
async fn what_the_person_does_on_the_picture_is_done_to_the_window_where_they_did_it() {
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);

    // The window is 400 by 300 points with its corner at (100, 50) on the screen of the Mac, and
    // the picture shows all of it.
    for input in [
        touch(0.5, 0.5, WindowPointer::Down(WindowButton::Left)),
        touch(0.5, 0.5, WindowPointer::Up(WindowButton::Left)),
        touch(0.25, 0.75, WindowPointer::Move),
        typed(0x04, true, Some("a")),
        typed(0x04, false, None),
        // Text that came with no key is typed once.
        typed(0, true, Some("ok")),
    ] {
        assert!(app.window_input(id, input.clone()), "{input:?}");
    }
    let expected = [
        left_down(300.0, 200.0),
        left_up(300.0, 200.0),
        pointer_post(200.0, 275.0, PointerAction::Move),
        key_post(0x04, true, Some("a")),
        key_post(0x04, false, None),
        key_post(NO_KEY, true, Some("ok")),
    ];
    until("the window to be done to", || {
        rig.platform.posts(shown.window).len() >= expected.len()
    })
    .await;
    assert_eq!(rig.platform.posts(shown.window), expected);

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_whole_display_is_judged_as_one_piece_and_a_window_by_what_lies_over_it() {
    // The Mac says that a window of another application lies over the middle of what the person
    // touches, and that another window has the keyboard.
    for display in [false, true] {
        let mut rig = Rig::controlled().await;
        let shown = rig.shown_with_control_of(display).await;
        let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);
        assert_eq!(
            (shown.request.display, shown.request.control),
            (display, true)
        );
        rig.platform.set_around(covered_without_the_keyboard());
        assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Down(WindowButton::Left))));

        if display {
            // What lies over a display is part of what is controlled, and a key goes to whatever
            // has the keyboard.
            assert!(app.window_input(id, typed(0x04, true, Some("a"))));
            until("both to be done to the display", || {
                rig.platform.posts(shown.window).len() == 2
            })
            .await;
            assert_eq!(
                rig.platform.posts(shown.window),
                [left_down(300.0, 200.0), key_post(0x04, true, Some("a"))]
            );
        } else {
            // The window is judged by what lies over it: the touch is not put to it, and the
            // person is told why.
            assert_eq!(rig.notice().await, (id, WindowNoticeReason::Covered, 1));
            assert_eq!(rig.platform.posts(shown.window), []);
        }

        assert!(app.end_window(id));
        rig.nothing_is_left().await;
    }
}

#[tokio::test]
async fn a_window_that_is_shared_without_control_is_never_put_to_and_the_mac_is_not_asked_about() {
    let mut rig = Rig::controlled().await;
    let shown = rig.share(false).await;
    assert!(!shown.request.control);
    assert_eq!(
        rig.send_laid_out(shown.link, 0, window_layout()).await,
        (BEGIN_KEY, SENT, 0)
    );
    rig.until_shown(0).await;

    // The app does not take it, and a platform that is asked nothing is not called.
    let id = shown.viewing.id;
    assert!(
        !rig.viewing
            .app
            .window_input(id, touch(0.5, 0.5, WindowPointer::Move))
    );
    assert!(
        !rig.viewing
            .app
            .window_input(id, typed(0x04, true, Some("a")))
    );
    tokio::time::sleep(QUIET).await;
    let asked = |call: &SharerCall| matches!(call, SharerCall::Surroundings | SharerCall::Post(_));
    assert_eq!(rig.platform.count(asked), 0);

    assert!(rig.viewing.app.end_window(id));
    rig.nothing_is_left().await;
    assert_eq!(rig.platform.calls(shown.window).len(), 4);
}

#[tokio::test]
async fn what_the_mac_does_not_do_is_told_to_the_person_with_the_reason() {
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);

    // The Mac is locked: nothing is done to the window, and the person is told why.
    rig.platform.set_around(Around {
        locked: true,
        ..Around::default()
    });
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    assert_eq!(rig.notice().await, (id, WindowNoticeReason::Locked, 1));
    assert_eq!(rig.platform.posts(shown.window), []);

    // It is unlocked, and what the person does is done.
    rig.platform.set_around(Around::default());
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    until("the window to be done to", || {
        rig.platform.posts(shown.window).len() == 1
    })
    .await;
    assert_eq!(
        rig.platform.posts(shown.window),
        [pointer_post(300.0, 200.0, PointerAction::Move)]
    );

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_button_is_let_go_of_when_the_mac_locks_under_it_without_a_word_from_the_person() {
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Down(WindowButton::Left))));
    assert!(app.window_input(id, typed(0x04, true, Some("a"))));
    until("the window to be pressed", || {
        rig.platform.posts(shown.window).len() == 2
    })
    .await;

    // Nothing arrives from the person. The Mac locks, and the thread that serves the window sees
    // it by itself, and lets go of both.
    rig.platform.set_around(Around {
        locked: true,
        ..Around::default()
    });
    until("both to be let go of", || {
        rig.platform.posts(shown.window).len() == 4
    })
    .await;
    let posts = rig.platform.posts(shown.window);
    assert!(posts.contains(&left_up(300.0, 200.0)), "{posts:?}");
    assert!(posts.contains(&key_post(0x04, false, None)), "{posts:?}");

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
    // They were let go of once, and not again when the window ended.
    assert_eq!(rig.platform.posts(shown.window).len(), 4);
}

#[tokio::test]
async fn a_pause_lets_go_of_what_the_person_holds_and_drops_what_they_do_until_it_goes_on() {
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Down(WindowButton::Left))));
    until("the button to be pressed", || {
        rig.platform.posts(shown.window).len() == 1
    })
    .await;

    // The platform pauses the window, for example because it was hidden: the button is let go of
    // at once, without waiting for the person or for a look at the Mac.
    assert_eq!(pause(&rig.registry, shown.link, PAUSE_HIDDEN), Ok(()));
    until("the button to be let go of", || {
        rig.platform.posts(shown.window).len() == 2
    })
    .await;
    assert_eq!(rig.platform.posts(shown.window)[1], left_up(300.0, 200.0));

    // What the person does meanwhile is dropped, and they are told why.
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    assert_eq!(rig.notice().await, (id, WindowNoticeReason::Paused, 1));
    assert_eq!(rig.platform.posts(shown.window).len(), 2);

    // The pictures go on, and so does what the person does.
    assert_eq!(resume(&rig.registry, shown.link), Ok(()));
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    until("the window to be done to", || {
        rig.platform.posts(shown.window).len() == 3
    })
    .await;
    assert_eq!(
        rig.platform.posts(shown.window)[2],
        pointer_post(300.0, 200.0, PointerAction::Move)
    );

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_window_that_ends_with_something_held_has_it_let_go_of_before_the_platform_is_told() {
    // However it ends: the person who shares it, the person who shows it, or the platform.
    for ending in ["sharer", "viewer", "platform"] {
        let mut rig = Rig::controlled().await;
        let shown = rig.shown_with_control().await;
        let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);
        assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Down(WindowButton::Left))));
        assert!(app.window_input(id, typed(0x04, true, Some("a"))));
        until("the window to be pressed", || {
            rig.platform.posts(shown.window).len() == 2
        })
        .await;

        match ending {
            "sharer" => assert!(rig.sharing.app.end_window(shown.sharing.id)),
            "viewer" => assert!(app.end_window(id)),
            _ => end(&rig.registry, shown.link, None).unwrap(),
        }
        rig.nothing_is_left().await;
        let calls = rig.platform.calls(shown.window);
        // The thread looks at the Mac for where the window is, lets go, and then the platform is
        // told. The window is released last, once.
        assert_eq!(
            calls[calls.len() - 4..],
            [
                SharerCall::Post(left_up(300.0, 200.0)),
                SharerCall::Post(key_post(0x04, false, None)),
                SharerCall::Ended,
                SharerCall::Release
            ],
            "{ending}: {calls:?}"
        );
    }
}

#[tokio::test]
async fn a_tap_on_a_picture_that_the_window_has_changed_since_is_dropped_once_the_picture_is_old() {
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);

    // The picture is old by now, but the window did not change after it: a tap is right.
    tokio::time::sleep(MAX_STALE + Duration::from_millis(200)).await;
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    until("the window to be done to", || {
        rig.platform.posts(shown.window).len() == 1
    })
    .await;

    // The platform says that it changed right after the picture, which is long ago.
    assert_eq!(changed(&rig.registry, shown.link, 1), Ok(()));
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    assert_eq!(rig.notice().await, (id, WindowNoticeReason::OutOfDate, 1));
    assert_eq!(rig.platform.posts(shown.window).len(), 1);

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_window_that_is_not_the_size_the_picture_showed_it_at_is_not_touched() {
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);

    // The Mac says that the window is wider than it was in the picture.
    rig.platform.set_around(Around {
        window: BatonRect {
            x: 100.0,
            y: 50.0,
            width: 500.0,
            height: 300.0,
        },
        ..Around::default()
    });
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    assert_eq!(rig.notice().await, (id, WindowNoticeReason::Resized, 1));
    assert_eq!(rig.platform.posts(shown.window), []);

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn pictures_with_no_layout_of_the_window_cannot_be_touched() {
    // A platform that says nothing of it, and one that was written when the struct ended before
    // the layout: what lies in the memory after the end of the struct is not read.
    for cut in [false, true] {
        let mut rig = Rig::controlled().await;
        let shown = rig.share(true).await;
        let kind = rig.wanted(shown.link).await;
        let size = if cut {
            u32::try_from(offset_of!(BatonPicture, geometry)).unwrap()
        } else {
            size_of_u32::<BatonPicture>()
        };
        let layout = if cut {
            window_layout()
        } else {
            BatonGeometry::default()
        };
        assert_eq!(
            sent_sized(&rig.registry, shown.link, &pic(kind, 0), layout, size),
            Ok((SENT, 0))
        );
        rig.until_shown(0).await;
        let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);

        // The person can touch the picture, and the Mac does not know where.
        assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
        assert_eq!(
            rig.notice().await,
            (id, WindowNoticeReason::PictureUnknown, 1),
            "cut: {cut}"
        );
        assert_eq!(rig.platform.posts(shown.window), [], "cut: {cut}");

        assert!(app.end_window(id));
        rig.nothing_is_left().await;
    }
}

#[tokio::test]
async fn the_thread_that_serves_a_window_keeps_time_as_the_registry_says() {
    // A press that the person never lets go of is let go of once they have said nothing for the
    // quiet time that the registry gives, and not after the ten seconds of the default.
    let mut rig = Rig::timed(Timing {
        poll: Duration::from_millis(10),
        quiet: Duration::from_millis(150),
    })
    .await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);

    let pressed = Instant::now();
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Down(WindowButton::Left))));
    until("the button to be let go of", || {
        rig.platform.posts(shown.window).len() == 2
    })
    .await;
    assert!(
        pressed.elapsed() < Duration::from_secs(5),
        "{:?}",
        pressed.elapsed()
    );
    assert_eq!(
        rig.platform.posts(shown.window),
        [left_down(300.0, 200.0), left_up(300.0, 200.0)]
    );

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn the_capture_times_of_pictures_and_of_changes_are_in_microseconds() {
    let mut rig = Rig::controlled().await;
    let shown = rig.share(true).await;
    // The picture was captured 16 ms into the session: 16,000 microseconds.
    assert_eq!(
        rig.send_laid_out(shown.link, 1, window_layout()).await,
        (BEGIN_KEY, SENT, 0)
    );
    rig.until_shown(0).await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);
    tokio::time::sleep(MAX_STALE + Duration::from_millis(200)).await;

    // The window changed at 15 ms, before the picture: it is of no matter to it.
    assert_eq!(changed(&rig.registry, shown.link, 15_000), Ok(()));
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    until("the window to be done to", || {
        rig.platform.posts(shown.window).len() == 1
    })
    .await;

    // It changed again at 20 ms, after the picture, which is old by now.
    assert_eq!(changed(&rig.registry, shown.link, 20_000), Ok(()));
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    assert_eq!(rig.notice().await, (id, WindowNoticeReason::OutOfDate, 1));
    assert_eq!(rig.platform.posts(shown.window).len(), 1);

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_picture_that_did_not_go_out_still_shows_that_the_window_changed() {
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let (app, id) = (rig.viewing.app.clone(), shown.viewing.id);
    tokio::time::sleep(MAX_STALE + Duration::from_millis(200)).await;
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    until("the window to be done to", || {
        rig.platform.posts(shown.window).len() == 1
    })
    .await;

    // The window is resized while a picture of it is made, so the picture is of the old size and
    // does not go out. It showed that the window changed all the same.
    assert_eq!(rig.wanted(shown.link).await, BEGIN_DELTA);
    assert_eq!(
        new_epoch(&rig.registry, shown.link, (CODEC_H264, 32, 24, 15)),
        Ok(())
    );
    assert_eq!(
        sent_with(
            &rig.registry,
            shown.link,
            &pic(BEGIN_DELTA, 1),
            window_layout()
        ),
        Ok((SENT_STALE, 0))
    );
    assert!(app.window_input(id, touch(0.5, 0.5, WindowPointer::Move)));
    assert_eq!(rig.notice().await, (id, WindowNoticeReason::OutOfDate, 1));
    assert_eq!(rig.platform.posts(shown.window).len(), 1);

    assert!(app.end_window(id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_picture_whose_struct_ends_before_the_layout_of_the_header_is_refused() {
    // A delta needs no parameter sets, so nothing but the size of the struct says that this one is
    // not whole: it ends where the count of the sets should be.
    let mut rig = Rig::new().await;
    let link = rig.share(false).await.link;
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    let delta = pic(BEGIN_DELTA, 1);
    let cut = BatonPicture {
        size: u32::try_from(offset_of!(BatonPicture, set_count)).unwrap(),
        flags: delta.flags,
        capture_time: delta.capture_time,
        length_size: delta.length_size,
        reserved: 0,
        sample: BatonBytes::of(&delta.sample),
        sets: ptr::null(),
        set_count: 0,
        geometry: BatonGeometry::default(),
    };
    assert_eq!(rig.wanted(link).await, BEGIN_DELTA);
    let (before_outcome, before_number) = (0xaaaa_aaaa_u32, 0xbbbb_bbbb_bbbb_bbbb_u64);
    let (mut outcome, mut number) = (before_outcome, before_number);
    // SAFETY: the sample is there, and lives through the call.
    let result = unsafe {
        fake_sharer::sent_raw(
            &rig.registry,
            link,
            Some(&cut),
            Some(&mut outcome),
            Some(&mut number),
        )
    };
    assert_eq!(result, Err(Error::Argument));
    assert_eq!((outcome, number), (before_outcome, before_number));

    assert!(rig.sharing.app.end_window(1));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_picture_does_not_go_out_before_the_gate_can_have_its_frame() {
    // An event names a picture by its number and is looked at under the lock of the gate. The lock
    // is taken before the number is made and held until the frame is recorded, so that an event
    // that names a picture that went out finds it recorded. A picture is therefore not sent while
    // someone else holds the gate.
    let mut rig = Rig::controlled().await;
    let shown = rig.share(true).await;
    let screen = rig.viewer.screens_made() as u64;
    assert_eq!(screen, 1, "the other device has the window");
    let input = rig.registry.source(shown.link).unwrap().input();

    let (held, is_held) = std::sync::mpsc::channel();
    let (let_go, to_let_go) = std::sync::mpsc::channel::<()>();
    let holder = thread::spawn(move || {
        let gate = input.gate();
        held.send(()).unwrap();
        // Also when the test fails and the sender is dropped.
        let _ = to_let_go.recv();
        drop(gate);
    });
    is_held.recv().unwrap();

    let kind = rig.wanted(shown.link).await;
    assert_eq!(kind, BEGIN_KEY);
    let (registry, link) = (Arc::clone(&rig.registry), shown.link);
    let sender = thread::spawn(move || sent_with(&registry, link, &pic(kind, 0), window_layout()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        rig.decoded(screen).is_empty(),
        "the picture went out while another thread held the gate"
    );

    let_go.send(()).unwrap();
    holder.join().unwrap();
    assert_eq!(sender.join().unwrap(), Ok((SENT, 0)));
    rig.until_shown(0).await;

    assert!(rig.sharing.app.end_window(shown.sharing.id));
    rig.nothing_is_left().await;
}

// Sound (ADR 0012).

/// A packet of sound as the other platform is given it to play, in a run with no loss.
fn played(flags: u32, number: u64, capture_time: u64, packet: &[u8]) -> Played {
    Played {
        flags,
        number,
        capture_time,
        lost_before: 0,
        packet: packet.to_vec(),
    }
}

#[tokio::test]
async fn the_sound_of_a_display_goes_from_the_functions_the_platform_calls_to_the_functions_the_other_platform_gets()
 {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, window, session) = (shown.link, shown.window, shown.viewing.id);
    let form = aac(48_000, 2, 1024);

    // Nothing is recorded before the person at the other device asks for it. What the platform
    // has anyway goes nowhere, and what it says about the sound changes nothing.
    assert_eq!(rig.platform.hear_calls(window), []);
    assert_eq!(
        sound(&rig.registry, link, 0, b"early"),
        Ok((SOUND_DROPPED, 0))
    );
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    assert_eq!(sound_silent(&rig.registry, link, SILENT_FAILED), Ok(()));
    assert_eq!(
        sound(&rig.registry, link, 0, b"early"),
        Ok((SOUND_DROPPED, 0))
    );

    // The person turns the sound on. The platform is told, and answers with what its packets are.
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(window) == [RECORD]
    })
    .await;
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    until("the other platform to be told what the packets are", || {
        rig.sound_calls(1) == [Call::Sound(form)]
    })
    .await;

    // The packets go out under the numbers they get, and the other platform plays them in the
    // order in which they were made, the first as the start of the run.
    let packets: [&[u8]; 3] = [b"one", b"two", b"three"];
    for (number, bytes) in (0_u64..).zip(packets) {
        let sent = sound(&rig.registry, link, 21_333 * number, bytes);
        assert_eq!(sent, Ok((SOUND_SENT, number)), "packet {number}");
    }
    until("the packets to be played", || rig.played(1).len() == 3).await;
    assert_eq!(
        rig.played(1),
        [
            played(PLAY_FIRST, 0, 0, b"one"),
            played(0, 1, 21_333, b"two"),
            played(0, 2, 42_666, b"three"),
        ]
    );

    // The person turns the sound off. The platform is told, the speaker of the other platform
    // stops, and a packet that was in the hands of the platform goes nowhere.
    assert!(rig.viewing.app.hear_window(session, false));
    until(
        "the platform to be told that the sound is not wanted",
        || rig.platform.hear_calls(window) == [RECORD, STOP],
    )
    .await;
    until("the speaker to stop", || {
        rig.sound_calls(1).last() == Some(&Call::Stopped(SILENT_OFF))
    })
    .await;
    assert_eq!(
        sound(&rig.registry, link, 64_000, b"late"),
        Ok((SOUND_DROPPED, 0))
    );
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    assert_eq!(sound_silent(&rig.registry, link, SILENT_OTHER), Ok(()));

    // The next time is a fresh start: the platform is told again, gives the form again, and the
    // first packet of the run begins it for the other platform.
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked again", || {
        rig.platform.hear_calls(window) == [RECORD, STOP, RECORD]
    })
    .await;
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    let (outcome, number) = sound(&rig.registry, link, 90_000, b"again").unwrap();
    assert_eq!(outcome, SOUND_SENT);
    until("the packet to be played", || rig.played(1).len() == 4).await;
    assert_eq!(
        rig.played(1)[3],
        played(PLAY_FIRST, number, 90_000, b"again")
    );
    // The forms and the packets that came while the sound was off never reached the speaker.
    let forms = rig
        .sound_calls(1)
        .into_iter()
        .filter(|call| matches!(call, Call::Sound(_)))
        .count();
    assert_eq!(forms, 2);

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_device_that_the_person_does_not_let_hear_is_told_so_and_the_platform_is_not_asked() {
    let mut rig = Rig::new().await;
    rig.sharing
        .app
        .set_hear_setting(&rig.viewing.id(), None)
        .unwrap();
    let shown = rig.share_of(true, false).await;
    let (window, session) = (shown.window, shown.viewing.id);

    // The speaker of the other platform is told that the sound stopped, for a reason that is
    // another one to it, and the platform that records is never asked.
    assert!(rig.viewing.app.hear_window(session, true));
    until("the speaker to be told why there is no sound", || {
        rig.sound_calls(1) == [Call::Stopped(SILENT_OTHER)]
    })
    .await;
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.platform.hear_calls(window), []);

    // The person at the Mac says yes, and the next request is taken in the session that runs.
    rig.sharing
        .app
        .set_hear_setting(&rig.viewing.id(), Some(true))
        .unwrap();
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(window) == [RECORD]
    })
    .await;

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_platform_that_cannot_record_says_why_and_the_person_can_ask_again() {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, window, session) = (shown.link, shown.window, shown.viewing.id);

    // Each reason is told to the speaker of the other platform. The person asks again after each,
    // with the sound still on, and the platform is told 1 again with no 0 between.
    let reasons = [SILENT_NO_PERMISSION, SILENT_FAILED, SILENT_OTHER];
    for (asked, reason) in (1..).zip(reasons) {
        assert!(rig.viewing.app.hear_window(session, true));
        until("the platform to be asked for the sound", || {
            rig.platform.hear_calls(window).len() == asked
        })
        .await;
        assert_eq!(sound_silent(&rig.registry, link, reason), Ok(()));
        until("the speaker to be told why there is no sound", || {
            rig.sound_calls(1).len() == asked
        })
        .await;
    }
    assert_eq!(rig.platform.hear_calls(window), [RECORD; 3]);
    assert_eq!(rig.sound_calls(1), reasons.map(Call::Stopped));

    // A reason that the platform may not give changes nothing: the person turning the sound off
    // and the session being of a window are for the library to say.
    for reason in [SILENT_OFF, SILENT_WINDOW, 5, u32::MAX] {
        assert_eq!(
            sound_silent(&rig.registry, link, reason),
            Err(Error::Argument),
            "reason {reason}"
        );
    }
    // Nothing of that reached the other device: what it is told next is the next reason that
    // works.
    assert_eq!(sound_silent(&rig.registry, link, SILENT_FAILED), Ok(()));
    until("the speaker to be told once more", || {
        rig.sound_calls(1).len() == 4
    })
    .await;
    assert_eq!(rig.sound_calls(1)[3], Call::Stopped(SILENT_FAILED));

    // The person asks once more, and this time the platform can: the run begins afresh.
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked once more", || {
        rig.platform.hear_calls(window).len() == 4
    })
    .await;
    let form = aac(44_100, 1, 960);
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    let sent = sound(&rig.registry, link, 0, b"first");
    assert_eq!(sent.map(|(outcome, _)| outcome), Ok(SOUND_SENT));
    until("the packet to be played", || rig.played(1).len() == 1).await;
    assert_eq!(rig.sound_calls(1)[4], Call::Sound(form));
    assert_eq!(rig.platform.hear_calls(window), [RECORD; 4]);

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
}

/// The next time the screens hear how the sound of the window `id` stands.
async fn next_sound(events: &mut Events, id: u64) -> WindowSound {
    next_matching(events, |event| match event {
        Event::WindowSound { id: of, sound } if of == id => Some(sound),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_speaker_that_cannot_play_says_so_and_the_platform_of_the_source_is_told_to_stop() {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, window, session) = (shown.link, shown.window, shown.viewing.id);
    let form = aac(48_000, 2, 1024);
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(window) == [RECORD]
    })
    .await;
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    until("the other platform to be told what the packets are", || {
        rig.sound_calls(1) == [Call::Sound(form)]
    })
    .await;
    assert_eq!(
        next_sound(&mut rig.viewing_events, session).await,
        WindowSound::Playing
    );

    // The speaker of the screen could not make the run, and says so from a thread of its own, with
    // the link that `open` gave for the screen. It calls the function of the interface, as the
    // platform does, and that reaches the session of this device and not the one that reports a
    // decoder that failed.
    let (registry, screen) = (Arc::clone(&rig.viewing_registry), rig.screen_link(1));
    let speaker = std::thread::spawn(move || {
        standing_in(&registry, || {
            exports::baton_stream_link_sound_failed(screen)
        })
    });
    assert_eq!(speaker.join().unwrap(), OK);
    // The person is told that it was this device. The platform of the source is told to stop, so
    // that the Mac plays again, and the speaker is stopped as when the person turns the sound off.
    assert_eq!(
        next_sound(&mut rig.viewing_events, session).await,
        WindowSound::CannotPlay
    );
    until("the platform of the source to be told to stop", || {
        rig.platform.hear_calls(window) == [RECORD, STOP]
    })
    .await;
    until("the speaker to stop", || {
        rig.sound_calls(1).last() == Some(&Call::Stopped(SILENT_OFF))
    })
    .await;
    // What the platform of the source hands over for the run goes nowhere.
    assert_eq!(
        sound(&rig.registry, link, 0, b"late"),
        Ok((SOUND_DROPPED, 0))
    );

    // A report about the run that is over changes nothing, and neither does a link that is not
    // open.
    assert_eq!(rig.viewing_registry.link_sound_failed(screen), Ok(()));
    assert_eq!(
        rig.viewing_registry.link_sound_failed(screen + 100),
        Err(Error::UnknownLink)
    );
    tokio::time::sleep(QUIET).await;
    assert_eq!(rig.platform.hear_calls(window), [RECORD, STOP]);

    // The person asks again. The platform of the source is asked afresh, and this time the
    // speaker plays.
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform of the source to be asked again", || {
        rig.platform.hear_calls(window) == [RECORD, STOP, RECORD]
    })
    .await;
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    let sent = sound(&rig.registry, link, 90_000, b"again");
    assert_eq!(sent.map(|(outcome, _)| outcome), Ok(SOUND_SENT));
    until("the packet to be played", || rig.played(1).len() == 1).await;
    let calls = rig.sound_calls(1);
    assert_eq!(calls.len(), 4, "{calls:?}");
    assert_eq!(calls[1], Call::Stopped(SILENT_OFF));
    assert_eq!(calls[2], Call::Sound(form));
    assert!(matches!(calls[3], Call::Play(_)), "{calls:?}");

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
    // The screen is released with the session, and its link is not open any more.
    assert_eq!(
        rig.viewing_registry.link_sound_failed(screen),
        Err(Error::UnknownLink)
    );
}

#[tokio::test]
async fn the_sound_of_a_window_is_asked_of_nobody_and_what_the_platform_hands_over_goes_nowhere() {
    let mut rig = Rig::new().await;
    let shown = rig.share(false).await;
    let (link, window, session) = (shown.link, shown.window, shown.viewing.id);

    // Sound goes with a whole screen. The session of a window says so (its `sound` is not set),
    // and the app asks no one for it: neither the platform of the source nor its device.
    assert!(!shown.viewing.sound);
    assert!(!rig.viewing.app.hear_window(session, true));
    assert!(!rig.viewing.app.hear_window(session, true));
    assert_eq!(rig.platform.hear_calls(window), []);
    // What the platform hands over anyway goes nowhere.
    assert_eq!(
        sound_begin(&rig.registry, link, &aac(48_000, 2, 1024)),
        Ok(())
    );
    assert_eq!(
        sound(&rig.registry, link, 0, b"packet"),
        Ok((SOUND_DROPPED, 0))
    );

    // The speaker was never called.
    rig.close_at_viewer(&shown).await;
    rig.nothing_is_left().await;
    assert_eq!(rig.sound_calls(1), []);
}

#[tokio::test]
async fn a_form_of_sound_that_is_wrong_or_changes_is_refused_and_nothing_goes_out_for_it() {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, window, session) = (shown.link, shown.window, shown.viewing.id);
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(window) == [RECORD]
    })
    .await;

    // A form that is not within what the protocol allows is refused, also while the sound is
    // wanted.
    let good = aac(48_000, 2, 1024);
    let wrong = [
        ("a codec that is not known", BatonSound { codec: 2, ..good }),
        ("no codec", BatonSound { codec: 0, ..good }),
        ("a rate that is too low", aac(7_999, 2, 1024)),
        ("a rate that is too high", aac(96_001, 2, 1024)),
        ("no channels", aac(48_000, 0, 1024)),
        ("three channels", aac(48_000, 3, 1024)),
        ("channels that do not fit a byte", aac(48_000, 258, 1024)),
        ("too few samples", aac(48_000, 2, 255)),
        ("too many samples", aac(48_000, 2, 2049)),
        ("samples that do not fit", aac(48_000, 2, 65_537)),
    ];
    for (what, form) in wrong {
        assert_eq!(
            sound_begin(&rig.registry, link, &form),
            Err(Error::Argument),
            "{what}"
        );
    }
    // So is a form that is not there, or that does not say it is whole.
    // SAFETY: no struct at all is one of the arguments that the function is made for.
    let nothing = unsafe { sound_begin_raw(&rig.registry, link, None) };
    assert_eq!(nothing, Err(Error::Argument));
    for size in [0, 4, size_of_u32::<BatonSound>() - 1] {
        let short = BatonSound { size, ..good };
        // SAFETY: a struct that lives through the call, and is refused for its size.
        let result = unsafe { sound_begin_raw(&rig.registry, link, Some(&short)) };
        assert_eq!(result, Err(Error::Argument), "size {size}");
    }
    // None of that began a run.
    assert_eq!(
        sound(&rig.registry, link, 0, b"packet"),
        Ok((SOUND_DROPPED, 0))
    );

    // A struct that says it is longer than this version knows is a newer one: what follows the
    // part that is known is not read.
    let newer = BatonSound {
        size: size_of_u32::<BatonSound>() + 8,
        ..good
    };
    // SAFETY: a struct that lives through the call; the library reads no more of it than it knows.
    let began = unsafe { sound_begin_raw(&rig.registry, link, Some(&newer)) };
    assert_eq!(began, Ok(()));
    until("the other platform to be told what the packets are", || {
        rig.sound_calls(1) == [Call::Sound(good)]
    })
    .await;

    // A session has one form of sound. Another is refused, and the run goes on as it was.
    for (what, other) in [
        ("another rate", aac(44_100, 2, 1024)),
        ("another number of channels", aac(48_000, 1, 1024)),
        ("another size of packet", aac(48_000, 2, 960)),
    ] {
        assert_eq!(
            sound_begin(&rig.registry, link, &other),
            Err(Error::Argument),
            "{what}"
        );
    }
    assert_eq!(
        sound(&rig.registry, link, 0, b"packet"),
        Ok((SOUND_SENT, 0))
    );
    // The same form again is a new run, which the other platform is told.
    assert_eq!(sound_begin(&rig.registry, link, &good), Ok(()));
    assert_eq!(
        sound(&rig.registry, link, 21_333, b"packet"),
        Ok((SOUND_SENT, 1))
    );
    until("the packets to be played", || rig.played(1).len() == 2).await;
    assert_eq!(
        rig.sound_calls(1)
            .iter()
            .filter(|call| matches!(call, Call::Sound(_)))
            .count(),
        2
    );

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_form_that_the_other_device_cannot_play_ends_the_run_and_the_platform_is_told_it_is_not_wanted()
 {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, window, session) = (shown.link, shown.window, shown.viewing.id);
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(window) == [RECORD]
    })
    .await;

    // The protocol allows packets of 2048 samples, and this device plays those of 960 or 1024.
    assert_eq!(
        sound_begin(&rig.registry, link, &aac(48_000, 2, 2048)),
        Ok(())
    );
    until(
        "the speaker to be told that there is nothing to play",
        || rig.sound_calls(1) == [Call::Stopped(SILENT_OTHER)],
    )
    .await;
    // The device turned the sound off by itself, and the platform is told so.
    until(
        "the platform to be told that the sound is not wanted",
        || rig.platform.hear_calls(window) == [RECORD, STOP],
    )
    .await;
    assert_eq!(
        sound(&rig.registry, link, 0, b"packet"),
        Ok((SOUND_DROPPED, 0))
    );
    assert_eq!(rig.played(1), []);

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
}

/// The person at the other device asks to hear the display, and the platform that shares it is
/// asked to record.
async fn asks_to_hear(rig: &Rig, shown: &Shown) {
    assert!(rig.viewing.app.hear_window(shown.viewing.id, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(shown.window) == [RECORD]
    })
    .await;
}

#[tokio::test]
async fn packets_that_are_not_there_are_refused_and_nothing_is_written_through_pointers_that_are_not()
 {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, session) = (shown.link, shown.viewing.id);
    asks_to_hear(&rig, &shown).await;
    assert_eq!(
        sound_begin(&rig.registry, link, &aac(48_000, 2, 1024)),
        Ok(())
    );

    let bytes = b"packet";
    let whole = size_of_u32::<BatonSoundPacket>();
    let packet = |size: u32| BatonSoundPacket {
        size,
        reserved: 0,
        capture_time: 0,
        bytes: BatonBytes::of(bytes),
    };
    let (before_outcome, before_number) = (0xaaaa_aaaa_u32, 0xbbbb_bbbb_bbbb_bbbb_u64);
    let (mut outcome, mut number) = (before_outcome, before_number);
    let wrong: Vec<(&str, Option<BatonSoundPacket>)> = vec![
        ("no packet", None),
        ("a struct with no size", Some(packet(0))),
        ("a struct that is too short", Some(packet(whole - 1))),
        (
            "no bytes",
            Some(BatonSoundPacket {
                bytes: BatonBytes::of(b""),
                ..packet(whole)
            }),
        ),
        (
            "bytes with no pointer",
            Some(BatonSoundPacket {
                bytes: BatonBytes {
                    ptr: ptr::null(),
                    len: bytes.len(),
                },
                ..packet(whole)
            }),
        ),
    ];
    for (what, bad) in wrong {
        // SAFETY: the bytes are there, and live through the call.
        let result = unsafe {
            sound_raw(
                &rig.registry,
                link,
                bad.as_ref(),
                Some(&mut outcome),
                Some(&mut number),
            )
        };
        assert_eq!(result, Err(Error::Argument), "{what}");
        assert_eq!((outcome, number), (before_outcome, before_number), "{what}");
    }
    // A missing place for what became of the packet is refused too, before it is sent.
    for (what, outcome_there, number_there) in [
        ("no place for the outcome", false, true),
        ("no place for the number", true, false),
    ] {
        let (mut outcome, mut number) = (before_outcome, before_number);
        let good = packet(whole);
        // SAFETY: as above.
        let result = unsafe {
            sound_raw(
                &rig.registry,
                link,
                Some(&good),
                outcome_there.then_some(&mut outcome),
                number_there.then_some(&mut number),
            )
        };
        assert_eq!(result, Err(Error::Argument), "{what}");
        assert_eq!((outcome, number), (before_outcome, before_number), "{what}");
    }
    // None of that sent anything: the first packet that goes out is the first of the run.
    assert_eq!(sound(&rig.registry, link, 0, b"first"), Ok((SOUND_SENT, 0)));
    until("the packet to be played", || rig.played(1).len() == 1).await;
    assert_eq!(rig.played(1), [played(PLAY_FIRST, 0, 0, b"first")]);

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_packet_larger_than_the_protocol_allows_is_not_read_and_the_largest_that_is_read_goes_out_whole()
 {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, session) = (shown.link, shown.viewing.id);
    asks_to_hear(&rig, &shown).await;
    assert_eq!(
        sound_begin(&rig.registry, link, &aac(48_000, 2, 1024)),
        Ok(())
    );

    // The pointer of such a packet leads nowhere, and it is not followed.
    let nowhere = BatonSoundPacket {
        size: size_of_u32::<BatonSoundPacket>(),
        reserved: 0,
        capture_time: 0,
        bytes: BatonBytes {
            ptr: 8 as *const u8,
            len: MAX_SOUND_PACKET_BYTES + 1,
        },
    };
    let (mut outcome, mut number) = (0xaaaa_aaaa_u32, 0xbbbb_bbbb_bbbb_bbbb_u64);
    // SAFETY: the bytes are described but are not read, as the header says of a length this large.
    let result = unsafe {
        sound_raw(
            &rig.registry,
            link,
            Some(&nowhere),
            Some(&mut outcome),
            Some(&mut number),
        )
    };
    assert_eq!(result, Ok(()));
    assert_eq!((outcome, number), (SOUND_TOO_LARGE, 0));
    // The largest packet that is read goes out whole, as the first of the run.
    let largest = vec![0xaa_u8; MAX_SOUND_PACKET_BYTES];
    assert_eq!(
        sound(&rig.registry, link, 21_333, &largest),
        Ok((SOUND_SENT, 0))
    );
    until("the packet to be played", || rig.played(1).len() == 1).await;
    assert_eq!(rig.played(1), [played(PLAY_FIRST, 0, 21_333, &largest)]);

    assert!(rig.viewing.app.end_window(session));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_packet_that_is_handed_over_as_the_session_ends_is_told_that_it_is_over() {
    let mut rig = Rig::new().await;
    let shown = rig.share_of(true, false).await;
    let (link, window, session) = (shown.link, shown.window, shown.viewing.id);
    let form = aac(48_000, 2, 1024);
    assert!(rig.viewing.app.hear_window(session, true));
    until("the platform to be asked for the sound", || {
        rig.platform.hear_calls(window) == [RECORD]
    })
    .await;
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    assert_eq!(sound(&rig.registry, link, 0, b"one"), Ok((SOUND_SENT, 0)));

    // The end is not told to the platform at once: a task of the library does that. Until it has,
    // the link is open, and a thread of the platform that is recording is answered.
    assert_eq!(end(&rig.registry, link, None), Ok(()));
    assert_eq!(
        sound(&rig.registry, link, 21_333, b"two"),
        Ok((SOUND_OVER, 0))
    );
    assert_eq!(sound_begin(&rig.registry, link, &form), Ok(()));
    assert_eq!(sound_silent(&rig.registry, link, SILENT_FAILED), Ok(()));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;

    // Once the platform is told, the link is closed, and whatever it says is answered so.
    assert_eq!(
        sound(&rig.registry, link, 42_666, b"three"),
        Err(Error::UnknownLink)
    );
    assert_eq!(
        sound_begin(&rig.registry, link, &form),
        Err(Error::UnknownLink)
    );
    assert_eq!(
        sound_silent(&rig.registry, link, SILENT_FAILED),
        Err(Error::UnknownLink)
    );
}

#[tokio::test]
async fn a_device_that_plays_no_sound_cannot_ask_for_it() {
    // A viewing platform that was written before sound was carried.
    let mut rig = Rig::with(|_, viewer| viewer.plays_no_sound()).await;
    let shown = rig.share_of(true, false).await;
    assert!(!rig.viewing.app.hear_window(shown.viewing.id, true));
    assert_eq!(rig.platform.hear_calls(shown.window), []);
    // What the sharing platform has anyway goes nowhere.
    assert_eq!(
        sound_begin(&rig.registry, shown.link, &aac(48_000, 2, 1024)),
        Ok(())
    );
    assert_eq!(
        sound(&rig.registry, shown.link, 0, b"packet"),
        Ok((SOUND_DROPPED, 0))
    );
    // The pictures go on as they would, and the speaker was never called.
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    rig.close_at_viewer(&shown).await;
    rig.nothing_is_left().await;
    assert_eq!(rig.sound_calls(1), []);
}

#[tokio::test]
async fn a_platform_that_sends_no_sound_is_not_asked_for_it() {
    // A sharing platform that was written before sound was carried.
    let mut rig = Rig::with(|platform, _| platform.sends_no_sound()).await;
    let shown = rig.share_of(true, false).await;
    assert!(!rig.viewing.app.hear_window(shown.viewing.id, true));
    assert_eq!(rig.platform.hear_calls(shown.window), []);
    assert_eq!(
        sound(&rig.registry, shown.link, 0, b"packet"),
        Ok((SOUND_DROPPED, 0))
    );
    // The pictures go on as they would, and the speaker was never called.
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    rig.close_at_viewer(&shown).await;
    rig.nothing_is_left().await;
    assert_eq!(rig.sound_calls(1), []);
}

/// A capture thread that sends sound and is not careful: it calls the functions about sound in an
/// order of its own, from the moment it is started until the link is closed. One that is `ender`
/// also ends the session, once, after a while.
fn hammer_sound(registry: &Registry, link: u64, seed: u64, ender: bool, calls: &AtomicU64) {
    let (form, changed_form) = (aac(48_000, 2, 1024), aac(44_100, 1, 960));
    let mut state = seed;
    let mut done = false;
    loop {
        // A small generator that needs nothing else.
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let step = (state >> 33) % 9;
        let answer = match step {
            0 => sound_begin(registry, link, &form),
            1 => sound_begin(registry, link, &changed_form),
            2 => sound_silent(registry, link, SILENT_FAILED),
            3..=5 => sound(registry, link, state >> 40, b"packet").map(drop),
            6 => clock(registry, link).map(drop),
            _ if ender && !done && calls.load(Ordering::SeqCst) > 1000 => {
                done = true;
                end(registry, link, None)
            }
            _ => Ok(()),
        };
        calls.fetch_add(1, Ordering::SeqCst);
        match answer {
            // A form that is not the first of the session, or a call that is not wanted.
            Ok(()) | Err(Error::Argument) => {}
            Err(Error::UnknownLink) => return,
            Err(other) => panic!("step {step}: the library answered {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn threads_of_the_platform_that_send_sound_when_a_session_ends_are_all_told_so() {
    // The session is ended by the person at the other device, by the platform itself from one of
    // its threads, and by the person at this one, while the person at the other device turns the
    // sound on and off.
    for how in [
        "the other device",
        "a thread of the platform",
        "this device",
    ] {
        let mut rig = Rig::new().await;
        let shown = rig.share_of(true, false).await;
        let calls = Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..4)
            .map(|number| {
                let (registry, calls) = (Arc::clone(&rig.registry), Arc::clone(&calls));
                let (link, ender) = (shown.link, number == 0 && how == "a thread of the platform");
                thread::spawn(move || hammer_sound(&registry, link, 77 + number, ender, &calls))
            })
            .collect();
        for round in 0..40 {
            let _ = rig
                .viewing
                .app
                .hear_window(shown.viewing.id, round % 3 != 2);
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        until("the threads to be at work", || {
            calls.load(Ordering::SeqCst) >= 500
        })
        .await;
        match how {
            "the other device" => rig.close_at_viewer(&shown).await,
            "this device" => assert!(rig.sharing.app.end_window(shown.sharing.id)),
            _ => {}
        }
        // Each thread was told at some time that the link is closed, and stopped.
        let _watch = Watchdog::start("a thread of the platform");
        for thread in threads {
            thread.join().unwrap();
        }
        rig.nothing_is_left().await;
        let end = rig.sharing_end().await;
        assert_eq!(
            end,
            match how {
                "the other device" => WindowEnd::OtherDevice,
                _ => WindowEnd::ThisDevice,
            },
            "{how}"
        );
        // The platform was told about the sound while the window ran and not outside it.
        let calls = rig.platform.calls(shown.window);
        assert_eq!(calls[1], SharerCall::Start { link: shown.link }, "{how}");
        assert!(
            calls[2..calls.len() - 2]
                .iter()
                .all(|call| matches!(call, SharerCall::Hear { .. })),
            "{how}: {calls:?}"
        );
        assert_eq!(
            calls[calls.len() - 2..],
            [SharerCall::Ended, SharerCall::Release],
            "{how}"
        );
    }
}

// The screen without a picker (ADR 0010).

/// A platform behind a registry of its own, as the sharer of the app sees it.
fn sharer_of(platform: &Arc<FakeSharer>) -> Arc<dyn WindowSharer> {
    let registry = Registry::new();
    platform.register_in(&registry).unwrap();
    registry.sharer().unwrap()
}

/// What the app is told when it asks for the screen and does not get it.
async fn screen_error(sharer: &Arc<dyn WindowSharer>) -> ScreenError {
    private_screen_error(sharer, Privacy::Open).await
}

/// The same, when the screen was asked for as `privacy`.
async fn private_screen_error(sharer: &Arc<dyn WindowSharer>, privacy: Privacy) -> ScreenError {
    match sharer.screen(privacy).await {
        Err(error) => error,
        Ok(_) => panic!("the platform gave a screen that it should not have given"),
    }
}

#[tokio::test]
async fn a_platform_that_gives_its_screen_is_asked_for_it_and_the_display_is_released_once() {
    let platform = FakeSharer::new();
    let sharer = sharer_of(&platform);
    assert!(sharer.can_give_screen());
    assert!(sharer.can_lift());

    let picked = sharer.screen(Privacy::Open).await.unwrap();
    // A whole display of the size the platform said, with no name, no application and no picker.
    assert!(picked.display());
    assert_eq!(picked.size(), (WIDTH, HEIGHT));
    assert_eq!(
        (picked.title(), picked.app()),
        (String::new(), String::new())
    );
    assert_eq!(platform.log(), [(0, SharerCall::Screen { flags: 0 })]);
    assert_eq!(platform.live_windows(), 1);

    // It is let go of when nobody needs it any more, without ever being started.
    drop(picked);
    assert_eq!(platform.live_windows(), 0);
    assert_eq!(platform.calls(1), [SharerCall::Release]);
    platform.assert_no_complaints();
}

#[tokio::test]
async fn a_platform_that_cannot_give_its_screen_is_not_asked_for_it() {
    // A table that ends at `post` is a platform from before the screen could be given without a
    // picker, and one that ends at `release` is older still: the function they lack reads as
    // missing, and the platform says that it cannot.
    for table in [
        FakeSharer::vtable_without_screen as fn(&Arc<FakeSharer>) -> _,
        FakeSharer::vtable_without_input,
    ] {
        let platform = FakeSharer::new();
        let registry = Registry::new();
        fake_sharer::register_table(&registry, &table(&platform)).unwrap();
        let sharer = registry.sharer().unwrap();
        assert!(!sharer.can_give_screen());
        assert!(!sharer.can_lift());
        assert_eq!(screen_error(&sharer).await, ScreenError::Unsupported);
        assert_eq!(platform.log(), []);
    }
}

#[tokio::test]
async fn a_platform_that_gives_its_screen_and_cannot_lift_says_so() {
    // A table that ends at `screen` is a platform from before the privacy of a screen could be
    // lifted. It gives the screen, and it says that it has nothing to lift with.
    let platform = FakeSharer::new();
    let registry = Registry::new();
    fake_sharer::register_table(&registry, &platform.vtable_without_lift()).unwrap();
    let sharer = registry.sharer().unwrap();
    assert!(sharer.can_give_screen());
    assert!(!sharer.can_lift());
}

#[tokio::test]
async fn a_sharer_written_for_the_first_version_is_refused() {
    // The first version's `screen` has no flags. Called as the second version has it, it would be
    // handed the wrong arguments, so the table is refused and nothing is called.
    for version in [0, 1, 3] {
        let platform = FakeSharer::new();
        let registry = Registry::new();
        let table = BatonSharerVTable {
            version,
            ..platform.vtable()
        };
        assert_eq!(
            fake_sharer::register_table(&registry, &table),
            Err(Error::Argument),
            "version {version}"
        );
        assert!(registry.sharer().is_none(), "version {version}");
        assert_eq!(platform.log(), [], "version {version}");
    }
}

#[tokio::test]
async fn why_the_platform_gave_no_screen_is_what_the_app_is_told() {
    let platform = FakeSharer::new();
    let sharer = sharer_of(&platform);
    let fails = |code, text: &[u8], claimed_len| ScreenPlan::Fails {
        code,
        text: text.to_vec(),
        claimed_len,
    };
    for (plan, told) in [
        // The two reasons with a number of their own, whatever the platform wrote.
        (
            fails(SCREEN_NEEDS_PERMISSION, b"Not for the log.", 16),
            ScreenError::NeedsPermission,
        ),
        (fails(SCREEN_UNAVAILABLE, b"", 0), ScreenError::Unavailable),
        // Any other number is a failure, with the sentence the platform could write.
        (
            fails(E_PLATFORM, b"No display is on.", 17),
            ScreenError::Failed(PlatformError::new("No display is on.")),
        ),
        // The number for the keyboard and the pointer means nothing for a screen that was not asked
        // for as private, so it is a failure like the others.
        (
            fails(SCREEN_NEEDS_INPUT_PERMISSION, b"The system said no.", 19),
            ScreenError::Failed(PlatformError::new("The system said no.")),
        ),
        (
            fails(4, b"The system said no.", 19),
            ScreenError::Failed(PlatformError::new("The system said no.")),
        ),
        // A length that claims more than the platform wrote is read as far as the text goes.
        (
            fails(E_PLATFORM, b"No display is on.", 9999),
            ScreenError::Failed(PlatformError::new("No display is on.")),
        ),
        (
            fails(-1, b"", 0),
            ScreenError::Failed(PlatformError::new("This device could not give its screen.")),
        ),
    ] {
        platform.plan_screen(plan.clone());
        assert_eq!(screen_error(&sharer).await, told, "{plan:?}");
    }
    // Nothing was given, so nothing is left to release.
    assert_eq!(
        platform.count(|call| matches!(call, SharerCall::Screen { .. })),
        7
    );
    assert_eq!(platform.windows_made(), 0);
    platform.assert_no_complaints();
}

#[tokio::test]
async fn a_screen_is_asked_for_as_private_only_when_the_app_says_so() {
    let platform = FakeSharer::new();
    let sharer = sharer_of(&platform);

    drop(sharer.screen(Privacy::Open).await.unwrap());
    drop(sharer.screen(Privacy::Private).await.unwrap());

    // The platform heard the flag, and nothing else of it: the display it gave is the same.
    let screens: Vec<_> = platform
        .log()
        .into_iter()
        .filter(|(_, call)| matches!(call, SharerCall::Screen { .. }))
        .collect();
    assert_eq!(
        screens,
        [
            (0, SharerCall::Screen { flags: 0 }),
            (
                0,
                SharerCall::Screen {
                    flags: SCREEN_PRIVATE
                }
            ),
        ]
    );
    assert_eq!(platform.live_windows(), 0);
    platform.assert_no_complaints();
}

#[tokio::test]
async fn a_platform_that_cannot_hold_the_input_for_a_private_screen_gives_nothing() {
    let platform = FakeSharer::new();
    let sharer = sharer_of(&platform);
    platform.plan_screen(ScreenPlan::Fails {
        code: SCREEN_NEEDS_INPUT_PERMISSION,
        text: b"Not for the log.".to_vec(),
        claimed_len: 16,
    });

    // Whatever the platform wrote, the app is told the reason by its number.
    assert_eq!(
        private_screen_error(&sharer, Privacy::Private).await,
        ScreenError::NeedsInputPermission
    );
    // Nothing was given, so nothing is left to release.
    assert_eq!(platform.windows_made(), 0);
    platform.assert_no_complaints();
}

#[tokio::test]
async fn an_answer_that_is_not_a_display_is_released_and_is_a_failure() {
    let platform = FakeSharer::new();
    let sharer = sharer_of(&platform);
    let not_whole = |kind, width, height, window| ScreenPlan::Writes {
        kind,
        width,
        height,
        window,
    };
    let mut made = 0;
    for (what, plan, given) in [
        ("a window", not_whole(PICKED_WINDOW, 800, 600, true), true),
        ("a kind of its own", not_whole(7, 800, 600, true), true),
        (
            "a display with no width",
            not_whole(PICKED_DISPLAY, 0, 600, true),
            true,
        ),
        (
            "a display with no height",
            not_whole(PICKED_DISPLAY, 800, 0, true),
            true,
        ),
        (
            "a display with no pointer",
            not_whole(PICKED_DISPLAY, 800, 600, false),
            false,
        ),
    ] {
        platform.plan_screen(plan);
        assert_eq!(
            screen_error(&sharer).await,
            ScreenError::Failed(PlatformError::new("This device could not give its screen.")),
            "{what}"
        );
        // What the platform gave is the platform's until the library has it, and the library does
        // not lose it: it is released at once, and never called.
        if given {
            made += 1;
            assert_eq!(platform.calls(made), [SharerCall::Release], "{what}");
        }
        assert_eq!(
            platform.windows_made(),
            usize::try_from(made).unwrap(),
            "{what}"
        );
        assert_eq!(platform.live_windows(), 0, "{what}");
    }
    platform.assert_no_complaints();
}

#[tokio::test]
async fn a_display_larger_than_the_protocol_allows_reads_as_the_largest() {
    let platform = FakeSharer::new();
    let sharer = sharer_of(&platform);
    // Each edge is held to the largest on its own.
    for (given, read) in [
        ((100_000, 5), (MAX_WINDOW_EDGE, 5)),
        ((5, 100_000), (5, MAX_WINDOW_EDGE)),
    ] {
        platform.plan_screen(ScreenPlan::Gives(given.0, given.1));
        let picked = sharer.screen(Privacy::Open).await.unwrap();
        assert_eq!(picked.size(), read, "a display of {given:?}");
    }
}

/// Opens the gate of a platform that waits when the test ends, also by a failure, so that the
/// thread that waits for it can end.
struct OpensAtTheEnd(Arc<FakeSharer>);

impl Drop for OpensAtTheEnd {
    fn drop(&mut self) {
        self.0.open_screen();
    }
}

#[tokio::test]
async fn a_screen_that_nobody_waits_for_any_more_is_released_when_the_platform_gives_it() {
    let platform = FakeSharer::new();
    let _opens = OpensAtTheEnd(Arc::clone(&platform));
    platform.plan_screen(ScreenPlan::Waits(WIDTH, HEIGHT));
    let sharer = sharer_of(&platform);

    // The app asks, the platform takes its time, and the app stops waiting.
    let asking = tokio::spawn(sharer.screen(Privacy::Open));
    until("the platform to be asked", || {
        platform.count(|call| matches!(call, SharerCall::Screen { .. })) == 1
    })
    .await;
    asking.abort();
    let _ = asking.await;
    assert_eq!(platform.windows_made(), 0);

    // The platform gives the display after all, and the library lets go of it.
    platform.open_screen();
    until("the platform's display to be released", || {
        platform.windows_made() == 1 && platform.live_windows() == 0
    })
    .await;
    assert_eq!(platform.calls(1), [SharerCall::Release]);
    platform.assert_no_complaints();
}

/// A decoder that counts the pictures it is given.
struct Counting(Arc<AtomicU64>);

impl Decoder for Counting {
    fn decode(&mut self, _frame: &FrameHeader, _picture: Vec<u8>) -> Result<(), DecodeError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn show(&mut self, _number: u64) {}

    fn source(&mut self, _message: SourceMessage) {}

    fn state(&self) -> DecoderState {
        DecoderState {
            decode_us: 1000,
            thermal: Thermal::Nominal,
        }
    }

    fn ended(&mut self) {}
}

/// A phone, as far as these tests need one: a node that shows the screen of another device and can
/// ask for it. The app has no call for the asking yet, so the asking is the node's own.
struct Phone {
    node: Arc<Node>,
    id: DeviceId,
    /// How many pictures its decoder was given.
    decoded: Arc<AtomicU64>,
    _dir: TempDir,
}

impl Phone {
    async fn start(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::generate();
        let trust = TrustStore::load(dir.path().join("devices.json")).unwrap();
        let config = NodeConfig::from_toml(&format!(
            "device_name = \"{name}\"\n\n[network]\nrelay = \"disabled\"\nlan_discovery = \
             false\nport_mapping = false\n"
        ))
        .unwrap();
        let node = Node::builder(&identity, trust, config)
            .window_handler(Refusing)
            .bind()
            .await
            .unwrap();
        Self {
            node: Arc::new(node),
            id: identity.device_id(),
            decoded: Arc::new(AtomicU64::new(0)),
            _dir: dir,
        }
    }

    /// Asks `mac` to start sharing its screen.
    async fn asks(&self, mac: &Dev, control: bool) -> baton_core::Result<StartOutcome> {
        let mac: DeviceId = mac.id().parse().unwrap();
        let viewing = Viewing::new(
            Counting(Arc::clone(&self.decoded)),
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

/// Turns down every offer that reaches the person.
struct Refusing;

impl baton_core::streaming::WindowHandler for Refusing {
    fn offered<'a>(
        &'a self,
        _from: &'a baton_core::PairedDevice,
        _offer: &'a baton_core::streaming::WindowOffer,
    ) -> BoxFuture<'a, baton_core::streaming::WindowVerdict> {
        Box::pin(std::future::ready(
            baton_core::streaming::WindowVerdict::Refuse,
        ))
    }

    fn started(
        &self,
        _from: &baton_core::PairedDevice,
        _handle: baton_core::streaming::ViewHandle,
    ) {
    }
}

/// Pairs the phone with the Mac the way two people would: the Mac shows a ticket, the phone enters
/// it, and the Mac says yes.
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

#[tokio::test]
async fn a_phone_that_the_person_allowed_gets_the_screen_of_the_platform_through_the_interface() {
    let rig = Rig::new().await;
    let phone = Phone::start("Pocket Phone").await;
    pair_phone(&rig.sharing, &phone).await;

    // What the platform was asked, but for what the app asks whenever it likes: whether the
    // person can let another device control a window.
    let asked = || {
        rig.platform
            .log()
            .into_iter()
            .filter(|(_, call)| *call != SharerCall::Control)
            .collect::<Vec<_>>()
    };

    // A device is allowed nothing until the person says so, and the platform is not asked.
    let refused = phone.asks(&rig.sharing, false).await.unwrap();
    assert!(
        matches!(
            refused,
            StartOutcome::NotStarted(baton_core::streaming::WhyNot::NotAllowed)
        ),
        "{refused:?}"
    );
    assert_eq!(asked(), []);

    rig.sharing
        .app
        .set_start_access(&phone.id.to_string(), StartAccess::Look)
        .unwrap();
    let StartOutcome::Started(handle) = phone.asks(&rig.sharing, false).await.unwrap() else {
        panic!("the screen was not started");
    };

    // The platform was asked for the screen and never for a picker, then prepared within what the
    // phone takes, and started with a link of its own.
    until("the platform to be started", || {
        rig.platform
            .calls(1)
            .iter()
            .any(|call| matches!(call, SharerCall::Start { .. }))
    })
    .await;
    assert_eq!(asked()[0], (0, SharerCall::Screen { flags: 0 }));
    assert_eq!(
        rig.platform
            .count(|call| matches!(call, SharerCall::Pick { .. })),
        0
    );
    assert_eq!(
        rig.platform.calls(1)[0],
        asked_to_prepare_within(1920, 1200, 60)
    );
    let link = rig.platform.link(1);

    // A picture goes the whole way to the decoder of the phone.
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    until("the phone to be given the picture", || {
        phone.decoded.load(Ordering::Relaxed) == 1
    })
    .await;
    assert!(!handle.control());
    // A screen that nobody controls is not one that is kept private, and has nothing to lift.
    assert!(!handle.private());
    assert!(!handle.lift(true, true));

    // The phone ends it, and everything is let go: the link is closed, the platform is told that
    // it is over, and the display is released.
    handle.end();
    rig.nothing_is_left().await;
    let calls = rig.platform.calls(1);
    assert_eq!(calls.last(), Some(&SharerCall::Release));
    assert!(calls.contains(&SharerCall::Ended));
}

/// A phone that the person allowed to look at the screen of the Mac and control it, and has asked
/// for it with control: the screen is given as private, and the session runs.
async fn controlling_phone(rig: &Rig) -> (Phone, baton_core::streaming::ViewHandle) {
    let phone = Phone::start("Pocket Phone").await;
    pair_phone(&rig.sharing, &phone).await;
    rig.sharing
        .app
        .set_start_access(&phone.id.to_string(), StartAccess::Control)
        .unwrap();
    let StartOutcome::Started(handle) = phone.asks(&rig.sharing, true).await.unwrap() else {
        panic!("the screen was not started");
    };
    until("the platform to be started", || {
        rig.platform
            .calls(1)
            .iter()
            .any(|call| matches!(call, SharerCall::Start { .. }))
    })
    .await;
    (phone, handle)
}

/// What the platform was told is lifted of the first window, in order.
fn told_lifted(platform: &FakeSharer) -> Vec<u32> {
    platform
        .calls(1)
        .into_iter()
        .filter_map(|call| match call {
            SharerCall::Lift { lifted } => Some(lifted),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn what_the_phone_lifts_of_the_privacy_of_the_screen_reaches_the_platform_as_the_header_says()
{
    let rig = Rig::with(|platform, _| platform.set_control(true)).await;
    let (_phone, handle) = controlling_phone(&rig).await;

    // The screen was asked for as private, and the phone is told that it can lift what keeps it
    // so. Nothing is lifted when the session starts: the platform is not told anything.
    assert!(handle.control());
    assert!(handle.private());
    assert_eq!(
        rig.platform.calls(1)[0],
        asked_to_prepare_within(1920, 1200, 60)
    );
    assert!(rig.platform.log().contains(&(
        0,
        SharerCall::Screen {
            flags: SCREEN_PRIVATE
        }
    )));
    assert_eq!(told_lifted(&rig.platform), Vec::<u32>::new());

    // The person switches the two as often as they like, and the platform hears each one whole.
    let mut heard = 0;
    for (screen, input, bits) in [
        (true, false, LIFT_SCREEN),
        (true, true, LIFT_SCREEN | LIFT_INPUT),
        (false, true, LIFT_INPUT),
        (false, false, 0),
        (true, false, LIFT_SCREEN),
        (false, false, 0),
    ] {
        assert!(handle.lift(screen, input));
        heard += 1;
        until("the platform to hear it", || {
            told_lifted(&rig.platform).len() >= heard
        })
        .await;
        assert_eq!(told_lifted(&rig.platform).last(), Some(&bits));
    }
    assert_eq!(told_lifted(&rig.platform), [1, 3, 2, 0, 1, 0]);

    // The session ends with something lifted, and the platform is told that it is over.
    assert!(handle.lift(true, true));
    until("the platform to hear it", || {
        told_lifted(&rig.platform).len() > heard
    })
    .await;
    handle.end();
    rig.nothing_is_left().await;
    let calls = rig.platform.calls(1);
    assert_eq!(calls.last(), Some(&SharerCall::Release));
    assert!(calls.contains(&SharerCall::Ended));
    rig.platform.assert_no_complaints();
}

#[tokio::test]
async fn a_platform_that_cannot_lift_gives_a_private_screen_that_the_phone_cannot_lift() {
    let rig = Rig::with(|platform, _| {
        platform.set_control(true);
        platform.set_cannot_lift();
    })
    .await;
    let (phone, handle) = controlling_phone(&rig).await;

    // The screen is still given as private, and stays so for the whole session; the phone is not
    // told that it may lift anything, and has no way to ask.
    assert!(rig.platform.log().contains(&(
        0,
        SharerCall::Screen {
            flags: SCREEN_PRIVATE
        }
    )));
    assert!(handle.control());
    assert!(!handle.private());
    assert!(!handle.lift(true, true));
    assert_eq!(told_lifted(&rig.platform), Vec::<u32>::new());

    // A picture goes the whole way to the phone, so that both control streams are up. A phone
    // that ends the session before that is noticed by the Mac only when it gives up waiting.
    let link = rig.platform.link(1);
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    until("the phone to be given the picture", || {
        phone.decoded.load(Ordering::Relaxed) == 1
    })
    .await;

    handle.end();
    rig.nothing_is_left().await;
    rig.platform.assert_no_complaints();
}

/// Hands over the first picture of the first window to the phone, so that both control streams of
/// the session are up: a phone that ends the session before that is noticed by the Mac only when
/// it gives up waiting.
async fn until_the_phone_has_a_picture(rig: &Rig, phone: &Phone) {
    let link = rig.platform.link(1);
    assert_eq!(rig.send(link, 0).await, (BEGIN_KEY, SENT, 0));
    until("the phone to be given the picture", || {
        phone.decoded.load(Ordering::Relaxed) == 1
    })
    .await;
}

/// What the platform was told when the first window was over, and then that it was let go of.
fn told_at_the_end(platform: &FakeSharer) -> Vec<SharerCall> {
    platform
        .calls(1)
        .into_iter()
        .filter(|call| {
            matches!(
                call,
                SharerCall::Ended | SharerCall::Leave | SharerCall::Release
            )
        })
        .collect()
}

#[tokio::test]
async fn the_person_who_chose_to_leave_the_mac_as_it_is_has_the_platform_told_leave_and_not_ended()
{
    let rig = Rig::with(|platform, _| platform.set_control(true)).await;
    let (phone, handle) = controlling_phone(&rig).await;
    // The Mac offered the choice with the screen, which it does for a private screen only.
    assert!(handle.private());
    assert!(handle.can_leave());
    until_the_phone_has_a_picture(&rig, &phone).await;

    // The phone asks, and ends the session. Nothing answers.
    assert!(handle.end_leaving());
    rig.nothing_is_left().await;
    assert_eq!(
        told_at_the_end(&rig.platform),
        [SharerCall::Leave, SharerCall::Release]
    );
    rig.platform.assert_no_complaints();
}

#[tokio::test]
async fn every_end_of_a_private_session_but_the_choice_to_leave_has_the_platform_told_ended() {
    // The phone ends the session without asking, or goes away without a word. The Mac locks.
    for how in ["end", "drop"] {
        let rig = Rig::with(|platform, _| platform.set_control(true)).await;
        let (phone, handle) = controlling_phone(&rig).await;
        assert!(handle.can_leave(), "{how}");
        until_the_phone_has_a_picture(&rig, &phone).await;
        match how {
            "end" => handle.end(),
            _ => drop(handle),
        }
        rig.nothing_is_left().await;
        assert_eq!(
            told_at_the_end(&rig.platform),
            [SharerCall::Ended, SharerCall::Release],
            "{how}"
        );
        rig.platform.assert_no_complaints();
    }
}

#[tokio::test]
async fn a_platform_that_cannot_leave_does_not_offer_it_and_is_told_ended_at_every_end() {
    let rig = Rig::with(|platform, _| {
        platform.set_control(true);
        platform.set_cannot_leave();
    })
    .await;
    let (phone, handle) = controlling_phone(&rig).await;
    // The screen is still private, with everything that a platform of this version can do about
    // it, and the phone is not offered the choice.
    assert!(handle.private());
    assert!(!handle.can_leave());
    until_the_phone_has_a_picture(&rig, &phone).await;

    // A phone that asks all the same ends the session as it ends every other, and is told that
    // the choice was not made.
    assert!(!handle.end_leaving());
    rig.nothing_is_left().await;
    assert_eq!(
        told_at_the_end(&rig.platform),
        [SharerCall::Ended, SharerCall::Release]
    );
    rig.platform.assert_no_complaints();
}

#[tokio::test]
async fn a_window_that_the_person_picked_is_not_told_what_is_lifted_whatever_the_session_says() {
    // The library drops a word about what is lifted when the session is not private, so the thread
    // of the window is the second guard: the `lift` of the platform is for a display that was
    // given as private and for no other window. The word is given to the events of the session as
    // the library would give it if it forwarded it, and then an event of the phone goes through.
    // The thread serves the word ahead of the events, so the platform has the event only after
    // the thread has dealt with the word.
    let mut rig = Rig::controlled().await;
    let shown = rig.shown_with_control().await;
    let input = rig.registry.source(shown.link).unwrap().input();
    Forward::new(input).lift(Lift {
        screen: true,
        input: true,
    });
    assert!(
        rig.viewing
            .app
            .window_input(shown.viewing.id, touch(0.5, 0.5, WindowPointer::Move))
    );
    until("the window to be asked and told", || {
        rig.platform.posts(shown.window).len() == 1
    })
    .await;
    assert_eq!(told_lifted(&rig.platform), Vec::<u32>::new());
    rig.platform.assert_no_complaints();

    assert!(rig.sharing.app.end_window(shown.sharing.id));
    rig.nothing_is_left().await;
}

#[tokio::test]
async fn a_window_from_the_picker_is_never_private_even_when_the_platform_can_lift() {
    // Only a screen that the app asked for as private is one. A window that the person picked is
    // shown as the person chose it, and neither device offers the switches for it, which would do
    // nothing there.
    let mut rig = Rig::controlled().await;
    assert!(rig.registry.sharer().unwrap().can_lift());
    let shown = rig.shown_with_control().await;
    assert!(!shown.sharing.private);
    assert!(!shown.viewing.private);

    assert!(rig.sharing.app.end_window(shown.sharing.id));
    rig.nothing_is_left().await;
}
