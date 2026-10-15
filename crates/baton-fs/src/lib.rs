// SPDX-License-Identifier: Apache-2.0

//! File system calls that Baton needs and the standard library does not have.
//!
//! There is one so far, [`rename_noreplace`]: move a file or a folder to a new name, and refuse
//! if something already has that name.
//!
//! `std::fs::rename` replaces what it finds: a file, and a folder that is empty (on Windows, from
//! Windows 10 on a file system such as NTFS). Looking first and renaming after leaves a gap in
//! which something else can take the name, and then what the person already had is lost. A file
//! transfer puts what it received in place with this call, so that nothing the person has is
//! overwritten and two transfers cannot end up under one name (see
//! `docs/adr/0008-file-transfer.md`).
//!
//! # Why this crate has `unsafe` code
//!
//! The standard library has no safe form of the three system calls that do this, and
//! `CONTRIBUTING.md` asks for `unsafe` code to live in a crate of its own, with the reason
//! written down. This is that crate. `unsafe` is denied everywhere in it except in the module
//! `sys`, which holds one short function for each platform:
//!
//! - Linux and Android: `renameat2` with `RENAME_NOREPLACE`. It is made through `syscall`, so
//!   it does not depend on the C library having the function (glibc before 2.28 and Android
//!   before API 30 do not).
//! - macOS, iOS and iPadOS: `renamex_np` with `RENAME_EXCL`.
//! - Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`.
//! - Any other system: [`rename_noreplace`] reports [`ErrorKind::Unsupported`].
//!
//! A volume that cannot do the call (some network and removable file systems) gives the same
//! answer. This crate does not work around that on purpose. Checking first and renaming after
//! would bring back the gap that the call exists to close, so the caller decides what to do.
//!
//! # What has not been run yet
//!
//! The tests run on Linux, macOS and Windows, in CI. The call for iPhone and iPad is the Mac's,
//! and has not been run on a device. Android has not been run at all, and two things need a check
//! on an emulator before the receiver is built for it. Apps on Android 8 to 10 run under a filter
//! that may not allow `renameat2` made through `syscall`; the app would then be ended, and
//! `ENOSYS` would not be the answer. The shared storage of Android 11 and later (the Download
//! folder) is a file system whose daemon may refuse the flag, which would make every move there
//! [`ErrorKind::Unsupported`].

use std::{io::Result, path::Path};

// Only the documentation names the kinds of error, so only the documentation needs the import.
#[cfg(doc)]
use std::io::ErrorKind;

// The one module that is allowed `unsafe` code. See the manifest.
#[allow(unsafe_code)]
mod sys;

/// Moves `from` to `to` in one step, and refuses if anything already has the name `to`.
///
/// `from` can be a file or a folder, with everything inside it. `to` must be on the same file
/// system as `from`, in a folder that exists. Nothing is created on the way and nothing is
/// copied: either the name moves, or nothing changes.
///
/// # Errors
///
/// - [`ErrorKind::AlreadyExists`]: something has the name `to`, whatever it is: a file, a
///   folder (empty or not), a link, even a link that points nowhere. Nothing was changed. A name
///   moved onto itself is the exception: Linux refuses it, and macOS and Windows answer `Ok` and
///   change nothing.
/// - [`ErrorKind::NotFound`]: `from` does not exist, or the folder of `to` does not.
/// - [`ErrorKind::CrossesDevices`]: `to` is on another file system than `from`.
/// - [`ErrorKind::Unsupported`]: this system, or the volume of `to`, cannot move without
///   replacing. Nothing was changed. The message has the system's own words. Linux says the
///   same for a folder moved into a folder inside itself, because it has one code for both.
/// - [`ErrorKind::InvalidInput`]: a path has a NUL character in it.
/// - Anything else the system says, such as `PermissionDenied` for a folder that cannot be
///   written, or a volume that is read-only. On Windows, `PermissionDenied` or a sharing
///   violation can also mean that a program, a virus scanner for one, has a file inside the
///   folder open for a moment. The move then works a little later, so a caller that moves a
///   folder there can try again for a short while before it gives up.
///
/// # Examples
///
/// ```
/// # use std::{fs, io::ErrorKind};
/// # let dir = tempfile::tempdir()?;
/// let staged = dir.path().join("staged.txt");
/// let wanted = dir.path().join("notes.txt");
/// fs::write(&staged, "new")?;
/// fs::write(&wanted, "what the person already had")?;
///
/// // The name is taken, so nothing moves.
/// let error = baton_fs::rename_noreplace(&staged, &wanted).unwrap_err();
/// assert_eq!(error.kind(), ErrorKind::AlreadyExists);
/// assert_eq!(fs::read_to_string(&wanted)?, "what the person already had");
///
/// // A name that is free is taken.
/// let free = dir.path().join("notes 2.txt");
/// baton_fs::rename_noreplace(&staged, &free)?;
/// assert_eq!(fs::read_to_string(&free)?, "new");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn rename_noreplace(from: impl AsRef<Path>, to: impl AsRef<Path>) -> Result<()> {
    sys::rename_noreplace(from.as_ref(), to.as_ref())
}
