// SPDX-License-Identifier: Apache-2.0

//! The error type shared by the core's storage and configuration code.

use std::{fmt, io, path::PathBuf};

use baton_proto::{DeviceId, pair::PairRejection};

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
    /// The device identity was made on a different machine.
    #[error(
        "{} holds the identity of another machine. It was probably copied here by a migration \
         tool or restored from a backup, and using it would make two machines answer to one \
         device identity. To give this machine its own identity, move the file away and pair \
         again. If this is the machine that made it (its system identifier changed), adopt \
         the identity here once no other copy is in use",
        path.display()
    )]
    WrongMachine {
        /// The key file.
        path: PathBuf,
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
    /// Setting up or using the network failed.
    #[error("{context}: {message}")]
    Network {
        /// What was being done, as a short phrase.
        context: &'static str,
        /// What went wrong.
        message: String,
    },
    /// The other device broke the protocol.
    #[error("the other device broke the protocol: {0}")]
    Protocol(String),
    /// A step took too long.
    #[error("timed out {0}")]
    Timeout(&'static str),
    /// There is no paired device with this id.
    #[error("{0} is not a paired device")]
    NotPaired(DeviceId),
    /// The other device closed the connection on purpose.
    #[error(
        "the other device closed the connection (code {code}){}",
        reason_suffix(reason)
    )]
    Closed {
        /// The application error code. See `baton_proto::close_code`.
        code: u64,
        /// The reason it gave, for logs.
        reason: String,
    },
    /// The pairing ticket's lifetime is over.
    #[error("this pairing ticket has expired; ask for a new one (and check this device's clock)")]
    TicketExpired,
    /// The pairing ticket was made by this very device.
    #[error("this pairing ticket was made by this device; enter it on the other device")]
    OwnTicket,
    /// The offering device did not pair with this one.
    #[error("pairing was not accepted: {}", rejection_text(*.0))]
    PairingRejected(PairRejection),
    /// An open pairing offer ended without pairing a device.
    #[error("the pairing offer ended: {0}")]
    OfferEnded(OfferEnd),
    /// The other device does not accept this kind of item.
    #[error("the other device does not support {0}")]
    Unsupported(String),
    /// The item to send breaks a limit.
    #[error("cannot send this: {0}")]
    BadItem(String),
    /// Something that should not happen did.
    #[error("internal error: {0}")]
    Internal(String),
    /// This node has been shut down.
    #[error("Baton is shutting down")]
    Stopped,
}

/// How a pairing offer ended without pairing anyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OfferEnd {
    /// It was withdrawn, or replaced by a newer offer.
    Cancelled,
    /// Its time ran out.
    Expired,
    /// The person declined the request.
    Declined,
}

impl fmt::Display for OfferEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "it was cancelled",
            Self::Expired => "it expired",
            Self::Declined => "the request was declined",
        })
    }
}

fn reason_suffix(reason: &str) -> String {
    if reason.is_empty() {
        String::new()
    } else {
        format!(": {reason}")
    }
}

fn rejection_text(rejection: PairRejection) -> &'static str {
    match rejection {
        PairRejection::Invalid => "the ticket was wrong, already used or expired",
        PairRejection::Declined => "the person on the other device declined",
        PairRejection::Busy => "the other device is busy with another request; try again",
        PairRejection::Other => "the other device refused for a reason this version does not know",
    }
}

impl Error {
    pub(crate) fn io(op: &'static str, path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { op, path, source }
    }

    pub(crate) fn network(context: &'static str, error: &(impl ToString + ?Sized)) -> Self {
        Self::Network {
            context,
            message: error.to_string(),
        }
    }

    pub(crate) fn protocol(message: &(impl ToString + ?Sized)) -> Self {
        Self::Protocol(message.to_string())
    }

    pub(crate) fn invalid(path: impl Into<PathBuf>, message: &(impl ToString + ?Sized)) -> Self {
        Self::Invalid {
            path: path.into(),
            message: message.to_string(),
        }
    }
}
