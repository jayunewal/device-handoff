// SPDX-License-Identifier: Apache-2.0

//! What the platform registered, and the links that its windows use to reach the Rust side.
//!
//! A [`Registry`] holds the viewer and the sharer of one device, and the links of the screens and
//! of the windows that run. There is one for the process ([`registered_windows`]), which the
//! functions of the interface use, and a test makes its own.

#[cfg(test)]
use std::cell::Cell;
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError},
};

use baton_app::{
    Windows,
    windows::{ScreenLink, WindowSharer, WindowViewer},
};

use crate::{
    abi::{E_ARGUMENT, E_PANIC, E_PLATFORM, E_REENTRANT, E_REGISTERED, E_UNKNOWN_LINK, OK},
    input::Timing,
    platform::{Calls, SharerCalls, inside_call},
    sharer::{AbiSharer, Picks, SourceLink},
    viewer::AbiViewer,
};

/// Why a call of the interface failed. The code is what the platform is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A null pointer, a wrong version or size, a function that is missing, or a value that the
    /// platform gave and the interface does not allow.
    #[error("a pointer, a size or a value that the platform gave is not valid")]
    Argument,
    /// A viewer, or a sharer, was registered already.
    #[error("a viewer or a sharer was registered already")]
    Registered,
    /// A call of the platform failed.
    #[error("a call of the platform failed")]
    Platform,
    /// The link is not open, or nobody waits for the request.
    #[error("the link is not open")]
    UnknownLink,
    /// Called from inside a call of the platform, on the thread that the call is made on.
    #[error("called from inside a call of the platform")]
    Reentrant,
    /// The Rust side went wrong inside the call.
    #[error("the Rust side went wrong")]
    Panic,
}

impl Error {
    /// The number that the platform gets for this error.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Argument => E_ARGUMENT,
            Self::Registered => E_REGISTERED,
            Self::Platform => E_PLATFORM,
            Self::UnknownLink => E_UNKNOWN_LINK,
            Self::Reentrant => E_REENTRANT,
            Self::Panic => E_PANIC,
        }
    }

    /// What a function of the interface returns for `result`.
    #[must_use]
    pub const fn status(result: Result<(), Self>) -> i32 {
        match result {
            Ok(()) => OK,
            Err(error) => error.code(),
        }
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a link stands for.
enum Linked {
    /// The screen of a window that is shown here: a decoder says that it failed with it.
    Screen(ScreenLink),
    /// The session of a window that is shared from here: the platform's capture uses it.
    Source(Arc<SourceLink>),
}

/// The links that are open, by the number that the platform was given for each: one for each
/// screen that shows a window and one for each window that is shared.
///
/// A number is never used twice, and one that is not open answers `UnknownLink`, so a platform
/// that calls with a link after its screen was released, or with a number it made up, is told so
/// and nothing else happens. A number is a screen's or a session's, never both: the function that
/// serves one does not find the other.
#[derive(Default)]
pub(crate) struct Links {
    table: Mutex<LinkTable>,
}

#[derive(Default)]
struct LinkTable {
    last: u64,
    open: BTreeMap<u64, Linked>,
}

impl Links {
    /// Opens a link for a screen and gives its number, which is not 0.
    pub(crate) fn add(&self, link: ScreenLink) -> u64 {
        self.insert(Linked::Screen(link))
    }

    /// Opens a link for a window that is shared and gives its number, which is not 0.
    pub(crate) fn add_source(&self, link: Arc<SourceLink>) -> u64 {
        self.insert(Linked::Source(link))
    }

    fn insert(&self, linked: Linked) -> u64 {
        let mut table = lock(&self.table);
        table.last = table.last.saturating_add(1);
        let number = table.last;
        table.open.insert(number, linked);
        number
    }

    /// Closes the link.
    pub(crate) fn remove(&self, number: u64) {
        lock(&self.table).open.remove(&number);
    }

    /// The screen's link that is open under `number`.
    fn screen(&self, number: u64) -> Option<ScreenLink> {
        match lock(&self.table).open.get(&number) {
            Some(Linked::Screen(link)) => Some(link.clone()),
            _ => None,
        }
    }

    /// The session's link that is open under `number`.
    fn source(&self, number: u64) -> Option<Arc<SourceLink>> {
        match lock(&self.table).open.get(&number) {
            Some(Linked::Source(link)) => Some(Arc::clone(link)),
            _ => None,
        }
    }

    /// How many links are open.
    pub(crate) fn len(&self) -> usize {
        lock(&self.table).open.len()
    }
}

/// What the platform of a device registered.
#[derive(Default)]
pub struct Registry {
    viewer: Mutex<Option<Arc<AbiViewer>>>,
    sharer: Mutex<Option<Arc<AbiSharer>>>,
    pub(crate) links: Arc<Links>,
    pub(crate) picks: Arc<Picks>,
    /// How the threads that put input to a window keep time.
    pub(crate) input_timing: Timing,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("viewer", &lock(&self.viewer).is_some())
            .field("sharer", &lock(&self.sharer).is_some())
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// A registry with nothing in it.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the viewer of the platform, once. `Calls` is made only by reading a vtable that the
    /// platform registered. The limits that it states are held to what the protocol allows, so a
    /// platform that is set up wrongly fails here and not at the first window.
    pub(crate) fn register(&self, calls: Calls) -> Result<(), Error> {
        let mut slot = lock(&self.viewer);
        if slot.is_some() {
            return Err(Error::Registered);
        }
        let limits = calls.limits()?;
        let viewer = AbiViewer::new(calls, &limits, Arc::clone(&self.links))?;
        viewer.limits().check().map_err(|_| Error::Argument)?;
        *slot = Some(Arc::new(viewer));
        Ok(())
    }

    /// Takes the sharer of the platform, once. `SharerCalls` is made only by reading a vtable
    /// that the platform registered.
    pub(crate) fn register_sharing(&self, calls: SharerCalls) -> Result<(), Error> {
        let mut slot = lock(&self.sharer);
        if slot.is_some() {
            return Err(Error::Registered);
        }
        let sharer = AbiSharer::new(
            calls,
            Arc::clone(&self.links),
            Arc::clone(&self.picks),
            self.input_timing,
        );
        *slot = Some(Arc::new(sharer));
        Ok(())
    }

    /// The viewer that was registered, if one was.
    #[must_use]
    pub fn viewer(&self) -> Option<Arc<dyn WindowViewer>> {
        let viewer = lock(&self.viewer).clone()?;
        Some(viewer)
    }

    /// The sharer that was registered, if one was.
    #[must_use]
    pub fn sharer(&self) -> Option<Arc<dyn WindowSharer>> {
        let sharer = lock(&self.sharer).clone()?;
        Some(sharer)
    }

    /// What a device is given to take part in window streaming with: the viewer and the sharer
    /// that were registered, if they were. A device that registered neither takes no part.
    #[must_use]
    pub fn windows(&self) -> Windows {
        let mut windows = Windows::none();
        if let Some(viewer) = self.viewer() {
            windows = windows.showing(viewer);
        }
        if let Some(sharer) = self.sharer() {
            windows = windows.sharing(sharer);
        }
        windows
    }

    /// The decoder of the screen that `link` was given for failed after it took a frame.
    ///
    /// # Errors
    ///
    /// [`Error::Reentrant`] if this thread is inside a call of the platform, and
    /// [`Error::UnknownLink`] if no screen is open under that number.
    pub fn link_decode_failed(&self, link: u64) -> Result<(), Error> {
        if inside_call() {
            return Err(Error::Reentrant);
        }
        let link = self.links.screen(link).ok_or(Error::UnknownLink)?;
        link.decode_failed();
        Ok(())
    }

    /// The speaker of the screen that `link` was given for could not play the sound: it could not
    /// make or start what decodes and plays a run.
    ///
    /// # Errors
    ///
    /// [`Error::Reentrant`] if this thread is inside a call of the platform, and
    /// [`Error::UnknownLink`] if no screen is open under that number.
    pub fn link_sound_failed(&self, link: u64) -> Result<(), Error> {
        if inside_call() {
            return Err(Error::Reentrant);
        }
        let link = self.links.screen(link).ok_or(Error::UnknownLink)?;
        link.sound_failed();
        Ok(())
    }

    /// The session of the window that `link` was given for, for the functions that the platform
    /// calls about a window that it shares.
    ///
    /// # Errors
    ///
    /// [`Error::Reentrant`] if this thread is inside a call of the platform, and
    /// [`Error::UnknownLink`] if no session is open under that number: it was never given, or
    /// its session is over.
    pub(crate) fn source(&self, link: u64) -> Result<Arc<SourceLink>, Error> {
        if inside_call() {
            return Err(Error::Reentrant);
        }
        self.links.source(link).ok_or(Error::UnknownLink)
    }

    /// How many links are open: the screens and the windows that were given to the platform and
    /// have not been closed. A screen that was released is not counted, so this is also how a
    /// test shows that nothing is left behind.
    #[must_use]
    pub fn open_links(&self) -> usize {
        self.links.len()
    }

    /// How many picks wait for the platform's answer.
    #[must_use]
    pub fn waiting_picks(&self) -> usize {
        self.picks.len()
    }
}

static GLOBAL: LazyLock<Registry> = LazyLock::new(Registry::new);

#[cfg(test)]
thread_local! {
    /// The registry that a test put in the place of the one of the process on this thread
    /// ([`standing_in`]).
    static STANDING_IN: Cell<Option<&'static Registry>> = const { Cell::new(None) };
}

/// The registry of this process, which the functions of the interface use.
pub(crate) fn global() -> &'static Registry {
    #[cfg(test)]
    if let Some(registry) = STANDING_IN.get() {
        return registry;
    }
    &GLOBAL
}

/// Runs `then` with `registry` in the place of the one of the process on this thread, so that a
/// test can call a function of the interface itself, and not the method of the registry that it
/// calls, on the registry of a device that it made. The one of the process takes a viewer and a
/// sharer once for as long as the process lives, which the tests of the functions of the
/// interface use.
#[cfg(test)]
pub(crate) fn standing_in<R>(registry: &Arc<Registry>, then: impl FnOnce() -> R) -> R {
    /// Puts the registry that stood there back, also when `then` panics.
    struct Back(Option<&'static Registry>);
    impl Drop for Back {
        fn drop(&mut self) {
            STANDING_IN.set(self.0);
        }
    }
    // The borrow has to live as long as the process; a test that leaks one registry is short.
    let registry: &'static Registry = Box::leak(Box::new(Arc::clone(registry)));
    let _back = Back(STANDING_IN.replace(Some(registry)));
    then()
}

/// What the device is given to take part in window streaming with, as far as the platform has
/// registered it: pass it to [`App::start_with`](baton_app::App::start_with). Call it after the
/// platform has registered, which on an Apple device is when the app launches, and before the
/// app starts; a platform that registers later is not heard by an app that has started.
#[must_use]
pub fn registered_windows() -> Windows {
    global().windows()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::mem::offset_of;

    use super::*;
    use crate::{
        abi::{ABI_VERSION, BatonLimits, BatonSharerVTable, BatonViewerVTable, CODEC_H264},
        fake::{Call, Fake, register_table},
        fake_sharer::{self, FakeSharer, PickPlan, SharerCall},
    };

    fn register(registry: &Registry, fake: &Arc<Fake>) -> Result<(), Error> {
        fake.register_in(registry)
    }

    #[test]
    fn a_registry_starts_with_nothing_and_a_device_that_takes_no_part() {
        let registry = Registry::new();
        assert!(registry.viewer().is_none());
        assert!(format!("{:?}", registry.windows()).contains("shows: false"));
        assert_eq!(registry.open_links(), 0);
    }

    #[test]
    fn a_registered_viewer_has_the_limits_that_the_platform_gave_once() {
        let registry = Registry::new();
        let fake = Fake::new();
        fake.set_limits(|limits| {
            limits.max_width = 2560;
            limits.max_height = 1600;
            limits.max_fps = 30;
        });
        register(&registry, &fake).unwrap();
        // Later changes are not heard: the limits were asked for once.
        fake.set_limits(|limits| limits.max_width = 1);
        let viewer = registry.viewer().unwrap();
        for _ in 0..2 {
            let limits = viewer.limits();
            assert_eq!(
                (limits.max_width, limits.max_height, limits.max_fps),
                (2560, 1600, 30)
            );
            assert_eq!(limits.codecs, [baton_proto::stream::Codec::H264]);
        }
        assert_eq!(fake.count(|call| *call == Call::Limits), 1);
        assert!(format!("{:?}", registry.windows()).contains("shows: true"));
    }

    #[test]
    fn limits_that_the_protocol_does_not_allow_are_refused_when_the_viewer_registers() {
        type Change = fn(&mut BatonLimits);
        let registry = Registry::new();
        let fake = Fake::new();
        let table = fake.vtable();
        // Each of these is a platform that is set up wrongly, and it fails at the registration
        // and not at the first window that is offered to it.
        let wrong: [(&str, Change); 8] = [
            ("no width", |limits| limits.max_width = 0),
            ("a width beyond the largest picture", |limits| {
                limits.max_width = 3841;
            }),
            ("no height", |limits| limits.max_height = 0),
            ("a height beyond the largest picture", |limits| {
                limits.max_height = 3841;
            }),
            ("no rate", |limits| limits.max_fps = 0),
            ("a rate beyond the highest", |limits| limits.max_fps = 121),
            ("no codec", |limits| limits.codecs = 0),
            ("a rate beyond 16 bits", |limits| limits.max_fps = 65_536),
        ];
        for (what, change) in wrong {
            fake.set_limits(|limits| *limits = Fake::default_limits());
            fake.set_limits(change);
            assert_eq!(
                register_table(&registry, &table),
                Err(Error::Argument),
                "{what}"
            );
            assert!(registry.viewer().is_none(), "{what}");
        }

        // The largest that the protocol allows is accepted, and so is the smallest.
        for (width, height, fps) in [(3840, 3840, 120), (1, 1, 1)] {
            let registry = Registry::new();
            fake.set_limits(|limits| {
                *limits = Fake::default_limits();
                limits.max_width = width;
                limits.max_height = height;
                limits.max_fps = fps;
            });
            register_table(&registry, &table).unwrap();
            let limits = registry.viewer().unwrap().limits();
            assert_eq!(
                (
                    limits.max_width,
                    limits.max_height,
                    u32::from(limits.max_fps)
                ),
                (width, height, fps)
            );
        }
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }

    #[test]
    fn a_viewer_can_be_registered_once() {
        let registry = Registry::new();
        let (first, second) = (Fake::new(), Fake::new());
        second.set_limits(|limits| limits.max_width = 800);
        register(&registry, &first).unwrap();
        assert_eq!(register(&registry, &second), Err(Error::Registered));
        // The second was not even asked, and the first is the one that stays.
        assert_eq!(second.log().len(), 0);
        assert_eq!(registry.viewer().unwrap().limits().max_width, 1920);
    }

    #[test]
    fn a_registration_that_fails_leaves_nothing_registered() {
        let registry = Registry::new();
        let fake = Fake::new();
        let table = fake.vtable();

        assert_eq!(
            register_table(
                &registry,
                &BatonViewerVTable {
                    version: 9,
                    ..table
                }
            ),
            Err(Error::Argument)
        );
        assert_eq!(
            register_table(
                &registry,
                &BatonViewerVTable {
                    release: None,
                    ..table
                }
            ),
            Err(Error::Argument)
        );
        // SAFETY: a null pointer is one of the cases that the function is given.
        #[allow(unsafe_code)]
        let null = unsafe { registry.register_viewer(std::ptr::null()) };
        assert_eq!(null, Err(Error::Argument));

        fake.fail_limits(-3);
        assert_eq!(register_table(&registry, &table), Err(Error::Platform));
        fake.fail_limits(0);

        fake.set_limits(|limits| limits.codecs = CODEC_H264 | 2);
        assert_eq!(
            register_table(&registry, &table),
            Err(Error::Argument),
            "a codec that is not known"
        );
        fake.set_limits(|limits| *limits = Fake::default_limits());
        fake.set_limits(|limits| limits.max_fps = 65_536);
        assert_eq!(
            register_table(&registry, &table),
            Err(Error::Argument),
            "a rate beyond 16 bits"
        );
        assert!(registry.viewer().is_none());

        // And a platform that gets it right afterwards is registered.
        fake.set_limits(|limits| *limits = Fake::default_limits());
        register_table(&registry, &table).unwrap();
        assert!(registry.viewer().is_some());
    }

    #[test]
    fn a_registry_has_no_sharer_until_the_platform_registers_one() {
        let registry = Registry::new();
        assert!(registry.sharer().is_none());
        assert!(format!("{:?}", registry.windows()).contains("shares: false"));
        assert_eq!(registry.waiting_picks(), 0);
    }

    #[test]
    fn a_registered_sharer_asks_the_platform_each_time_whether_a_window_can_be_controlled() {
        let registry = Registry::new();
        let platform = FakeSharer::new();
        platform.register_in(&registry).unwrap();
        // Registering reads the table and asks nothing.
        assert_eq!(platform.log(), []);
        assert!(format!("{:?}", registry.windows()).contains("shares: true"));
        assert!(format!("{:?}", registry.windows()).contains("shows: false"));

        let sharer = registry.sharer().unwrap();
        assert!(!sharer.control());
        platform.set_control(true);
        assert!(sharer.control());
        platform.set_control(false);
        assert!(!sharer.control());
        // Only a 1 is a yes: the header says 1 or 0.
        platform.set_control_number(2);
        assert!(!sharer.control());
        assert_eq!(platform.count(|call| *call == SharerCall::Control), 4);
        assert!(
            platform.complaints().is_empty(),
            "{:?}",
            platform.complaints()
        );
    }

    #[test]
    fn a_sharer_that_cannot_post_cannot_let_another_device_control_a_window() {
        let register = |table: &BatonSharerVTable| {
            let registry = Registry::new();
            fake_sharer::register_table(&registry, table).unwrap();
            registry
        };
        let platform = FakeSharer::new();
        platform.set_control(true);
        let whole = platform.vtable();
        let none = BatonSharerVTable {
            surroundings: None,
            post: None,
            ..whole
        };
        let before = u32::try_from(offset_of!(BatonSharerVTable, surroundings)).unwrap();
        let tables = [
            // Written before input was carried: the table ends at `release`.
            (
                "a table that ends at release",
                BatonSharerVTable {
                    size: before,
                    ..none
                },
            ),
            ("a table without the two", none),
            (
                "a table with only the one that asks",
                BatonSharerVTable {
                    post: None,
                    ..whole
                },
            ),
            (
                "a table with only the one that posts",
                BatonSharerVTable {
                    surroundings: None,
                    ..whole
                },
            ),
            // A size that cuts a function in two gives half of a pointer to nobody: the function
            // is not read.
            (
                "a size that cuts the first",
                BatonSharerVTable {
                    size: before + 4,
                    ..whole
                },
            ),
            (
                "a size that cuts the second",
                BatonSharerVTable {
                    size: before + 12,
                    ..whole
                },
            ),
        ];
        for (what, table) in tables {
            let registry = register(&table);
            // The platform says yes, and the library does not pass it on, and does not even ask.
            assert!(!registry.sharer().unwrap().control(), "{what}");
        }
        assert_eq!(platform.count(|call| *call == SharerCall::Control), 0);

        // With both, the platform is asked, and its answer is the answer.
        let registry = register(&whole);
        assert!(registry.sharer().unwrap().control());
        assert_eq!(platform.count(|call| *call == SharerCall::Control), 1);
        // A table that is longer than this version knows is read as far as this version goes.
        let longer = BatonSharerVTable {
            size: whole.size + 16,
            ..whole
        };
        assert!(register(&longer).sharer().unwrap().control());
    }

    #[test]
    fn a_sharer_can_be_registered_once() {
        let registry = Registry::new();
        let (first, second) = (FakeSharer::new(), FakeSharer::new());
        second.set_control(true);
        first.register_in(&registry).unwrap();
        assert_eq!(second.register_in(&registry), Err(Error::Registered));
        // The first is the one that stays.
        assert!(!registry.sharer().unwrap().control());
        assert_eq!(second.log(), []);
    }

    #[test]
    fn a_viewer_and_a_sharer_are_registered_separately_and_in_any_order() {
        for sharer_first in [false, true] {
            let registry = Registry::new();
            let (viewer, platform) = (Fake::new(), FakeSharer::new());
            if sharer_first {
                platform.register_in(&registry).unwrap();
                assert!(registry.viewer().is_none());
                register(&registry, &viewer).unwrap();
            } else {
                register(&registry, &viewer).unwrap();
                assert!(registry.sharer().is_none());
                platform.register_in(&registry).unwrap();
            }
            let windows = format!("{:?}", registry.windows());
            assert!(windows.contains("shows: true"), "{windows}");
            assert!(windows.contains("shares: true"), "{windows}");
            // One of each, and a second of either is refused whatever the other did.
            assert_eq!(register(&registry, &Fake::new()), Err(Error::Registered));
            assert_eq!(
                FakeSharer::new().register_in(&registry),
                Err(Error::Registered)
            );
        }
        // A viewer that is refused does not take the place of the sharer, or the other way round.
        let registry = Registry::new();
        let viewer = Fake::new();
        viewer.set_limits(|limits| limits.max_width = 0);
        assert_eq!(register(&registry, &viewer), Err(Error::Argument));
        FakeSharer::new().register_in(&registry).unwrap();
        assert!(registry.viewer().is_none());
    }

    #[test]
    fn the_app_is_told_whether_the_platform_plays_and_whether_it_sends_sound() {
        // A platform with the functions of the sound plays and sends it.
        let registry = Registry::new();
        let (viewer, platform) = (Fake::new(), FakeSharer::new());
        register(&registry, &viewer).unwrap();
        platform.register_in(&registry).unwrap();
        assert!(registry.viewer().unwrap().plays_sound());
        assert!(registry.sharer().unwrap().sends_sound());
        // The question is answered by the tables, and the platform is asked nothing about it.
        assert_eq!(viewer.log(), [(0, Call::Limits)]);
        assert_eq!(platform.log(), []);

        // A platform from before the sound does neither, and is registered all the same.
        let registry = Registry::new();
        let (viewer, platform) = (Fake::new(), FakeSharer::new());
        viewer.register_without_sound_in(&registry).unwrap();
        platform.register_without_hear_in(&registry).unwrap();
        assert!(!registry.viewer().unwrap().plays_sound());
        assert!(!registry.sharer().unwrap().sends_sound());

        // The two are told apart: a device may play sound and send none, or the other way round.
        let registry = Registry::new();
        let (viewer, platform) = (Fake::new(), FakeSharer::new());
        register(&registry, &viewer).unwrap();
        platform.register_without_hear_in(&registry).unwrap();
        assert!(registry.viewer().unwrap().plays_sound());
        assert!(!registry.sharer().unwrap().sends_sound());
        let registry = Registry::new();
        let (viewer, platform) = (Fake::new(), FakeSharer::new());
        viewer.register_without_sound_in(&registry).unwrap();
        platform.register_in(&registry).unwrap();
        assert!(!registry.viewer().unwrap().plays_sound());
        assert!(registry.sharer().unwrap().sends_sound());
    }

    #[test]
    fn a_table_of_the_sound_that_is_not_whole_is_taken_without_it() {
        // The part that every version has ends at `release`: a viewer table that ends there is
        // taken, and one byte short of it is not.
        let registry = Registry::new();
        let fake = Fake::new();
        let table = fake.vtable();
        let before_sound = u32::try_from(offset_of!(BatonViewerVTable, sound)).unwrap();
        assert_eq!(
            register_table(
                &registry,
                &BatonViewerVTable {
                    size: before_sound - 1,
                    ..table
                }
            ),
            Err(Error::Argument)
        );
        assert!(registry.viewer().is_none());
        register_table(
            &registry,
            &BatonViewerVTable {
                size: before_sound,
                ..table
            },
        )
        .unwrap();
        assert!(!registry.viewer().unwrap().plays_sound());

        // The same for a sharer table that ends at `lift`, which is where it ended before sound,
        // and one that is longer than this version knows.
        let registry = Registry::new();
        let platform = FakeSharer::new();
        let table = platform.vtable();
        let before_hear = u32::try_from(offset_of!(BatonSharerVTable, hear)).unwrap();
        fake_sharer::register_table(
            &registry,
            &BatonSharerVTable {
                size: before_hear,
                ..table
            },
        )
        .unwrap();
        assert!(!registry.sharer().unwrap().sends_sound());
        let registry = Registry::new();
        fake_sharer::register_table(
            &registry,
            &BatonSharerVTable {
                size: table.size + 16,
                ..table
            },
        )
        .unwrap();
        assert!(registry.sharer().unwrap().sends_sound());
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
        platform.assert_no_complaints();
    }

    #[test]
    fn a_sharer_table_that_is_not_whole_is_refused_and_leaves_nothing_registered() {
        let registry = Registry::new();
        let platform = FakeSharer::new();
        let table = platform.vtable();
        let refused = |table: BatonSharerVTable| fake_sharer::register_table(&registry, &table);

        assert_eq!(
            refused(BatonSharerVTable {
                version: ABI_VERSION + 1,
                ..table
            }),
            Err(Error::Argument)
        );
        assert_eq!(
            refused(BatonSharerVTable {
                version: 0,
                ..table
            }),
            Err(Error::Argument)
        );
        // The part that every version has ends at `release`; one byte short of it is refused.
        let before_input = u32::try_from(offset_of!(BatonSharerVTable, surroundings)).unwrap();
        for size in [0, 4, 8, before_input - 1] {
            assert_eq!(
                refused(BatonSharerVTable { size, ..table }),
                Err(Error::Argument),
                "size {size}"
            );
        }
        let missing: [(&str, BatonSharerVTable); 7] = [
            (
                "control",
                BatonSharerVTable {
                    control: None,
                    ..table
                },
            ),
            (
                "pick",
                BatonSharerVTable {
                    pick: None,
                    ..table
                },
            ),
            (
                "cancel_pick",
                BatonSharerVTable {
                    cancel_pick: None,
                    ..table
                },
            ),
            (
                "prepare",
                BatonSharerVTable {
                    prepare: None,
                    ..table
                },
            ),
            (
                "start",
                BatonSharerVTable {
                    start: None,
                    ..table
                },
            ),
            (
                "ended",
                BatonSharerVTable {
                    ended: None,
                    ..table
                },
            ),
            (
                "release",
                BatonSharerVTable {
                    release: None,
                    ..table
                },
            ),
        ];
        for (name, table) in missing {
            assert_eq!(refused(table), Err(Error::Argument), "{name} is missing");
        }
        // SAFETY: a null pointer is one of the cases that the function is given.
        #[allow(unsafe_code)]
        let null = unsafe { registry.register_sharer(std::ptr::null()) };
        assert_eq!(null, Err(Error::Argument));
        // SAFETY: a pointer that is not aligned is refused before anything is read through it.
        #[allow(unsafe_code)]
        let unaligned = unsafe {
            let bytes = [0_u64; 16];
            registry.register_sharer(bytes.as_ptr().cast::<u8>().add(1).cast())
        };
        assert_eq!(unaligned, Err(Error::Argument));
        assert!(registry.sharer().is_none());

        // The platform was asked nothing by any of this, and one that is whole is registered
        // afterwards, also when it is of a newer version of the struct (which is longer).
        assert_eq!(platform.log(), []);
        refused(BatonSharerVTable {
            size: table.size + 16,
            ..table
        })
        .unwrap();
        assert!(registry.sharer().is_some());
    }

    #[test]
    fn a_sharer_cannot_be_registered_from_inside_a_call_of_the_platform() {
        let registry = Arc::new(Registry::new());
        let platform = FakeSharer::new();
        platform.register_in(&registry).unwrap();
        platform.plan_pick(PickPlan::Later);
        let inside = Arc::new(Mutex::new(Vec::new()));
        let (heard, other, mine) = (
            Arc::clone(&inside),
            FakeSharer::new(),
            Arc::clone(&registry),
        );
        platform.on_call(move |_, call| {
            if matches!(call, SharerCall::Pick { .. }) {
                // A call that was let in would find the sharer registered already, and a refusal
                // for the wrong reason.
                let fresh = Registry::new();
                heard
                    .lock()
                    .unwrap()
                    .push((other.register_in(&fresh), other.register_in(&mine)));
            }
        });
        let sharer = registry.sharer().unwrap();
        let mut pick = sharer.pick();
        let waker = std::task::Waker::noop();
        assert!(
            pick.as_mut()
                .poll(&mut std::task::Context::from_waker(waker))
                .is_pending()
        );
        assert_eq!(
            *inside.lock().unwrap(),
            [(Err(Error::Reentrant), Err(Error::Reentrant))]
        );
        drop(pick);
        assert_eq!(registry.waiting_picks(), 0);
    }

    #[test]
    fn the_numbers_of_links_are_never_used_twice_and_never_zero() {
        let links = Links::default();
        let numbers: Vec<u64> = (0..5).map(|_| links.add(ScreenLink::default())).collect();
        assert_eq!(numbers, [1, 2, 3, 4, 5]);
        links.remove(3);
        links.remove(5);
        assert_eq!(links.add(ScreenLink::default()), 6);
        assert_eq!(links.len(), 4);
        // A number that is closed stays closed.
        links.remove(3);
        assert_eq!(links.len(), 4);
    }

    #[test]
    fn a_link_that_is_not_open_is_told_so() {
        let registry = Registry::new();
        for number in [0, 1, 99, u64::MAX] {
            assert_eq!(registry.link_decode_failed(number), Err(Error::UnknownLink));
            assert_eq!(registry.link_sound_failed(number), Err(Error::UnknownLink));
        }
        let number = registry.links.add(ScreenLink::default());
        assert_eq!(registry.link_decode_failed(number), Ok(()));
        assert_eq!(registry.link_sound_failed(number), Ok(()));
        assert_eq!(registry.open_links(), 1);
        // A number that is not the one that is open is not taken for it.
        for other in [number + 1, number + 100, number - 1] {
            assert_eq!(registry.link_decode_failed(other), Err(Error::UnknownLink));
            assert_eq!(registry.link_sound_failed(other), Err(Error::UnknownLink));
        }
        registry.links.remove(number);
        assert_eq!(registry.link_decode_failed(number), Err(Error::UnknownLink));
        assert_eq!(registry.link_sound_failed(number), Err(Error::UnknownLink));
        assert_eq!(registry.open_links(), 0);
    }

    #[test]
    fn each_error_has_its_own_number_in_the_header() {
        let errors = [
            Error::Argument,
            Error::Registered,
            Error::Platform,
            Error::UnknownLink,
            Error::Reentrant,
            Error::Panic,
        ];
        let numbers: Vec<i32> = errors.iter().map(|error| error.code()).collect();
        assert_eq!(numbers, [-1, -2, -3, -4, -5, -6]);
        assert_eq!(Error::status(Ok(())), 0);
        assert_eq!(Error::status(Err(Error::Platform)), -3);
    }
}
