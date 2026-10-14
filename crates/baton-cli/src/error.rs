// SPDX-License-Identifier: Apache-2.0

//! What a command can fail with, and how that is told to the person.

use std::{fmt, process::ExitCode};

/// A failure to report to the person who ran a command.
///
/// The message is written for them, not for a log: it says what went wrong and, where
/// there is something to do about it, what.
///
/// It is also escaped for the terminal when the error is made, so that nothing which
/// prints it has to remember to. Some of what goes into a message was chosen by another
/// device: the reason in its close frame, the transport's text about why it hung up, the
/// name it calls itself. Printed raw, those could move the cursor, change the terminal's
/// title or, in terminals that allow it, set the clipboard.
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
    pub fn new(message: impl AsRef<str>) -> Self {
        Self {
            message: crate::term::block(message.as_ref()),
            kind: Kind::Failed,
        }
    }

    /// An item that reached the other device, which did not take it.
    pub fn declined(message: impl AsRef<str>) -> Self {
        Self {
            message: crate::term::block(message.as_ref()),
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
            // What is left can hold text another device chose (why it hung up), so it goes
            // on one line and cannot pass for a line of Baton's own.
            other => crate::term::line(&other.to_string()),
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
        other => {
            // Like the last arm of `From` above: this can be the other device's own words.
            let detail = crate::term::line(&other.to_string());
            CliError::new(format!(
                "Could not reach {name}: {detail}. Is `baton listen` running there, and are \
                 both devices online?"
            ))
        }
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

    /// Text that would set the clipboard in terminals that allow it (OSC 52), clear the
    /// screen, and ring the bell.
    const HOSTILE: &str = "\x1b]52;c;aGk=\x07\x1b[2J\u{9b}31m\u{202e}";

    fn assert_safe(text: &str) {
        for c in text.chars() {
            assert!(
                !c.is_control() || c == '\n' || c == '\t',
                "{c:?} would reach the terminal in {text:?}"
            );
        }
        assert!(!text.contains('\u{202e}'), "{text:?}");
        assert!(
            text.contains("\\u{1b}"),
            "the escape should stay visible: {text:?}"
        );
    }

    #[test]
    fn the_reason_the_other_device_gave_for_closing_is_escaped() {
        let closed = baton_core::Error::Closed {
            code: 7,
            reason: HOSTILE.to_owned(),
        };
        let error = unreachable(closed, &device());
        assert_safe(error.message());
        assert_safe(&error.to_string());
    }

    #[test]
    fn the_transports_text_about_a_hang_up_is_escaped() {
        let network = baton_core::Error::Network {
            context: "connecting to the other device",
            message: format!("aborted by peer: {HOSTILE}"),
        };
        assert_safe(unreachable(network, &device()).message());
        let network = baton_core::Error::Network {
            context: "connecting to the other device",
            message: format!("aborted by peer: {HOSTILE}"),
        };
        assert_safe(CliError::from(network).message());
    }

    #[test]
    fn any_message_is_escaped_whatever_made_it() {
        assert_safe(CliError::new(format!("x {HOSTILE}")).message());
        assert_safe(&CliError::declined(format!("x {HOSTILE}")).to_string());
        assert_safe(&CliError::from(std::io::Error::other(HOSTILE)).to_string());
    }

    #[test]
    fn line_breaks_in_a_message_are_kept() {
        let error = CliError::new("first\nsecond\tthird");
        assert_eq!(error.message(), "first\nsecond\tthird");
    }

    /// What a device that hangs up could say, to look like a line of Baton's own.
    const LOOKALIKE: &str = "bye\n\nPaired with Laptop (aaaa-bbbb-cccc-dddd-eeee)";

    #[test]
    fn what_the_other_device_says_stays_on_one_line() {
        let closed = baton_core::Error::Closed {
            code: 7,
            reason: LOOKALIKE.to_owned(),
        };
        let text = unreachable(closed, &device()).message().to_owned();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert!(text.contains("bye\\n\\nPaired with Laptop"), "{text:?}");

        let network = || baton_core::Error::Network {
            context: "connecting to the other device",
            message: format!("aborted by peer: {LOOKALIKE}"),
        };
        let text = unreachable(network(), &device()).message().to_owned();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        let text = CliError::from(network()).message().to_owned();
        assert_eq!(text.lines().count(), 1, "{text:?}");
    }
}
