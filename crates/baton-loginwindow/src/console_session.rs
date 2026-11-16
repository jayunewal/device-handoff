// SPDX-License-Identifier: Apache-2.0

//! Whether anyone is signed in to the Mac's own console session
//! (`docs/adr/0020-screen-sharing-at-the-login-window.md`), read from a plain launch daemon.
//!
//! The program is not bound to a session that the system ends at sign-in, so it asks instead: it
//! reads the key `State:/Users/ConsoleUser` of the system configuration store, which names the
//! account that owns the Mac's own console session, or is absent while nobody does. A daemon can
//! read it with no window-server session of its own, unlike `CGSessionCopyCurrentDictionary`,
//! which the first shape of this record used and which answers for the caller's own window-server
//! session only (a plain daemon has none).
//!
//! What this answers, and what it does not: on a Mac that is only ever signed in to one account
//! at a time, an absent key should mean nobody is signed in, and a present one should name who
//! is. Whether it still says so correctly while a Mac with several accounts shows the login
//! window again over an account that stayed signed in (a switch of user) is not yet confirmed; it
//! is what the record's V8 tries on a real Mac, and this module answers as the record says to
//! until that is known: refuse unless the answer says, positively, that nobody is signed in.

/// What reading the console session found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsoleSession {
    /// The key was absent: nobody is signed in to the console.
    NobodySignedIn,
    /// The key was present: someone is signed in to the console.
    SomeoneSignedIn,
    /// The store could not be opened or read. Treated the same as someone being signed in: an
    /// answer that cannot be read this way is not the same as nobody being there.
    CouldNotCheck,
}

/// Whether a `Start` should be refused because of who is signed in: every answer but a positive
/// "nobody is", including one that could not be read at all.
pub(crate) fn refuses_start(session: ConsoleSession) -> bool {
    !matches!(session, ConsoleSession::NobodySignedIn)
}

/// `kSCStatusNoKey`, from `SystemConfiguration.h`: the one status that means a key asked for
/// truly was not in the store. Every other status a read can come back with — including
/// success, since that means a value was found — must not be read as nobody being signed in.
const NO_SUCH_KEY: i32 = 1004;

/// Turns a store lookup into a [`ConsoleSession`]: `found` is whether the value read back was
/// non-null, and `status` is what the platform's own error call said right after that read. A
/// plain function, kept apart from the call it follows, so a test can pin it without a Mac to
/// read a real store on.
fn answer(found: bool, status: i32) -> ConsoleSession {
    if found {
        ConsoleSession::SomeoneSignedIn
    } else if status == NO_SUCH_KEY {
        ConsoleSession::NobodySignedIn
    } else {
        ConsoleSession::CouldNotCheck
    }
}

/// Reads the console session now. On a Mac, this is `State:/Users/ConsoleUser`
/// ([`mac::read`]); elsewhere there is no console session to read.
pub(crate) fn read() -> ConsoleSession {
    #[cfg(target_os = "macos")]
    {
        mac::read()
    }
    #[cfg(not(target_os = "macos"))]
    {
        ConsoleSession::CouldNotCheck
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac {
    //! The handful of C calls this needs, kept to this module and written down here
    //! (`CONTRIBUTING.md` asks for `unsafe` code to live in a crate or module of its own, with
    //! the reason written down): opening the dynamic store and reading one key of it, both from
    //! `SystemConfiguration.framework`, and the two calls of `CoreFoundation.framework` that make
    //! and free the strings the other two take. No pointer is kept past the call it was given in.

    use std::ffi::{CStr, c_char, c_void};

    use super::ConsoleSession;

    type CFAllocatorRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFTypeRef = *const c_void;
    type SCDynamicStoreRef = *const c_void;

    /// `kCFStringEncodingUTF8`, from `CFString.h`.
    const UTF8: u32 = 0x0800_0100;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: CFAllocatorRef,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCDynamicStoreCreate(
            allocator: CFAllocatorRef,
            name: CFStringRef,
            callout: *const c_void,
            context: *const c_void,
        ) -> SCDynamicStoreRef;
        fn SCDynamicStoreCopyValue(store: SCDynamicStoreRef, key: CFStringRef) -> CFTypeRef;
        /// The status of the last `SystemConfiguration` call on this thread (`SCError`, from
        /// `SystemConfiguration.h`). Read right after a call that can fail, before any other
        /// call that might set it to something else.
        fn SCError() -> i32;
    }

    /// A `CFStringRef` made from a Rust string that has no `NUL` in it, released when it is
    /// dropped. `with` is given it only for the length of the call it is made for.
    struct OwnedCFString(CFStringRef);

    impl OwnedCFString {
        fn new(value: &CStr) -> Option<Self> {
            let string =
                unsafe { CFStringCreateWithCString(std::ptr::null(), value.as_ptr(), UTF8) };
            if string.is_null() {
                None
            } else {
                Some(Self(string))
            }
        }
    }

    impl Drop for OwnedCFString {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    /// Reads `State:/Users/ConsoleUser` from the system configuration store: present means
    /// someone is signed in to the console, and absent means nobody is — but only when `SCError`
    /// says the key itself was not there (`kSCStatusNoKey`). The store failing to open, the key
    /// failing to be asked for, or any other reason the read came back empty are all
    /// `CouldNotCheck`: nothing here may read "the read failed" as "nobody is signed in".
    pub(super) fn read() -> ConsoleSession {
        let Some(name) = OwnedCFString::new(c"baton-loginwindow") else {
            return ConsoleSession::CouldNotCheck;
        };
        let store = unsafe {
            SCDynamicStoreCreate(std::ptr::null(), name.0, std::ptr::null(), std::ptr::null())
        };
        if store.is_null() {
            return ConsoleSession::CouldNotCheck;
        }
        let Some(key) = OwnedCFString::new(c"State:/Users/ConsoleUser") else {
            unsafe { CFRelease(store) };
            return ConsoleSession::CouldNotCheck;
        };
        let value = unsafe { SCDynamicStoreCopyValue(store, key.0) };
        let status = unsafe { SCError() };
        unsafe { CFRelease(store) };
        let found = !value.is_null();
        if found {
            unsafe { CFRelease(value) };
        }
        super::answer(found, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_positive_nobody_lets_a_start_through() {
        assert!(!refuses_start(ConsoleSession::NobodySignedIn));
        assert!(refuses_start(ConsoleSession::SomeoneSignedIn));
        assert!(refuses_start(ConsoleSession::CouldNotCheck));
    }

    #[test]
    fn only_the_true_absence_of_the_key_is_nobody_signed_in() {
        assert_eq!(answer(false, NO_SUCH_KEY), ConsoleSession::NobodySignedIn);
        assert_eq!(answer(false, 0), ConsoleSession::CouldNotCheck);
        assert_eq!(answer(false, 1001), ConsoleSession::CouldNotCheck);
        assert_eq!(answer(true, 0), ConsoleSession::SomeoneSignedIn);
        assert_eq!(answer(true, NO_SUCH_KEY), ConsoleSession::SomeoneSignedIn);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_a_mac_there_is_no_console_session_to_read() {
        assert_eq!(read(), ConsoleSession::CouldNotCheck);
    }
}
