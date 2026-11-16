// SPDX-License-Identifier: Apache-2.0

//! What goes wrong, in sentences for the person at the Mac.

use std::fmt;

/// What a caller can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The folder for the program's own files was not given, cannot be made, or may be read or
    /// changed by someone who should not.
    Folder,
    /// The platform that gives the login window is missing, or does more than this program allows.
    Platform,
    /// Another program holds the folder: this one, or the running program, is already using it.
    Busy,
    /// A pairing did not happen: the digits that the phone shows were not typed, it ran out, or it
    /// was given up.
    Pairing,
    /// No paired device, or more than one, is called what the person typed.
    NoDevice,
    /// Anything else: the files could not be read or kept, or the network could not start.
    Other,
}

/// What went wrong, with a sentence for the person who ran the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

/// What the calls of this crate return.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// What kind of thing went wrong.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The sentence for the person.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<baton_app::Error> for Error {
    fn from(error: baton_app::Error) -> Self {
        let kind = match error.kind() {
            baton_app::ErrorKind::AlreadyRunning => ErrorKind::Busy,
            _ => ErrorKind::Other,
        };
        Self::new(kind, error.message())
    }
}

impl From<baton_core::Error> for Error {
    fn from(error: baton_core::Error) -> Self {
        let kind = match error {
            baton_core::Error::Locked { .. } => ErrorKind::Busy,
            _ => ErrorKind::Other,
        };
        Self::new(kind, error.to_string())
    }
}
