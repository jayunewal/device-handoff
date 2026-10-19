// SPDX-License-Identifier: Apache-2.0

//! What the platform registered, and the links that its screens use to say that a decoder failed.
//!
//! A [`Registry`] holds the viewer of one device. There is one for the process
//! ([`registered_windows`]), which the functions of the interface use, and a test makes its own.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError},
};

use baton_app::{
    Windows,
    windows::{ScreenLink, WindowViewer},
};

use crate::{
    abi::{E_ARGUMENT, E_PANIC, E_PLATFORM, E_REENTRANT, E_REGISTERED, E_UNKNOWN_LINK, OK},
    platform::{Calls, inside_call},
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
    /// A viewer was registered already.
    #[error("a viewer was registered already")]
    Registered,
    /// A call of the platform failed.
    #[error("a call of the platform failed")]
    Platform,
    /// The link is not open.
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

/// The links of the screens that are open, by the number that the platform was given for each.
///
/// A number is never used twice, and one that is not open answers `UnknownLink`, so a platform
/// that calls with a link after its screen was released, or with a number it made up, is told so
/// and nothing else happens.
#[derive(Default)]
pub(crate) struct Links {
    table: Mutex<LinkTable>,
}

#[derive(Default)]
struct LinkTable {
    last: u64,
    open: BTreeMap<u64, ScreenLink>,
}

impl Links {
    /// Opens a link and gives its number, which is not 0.
    pub(crate) fn add(&self, link: ScreenLink) -> u64 {
        let mut table = lock(&self.table);
        table.last = table.last.saturating_add(1);
        let number = table.last;
        table.open.insert(number, link);
        number
    }

    /// Closes the link.
    pub(crate) fn remove(&self, number: u64) {
        lock(&self.table).open.remove(&number);
    }

    /// The link that is open under `number`.
    fn get(&self, number: u64) -> Option<ScreenLink> {
        lock(&self.table).open.get(&number).cloned()
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
    pub(crate) links: Arc<Links>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("viewer", &lock(&self.viewer).is_some())
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
    /// platform registered.
    pub(crate) fn register(&self, calls: Calls) -> Result<(), Error> {
        let mut slot = lock(&self.viewer);
        if slot.is_some() {
            return Err(Error::Registered);
        }
        let limits = calls.limits()?;
        let viewer = AbiViewer::new(calls, &limits, Arc::clone(&self.links))?;
        *slot = Some(Arc::new(viewer));
        Ok(())
    }

    /// The viewer that was registered, if one was.
    #[must_use]
    pub fn viewer(&self) -> Option<Arc<dyn WindowViewer>> {
        let viewer = lock(&self.viewer).clone()?;
        Some(viewer)
    }

    /// What a device is given to take part in window streaming with: the viewer that was
    /// registered, and nothing else yet.
    #[must_use]
    pub fn windows(&self) -> Windows {
        match self.viewer() {
            Some(viewer) => Windows::none().showing(viewer),
            None => Windows::none(),
        }
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
        let link = self.links.get(link).ok_or(Error::UnknownLink)?;
        link.decode_failed();
        Ok(())
    }

    /// How many screens are open: the links that were given to the platform and have not been
    /// closed. A screen that was released is not counted, so this is also how a test shows that
    /// nothing is left behind.
    #[must_use]
    pub fn open_links(&self) -> usize {
        self.links.len()
    }
}

static GLOBAL: LazyLock<Registry> = LazyLock::new(Registry::new);

/// The registry of this process, which the functions of the interface use.
pub(crate) fn global() -> &'static Registry {
    &GLOBAL
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
    use super::*;
    use crate::{
        abi::{BatonViewerVTable, CODEC_H264},
        fake::{Call, Fake, register_table},
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
    fn the_codecs_that_the_app_is_told_are_the_ones_the_platform_took() {
        let registry = Registry::new();
        let fake = Fake::new();
        // A platform that decodes none names none. Whether that is a viewer is for the app to say
        // when it starts, which asks for one codec at least.
        fake.set_limits(|limits| limits.codecs = 0);
        register(&registry, &fake).unwrap();
        assert_eq!(registry.viewer().unwrap().limits().codecs, Vec::new());
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
        }
        let number = registry.links.add(ScreenLink::default());
        assert_eq!(registry.link_decode_failed(number), Ok(()));
        assert_eq!(registry.open_links(), 1);
        // A number that is not the one that is open is not taken for it.
        for other in [number + 1, number + 100, number - 1] {
            assert_eq!(registry.link_decode_failed(other), Err(Error::UnknownLink));
        }
        registry.links.remove(number);
        assert_eq!(registry.link_decode_failed(number), Err(Error::UnknownLink));
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
