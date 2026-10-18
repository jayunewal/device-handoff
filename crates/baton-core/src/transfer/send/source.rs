// SPDX-License-Identifier: Apache-2.0

//! Opening and reading the files of a transfer.
//!
//! The walk looked at the files once ([`selection`](super::selection)). Reading is the second look,
//! some time later, and what is on the disk then may not be what the walk saw. Every read of a file
//! for a transfer goes through [`Source`], which makes sure of two things.
//!
//! - **A file is a plain file.** A name in the list can have been given to something else since the
//!   walk, and whoever can write in the folder can do that. A link put there would send what it
//!   points to (`~/.ssh`, say), and a pipe would make the read wait for a writer that never comes,
//!   and hold a thread for good. So a file is opened without following a link and without waiting
//!   on a pipe, and it is refused if it is not a plain file. This holds for the last name of the
//!   path, and on Apple systems for every name of it (`O_NOFOLLOW_ANY`), where it costs nothing: the
//!   path is the one the walk resolved. Elsewhere a link in a folder above the file is not looked
//!   for: a chosen item was resolved when it was chosen, and a program that can swap a folder for a
//!   link can write the files in it as well.
//! - **A file is the file the list says.** Its size and its modification time are compared with its
//!   entry in the list when it is opened and after every read (section 8, Rules for the sender),
//!   and a difference fails the file as `Changed`. The comparison is made on the open file and
//!   not on its name, so a file that is replaced by another under the same name is not read as
//!   if it were the old one: the open file stays the old one, which is not changed, and the next
//!   attempt finds the new one different from the list.
//!
//! Every failure comes out as the [`SkipReason`] that the file is skipped with, since a file that
//! fails fails alone.

#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "used by the lanes that write these frames, which come next"
    )
)]

use std::{
    fs::{self, File},
    io,
    path::Path,
};

use baton_proto::transfer::{hash::SPAN_LEN, lane::SkipReason, list::ListEntry};

use super::selection::modified_ms;

/// A file that is open to be sent, and what the list says it is.
#[derive(Debug)]
pub(crate) struct Source {
    file: File,
    size: u64,
    modified_ms: Option<i64>,
}

impl Source {
    /// Opens `path`, which the walk found for `entry`, and checks that it is what `entry` says.
    pub(crate) fn open(path: &Path, entry: &ListEntry) -> Result<Self, SkipReason> {
        let source = Self {
            file: open_plain(path).map_err(|error| why(&error))?,
            size: entry.size,
            modified_ms: entry.modified_ms,
        };
        source.check()?;
        Ok(source)
    }

    /// Reads span `index` into `into`, which is exactly as long as the span, and checks the file
    /// again.
    pub(crate) fn read_span(&self, index: u32, into: &mut [u8]) -> Result<(), SkipReason> {
        let offset = u64::from(index) * SPAN_LEN;
        read_exact_at(&self.file, into, offset).map_err(|error| match error.kind() {
            // The file has fewer bytes than the list says: it was cut short.
            io::ErrorKind::UnexpectedEof => SkipReason::Changed,
            _ => why(&error),
        })?;
        self.check()
    }

    /// Whether the open file is still what the list says.
    pub(crate) fn check(&self) -> Result<(), SkipReason> {
        let metadata = self.file.metadata().map_err(|error| why(&error))?;
        if metadata.len() == self.size && modified_ms(&metadata) == self.modified_ms {
            Ok(())
        } else {
            Err(SkipReason::Changed)
        }
    }
}

/// What a file is skipped with when the system says `error`.
fn why(error: &io::Error) -> SkipReason {
    match error.kind() {
        io::ErrorKind::NotFound => SkipReason::Missing,
        _ => SkipReason::Unreadable,
    }
}

/// Opens a file to read, without following a link and without waiting on a pipe, and refuses
/// anything that is not a plain file.
#[cfg(unix)]
fn open_plain(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(NO_LINKS | libc::O_NONBLOCK)
        .open(path)?;
    plain(file)
}

/// The flag that makes `open` refuse a link. On Apple systems it is the one that covers every name
/// of the path and not only the last.
#[cfg(target_vendor = "apple")]
const NO_LINKS: libc::c_int = libc::O_NOFOLLOW_ANY;

#[cfg(all(unix, not(target_vendor = "apple")))]
const NO_LINKS: libc::c_int = libc::O_NOFOLLOW;

/// Opens a file to read, without following a link, and refuses anything that is not a plain file.
#[cfg(windows)]
fn open_plain(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    // `FILE_FLAG_OPEN_REPARSE_POINT`: open a link itself and not what it points to, which `plain`
    // then refuses.
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)?;
    plain(file)
}

#[cfg(not(any(unix, windows)))]
fn open_plain(path: &Path) -> io::Result<File> {
    plain(File::open(path)?)
}

fn plain(file: File) -> io::Result<File> {
    if file.metadata()?.is_file() {
        Ok(file)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a plain file",
        ))
    }
}

/// Reads at `offset` without moving a position that the file has, so that lanes can read different
/// spans of one open file at the same time.
#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(not(any(unix, windows)))]
fn read_at(_file: &File, _buf: &mut [u8], _offset: u64) -> io::Result<usize> {
    Err(io::ErrorKind::Unsupported.into())
}

fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        match read_at(file, buf, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => {
                buf = buf.split_at_mut(read).1;
                offset += u64::try_from(read).unwrap_or(u64::MAX);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        io::Write,
        path::PathBuf,
        time::{Duration, SystemTime},
    };

    use baton_proto::transfer::hash::{FileHasher, hash_span, span_count, span_size};

    use super::*;
    use crate::transfer::testing::bytes;

    /// A folder for a test, and its path with every link in it resolved. On a Mac the temporary
    /// folder is behind a link (`/var` is `/private/var`), and a file is opened by the path the walk
    /// resolved, so the tests do the same.
    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap();
        (dir, path)
    }

    /// The entry the walk would make for the file at `path`.
    fn entry_of(path: &Path) -> ListEntry {
        let metadata = fs::metadata(path).unwrap();
        ListEntry {
            path: "f".to_owned(),
            folder: false,
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        }
    }

    fn written(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, data).unwrap();
        path
    }

    /// Moves the modification time of `path` on by a second.
    fn touch(path: &Path) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        let now = fs::metadata(path).unwrap().modified().unwrap();
        file.set_modified(now + Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn a_plain_file_is_opened_and_read_span_by_span() {
        let (_scratch, dir) = scratch();
        for size in [
            1_u64,
            1023,
            SPAN_LEN - 1,
            SPAN_LEN,
            SPAN_LEN + 1,
            2 * SPAN_LEN + 5,
        ] {
            let data = bytes(7, usize::try_from(size).unwrap());
            let path = written(&dir, "plain", &data);
            let source = Source::open(&path, &entry_of(&path)).unwrap();

            let mut hasher = FileHasher::new(size).unwrap();
            let mut read = BTreeMap::new();
            // The last span first, as lanes do not read in order.
            let spans = u32::try_from(span_count(size)).unwrap();
            for index in (0..spans).rev() {
                let mut span = vec![0; usize::try_from(span_size(size, index).unwrap()).unwrap()];
                source.read_span(index, &mut span).unwrap();
                hasher
                    .insert(index, hash_span(size, index, &span).unwrap())
                    .unwrap();
                read.insert(index, span);
            }
            let read: Vec<u8> = read.into_values().flatten().collect();
            assert_eq!(read, data, "{size}");
            assert_eq!(hasher.finish().unwrap(), blake3::hash(&data), "{size}");
        }
    }

    #[test]
    fn an_empty_file_is_opened_and_has_nothing_to_read() {
        let (_scratch, dir) = scratch();
        let path = written(&dir, "empty", b"");
        let source = Source::open(&path, &entry_of(&path)).unwrap();
        assert_eq!(source.check(), Ok(()));
    }

    #[test]
    fn a_file_that_is_gone_is_missing() {
        let (_scratch, dir) = scratch();
        let path = written(&dir, "gone", b"here");
        let entry = entry_of(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(
            Source::open(&path, &entry).unwrap_err(),
            SkipReason::Missing
        );
    }

    #[test]
    fn a_file_whose_size_or_time_is_not_the_lists_is_changed_when_it_is_opened() {
        let (_scratch, dir) = scratch();
        let path = written(&dir, "f", b"as it was");
        let entry = entry_of(&path);

        touch(&path);
        assert_eq!(
            Source::open(&path, &entry).unwrap_err(),
            SkipReason::Changed
        );

        let mut again = entry_of(&path);
        again.size += 1;
        assert_eq!(
            Source::open(&path, &again).unwrap_err(),
            SkipReason::Changed
        );
        assert!(Source::open(&path, &entry_of(&path)).is_ok());
    }

    #[test]
    fn a_file_that_changes_while_it_is_read_is_changed() {
        let (_scratch, dir) = scratch();
        let data = bytes(1, usize::try_from(2 * SPAN_LEN).unwrap());
        let path = written(&dir, "f", &data);
        let source = Source::open(&path, &entry_of(&path)).unwrap();
        let mut span = vec![0; usize::try_from(SPAN_LEN).unwrap()];
        source.read_span(0, &mut span).unwrap();

        // The same size and a new time, as an editor leaves a file.
        touch(&path);
        assert_eq!(source.read_span(1, &mut span), Err(SkipReason::Changed));
        assert_eq!(source.check(), Err(SkipReason::Changed));
    }

    #[test]
    fn a_file_that_is_cut_short_while_it_is_read_is_changed() {
        let (_scratch, dir) = scratch();
        let data = bytes(1, usize::try_from(2 * SPAN_LEN).unwrap());
        let path = written(&dir, "f", &data);
        let source = Source::open(&path, &entry_of(&path)).unwrap();

        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(SPAN_LEN + 10).unwrap();
        let mut span = vec![0; usize::try_from(SPAN_LEN).unwrap()];
        assert_eq!(source.read_span(1, &mut span), Err(SkipReason::Changed));
    }

    #[test]
    fn a_file_that_grows_while_it_is_read_is_changed() {
        let (_scratch, dir) = scratch();
        let path = written(&dir, "f", b"short");
        let source = Source::open(&path, &entry_of(&path)).unwrap();

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b" and longer").unwrap();
        let mut span = vec![0; 5];
        assert_eq!(source.read_span(0, &mut span), Err(SkipReason::Changed));
    }

    #[test]
    fn a_folder_is_not_a_file() {
        let (_scratch, dir) = scratch();
        let entry = ListEntry {
            path: "d".to_owned(),
            folder: false,
            size: 0,
            modified_ms: modified_ms(&fs::metadata(&dir).unwrap()),
        };
        assert_eq!(
            Source::open(&dir, &entry).unwrap_err(),
            SkipReason::Unreadable
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_took_the_place_of_a_file_is_not_followed() {
        let (_scratch, dir) = scratch();
        let secret = written(&dir, "secret", b"not for sending");
        let path = written(&dir, "photo", b"a photo");
        let entry = entry_of(&secret);
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&secret, &path).unwrap();

        // Whatever the entry says, the link itself is not opened as a file.
        assert_eq!(
            Source::open(&path, &entry).unwrap_err(),
            SkipReason::Unreadable
        );
    }

    // Apple systems refuse a link anywhere on the path, and not only as the last name.
    #[cfg(target_vendor = "apple")]
    #[test]
    fn a_link_that_took_the_place_of_a_folder_above_the_file_is_not_followed_on_apple_systems() {
        let (_scratch, dir) = scratch();
        let elsewhere = dir.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let file = written(&elsewhere, "f", b"not for sending");
        let entry = entry_of(&file);
        std::os::unix::fs::symlink(&elsewhere, dir.join("folder")).unwrap();

        assert_eq!(
            Source::open(&dir.join("folder").join("f"), &entry).unwrap_err(),
            SkipReason::Unreadable
        );
        assert!(Source::open(&file, &entry).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_that_took_the_place_of_a_file_does_not_hold_the_read() {
        let (_scratch, dir) = scratch();
        let path = dir.join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        let entry = ListEntry {
            path: "fifo".to_owned(),
            folder: false,
            size: 0,
            modified_ms: None,
        };

        // Without the flag that keeps the open from waiting, this call never returns.
        assert_eq!(
            Source::open(&path, &entry).unwrap_err(),
            SkipReason::Unreadable
        );
    }

    #[test]
    fn the_time_of_a_file_is_compared_the_way_the_walk_read_it() {
        let (_scratch, dir) = scratch();
        let path = written(&dir, "f", b"x");
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        // A time before 1970 is negative in the list.
        file.set_modified(SystemTime::UNIX_EPOCH - Duration::from_hours(24))
            .unwrap();
        let entry = entry_of(&path);
        assert_eq!(entry.modified_ms, Some(-86_400_000));
        assert!(Source::open(&path, &entry).is_ok());
    }
}
