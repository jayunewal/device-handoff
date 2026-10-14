// SPDX-License-Identifier: Apache-2.0

//! The error type shared by the core's storage and configuration code.

use std::{io, path::PathBuf};

/// A specialised `Result` for the core.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Something went wrong reading or writing Baton's own state.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A file or directory operation failed.
    #[error("could not {op} {}: {source}", path.display())]
    Io {
        /// What was being done, as a verb phrase: "read", "write", "create".
        op: &'static str,
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// A file exists but its contents are not valid.
    #[error("{} is not valid: {message}", path.display())]
    Invalid {
        /// The file involved.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
    /// A file was written by a newer, incompatible version.
    #[error(
        "{} uses format {found}, but this version only understands up to {supported}; \
         upgrade Baton",
        path.display()
    )]
    UnsupportedVersion {
        /// The file involved.
        path: PathBuf,
        /// The format version in the file.
        found: u32,
        /// The newest format version this build reads.
        supported: u32,
    },
    /// A file holding a secret, or the list of trusted devices, can be reached by other users.
    #[error(
        "{} can be read or changed by other users (mode {mode:o}); \
         run `chmod 600` on it and try again",
        path.display()
    )]
    InsecurePermissions {
        /// The file involved.
        path: PathBuf,
        /// Its Unix permission bits.
        mode: u32,
    },
    /// A file or directory that Baton relies on can be changed by other users.
    #[error(
        "{} can be changed by other users (mode {mode:o}); \
         run `chmod go-w` on it and try again",
        path.display()
    )]
    WritableByOthers {
        /// The file or directory involved.
        path: PathBuf,
        /// Its Unix permission bits.
        mode: u32,
    },
    /// Another process already holds the state directory.
    #[error("another Baton process is already running with {}", path.display())]
    Locked {
        /// The lock file.
        path: PathBuf,
    },
    /// The settings are not acceptable.
    #[error("invalid configuration: {0}")]
    Config(String),
    /// The paired-device list is full.
    #[error("cannot pair more than {limit} devices")]
    TooManyDevices {
        /// The limit.
        limit: usize,
    },
}

impl Error {
    pub(crate) fn io(op: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { op, path, source }
    }

    pub(crate) fn invalid(path: impl Into<PathBuf>, message: &(impl ToString + ?Sized)) -> Self {
        Self::Invalid {
            path: path.into(),
            message: message.to_string(),
        }
    }
}
