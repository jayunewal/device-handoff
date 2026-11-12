// SPDX-License-Identifier: Apache-2.0

//! The folder that holds the program's own files: its key, the devices paired with it and its
//! settings.
//!
//! It is the program's alone. The Baton app has a folder of its own, in its sandbox, and the two
//! share nothing (`docs/adr/0019-baton-at-the-login-window.md`): they are two devices, with two
//! keys and two lists.

use std::path::{Path, PathBuf};

use baton_core::DataDir;

use crate::{Error, ErrorKind, Result};

/// Where the installed program keeps its files on a Mac. The install script makes the folders
/// above it, owned by the program's own user and writable by no one else.
#[cfg(target_os = "macos")]
const MAC_FOLDER: &str = "/Library/Application Support/Baton/LoginWindow/state";

/// The folder of the program, opened: it is there, it is the owner's alone, and nothing has been
/// read from it yet.
#[derive(Debug, Clone)]
pub struct State {
    dir: DataDir,
}

impl State {
    /// The folder that the installed program uses on this computer. There is one only on a Mac:
    /// anywhere else the program is run for tests, and the folder is given.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            Some(PathBuf::from(MAC_FOLDER))
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    /// Opens the folder `path`, or the default one, and makes it if it is not there.
    ///
    /// A folder that is made here can be used by its owner alone. One that is there already is
    /// refused if anyone else can read or change it: it holds the key of the program and a list
    /// of the devices that may open the login window.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Folder`] if there is no folder to use, it cannot be made, or other users can
    /// read or change it.
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let path = path.or_else(Self::default_path).ok_or_else(|| {
            Error::new(
                ErrorKind::Folder,
                "There is no folder to keep this program's files in on this computer. Give one \
                 with --state-dir.",
            )
        })?;
        let dir = DataDir::open(&path).map_err(|error| {
            Error::new(
                ErrorKind::Folder,
                format!("The folder {} cannot be used: {error}", path.display()),
            )
        })?;
        check_private(&path)?;
        Ok(Self { dir })
    }

    /// The folder.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.root()
    }

    pub(crate) fn dir(&self) -> &DataDir {
        &self.dir
    }
}

/// Refuses a folder that anyone but its owner can read or change.
#[cfg(unix)]
fn check_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .map_err(|error| {
            Error::new(
                ErrorKind::Folder,
                format!("The folder {} cannot be read: {error}", path.display()),
            )
        })?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        return Err(Error::new(
            ErrorKind::Folder,
            format!(
                "The folder {} can be read or changed by other users (its mode is {:o}). It holds \
                 the key of this program and the list of devices that may open the login window, so \
                 it must be for its owner alone. Run `chmod 700` on it and try again.",
                path.display(),
                mode & 0o777
            ),
        ));
    }
    Ok(())
}

/// Off Unix there are no modes to check.
#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn check_private(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_is_made_is_the_owners_alone() {
        let parent = tempfile::tempdir().unwrap();
        let state = State::open(Some(parent.path().join("state"))).unwrap();
        assert!(state.path().is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(state.path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn a_folder_that_is_there_is_used_again_as_it_is() {
        let parent = tempfile::tempdir().unwrap();
        let first = State::open(Some(parent.path().join("state"))).unwrap();
        let second = State::open(Some(first.path().to_owned())).unwrap();
        assert_eq!(first.path(), second.path());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_others_can_read_or_change_is_refused_and_not_changed() {
        use std::os::unix::fs::PermissionsExt;

        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("state");
        std::fs::create_dir(&folder).unwrap();
        for mode in [0o755, 0o750, 0o705, 0o770, 0o777] {
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).unwrap();
            let error = State::open(Some(folder.clone())).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Folder, "mode {mode:o}: {error}");
            assert_eq!(
                std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
                mode,
                "the folder was left as it was"
            );
        }
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        State::open(Some(folder)).unwrap();
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn off_a_mac_the_folder_has_to_be_given() {
        let error = State::open(None).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Folder);
        assert!(error.message().contains("--state-dir"), "{error}");
    }

    #[test]
    fn a_path_that_is_a_file_is_refused() {
        let parent = tempfile::tempdir().unwrap();
        let file = parent.path().join("not-a-folder");
        std::fs::write(&file, b"x").unwrap();
        let error = State::open(Some(file)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Folder, "{error}");
    }
}
