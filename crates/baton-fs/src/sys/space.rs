// SPDX-License-Identifier: Apache-2.0

//! Asking how much room a volume has.

pub(crate) use imp::free_space;

/// Linux and Android: `statvfs`, which gives the number of blocks that a user can still use, and
/// the size of the block that they are counted in.
#[cfg(any(target_os = "linux", target_os = "android"))]
mod imp {
    use std::{
        io::{Error, Result},
        mem::MaybeUninit,
        path::Path,
    };

    use super::super::posix;

    pub(crate) fn free_space(dir: &Path) -> Result<u64> {
        let dir = posix::c_string(dir)?;
        let mut stat = MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `dir` is a NUL-terminated string that is alive until the call returns, and
        // `stat` is room for a `statvfs`, which is all that the call writes to.
        let returned = unsafe { libc::statvfs(dir.as_ptr(), stat.as_mut_ptr()) };
        if returned != 0 {
            return Err(Error::last_os_error());
        }
        // SAFETY: the call returned 0, which means that it filled the struct in.
        let stat = unsafe { stat.assume_init() };
        Ok(widen(stat.f_bavail).saturating_mul(widen(stat.f_frsize)))
    }

    /// A count that the system gives as a `u32` or as a `u64`, depending on the system, as a `u64`.
    fn widen(count: impl Into<u64>) -> u64 {
        count.into()
    }
}

/// macOS, iOS and iPadOS: `statfs`. It counts in 64 bits, where the counts of `statvfs` are 32
/// bits on these systems, which is not enough for a volume of more than 16 TiB.
#[cfg(target_vendor = "apple")]
mod imp {
    use std::{
        io::{Error, Result},
        mem::MaybeUninit,
        path::Path,
    };

    use super::super::posix;

    pub(crate) fn free_space(dir: &Path) -> Result<u64> {
        let dir = posix::c_string(dir)?;
        let mut stat = MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: `dir` is a NUL-terminated string that is alive until the call returns, and
        // `stat` is room for a `statfs`, which is all that the call writes to.
        let returned = unsafe { libc::statfs(dir.as_ptr(), stat.as_mut_ptr()) };
        if returned != 0 {
            return Err(Error::last_os_error());
        }
        // SAFETY: the call returned 0, which means that it filled the struct in.
        let stat = unsafe { stat.assume_init() };
        Ok(stat.f_bavail.saturating_mul(u64::from(stat.f_bsize)))
    }
}

/// Windows: `GetDiskFreeSpaceExW`, which answers for the user who calls it, so a quota is counted.
#[cfg(windows)]
mod imp {
    use std::{
        io::{Error, Result},
        path::Path,
        ptr,
    };

    use super::super::imp::wide;

    /// The backslash, as UTF-16.
    const BACKSLASH: u16 = b'\\' as u16;

    // SAFETY: the declaration is the signature that the documentation gives for the function
    // (`BOOL` is a 32-bit integer, `PULARGE_INTEGER` a pointer to a 64-bit integer, and the name
    // is an `LPCWSTR`), and `kernel32` is part of every Windows system.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// <https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-getdiskfreespaceexw>
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }

    pub(crate) fn free_space(dir: &Path) -> Result<u64> {
        let mut name = wide(dir)?;
        // A name of the form `\\server\share` must end with a backslash, and a folder may.
        name.pop();
        if name.last() != Some(&BACKSLASH) {
            name.push(BACKSLASH);
        }
        name.push(0);
        let mut free = 0_u64;
        // SAFETY: `name` is a NUL-terminated UTF-16 string that is alive until the call returns,
        // and `free` is a 64-bit integer that the call writes to. The other two answers are not
        // wanted, which the call allows by taking null for them.
        let worked = unsafe {
            GetDiskFreeSpaceExW(
                name.as_ptr(),
                &raw mut free,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if worked == 0 {
            Err(Error::last_os_error())
        } else {
            Ok(free)
        }
    }
}

/// Any other system has no call to make.
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
mod imp {
    use std::{
        io::{Error, ErrorKind, Result},
        path::Path,
    };

    pub(crate) fn free_space(_dir: &Path) -> Result<u64> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "there is no call to ask for free space on this system",
        ))
    }
}
