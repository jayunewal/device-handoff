// SPDX-License-Identifier: Apache-2.0

//! File helpers that keep Baton's state intact and private.

use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::Path,
};

use crate::error::{Error, Result};

/// Largest state file Baton will read, to bound memory on a damaged or hostile file.
pub(crate) const MAX_STATE_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Opens the state directory `dir`, creating it (and its parents) if it is missing.
///
/// A directory made here is private to its owner from the moment it exists. One that already
/// exists is left exactly as it is, because it may be somewhere else's, as with
/// `BATON_HOME=$HOME`. It is refused if other users can write to it, since they could replace
/// the files inside whatever the modes of those files are. Off Unix the mode checks are skipped.
pub(crate) fn open_state_dir(dir: &Path) -> Result<()> {
    if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(Error::io("create", parent))?;
    }
    match make_private_dir(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => check_existing_dir(dir),
        Err(e) => Err(Error::io("create", dir)(e)),
    }
}

#[cfg(unix)]
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(dir)
}

#[cfg(not(unix))]
fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir(dir)
}

fn check_existing_dir(dir: &Path) -> Result<()> {
    let meta = fs::metadata(dir).map_err(Error::io("inspect", dir))?;
    if !meta.is_dir() {
        return Err(Error::io("use", dir)(std::io::Error::new(
            ErrorKind::NotADirectory,
            "not a directory",
        )));
    }
    check_not_writable_by_others(&meta, dir)
}

/// Replaces the contents of `path` so that a crash leaves either the old or the new file, never half of one.
///
/// The data is written to a temporary file in the same directory, flushed to disk, then renamed over
/// the target. The file is readable by its owner only.
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(Error::io("create", dir))?;
    temp.write_all(contents).map_err(Error::io("write", path))?;
    temp.as_file()
        .sync_all()
        .map_err(Error::io("flush", path))?;
    temp.persist(path)
        .map_err(|e| Error::io("replace", path)(e.error))?;
    sync_dir(dir);
    Ok(())
}

/// Makes the rename durable. Best effort: not every platform or filesystem supports it.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Who else on this machine may touch a state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// Nobody: other users can neither read nor change it. For secrets, and for the list of
    /// paired devices, which decides who may connect.
    OwnerOnly,
    /// Other users may read it but not change it. For settings that a person edits by hand,
    /// where a text editor is likely to leave the file readable.
    OthersMayRead,
}

/// Reads a small text file, returning `None` if it does not exist.
///
/// On Unix the file is refused, before any of it is read, if other users have more access to it
/// than `access` allows. The check is made on the file that is then read, so nothing can be
/// swapped in between.
pub(crate) fn read_text(path: &Path, access: Access) -> Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io("open", path)(e)),
    };
    let meta = file.metadata().map_err(Error::io("inspect", path))?;
    match access {
        Access::OwnerOnly => check_owner_only(&meta, path)?,
        Access::OthersMayRead => check_not_writable_by_others(&meta, path)?,
    }
    let mut text = String::new();
    file.take(MAX_STATE_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| match e.kind() {
            ErrorKind::InvalidData => Error::invalid(path, "not valid UTF-8 text"),
            _ => Error::io("read", path)(e),
        })?;
    if text.len() as u64 > MAX_STATE_FILE_BYTES {
        return Err(Error::invalid(path, "file is unreasonably large"));
    }
    Ok(Some(text))
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg_attr(
    not(unix),
    expect(
        clippy::unnecessary_wraps,
        reason = "there is no mode to check off Unix, but the callers are the same"
    )
)]
fn check_owner_only(meta: &fs::Metadata, path: &Path) -> Result<()> {
    #[cfg(unix)]
    if mode_of(meta) & 0o077 != 0 {
        return Err(Error::InsecurePermissions {
            path: path.to_owned(),
            mode: mode_of(meta),
        });
    }
    #[cfg(not(unix))]
    let _ = (meta, path);
    Ok(())
}

#[cfg_attr(
    not(unix),
    expect(
        clippy::unnecessary_wraps,
        reason = "there is no mode to check off Unix, but the callers are the same"
    )
)]
fn check_not_writable_by_others(meta: &fs::Metadata, path: &Path) -> Result<()> {
    #[cfg(unix)]
    if mode_of(meta) & 0o022 != 0 {
        return Err(Error::WritableByOthers {
            path: path.to_owned(),
            mode: mode_of(meta),
        });
    }
    #[cfg(not(unix))]
    let _ = (meta, path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_creates_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, b"one").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        write_atomic(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
    }

    #[test]
    fn write_atomic_leaves_no_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        for i in 0..5 {
            write_atomic(&path, format!("{i}").as_bytes()).unwrap();
        }
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["state.json"]);
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[cfg(unix)]
    #[test]
    fn written_files_are_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        write_atomic(&path, b"x").unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(
            read_text(&path, Access::OwnerOnly).unwrap().as_deref(),
            Some("x")
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_files_are_refused_if_others_can_read_or_change_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        write_atomic(&path, b"x").unwrap();
        for bad in [0o644, 0o640, 0o604, 0o620, 0o602, 0o666, 0o777] {
            set_mode(&path, bad);
            assert!(
                matches!(
                    read_text(&path, Access::OwnerOnly),
                    Err(Error::InsecurePermissions { mode, .. }) if mode == bad
                ),
                "{bad:o}"
            );
        }
        // Owner-only means what it says, including a file the owner cannot write.
        set_mode(&path, 0o400);
        assert!(read_text(&path, Access::OwnerOnly).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn files_others_may_read_are_refused_only_if_others_can_change_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings");
        write_atomic(&path, b"x").unwrap();
        for ok in [0o600, 0o644, 0o640, 0o444] {
            set_mode(&path, ok);
            assert!(read_text(&path, Access::OthersMayRead).is_ok(), "{ok:o}");
        }
        for bad in [0o664, 0o646, 0o666, 0o620, 0o602] {
            set_mode(&path, bad);
            assert!(
                matches!(
                    read_text(&path, Access::OthersMayRead),
                    Err(Error::WritableByOthers { mode, .. }) if mode == bad
                ),
                "{bad:o}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_state_directory_that_is_created_is_private_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        open_state_dir(&nested).unwrap();
        assert_eq!(mode(&nested), 0o700);
        // Only the directory itself is Baton's; the parents are made the ordinary way.
        assert_ne!(mode(&dir.path().join("a")), 0o700);
        // Opening it again changes nothing.
        open_state_dir(&nested).unwrap();
        assert_eq!(mode(&nested), 0o700);
    }

    #[cfg(unix)]
    #[test]
    fn a_state_directory_that_exists_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        for (name, kept) in [("shared", 0o755), ("group", 0o750), ("open_to_read", 0o705)] {
            let path = dir.path().join(name);
            fs::create_dir(&path).unwrap();
            set_mode(&path, kept);
            open_state_dir(&path).unwrap();
            assert_eq!(mode(&path), kept, "{name} must not be re-moded");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_state_directory_that_others_can_write_to_is_refused_and_not_changed() {
        let dir = tempfile::tempdir().unwrap();
        // 0o1777 is how /tmp looks: re-moding it would be a disaster.
        for bad in [0o770, 0o707, 0o777, 0o1777] {
            let path = dir.path().join(format!("d{bad:o}"));
            fs::create_dir(&path).unwrap();
            set_mode(&path, bad);
            let err = open_state_dir(&path).unwrap_err();
            assert!(
                matches!(err, Error::WritableByOthers { .. }),
                "{bad:o}: {err}"
            );
            assert_eq!(mode(&path), bad, "must not be changed");
        }
    }

    #[test]
    fn a_file_where_the_state_directory_should_be_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        fs::write(&path, "x").unwrap();
        let err = open_state_dir(&path).unwrap_err();
        assert!(matches!(err, Error::Io { .. }), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_private_directory_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        open_state_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        open_state_dir(&link).unwrap();
        assert_eq!(mode(&real), 0o700);
    }

    #[test]
    fn read_text_distinguishes_missing_from_broken() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_text(&dir.path().join("nope"), Access::OthersMayRead).unwrap(),
            None
        );

        let binary = dir.path().join("binary");
        fs::write(&binary, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(matches!(
            read_text(&binary, Access::OthersMayRead),
            Err(Error::Invalid { .. })
        ));
    }

    #[test]
    fn read_text_refuses_huge_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_STATE_FILE_BYTES + 1).unwrap();
        assert!(matches!(
            read_text(&path, Access::OthersMayRead),
            Err(Error::Invalid { .. })
        ));
    }
}
