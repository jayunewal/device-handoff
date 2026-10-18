// SPDX-License-Identifier: Apache-2.0

//! Marking a file as one that came from elsewhere.

pub(crate) use imp::mark_received;

/// The value of the extended attribute `com.apple.quarantine`: the flags (`0083`: a download that
/// the person has not approved yet), the time in seconds since 1970 in hexadecimal, and the name of
/// the program that got the file. The fourth field, the id of an entry in the system's record of
/// downloads, is left out, since Baton makes no entry there.
//
// Only the Mac uses it, and the tests check it on every system.
#[cfg(any(target_os = "macos", test))]
fn quarantine_value(seconds: u64) -> String {
    format!("0083;{seconds:x};Baton;")
}

/// What Windows keeps in the `Zone.Identifier` stream of a file that came from the Internet: zone
/// 3. Browsers add where the file came from, and Baton has no web address to give.
//
// Only Windows uses it, and the tests check it on every system.
#[cfg(any(windows, test))]
const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";

/// The Mac marks files with an extended attribute.
#[cfg(target_os = "macos")]
mod imp {
    use std::{
        io::{Error, Result},
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{super::posix, quarantine_value};

    /// The name of the attribute, as a C string.
    const QUARANTINE: &[u8] = b"com.apple.quarantine\0";

    /// `setxattr(file, "com.apple.quarantine", value, XATTR_NOFOLLOW)`. A mark that is there
    /// already is replaced by this one.
    pub(crate) fn mark_received(file: &Path) -> Result<()> {
        let file = posix::c_string(file)?;
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let value = quarantine_value(seconds);
        // SAFETY: `file` and `QUARANTINE` are NUL-terminated strings, and `value` is a buffer of
        // the length that is passed. All three are alive until the call returns, and the call
        // reads them and nothing else. The position is 0, as it must be for any attribute but the
        // resource fork, and `XATTR_NOFOLLOW` marks a link itself and not what it points to.
        let returned = unsafe {
            libc::setxattr(
                file.as_ptr(),
                QUARANTINE.as_ptr().cast(),
                value.as_ptr().cast(),
                value.len(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        if returned == 0 {
            Ok(())
        } else {
            Err(Error::last_os_error())
        }
    }
}

/// Windows marks a file with a stream of the file system, which has the name of the file and
/// `:Zone.Identifier`. It is made by writing to that name, which the standard library can do.
#[cfg(windows)]
mod imp {
    use std::{fs, io::Result, path::Path};

    use super::ZONE_IDENTIFIER;

    pub(crate) fn mark_received(file: &Path) -> Result<()> {
        // Writing to the stream of a name that does not exist makes the file, empty. A file that
        // was there a moment ago and is gone now is the caller's affair, but a file that was never
        // there is an error, and makes nothing.
        fs::symlink_metadata(file)?;
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        fs::write(stream, ZONE_IDENTIFIER)
    }
}

/// The other systems have no such mark: iOS and iPadOS do not quarantine what an app writes, and
/// Linux and Android have nothing that other programs look at. The path is still looked at, so that
/// a path with nothing at it is an error on every system and not only on two.
#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use std::{fs, io::Result, path::Path};

    pub(crate) fn mark_received(file: &Path) -> Result<()> {
        fs::symlink_metadata(file)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ZONE_IDENTIFIER, quarantine_value};

    #[test]
    fn the_quarantine_value_has_the_flags_the_time_and_the_name_of_the_program() {
        assert_eq!(quarantine_value(0), "0083;0;Baton;");
        assert_eq!(quarantine_value(0x5f0e_4b2a), "0083;5f0e4b2a;Baton;");
        // Midnight at the start of 2026-10-15 (UTC), in the form the system writes it: lower case,
        // and no leading zeros.
        assert_eq!(quarantine_value(1_792_022_400), "0083;6ad01780;Baton;");
    }

    #[test]
    fn the_zone_stream_is_what_windows_writes_for_a_file_from_the_internet() {
        assert_eq!(ZONE_IDENTIFIER, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
}
