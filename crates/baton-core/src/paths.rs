// SPDX-License-Identifier: Apache-2.0

//! Where Baton keeps its state, and a lock so only one process runs a node with it at a time.

use std::{
    env,
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;

use crate::{
    error::{Error, Result},
    fsutil::open_state_dir,
};

/// Environment variable that overrides where Baton keeps its state.
pub const HOME_ENV: &str = "BATON_HOME";

/// The directory holding one device's identity, trust store, settings and inbox.
///
/// Everything in it belongs to a single device identity, so only one process at a time may
/// run a node with a directory, or change which devices are paired: it takes
/// [`DataDir::lock`] first. Reading the files needs no lock. The inbox is shared on purpose
/// and has a lock of its own, so that a command can read or empty it while the listener adds
/// to it. Tests and people running several instances on one machine give each its own
/// directory.
#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    /// Where state lives by default: `$BATON_HOME` if set, otherwise the platform's
    /// per-user local data directory.
    ///
    /// The device key must never be synced between machines, so on Windows this is
    /// the local (non-roaming) application data folder.
    pub fn default_root() -> Result<PathBuf> {
        if let Some(home) = env::var_os(HOME_ENV).filter(|v| !v.is_empty()) {
            return Ok(PathBuf::from(home));
        }
        ProjectDirs::from("", "", "baton")
            .map(|dirs| dirs.data_local_dir().to_owned())
            .ok_or_else(|| {
                Error::Config(format!(
                    "could not find a home directory; set {HOME_ENV} to say where to keep state"
                ))
            })
    }

    /// Opens the state directory at `root`, creating it if needed.
    ///
    /// A directory created here is private to its owner on Unix. One that already exists is
    /// not changed, since it may be used for other things, but it is refused if other users
    /// can write to it: they could replace the files in it. That includes pointing
    /// `BATON_HOME` at a shared directory such as `/tmp`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        open_state_dir(&root)?;
        Ok(Self { root })
    }

    /// The directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file holding this device's secret key.
    pub fn identity_file(&self) -> PathBuf {
        self.root.join("identity.toml")
    }

    /// The file listing the devices this one is paired with.
    pub fn trust_file(&self) -> PathBuf {
        self.root.join("devices.json")
    }

    /// The settings file.
    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// The file of received items.
    pub fn inbox_file(&self) -> PathBuf {
        self.root.join("inbox.json")
    }

    /// Takes exclusive ownership of the directory for as long as the returned guard lives.
    ///
    /// Fails with [`Error::Locked`] if another process holds it. The operating
    /// system releases the lock if the process dies.
    pub fn lock(&self) -> Result<DirLock> {
        let path = self.root.join("baton.lock");
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Nothing in the file matters, but another user who can open it could hold the lock.
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(Error::io("open", &path))?;
        match file.try_lock() {
            Ok(()) => Ok(DirLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::Locked { path }),
            Err(TryLockError::Error(e)) => Err(Error::io("lock", path)(e)),
        }
    }
}

/// Proof that this process owns a [`DataDir`]. Dropping it releases the directory.
#[derive(Debug)]
pub struct DirLock {
    _file: File,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("a/b/c");
        let dir = DataDir::open(&root).unwrap();
        assert!(root.is_dir());
        assert_eq!(dir.root(), root);
        assert_eq!(dir.trust_file(), root.join("devices.json"));
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_is_created_is_private_and_one_that_exists_is_not_changed() {
        use std::os::unix::fs::PermissionsExt;
        let bits = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
        let tmp = tempfile::tempdir().unwrap();

        let made = tmp.path().join("made");
        DataDir::open(&made).unwrap();
        assert_eq!(bits(&made), 0o700);

        // Something else's directory, such as BATON_HOME=$HOME, keeps its mode.
        let shared = tmp.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
        DataDir::open(&shared).unwrap();
        assert_eq!(bits(&shared), 0o755);

        // One that other users can write to is refused, and left as it was.
        let open = tmp.path().join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let err = DataDir::open(&open).unwrap_err();
        assert!(matches!(err, Error::WritableByOthers { .. }), "{err}");
        assert_eq!(bits(&open), 0o1777);
    }

    #[cfg(unix)]
    #[test]
    fn the_lock_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::open(tmp.path().join("state")).unwrap();
        let _lock = dir.lock().unwrap();
        let mode = std::fs::metadata(dir.root().join("baton.lock"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn the_directory_can_be_locked_by_one_holder_at_a_time() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = DataDir::open(tmp.path()).unwrap();

        let first = dir.lock().unwrap();
        assert!(matches!(dir.lock(), Err(Error::Locked { .. })));
        // A second handle to the same directory is also refused.
        assert!(matches!(
            DataDir::open(tmp.path()).unwrap().lock(),
            Err(Error::Locked { .. })
        ));

        drop(first);
        // The lock belongs to the open file, so a child process that another test is
        // starting holds a copy of it until it runs its program, and the directory can look
        // taken for a moment after the guard is gone. macOS CI showed it with the inbox's
        // lock. Wait that moment out; a lock that never comes free is still a failure.
        let again = (0..200).find_map(|_| {
            dir.lock().ok().or_else(|| {
                std::thread::sleep(std::time::Duration::from_millis(25));
                None
            })
        });
        assert!(again.is_some(), "the lock never came free");
    }
}
