// SPDX-License-Identifier: Apache-2.0

//! Failures, as the app receives them.

/// What kind of thing went wrong, so that a screen can decide what to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Another Baton already has this device's folder open.
    AlreadyRunning,
    /// Baton has been stopped.
    Stopped,
    /// The files in the device's folder cannot be used: damaged, from a newer version, or
    /// open to other users.
    Storage,
    /// The device's identity was made on a different machine, which is what a restored
    /// backup looks like.
    WrongMachine,
    /// The network could not be started.
    Network,
    /// The id does not belong to a paired device.
    NotPaired,
    /// A paired device could not be reached, or did not answer in time.
    Unreachable,
    /// The other device answered but did not take the item.
    Declined,
    /// What was to be sent is not acceptable: empty, malformed or too large.
    BadItem,
    /// The name given for this device has nothing visible in it.
    BadName,
    /// The pairing ticket cannot be used: not a ticket, out of date, or made by this device.
    BadTicket,
    /// The other device did not pair: declined, busy, or the ticket was used up.
    PairingFailed,
    /// The person stopped what was going on.
    Cancelled,
    /// Anything else.
    Other,
}

/// A failure, with a kind to react to and a sentence to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatonError {
    /// What kind of failure this is.
    pub kind: ErrorKind,
    /// One sentence for the person. It is on one line and holds no control characters.
    pub message: String,
}

impl std::fmt::Display for BatonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BatonError {}
