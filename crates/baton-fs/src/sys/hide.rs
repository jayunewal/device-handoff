// SPDX-License-Identifier: Apache-2.0

//! Hiding a file or a folder from the person who looks in the folder it is in.

pub(crate) use imp::hide;

/// A name that starts with a dot is hidden already on every system but Windows (the Finder, the
/// Files app and the file managers of Linux and Android do not show it), so there is nothing to
/// call. The path is still looked at, so that a path with nothing at it is an error on every
/// system and not only on one.
#[cfg(not(windows))]
mod imp {
    use std::{fs, io::Result, path::Path};

    pub(crate) fn hide(path: &Path) -> Result<()> {
        fs::symlink_metadata(path)?;
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::{
        io::{Error, Result},
        path::Path,
    };

    use super::super::imp::wide;

    /// What `GetFileAttributesW` answers when it fails.
    const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
    /// The attribute that Explorer hides a file or a folder by.
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    /// An attribute that is only valid alone, and means that there is no other.
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;

    // SAFETY: the declarations are the signatures that the documentation gives for the functions
    // (`DWORD` is a `u32`, `BOOL` a 32-bit integer, and the name is an `LPCWSTR`), and `kernel32`
    // is part of every Windows system.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// <https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-getfileattributesw>
        fn GetFileAttributesW(name: *const u16) -> u32;
        /// <https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-setfileattributesw>
        fn SetFileAttributesW(name: *const u16, attributes: u32) -> i32;
    }

    /// Adds `FILE_ATTRIBUTE_HIDDEN` to the attributes the path has, and keeps the others.
    pub(crate) fn hide(path: &Path) -> Result<()> {
        let name = wide(path)?;
        // SAFETY: `name` is a NUL-terminated UTF-16 string that is alive until the call returns,
        // and the call reads that string and nothing else.
        let attributes = unsafe { GetFileAttributesW(name.as_ptr()) };
        if attributes == INVALID_FILE_ATTRIBUTES {
            return Err(Error::last_os_error());
        }
        if attributes & FILE_ATTRIBUTE_HIDDEN != 0 {
            return Ok(());
        }
        let hidden = (attributes & !FILE_ATTRIBUTE_NORMAL) | FILE_ATTRIBUTE_HIDDEN;
        // SAFETY: as above; the call reads the string and changes the attributes of the one
        // file or folder it names, which is what it is for.
        let set = unsafe { SetFileAttributesW(name.as_ptr(), hidden) };
        if set == 0 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
