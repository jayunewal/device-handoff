// SPDX-License-Identifier: Apache-2.0

//! File system calls that Baton needs and the standard library does not have. A file transfer
//! (see `docs/adr/0008-file-transfer.md`) uses them to put what it received where the person
//! looks.
//!
//! - [`rename_noreplace`]: move a file or a folder to a new name, and refuse if something
//!   already has that name.
//! - [`mark_received`]: mark a file as one that came from elsewhere, the way a web browser marks
//!   what it downloaded.
//! - [`hide`]: hide a file or a folder from the person who looks in the folder it is in.
//! - [`free_space`]: how many bytes can still be written to the volume that a folder is on.
//!
//! # Moving without replacing
//!
//! `std::fs::rename` replaces what it finds: a file, and a folder that is empty (on Windows, from
//! Windows 10 on a file system such as NTFS). Looking first and renaming after leaves a gap in
//! which something else can take the name, and then what the person already had is lost. A file
//! transfer puts what it received in place with [`rename_noreplace`], so that nothing the person
//! has is overwritten and two transfers cannot end up under one name.
//!
//! # Why this crate has `unsafe` code
//!
//! The standard library has no safe form of the system calls that do these things, and
//! `CONTRIBUTING.md` asks for `unsafe` code to live in a crate of its own, with the reason
//! written down. This is that crate. `unsafe` is denied everywhere in it except in the module
//! `sys`, which holds one short function for each call and platform:
//!
//! - **Moving.** Linux and Android: `renameat2` with `RENAME_NOREPLACE`. It is made through
//!   `syscall`, so it does not depend on the C library having the function (glibc before 2.28
//!   and Android before API 30 do not). macOS, iOS and iPadOS: `renamex_np` with `RENAME_EXCL`.
//!   Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`. Any other system:
//!   [`rename_noreplace`] reports [`ErrorKind::Unsupported`].
//! - **Marking.** macOS: `getxattr` and `setxattr` for the attribute `com.apple.quarantine`.
//!   Windows needs no `unsafe` code: the mark is a stream of the file, which the standard library
//!   can write. The other systems have no such mark, and [`mark_received`] does nothing there.
//! - **Hiding.** Windows: `GetFileAttributesW` and `SetFileAttributesW`. The other systems hide
//!   a name that starts with a dot already, and [`hide`] does nothing there.
//! - **Free space.** Linux and Android: `statvfs`. macOS, iOS and iPadOS: `statfs`. Windows:
//!   `GetDiskFreeSpaceExW`. Any other system: [`free_space`] reports [`ErrorKind::Unsupported`].
//!
//! A volume that cannot move without replacing (some network and removable file systems) gives
//! the same answer as a system that cannot. This crate does not work around that on purpose.
//! Checking first and renaming after would bring back the gap that the call exists to close, so
//! the caller decides what to do. A volume that cannot hold a mark (FAT and exFAT, most network
//! shares) is the same for [`mark_received`]: the error says so, and the caller decides whether
//! that matters.
//!
//! # What has not been run yet
//!
//! The tests run on Linux, macOS and Windows, in CI. The calls for iPhone and iPad are the Mac's,
//! and have not been run on a device. Android has not been run at all, and two things need a check
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

/// Marks `file` as one that came from elsewhere, the way a web browser marks what it downloaded,
/// so that the system is careful with it the first time the person opens it.
///
/// - **macOS** sets the extended attribute `com.apple.quarantine` to `0083;<time>;Baton;`, unless
///   the file has it already: a mark that is there stays as it is, whoever put it there.
///   Gatekeeper checks a program that has it and asks the person before it first runs, and other
///   programs read it as "downloaded". It makes no entry in the system's record of downloads.
///   Apple does not document what the flags mean; they are the ones a browser's downloads have.
/// - **Windows** writes the stream `Zone.Identifier` of the file, with `ZoneId=3`, which is the
///   Internet zone. It is the "mark of the web" that `SmartScreen` and Office look for, and it
///   stays with the file when it is moved or copied to another NTFS volume. There is no address
///   in it, since a file from another device has none.
/// - **Other systems** have no such mark (iOS and iPadOS do not quarantine what an app writes, and
///   Linux and Android have nothing that other programs look at). The call checks that the file
///   is there, and does nothing else.
///
/// The mark is meant to be put on once, when a file has been received and checked. Putting it on
/// again changes nothing that matters: the Mac keeps the mark that is there, and Windows writes
/// the same stream again.
///
/// # Errors
///
/// - [`ErrorKind::NotFound`]: there is nothing at `file`. Nothing is made.
/// - [`ErrorKind::InvalidInput`]: the path has a NUL character in it.
/// - Anything else the system says. The usual cause is a volume that cannot hold the mark: FAT
///   and exFAT, and most network shares, have no extended attributes and no streams. The file is
///   then not marked, and it is for the caller to decide whether that stops it from being handed
///   to the person. For a file that was received and checked, it should not.
///
/// # Examples
///
/// ```
/// # let dir = tempfile::tempdir()?;
/// let received = dir.path().join("photo.jpg");
/// std::fs::write(&received, "the bytes")?;
///
/// // A volume that cannot hold a mark is an error, and the file stays as it is.
/// let _marked = baton_fs::mark_received(&received).is_ok();
/// assert_eq!(std::fs::read_to_string(&received)?, "the bytes");
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn mark_received(file: impl AsRef<Path>) -> Result<()> {
    sys::mark_received(file.as_ref())
}

/// Hides `path`, a file or a folder, from the person who looks in the folder it is in.
///
/// - **Windows** adds the attribute `FILE_ATTRIBUTE_HIDDEN`, which Explorer honours unless the
///   person has asked to see hidden files. The other attributes are kept.
/// - **Other systems** hide a name that starts with a dot already (the Finder, the Files app and
///   the file managers of Linux and Android do), so the call checks that something is there and
///   does nothing else. The name of the staging folder of a transfer starts with a dot.
///
/// Hiding is for tidiness and not for secrecy: anyone who types the name, or looks at hidden
/// files, finds it.
///
/// # Errors
///
/// - [`ErrorKind::NotFound`]: there is nothing at `path`.
/// - [`ErrorKind::InvalidInput`]: the path has a NUL character in it.
/// - Anything else the system says, such as `PermissionDenied` for a folder whose attributes the
///   person cannot change, or a volume that has no such attribute.
///
/// # Examples
///
/// ```
/// # let dir = tempfile::tempdir()?;
/// let staging = dir.path().join(".baton-incoming");
/// std::fs::create_dir(&staging)?;
/// baton_fs::hide(&staging)?;
///
/// // It is hidden, and nothing else about it changed.
/// std::fs::write(staging.join("part"), "still writable")?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn hide(path: impl AsRef<Path>) -> Result<()> {
    sys::hide(path.as_ref())
}

/// How many bytes can still be written to the volume that the folder `dir` is on, by the user
/// who runs the program.
///
/// It is what the system says is free to this user: space reserved for the system's own use is
/// not counted, and where there is a quota, the quota is. It is an answer for this moment, and
/// other programs write too, so it is a check to refuse a transfer that cannot fit before the
/// first byte is written, and not a promise that the last byte will fit. A volume that is full
/// later still gives `NoSpace` from the write.
///
/// - Linux and Android: `statvfs`, in blocks that a user can use (`f_bavail`).
/// - macOS, iOS and iPadOS: `statfs`. This counts what is free now. It does not count what the
///   system would make free when it is needed: caches, apps that can be offloaded, and photos
///   that are kept in iCloud (purgeable space). On an iPhone or iPad that can be gigabytes, so
///   the figure can be far below what can be written, and the Settings app shows more. A caller
///   that refuses a transfer on this figure alone refuses some that would fit. What Apple
///   recommends is `volumeAvailableCapacityForImportantUsage`, which counts the purgeable space.
///   It is a key of Foundation and not a system call, so it is for the app to ask. This is from
///   Apple's documentation and has not been measured on a device.
/// - Windows: `GetDiskFreeSpaceExW`, the free bytes available to the user who calls it.
/// - Any other system: [`ErrorKind::Unsupported`].
///
/// # Errors
///
/// - [`ErrorKind::NotFound`]: there is no folder at `dir`.
/// - [`ErrorKind::InvalidInput`]: the path has a NUL character in it.
/// - Anything else the system says.
///
/// # Examples
///
/// ```
/// # let dir = tempfile::tempdir()?;
/// // A transfer of 3 GiB, and 1 GiB that the person keeps for themselves.
/// let (needed, reserve) = (3_u64 << 30, 1_u64 << 30);
/// let fits = baton_fs::free_space(dir.path())? >= needed + reserve;
/// # let _ = fits;
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn free_space(dir: impl AsRef<Path>) -> Result<u64> {
    sys::free_space(dir.as_ref())
}
