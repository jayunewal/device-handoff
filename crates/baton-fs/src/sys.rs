// SPDX-License-Identifier: Apache-2.0

//! The system calls: one short function for each platform.
//!
//! Each function hands a path (two, for a move) or an open file to one system call and turns the
//! answer into an `io::Error`. That is all that `unsafe` is used for in this crate, and each
//! block says why it is sound. The move is in this file. The other calls have a module each, next
//! to it: `blocking`, `hide`, `mark` and `space`.

use std::{
    io::{Error, ErrorKind, Result},
    path::Path,
};

mod blocking;
mod hide;
mod mark;
mod space;

pub(crate) use blocking::clear_nonblocking;
pub(crate) use hide::hide;
pub(crate) use imp::rename_noreplace;
pub(crate) use mark::mark_received;
pub(crate) use space::free_space;

/// What a caller is told when the system or the volume cannot move without replacing. `why` is
/// what the system said, so that a log can tell the causes apart.
//
// Which platforms use this helper and the next one differs, so a platform that has no use for
// one of them must not warn about it.
#[allow(dead_code)]
fn unsupported(why: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        format!("this system or volume cannot move without replacing what has the new name: {why}"),
    )
}

/// What a caller is told when a path has a NUL character in it, which no system call can take.
#[allow(dead_code)]
fn nul_in_path() -> Error {
    Error::new(ErrorKind::InvalidInput, "a path has a NUL character in it")
}

/// The calls of the systems that have the same two paths and the same kind of answer.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
mod posix {
    use std::{ffi::CString, io::Error, io::Result, os::unix::ffi::OsStrExt, path::Path};

    /// The path as the C string that a system call takes.
    pub(super) fn c_string(path: &Path) -> Result<CString> {
        CString::new(path.as_os_str().as_bytes()).map_err(|_| super::nul_in_path())
    }

    /// Tells a system that cannot do the call from a path that is wrong.
    ///
    /// `ENOSYS` is a kernel without the call (or a sandbox that filters it), and `ENOTSUP` and
    /// `EOPNOTSUPP` are what macOS says for a volume that cannot. They are not mistakes of the
    /// caller, and they are told apart from the errors that are, such as `EEXIST`.
    ///
    /// `EINVAL` is what Linux says for flags that a file system does not know, and also for a
    /// folder moved into a folder inside itself. The two cannot be told apart, so it is
    /// `Unsupported` there, with the system's words in the message. On Apple systems it is
    /// only the second kind (or `.` and `..`), since they say `ENOTSUP` for a volume that
    /// cannot, so it is left as it is and std calls it `InvalidInput`.
    pub(super) fn classify(error: Error) -> Error {
        let cannot: &[libc::c_int] = if cfg!(target_vendor = "apple") {
            &[libc::ENOSYS, libc::ENOTSUP, libc::EOPNOTSUPP]
        } else {
            &[libc::EINVAL, libc::ENOSYS, libc::ENOTSUP, libc::EOPNOTSUPP]
        };
        match error.raw_os_error() {
            Some(code) if cannot.contains(&code) => super::unsupported(error),
            _ => error,
        }
    }

    /// The call's answer, as the `Result` of a function that returns 0 or -1.
    pub(super) fn answer(returned: libc::c_long) -> Result<()> {
        if returned == 0 {
            Ok(())
        } else {
            Err(classify(Error::last_os_error()))
        }
    }

    #[cfg(test)]
    mod tests {
        use std::io::{Error, ErrorKind};

        use super::classify;

        #[test]
        fn a_volume_that_cannot_is_told_apart_from_a_path_that_is_wrong() {
            for code in [libc::ENOSYS, libc::ENOTSUP, libc::EOPNOTSUPP] {
                let system = Error::from_raw_os_error(code);
                let words = system.to_string();
                let error = classify(system);
                assert_eq!(error.kind(), ErrorKind::Unsupported, "{code}");
                assert!(
                    error.to_string().contains(&words),
                    "the system's words: {error}"
                );
            }
            // The one code that means a flag that is not known on Linux, and a mistake of the
            // caller on Apple systems.
            let einval = classify(Error::from_raw_os_error(libc::EINVAL));
            if cfg!(target_vendor = "apple") {
                assert_eq!(einval.kind(), ErrorKind::InvalidInput);
                assert_eq!(einval.raw_os_error(), Some(libc::EINVAL));
            } else {
                assert_eq!(einval.kind(), ErrorKind::Unsupported);
                assert!(einval.to_string().contains("(os error 22)"), "{einval}");
            }
            for (code, kind) in [
                (libc::EEXIST, ErrorKind::AlreadyExists),
                (libc::ENOENT, ErrorKind::NotFound),
                (libc::EXDEV, ErrorKind::CrossesDevices),
                (libc::EACCES, ErrorKind::PermissionDenied),
            ] {
                let error = classify(Error::from_raw_os_error(code));
                assert_eq!(error.kind(), kind, "{code}");
                assert_eq!(error.raw_os_error(), Some(code));
            }
        }
    }
}

/// The part of Windows path handling that is only text, kept apart so that it is tested on every
/// platform.
#[cfg(any(windows, test))]
mod long_path {
    /// The length from which a path gets the `\\?\` prefix. The system's limit for a path is 260
    /// characters with the NUL, and the prefix lifts it. 248 is what the standard library uses
    /// for a folder, which a file name has to fit into.
    const LONG: usize = 248;

    /// The path (as UTF-16, without the NUL) in the form that Win32 takes: the same path, with
    /// the prefix when it is long.
    pub(super) fn for_win32(wide: Vec<u16>) -> Vec<u16> {
        if wide.len() >= LONG {
            verbatim(wide)
        } else {
            wide
        }
    }

    /// The same path with the `\\?\` prefix, unless it has a prefix of this kind already.
    fn verbatim(wide: Vec<u16>) -> Vec<u16> {
        let text = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        if wide.starts_with(&text(r"\\?\")) || wide.starts_with(&text(r"\\.\")) {
            return wide;
        }
        let mut out;
        if wide.starts_with(&text(r"\\")) {
            // `\\server\share\folder` becomes `\\?\UNC\server\share\folder`.
            out = text(r"\\?\UNC\");
            out.extend_from_slice(wide.get(2..).unwrap_or_default());
        } else {
            // `C:\folder` becomes `\\?\C:\folder`.
            out = text(r"\\?\");
            out.extend_from_slice(&wide);
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::{LONG, for_win32, verbatim};

        fn units(s: &str) -> Vec<u16> {
            s.encode_utf16().collect()
        }

        #[test]
        fn a_long_path_gets_the_prefix_that_suits_its_kind() {
            assert_eq!(verbatim(units(r"C:\a\b")), units(r"\\?\C:\a\b"));
            assert_eq!(
                verbatim(units(r"\\server\share\a")),
                units(r"\\?\UNC\server\share\a")
            );
        }

        #[test]
        fn a_path_with_a_prefix_is_left_as_it_is() {
            for path in [r"\\?\C:\a\b", r"\\?\UNC\server\share\a", r"\\.\pipe\x"] {
                assert_eq!(verbatim(units(path)), units(path));
            }
        }

        #[test]
        fn only_a_long_path_gets_a_prefix() {
            let path = |len: usize| units(&format!(r"C:\{}", "a".repeat(len - 3)));
            assert_eq!(for_win32(path(LONG - 1)), path(LONG - 1));
            let long = for_win32(path(LONG));
            assert!(long.starts_with(&units(r"\\?\C:\")));
            assert_eq!(long.len(), LONG + 4);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod imp {
    use super::{Path, Result, posix};

    /// `AT_FDCWD` and `RENAME_NOREPLACE` as the `long`s that the variadic `syscall` takes. The
    /// flag is a `c_uint` for some C libraries and a `c_int` for others, so it is cast.
    const AT_FDCWD: libc::c_long = libc::AT_FDCWD as libc::c_long;
    const NOREPLACE: libc::c_long = libc::RENAME_NOREPLACE as libc::c_long;

    /// `renameat2(AT_FDCWD, from, AT_FDCWD, to, RENAME_NOREPLACE)`.
    ///
    /// The call is made through `syscall` and not through a function of the C library, which
    /// glibc before 2.28 and Android before API 30 do not have. A kernel before 3.15 does not
    /// have the call either, and answers `ENOSYS`.
    pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
        let from = posix::c_string(from)?;
        let to = posix::c_string(to)?;
        // SAFETY: `from` and `to` are NUL-terminated strings that are alive until the call
        // returns, and the call reads those two strings and nothing else. Every argument of the
        // variadic `syscall` is as wide as a `long` (a pointer is, on every Linux and Android
        // target), which is what the C library passes on to the kernel. The call number and the
        // flag are constants of the system.
        let returned = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                AT_FDCWD,
                from.as_ptr(),
                AT_FDCWD,
                to.as_ptr(),
                NOREPLACE,
            )
        };
        posix::answer(returned)
    }
}

#[cfg(target_vendor = "apple")]
mod imp {
    use super::{Path, Result, posix};

    /// `renamex_np(from, to, RENAME_EXCL)`, which macOS has had since 10.12 and iOS since 10.
    /// A volume that cannot do it (FAT and exFAT, for one) answers `ENOTSUP`.
    pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
        let from = posix::c_string(from)?;
        let to = posix::c_string(to)?;
        // SAFETY: `from` and `to` are NUL-terminated strings that are alive until the call
        // returns, and the call reads those two strings and nothing else. `RENAME_EXCL` is the
        // flag that the system defines for this call.
        let returned = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        posix::answer(libc::c_long::from(returned))
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;

    use super::{Path, Result, long_path::for_win32, nul_in_path};

    // SAFETY: the declaration is the signature that the documentation gives for the function
    // (`BOOL` is a 32-bit integer, `DWORD` a `u32`, and the two names are `LPCWSTR`), and
    // `kernel32` is part of every Windows system.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// <https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-movefileexw>
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    /// `MoveFileExW(from, to, 0)`: no `MOVEFILE_REPLACE_EXISTING`, so a name that is taken is an
    /// error (`ERROR_ALREADY_EXISTS`), and no `MOVEFILE_COPY_ALLOWED`, so a move to another
    /// volume is an error too and is not made by copying.
    pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
        let from = wide(from)?;
        let to = wide(to)?;
        // SAFETY: `from` and `to` are NUL-terminated UTF-16 strings that are alive until the call
        // returns, and the call reads those two strings and nothing else. 0 sets no flag.
        let moved = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
        if moved == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// The path as the UTF-16 string that Win32 takes, with the NUL at the end.
    ///
    /// A path that is long gets the `\\?\` prefix, as the standard library gives one to the
    /// paths that it uses. Win32 takes a path with that prefix literally, so it is made
    /// absolute first, which also takes `.` and `..` out of it.
    ///
    /// The other calls of this crate that take a path on Windows use it too.
    pub(super) fn wide(path: &Path) -> Result<Vec<u16>> {
        let path = std::path::absolute(path)?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(nul_in_path());
        }
        let mut wide = for_win32(wide);
        wide.push(0);
        Ok(wide)
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
mod imp {
    use super::{Path, Result, unsupported};

    /// There is no call to make on this system.
    pub(crate) fn rename_noreplace(_from: &Path, _to: &Path) -> Result<()> {
        Err(unsupported("there is no such call on this system"))
    }
}
