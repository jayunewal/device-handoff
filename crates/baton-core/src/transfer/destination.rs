// SPDX-License-Identifier: Apache-2.0

//! The folder that received transfers are put in, and what the volume under it is like.
//!
//! A transfer is received into a **staging folder** on the same volume as the folder it is going
//! to, so that each top-level item can be moved into place in one step that never replaces
//! anything (ADR 0008). The staging folder is a hidden folder inside the destination, named for
//! the device that owns it, and every transfer has a folder of its own in it, which is a
//! [`Staging`]. A person never finds a transfer that is still being received.
//!
//! What a volume does with names decides how the names of a transfer are chosen (see
//! [`names`](super::names)): whether two names that differ only in case are one name, and whether
//! two names that differ only in the form of their Unicode text are. That depends on the volume
//! and not only on the system, so [`Destination::open`] asks the volume, with two files that it
//! makes in the staging folder and removes again.
//!
//! What an earlier run left in the staging folder is removed when the destination is opened. A
//! device runs one process at a time (it holds the lock of its data directory), so what is in its
//! staging folder when it starts is what a run before it left behind. A transfer that was being
//! received then is not resumed: the values that its files are checked against were kept in
//! memory only, so nothing is left to check what is on the disk against. A transfer that is
//! resumed on a new connection within a run (protocol.md section 8, Resuming) keeps its
//! [`Staging`] and does not come here.

use std::{
    fmt, fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use baton_proto::{
    device::Platform,
    id::{DeviceId, TransferId},
    transfer::list::Listing,
};
use data_encoding::HEXLOWER;

use super::{
    names::Rules,
    staging::{self, StageError, Staging},
};
use crate::util::random_bytes;

/// What the name of a device's staging folder starts with. A name that starts with a dot is
/// hidden by the Finder, the Files app and most file managers on Linux and Android.
const STAGING_PREFIX: &str = ".baton-incoming-";

/// Why a destination could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The path is not a folder.
    #[error("the destination is not a folder")]
    NotAFolder,
    /// There is no folder there, or it cannot be written. The text is the system's own and has no
    /// name of a folder in it.
    #[error("the destination cannot be used: {0}")]
    Io(#[from] io::Error),
}

/// Where received transfers are put, and what the volume under it is like.
#[derive(Clone)]
pub struct Destination {
    root: PathBuf,
    staging_root: PathBuf,
    rules: Rules,
}

impl fmt::Debug for Destination {
    // The folder is the person's own, and a log gets pasted into a bug report.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Destination")
            .field("rules", &self.rules)
            .finish_non_exhaustive()
    }
}

impl Destination {
    /// Opens the folder `root`, which must exist, for the device `owner`.
    ///
    /// It makes the staging folder of the device in it if there is none, removes what an earlier
    /// run left there, and finds out what the volume does with names.
    pub fn open(root: impl Into<PathBuf>, owner: &DeviceId) -> Result<Self, OpenError> {
        let root = root.into();
        if !fs::metadata(&root)?.is_dir() {
            return Err(OpenError::NotAFolder);
        }
        let staging_root = root.join(format!("{STAGING_PREFIX}{}", owner.short()));
        match staging::make_dir(&staging_root, true) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // A link in its place would take every transfer somewhere else.
                if !fs::symlink_metadata(&staging_root)?.is_dir() {
                    return Err(OpenError::NotAFolder);
                }
            }
            Err(e) => return Err(e.into()),
        }
        sweep(&staging_root);
        let (ignores_case, ignores_normalisation) = probe(&staging_root)?;
        Ok(Self {
            root,
            staging_root,
            rules: Rules::new(Platform::current(), ignores_case, ignores_normalisation),
        })
    }

    /// The folder that transfers are put in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// What the volume does with names, which is what the names of a transfer are chosen by.
    pub fn rules(&self) -> Rules {
        self.rules
    }

    /// Starts receiving the transfer `id`, whose list has been checked against its offer: makes
    /// the folder of the transfer, and the folders of the list in it.
    pub fn stage(&self, id: TransferId, listing: &Listing) -> Result<Staging, StageError> {
        Staging::create(&self.staging_root, id, listing, self.rules)
    }
}

/// Removes everything in the staging folder, and goes on if something cannot be removed: it is
/// tried again at the next start, and a folder of a transfer that is not there is never used.
fn sweep(staging_root: &Path) {
    let Ok(children) = fs::read_dir(staging_root) else {
        return;
    };
    let mut failed = 0_usize;
    for child in children.flatten() {
        let path = child.path();
        let removed = match child.file_type() {
            Ok(kind) if kind.is_dir() => fs::remove_dir_all(&path),
            _ => fs::remove_file(&path),
        };
        if removed.is_err() {
            failed += 1;
        }
    }
    if failed > 0 {
        tracing::warn!(
            failed,
            "could not remove everything that an earlier run left in the staging folder"
        );
    }
}

/// Asks the volume that `dir` is on, with files that it makes there and removes again, whether
/// names that differ only in case are one name, and whether names that differ only in the form of
/// their Unicode text are.
fn probe(dir: &Path) -> io::Result<(bool, bool)> {
    let tag = HEXLOWER.encode(&random_bytes::<8>().map_err(io::Error::other)?);
    let case = same_name(dir, &format!("Probe-{tag}"), &format!("probe-{tag}"))?;
    // `é` as one character, and as an `e` with an accent after it. A volume that cannot hold such
    // a name at all is taken to treat the two as one, which only ever gives a number to a name
    // that could have kept its own, and never lets two names fail by being one.
    let normalisation =
        same_name(dir, &format!("{tag}-\u{e9}"), &format!("{tag}-e\u{301}")).unwrap_or(true);
    Ok((case, normalisation))
}

/// Makes a file called `made` in `dir`, finds out whether `looked_for` is it, and removes it.
fn same_name(dir: &Path, made: &str, looked_for: &str) -> io::Result<bool> {
    let path = dir.join(made);
    fs::File::create_new(&path)?;
    let found = fs::symlink_metadata(dir.join(looked_for));
    // A file of ours that stays would be removed by the next sweep.
    let _ = fs::remove_file(&path);
    match found {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(byte: u8) -> DeviceId {
        DeviceId::from_bytes([byte; DeviceId::LEN])
    }

    /// What the volume of `dir` does, found out by making two files, as a person would.
    fn observed(dir: &Path) -> (bool, bool) {
        fs::write(dir.join("Observed"), b"x").unwrap();
        let case = dir.join("observed").exists();
        fs::write(dir.join("\u{e9}-observed"), b"x").unwrap();
        let normalisation = dir.join("e\u{301}-observed").exists();
        (case, normalisation)
    }

    #[test]
    fn the_rules_are_what_the_volume_does() {
        let scratch = tempfile::tempdir().unwrap();
        let (case, normalisation) = observed(scratch.path());
        let destination = Destination::open(scratch.path(), &owner(1)).unwrap();
        assert_eq!(destination.rules().ignores_case, case);
        assert_eq!(destination.rules().ignores_normalisation, normalisation);
        assert_eq!(destination.rules().platform, Platform::current());
    }

    #[test]
    fn the_probe_leaves_nothing_behind() {
        let scratch = tempfile::tempdir().unwrap();
        let destination = Destination::open(scratch.path(), &owner(1)).unwrap();
        let left: Vec<_> = fs::read_dir(&destination.staging_root).unwrap().collect();
        assert!(left.is_empty());
        // And nothing in the folder itself but the staging folder.
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);
    }

    #[test]
    fn what_an_earlier_run_left_is_removed_and_what_is_anothers_is_not() {
        let scratch = tempfile::tempdir().unwrap();
        let mine = scratch
            .path()
            .join(format!("{STAGING_PREFIX}{}", owner(1).short()));
        let other = scratch
            .path()
            .join(format!("{STAGING_PREFIX}{}", owner(2).short()));
        for staging in [&mine, &other] {
            fs::create_dir_all(staging.join("0123").join("folder")).unwrap();
            fs::write(staging.join("0123").join("folder").join("file"), b"data").unwrap();
            fs::write(staging.join("loose"), b"data").unwrap();
        }
        Destination::open(scratch.path(), &owner(1)).unwrap();
        assert_eq!(fs::read_dir(&mine).unwrap().count(), 0);
        assert!(other.join("0123").join("folder").join("file").exists());
        assert!(other.join("loose").exists());
    }

    #[test]
    fn a_destination_that_is_not_a_folder_is_refused() {
        let scratch = tempfile::tempdir().unwrap();
        let file = scratch.path().join("file");
        fs::write(&file, b"x").unwrap();
        assert!(matches!(
            Destination::open(&file, &owner(1)),
            Err(OpenError::NotAFolder)
        ));
        assert!(matches!(
            Destination::open(scratch.path().join("missing"), &owner(1)),
            Err(OpenError::Io(e)) if e.kind() == ErrorKind::NotFound
        ));
    }

    #[cfg(unix)]
    #[test]
    fn the_staging_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = tempfile::tempdir().unwrap();
        let destination = Destination::open(scratch.path(), &owner(1)).unwrap();
        let mode = fs::metadata(&destination.staging_root)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_place_of_the_staging_folder_is_refused() {
        let scratch = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let place = scratch
            .path()
            .join(format!("{STAGING_PREFIX}{}", owner(1).short()));
        std::os::unix::fs::symlink(elsewhere.path(), &place).unwrap();
        assert!(matches!(
            Destination::open(scratch.path(), &owner(1)),
            Err(OpenError::NotAFolder)
        ));
    }

    #[test]
    fn the_debug_form_has_no_folder_in_it() {
        let scratch = tempfile::tempdir().unwrap();
        let destination = Destination::open(scratch.path(), &owner(1)).unwrap();
        let shown = format!("{destination:?}");
        assert!(!shown.contains(scratch.path().to_str().unwrap()), "{shown}");
    }
}
