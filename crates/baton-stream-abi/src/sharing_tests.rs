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
    mem, ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use baton_app::{
    Event, Events, WindowEnd, WindowRequest, WindowRole, WindowSession, WindowShared, Windows,
    windows::{PickedWindow, PlatformError, ScreenLink, SharingFailure, WindowSharer},
};
use baton_core::{
    streaming::{SourceEvents, SourceHandle, Takes},
    util::BoxFuture,
};
use baton_proto::limits::MAX_PICTURE_BYTES;
use baton_stream::control::Config as Format;
use tokio::task::JoinHandle;

use crate::{
    Error, Registry,
    abi::{
        BEGIN_DELTA, BEGIN_KEY, BEGIN_NOT_NOW, BatonBytes, BatonFormat, BatonPicked, BatonPicture,
        CODEC_H264, FRAME_KEY, FRAME_RECOVERY, FRAME_REFERENCED, PAUSE_HIDDEN, PAUSE_LOCKED,
        PAUSE_OTHER, SENT, SENT_OVER, SENT_STALE, SENT_TOO_LARGE, SOURCE_CONFIG, SOURCE_PAUSE,
        SOURCE_RESUME, size_of_u32,
    },
    fake::{Call, Decoded, Fake, Watchdog},
    fake_sharer::{
        self, FakeSharer, Pic, PickPlan, PreparePlan, SharerCall, StartPlan, begin, cancel, clock,
        end, new_epoch, new_epoch_raw, pause, recovery_pending, resume, sent,
    },
    fixture::{HEIGHT, PPS, SPS, WIDTH, pictures, sps},
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
        let (platform, viewer) = (FakeSharer::new(), Fake::new());
        setup(&platform, &viewer);
        let (registry, viewing_registry) = (Arc::new(Registry::new()), Arc::new(Registry::new()));
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

    /// Offers the window and has the person at the other device answer.
    async fn offer(
        &mut self,
        control: bool,
        accept: bool,
    ) -> (baton_app::Result<WindowShared>, WindowRequest) {
        let share = self.begin_share(control);
        self.pick_report().await;
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
        let (shared, request) = self.offer(control, true).await;
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
async fn a_window_cannot_be_offered_with_control_by_a_platform_that_cannot_grant_it() {
    let mut rig = Rig::new().await;
    let to = rig.viewing.id();
    let Err(refused) = rig.sharing.app.share_window(&to, true).await else {
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
    };
    let whole = size_of_u32::<BatonPicture>();
    let (before_outcome, before_number) = (0xaaaa_aaaa_u32, 0xbbbb_bbbb_bbbb_bbbb_u64);
    let (mut outcome, mut number) = (before_outcome, before_number);
    let wrong: Vec<(&str, Option<BatonPicture>)> = vec![
        ("no picture", None),
        ("a struct with no size", Some(picture(0, 4))),
        ("a struct that is too short", Some(picture(whole - 1, 4))),
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
        reserved: 0,
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

    // A window that is shared, and ended: Prepare, Start, Ended and Release as well.
    let shown = rig.share(true).await;
    assert_eq!(rig.send(shown.link, 0).await, (BEGIN_KEY, SENT, 0));
    assert!(rig.sharing.app.end_window(shown.sharing.id));
    assert_eq!(rig.sharing_end().await, WindowEnd::ThisDevice);
    rig.nothing_is_left().await;

    let answers = answers.lock().unwrap();
    let kinds: HashSet<_> = answers
        .iter()
        .map(|(call, ..)| mem::discriminant(call))
        .collect();
    assert_eq!(kinds.len(), 7, "every kind of call of the library");
    // Every function was tried inside every call, and none was let in.
    assert!(answers.len() >= 7 * 11, "{} answers", answers.len());
    for (call, name, answer) in answers.iter() {
        assert_eq!(*answer, Err(Error::Reentrant), "{name} inside {call:?}");
    }
    // None of them was taken for a call from elsewhere: the window ran as it would have.
    assert_eq!(
        rig.platform.calls(1),
        [
            asked_to_prepare(),
            SharerCall::Start { link: shown.link },
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
