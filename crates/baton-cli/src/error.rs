// SPDX-License-Identifier: Apache-2.0

//! What a command can fail with, and how that is told to the person.

use std::{fmt, process::ExitCode};

/// A failure to report to the person who ran a command.
///
/// The message is written for them, not for a log: it says what went wrong and, where
/// there is something to do about it, what.
#[derive(Debug)]
pub struct CliError {
    message: String,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The command could not do what was asked.
    Failed,
    /// It worked, and the other device chose not to take what was sent.
    Declined,
}

impl CliError {
    /// A command that could not do what was asked.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: Kind::Failed,
        }
    }

    /// An item that reached the other device, which did not take it.
    pub fn declined(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: Kind::Declined,
        }
    }

    /// What to tell the person. Tests read it; the program prints the error itself.
    #[cfg(test)]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The exit status: 1 for a command that failed, 3 when the other device declined.
    /// (2 is what usage mistakes exit with.)
    pub fn exit_code(&self) -> ExitCode {
        match self.kind {
            Kind::Failed => ExitCode::from(1),
            Kind::Declined => ExitCode::from(3),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<baton_core::Error> for CliError {
    fn from(error: baton_core::Error) -> Self {
        use baton_core::Error;
        let message = match &error {
            Error::Locked { .. } => {
                "Baton is already running with this folder, probably `baton listen` in another \
                 window. Use that window (type `help` there), or close it and try again. \
                 `baton devices` and `baton inbox` work while it runs."
                    .to_owned()
            }
            Error::NotPaired(_) => {
                "That device is no longer paired. Run `baton devices` to see which are.".to_owned()
            }
            other => other.to_string(),
        };
        Self::new(message)
    }
}

/// An error from reaching a paired device, with the likeliest cause when it is a network
/// one.
pub fn unreachable(error: baton_core::Error, device: &baton_core::PairedDevice) -> CliError {
    use baton_core::Error;
    use baton_proto::close_code;

    let name = crate::term::line(device.display_name());
    match error {
        Error::NotPaired(_) | Error::BadItem(_) | Error::Unsupported(_) | Error::Stopped => {
            CliError::from(error)
        }
        Error::Closed { code, .. } if code == u64::from(close_code::NOT_PAIRED) => {
            CliError::new(format!(
                "{name} does not recognise this device any more, so it was probably unpaired \
                 there. Unpair it here too (`baton unpair`), then pair again if you want to keep \
                 using it."
            ))
        }
        Error::Closed { code, .. } if code == u64::from(close_code::OVERLOADED) => {
            CliError::new(format!("{name} is busy right now. Try again in a moment."))
        }
        other => CliError::new(format!(
            "Could not reach {name}: {other}. Is `baton listen` running there, and are both \
             devices online?"
        )),
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::new(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_locked_folder_says_what_to_do() {
        let error = CliError::from(baton_core::Error::Locked {
            path: "/x/baton.lock".into(),
        });
        assert!(error.message().contains("baton listen"));
        assert!(error.message().contains("baton inbox"));
    }

    #[test]
    fn other_errors_keep_their_own_wording() {
        let error = CliError::from(baton_core::Error::TicketExpired);
        assert!(error.message().contains("expired"));
    }

    #[test]
    fn a_decline_has_its_own_exit_status() {
        assert_eq!(CliError::new("x").exit_code(), ExitCode::from(1));
        assert_eq!(CliError::declined("x").exit_code(), ExitCode::from(3));
    }

    fn device() -> baton_core::PairedDevice {
        use baton_core::{DevicePolicy, Provenance};
        use baton_proto::{DeviceId, Platform};
        baton_core::PairedDevice {
            id: DeviceId::from_bytes([3; 32]),
            name: "Laptop".to_owned(),
            local_name: None,
            name_changed: false,
            platform: Platform::MacOs,
            added_at: 0,
            provenance: Provenance::Paired,
            last_seen_at: None,
            relay: None,
            addrs: Vec::new(),
            policy: DevicePolicy::default(),
        }
    }

    fn closed(code: u32) -> baton_core::Error {
        baton_core::Error::Closed {
            code: u64::from(code),
            reason: "not paired".to_owned(),
        }
    }

    #[test]
    fn a_device_that_forgot_us_is_not_reported_as_offline() {
        let error = unreachable(closed(baton_proto::close_code::NOT_PAIRED), &device());
        let text = error.message();
        assert!(text.contains("Laptop"), "{text}");
        assert!(text.contains("unpaired"), "{text}");
        assert!(!text.contains("online"), "{text}");
    }

    #[test]
    fn a_busy_device_is_told_as_such() {
        let error = unreachable(closed(baton_proto::close_code::OVERLOADED), &device());
        assert!(error.message().contains("busy"), "{}", error.message());
    }

    #[test]
    fn a_network_failure_names_the_likely_cause() {
        let error = unreachable(baton_core::Error::Timeout("connecting"), &device());
        let text = error.message();
        assert!(text.contains("Could not reach Laptop"), "{text}");
        assert!(text.contains("baton listen"), "{text}");
    }

    #[test]
    fn a_device_name_cannot_carry_escape_sequences_into_the_message() {
        let mut device = device();
        device.name = "x\u{1b}[2J".to_owned();
        let error = unreachable(baton_core::Error::Timeout("connecting"), &device);
        assert!(!error.message().contains('\u{1b}'));
    }
}
