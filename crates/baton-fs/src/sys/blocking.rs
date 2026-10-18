// SPDX-License-Identifier: Apache-2.0

//! Making an open file wait for its data again.

pub(crate) use imp::clear_nonblocking;

/// Unix: `fcntl` reads the flags of the open file, and sets them again without `O_NONBLOCK`.
#[cfg(unix)]
mod imp {
    use std::{
        fs::File,
        io::{Error, Result},
        os::fd::AsRawFd,
    };

    pub(crate) fn clear_nonblocking(file: &File) -> Result<()> {
        let fd = file.as_raw_fd();
        // SAFETY: `fd` is open, because `file` is borrowed until the call returns, and `F_GETFL`
        // takes nothing but the descriptor and reads the flags of the open file.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 {
            return Err(Error::last_os_error());
        }
        if flags & libc::O_NONBLOCK == 0 {
            return Ok(());
        }
        // SAFETY: as above. `F_SETFL` takes the new flags as an `int`. They are the flags that
        // were read, less the one that is wanted off, and the call changes only the flags that
        // can change after an open (the access mode is not one of them).
        let returned = unsafe { libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK) };
        if returned == -1 {
            Err(Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use std::{
            fs::{self, File, OpenOptions},
            io::{Error, Read},
            os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
        };

        use super::clear_nonblocking;

        /// The status flags of the open file, as `fcntl` gives them.
        fn flags(file: &File) -> libc::c_int {
            // SAFETY: `file` is open until the call returns, and `F_GETFL` reads the flags of the
            // open file and nothing else.
            let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
            assert_ne!(flags, -1, "{}", Error::last_os_error());
            flags
        }

        fn written(dir: &tempfile::TempDir) -> std::path::PathBuf {
            let path = dir.path().join("f");
            fs::write(&path, "the bytes").unwrap();
            path
        }

        #[test]
        fn a_file_that_was_opened_without_waiting_waits_after_the_call() {
            let dir = tempfile::tempdir().unwrap();
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(written(&dir))
                .unwrap();
            assert_ne!(
                flags(&file) & libc::O_NONBLOCK,
                0,
                "the flag is set to begin with"
            );

            clear_nonblocking(&file).unwrap();

            assert_eq!(flags(&file) & libc::O_NONBLOCK, 0);
            let mut read = String::new();
            file.read_to_string(&mut read).unwrap();
            assert_eq!(read, "the bytes");
        }

        #[test]
        fn the_other_flags_of_the_open_file_are_kept() {
            let dir = tempfile::tempdir().unwrap();
            let file = OpenOptions::new()
                .append(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(written(&dir))
                .unwrap();
            let before = flags(&file);
            assert_ne!(before & libc::O_APPEND, 0);

            clear_nonblocking(&file).unwrap();

            // The access mode and `O_APPEND` are still there, and the one flag is gone.
            assert_eq!(flags(&file), before & !libc::O_NONBLOCK);
        }

        #[test]
        fn a_file_that_waits_already_is_left_as_it_is() {
            let dir = tempfile::tempdir().unwrap();
            let file = File::open(written(&dir)).unwrap();
            let before = flags(&file);
            assert_eq!(before & libc::O_NONBLOCK, 0);

            clear_nonblocking(&file).unwrap();
            clear_nonblocking(&file).unwrap();

            assert_eq!(flags(&file), before);
        }
    }
}

/// Other systems: a file that the standard library opens has no such flag, so there is nothing
/// to take off.
#[cfg(not(unix))]
mod imp {
    use std::{fs::File, io::Result};

    #[expect(
        clippy::unnecessary_wraps,
        reason = "there is nothing to take off here, but the call is the one that can fail on Unix"
    )]
    pub(crate) fn clear_nonblocking(_file: &File) -> Result<()> {
        Ok(())
    }
}
