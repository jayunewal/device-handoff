// SPDX-License-Identifier: Apache-2.0

//! The functions of the interface that the platform calls: the C symbols of
//! `include/baton_stream_abi.h`.
//!
//! This is one of the two modules of the crate that may hold `unsafe` code (the other is
//! `platform`, where Rust calls the platform). Each function checks what it is given, does its
//! work in the registry of the process, and answers with a number: nothing a platform passes can
//! make one of them do more than answer with an error. A panic is caught and answered the same
//! way, since one that unwound into the platform's frames would end the process.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{
    Error,
    abi::{ABI_VERSION, BatonViewerVTable},
    registry::global,
};

/// Runs `call` and gives the number that the platform is told: its result, or the number of a
/// panic.
fn answer(call: impl FnOnce() -> Result<(), Error>) -> i32 {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => Error::status(result),
        Err(_) => Error::Panic.code(),
    }
}

/// The version of the interface that this library was built with.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_abi_version() -> u32 {
    ABI_VERSION
}

/// Registers the platform's viewer, once, before the app starts. See the header.
///
/// # Safety
///
/// `vtable` is either null, or points to a struct that begins with a `u32` version and a `u32`
/// size and is `size` bytes long, as the header says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn baton_stream_register_viewer(vtable: *const BatonViewerVTable) -> i32 {
    // SAFETY: the caller's promise about `vtable` is this function's own.
    answer(|| unsafe { global().register_viewer(vtable) })
}

/// The decoder of the screen that `link` was given for failed. See the header.
#[unsafe(no_mangle)]
pub extern "C" fn baton_stream_link_decode_failed(link: u64) -> i32 {
    answer(|| global().link_decode_failed(link))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::{
        ptr,
        sync::{Arc, Mutex},
    };

    use baton_app::windows::ScreenLink;

    use super::*;
    use crate::{
        abi::{E_ARGUMENT, E_PLATFORM, E_REENTRANT, E_REGISTERED, E_UNKNOWN_LINK, OK},
        fake::{Call, Fake, Watchdog},
        registered_windows,
    };

    #[test]
    fn a_panic_is_answered_with_its_number_and_not_let_through() {
        assert_eq!(answer(|| panic!("a bug")), Error::Panic.code());
        assert_eq!(answer(|| Ok(())), 0);
        assert_eq!(answer(|| Err(Error::Argument)), Error::Argument.code());
    }

    /// The one test that registers a viewer in the registry of the process, since that can be done
    /// once for as long as the process lives. Every other test makes a registry of its own.
    #[test]
    fn the_functions_of_the_interface_work_on_the_registry_of_the_process() {
        assert_eq!(baton_stream_abi_version(), ABI_VERSION);
        assert!(global().viewer().is_none());
        assert!(format!("{:?}", registered_windows()).contains("shows: false"));

        // Whatever the platform passes, what it gets back is a number.
        // SAFETY: a null vtable is one of the arguments that the function is made for.
        assert_eq!(
            unsafe { baton_stream_register_viewer(ptr::null()) },
            E_ARGUMENT
        );
        let fake = Fake::new();
        let newer = BatonViewerVTable {
            version: ABI_VERSION + 1,
            ..fake.vtable()
        };
        // SAFETY: a vtable that is whole, and that nothing writes to meanwhile.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&raw const newer) },
            E_ARGUMENT
        );
        fake.fail_limits(-7);
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&fake.vtable()) },
            E_PLATFORM
        );
        fake.fail_limits(OK);
        assert!(
            global().viewer().is_none(),
            "a registration that failed left a viewer"
        );

        // The platform calls the interface from inside the call that is registering it, which
        // would wait for itself if it were let in.
        let inside = Arc::new(Mutex::new(Vec::new()));
        let platform = Arc::downgrade(&fake);
        let heard = Arc::clone(&inside);
        fake.on_call(move |call| {
            if *call == Call::Limits {
                let nested = platform.upgrade().unwrap().vtable();
                // A call that was let in would wait for the registration that is being made.
                let _watch = Watchdog::start("a call into the interface from inside a call");
                // SAFETY: as above.
                let registering = unsafe { baton_stream_register_viewer(&raw const nested) };
                heard
                    .lock()
                    .unwrap()
                    .push((registering, baton_stream_link_decode_failed(1)));
            }
        });
        let vtable = fake.vtable();
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&raw const vtable) },
            OK
        );
        assert_eq!(*inside.lock().unwrap(), [(E_REENTRANT, E_REENTRANT)]);
        assert!(global().viewer().is_some());
        assert!(format!("{:?}", registered_windows()).contains("shows: true"));

        // A viewer is registered once, whatever registers the second.
        // SAFETY: as above.
        assert_eq!(
            unsafe { baton_stream_register_viewer(&raw const vtable) },
            E_REGISTERED
        );
        // Limits were asked of the platform for the registration that failed and the one that
        // worked, and for none of the others.
        assert_eq!(fake.count(|call| *call == Call::Limits), 2);

        // A link is open from the moment `open` gives it to the platform until its screen is
        // released, and a number that is not open is told so.
        assert_eq!(baton_stream_link_decode_failed(0), E_UNKNOWN_LINK);
        assert_eq!(baton_stream_link_decode_failed(u64::MAX), E_UNKNOWN_LINK);
        let link = global().links.add(ScreenLink::default());
        assert_eq!(baton_stream_link_decode_failed(link), OK);
        global().links.remove(link);
        assert_eq!(baton_stream_link_decode_failed(link), E_UNKNOWN_LINK);
        assert!(fake.complaints().is_empty(), "{:?}", fake.complaints());
    }
}
