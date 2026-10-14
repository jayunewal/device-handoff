// SPDX-License-Identifier: Apache-2.0

//! Where Baton keeps its state, and a lock so only one process uses it at a time.

use std::{
    env,
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

use directories::ProjectDirs;

use crate::{
    error::{Error, Result},
    fsutil::create_private_dir,
};

/// Environment variable that overrides where Baton keeps its state.
pub const HOME_ENV: &str = "BATON_HOME";

/// The directory holding one device's identity, trust store, settings and inbox.
///
/// Everything in it belongs to a single device identity, so only one running
/// process may use a directory at a time (see [`DataDir::lock`]). Tests and
/// people running several instances on one machine give each its own directory.
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

    /// Opens the state directory at `root`, creating it (owner-only on Unix) if needed.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        create_private_dir(&root)?;
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
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(Error::io("open", &path))?;
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
        assert!(dir.lock().is_ok());
    }
}
