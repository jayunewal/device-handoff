// SPDX-License-Identifier: Apache-2.0

//! File helpers that keep Baton's state intact and private.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

use crate::error::{Error, Result};

/// Largest state file Baton will read, to bound memory on a damaged or hostile file.
pub(crate) const MAX_STATE_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Creates `dir` and its parents. On Unix the directory is made readable by its owner only.
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(Error::io("create", dir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(Error::io("restrict", dir))?;
    }
    Ok(())
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

/// Reads a small text file, returning `None` if it does not exist.
pub(crate) fn read_text(path: &Path) -> Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io("open", path)(e)),
    };
    let mut text = String::new();
    file.take(MAX_STATE_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidData => Error::invalid(path, "not valid UTF-8 text"),
            _ => Error::io("read", path)(e),
        })?;
    if text.len() as u64 > MAX_STATE_FILE_BYTES {
        return Err(Error::invalid(path, "file is unreasonably large"));
    }
    Ok(Some(text))
}

/// Fails if `path` can be read or written by anyone but its owner. Does nothing off Unix.
pub(crate) fn require_owner_only(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(Error::io("inspect", path))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(Error::InsecurePermissions {
                path: path.to_owned(),
                mode: mode & 0o777,
            });
        }
    }
    #[cfg(not(unix))]
    let _ = path;
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
    #[test]
    fn written_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret");
        write_atomic(&path, b"x").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(require_owner_only(&path).is_ok());

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            require_owner_only(&path),
            Err(Error::InsecurePermissions { mode: 0o644, .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn private_dirs_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        create_private_dir(&nested).unwrap();
        assert_eq!(
            fs::metadata(&nested).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn read_text_distinguishes_missing_from_broken() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_text(&dir.path().join("nope")).unwrap(), None);

        let binary = dir.path().join("binary");
        fs::write(&binary, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(matches!(read_text(&binary), Err(Error::Invalid { .. })));
    }

    #[test]
    fn read_text_refuses_huge_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_STATE_FILE_BYTES + 1).unwrap();
        assert!(matches!(read_text(&path), Err(Error::Invalid { .. })));
    }
}
