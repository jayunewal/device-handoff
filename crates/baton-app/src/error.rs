// SPDX-License-Identifier: Apache-2.0

//! Failures, as a screen needs them.
//!
//! The core's errors are written for a person at a terminal: they mention file names and,
//! in two cases, a shell command to run. A screen wants two things instead: a kind to decide
//! what to offer (try again, pair again, open settings) and one sentence it can show as it
//! is. Both come with every [`Error`]. The sentence never contains a line break or a control
//! character, whatever the other device put into the text it came from, and it never tells
//! the person to run a command.

use std::{fmt, path::Path};

use baton_core::{
    Error as Core,
    transfer::{
        destination::OpenError,
        send::{Error as SendError, Outcome, Why, selection::SelectionError},
    },
};
use baton_proto::{close_code, device::sanitize_text, message::DeclineReason};

/// The longest message kept, in characters.
const MAX_MESSAGE_CHARS: usize = 300;

/// The longest part of what the core said that is repeated inside a longer sentence, in
/// characters. The sentence is cut at [`MAX_MESSAGE_CHARS`], and what is cut must not be the
/// advice at its end.
const MAX_DETAIL_CHARS: usize = 120;

/// What kind of thing went wrong, so that a screen can decide what to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Another Baton already has this device's folder open.
    AlreadyRunning,
    /// The [`App`](crate::App) has been stopped.
    Stopped,
    /// The files in the device's folder cannot be used: damaged, from a newer version, or
    /// open to other users.
    Storage,
    /// The device's identity was made on a different machine, which is what a restored
    /// backup looks like. Using it would let two machines answer to one identity.
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
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    /// What kind of failure this is.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// One sentence for the person. It is on one line and holds no control characters.
    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn new(kind: ErrorKind, message: impl AsRef<str>) -> Self {
        Self {
            kind,
            message: one_line(message.as_ref()),
        }
    }

    pub(crate) fn stopped() -> Self {
        Self::new(ErrorKind::Stopped, "Baton has stopped.")
    }

    pub(crate) fn not_paired() -> Self {
        Self::new(
            ErrorKind::NotPaired,
            "That device is not paired with this one.",
        )
    }

    pub(crate) fn bad_name() -> Self {
        Self::new(
            ErrorKind::BadName,
            "Give this device a name with something visible in it.",
        )
    }

    /// A failure to start: the folder, the settings, or the network.
    pub(crate) fn starting(error: &Core) -> Self {
        match error {
            Core::Network { .. } | Core::Timeout(_) => Self::new(
                ErrorKind::Network,
                format!(
                    "Baton could not start its network connection ({}).",
                    detail(error)
                ),
            ),
            other => Self::local(other),
        }
    }

    /// A failure that has to do with this device's own files, or one that needs no more
    /// than its message.
    pub(crate) fn local(error: &Core) -> Self {
        match error {
            Core::Locked { .. } => Self::new(
                ErrorKind::AlreadyRunning,
                "Baton is already running on this device.",
            ),
            Core::WrongMachine { .. } => Self::new(
                ErrorKind::WrongMachine,
                "The identity Baton saved here was made on a different device, probably \
                 because the app's data was restored from a backup. Starting over with a new \
                 identity means pairing again.",
            ),
            // The core's own wording for these two ends in a shell command.
            Core::InsecurePermissions { path, .. } => Self::new(
                ErrorKind::Storage,
                format!(
                    "Other users of this device can read or change {}, so Baton will not use it.",
                    file_name(path)
                ),
            ),
            Core::WritableByOthers { path, .. } => {
                let what = if path.is_dir() {
                    "the folder Baton keeps its files in".to_owned()
                } else {
                    file_name(path)
                };
                Self::new(
                    ErrorKind::Storage,
                    format!(
                        "Other users of this device can change {what}, so Baton will not use it."
                    ),
                )
            }
            Core::Io { .. }
            | Core::Invalid { .. }
            | Core::UnsupportedVersion { .. }
            | Core::Config(_) => Self::new(ErrorKind::Storage, error.to_string()),
            Core::NotPaired(_) => Self::not_paired(),
            Core::Stopped => Self::stopped(),
            other => Self::new(ErrorKind::Other, other.to_string()),
        }
    }

    /// A failure while pairing, on either side.
    pub(crate) fn pairing(error: &Core) -> Self {
        match error {
            Core::TicketExpired | Core::OwnTicket => {
                Self::new(ErrorKind::BadTicket, error.to_string())
            }
            Core::PairingRejected(_) | Core::TooManyDevices { .. } | Core::OfferEnded(_) => {
                Self::new(ErrorKind::PairingFailed, error.to_string())
            }
            Core::Network { .. } | Core::Timeout(_) | Core::Closed { .. } | Core::Protocol(_) => {
                Self::new(
                    ErrorKind::Unreachable,
                    format!(
                        "Could not reach the other device ({}). Baton has to be open on \
                         it, and both devices need to be online.",
                        detail(error)
                    ),
                )
            }
            other => Self::local(other),
        }
    }

    /// A failure while talking to the paired device called `device`.
    pub(crate) fn reaching(error: &Core, device: &str) -> Self {
        match error {
            Core::Network { .. } | Core::Timeout(_) | Core::Closed { .. } | Core::Protocol(_) => {
                Self::new(
                    ErrorKind::Unreachable,
                    format!(
                        "Could not reach {device} ({}). Baton has to be open on it, and \
                         both devices need to be online.",
                        detail(error)
                    ),
                )
            }
            Core::Unsupported(_) => Self::new(
                ErrorKind::Declined,
                format!(
                    "{device} does not handle this kind of item. It may need a newer version \
                     of Baton."
                ),
            ),
            Core::BadItem(_) => Self::new(ErrorKind::BadItem, error.to_string()),
            other => Self::local(other),
        }
    }

    /// A failure while offering a window to the paired device called `device`.
    pub(crate) fn sharing(error: &Core, device: &str) -> Self {
        match error {
            Core::Busy => Self::new(
                ErrorKind::Other,
                "A window is being shared or shown already. End it first.",
            ),
            Core::Unsupported(_) => Self::new(
                ErrorKind::Declined,
                format!("{device} cannot show windows. It may need a newer version of Baton."),
            ),
            Core::BadItem(_) => Self::new(
                ErrorKind::BadItem,
                "That window cannot be shared: its title or size is beyond what Baton allows.",
            ),
            other => Self::reaching(other, device),
        }
    }

    /// The device `device` answered and did not take the `what` that was sent.
    pub(crate) fn declined(device: &str, what: &str, reason: DeclineReason) -> Self {
        let why = match reason {
            DeclineReason::Unsupported => {
                "it does not handle this kind of item, which an older version of Baton may not"
            }
            DeclineReason::TooLarge => "it is too large for that device to keep",
            DeclineReason::RateLimited => {
                "it is receiving too fast from this device; wait a moment"
            }
            DeclineReason::Blocked => "its settings for this device do not allow this kind of item",
            DeclineReason::Invalid => {
                "it did not accept the content; for a link, check the address"
            }
            DeclineReason::NoSpace => "it has no room for it",
            DeclineReason::Refused => "nobody there accepted it",
            DeclineReason::Other => "it gave a reason this version does not know",
        };
        Self::new(
            ErrorKind::Declined,
            format!("{device} did not take the {what}: {why}."),
        )
    }

    /// What was chosen to be sent cannot be made into a transfer.
    pub(crate) fn selection(error: &SelectionError) -> Self {
        let message = match error {
            SelectionError::Empty(left_out) if left_out.total() == 0 => {
                "There is nothing to send in what you chose.".to_owned()
            }
            SelectionError::Empty(_) => "Nothing in what you chose can be sent. Links inside a \
                 folder, files that cannot be read and files that are too large are left out."
                .to_owned(),
            SelectionError::TooMany { what, max } => format!(
                "There are more than {max} {what} in what you chose. Send a smaller folder, or \
                 send it in parts."
            ),
            SelectionError::ListTooLong => "The names of the files in what you chose take more \
                 room than one transfer allows. Send it in parts."
                .to_owned(),
            SelectionError::TooBig => {
                "What you chose is larger than one transfer can hold. Send it in parts.".to_owned()
            }
        };
        Self::new(ErrorKind::BadItem, message)
    }

    /// A transfer to `device` that did not end with the files there. `outcome` is not
    /// `Outcome::Done`.
    pub(crate) fn not_sent(outcome: &Outcome, device: &str) -> Self {
        match outcome {
            Outcome::Declined(reason) => Self::declined(device, "files", *reason),
            Outcome::Stopped(code) => Self::stopped_by(device, *code),
            Outcome::Cancelled { .. } => Self::new(ErrorKind::Cancelled, "You stopped sending."),
            Outcome::Failed(error) => Self::transfer_failed(error, device),
            _ => Self::new(
                ErrorKind::Other,
                format!("The files did not reach {device}."),
            ),
        }
    }

    /// The device `device` ended the transfer by stopping its lanes with `code`.
    fn stopped_by(device: &str, code: u32) -> Self {
        match code {
            close_code::CANCELLED => Self::new(
                ErrorKind::Declined,
                format!("{device} stopped the transfer."),
            ),
            close_code::NO_SPACE => Self::new(
                ErrorKind::Declined,
                format!("{device} has no room for the files."),
            ),
            close_code::NOT_PAIRED => Self::new(
                ErrorKind::NotPaired,
                format!(
                    "{device} no longer has this device paired. Pair them again to send files."
                ),
            ),
            other => Self::new(
                ErrorKind::Other,
                format!(
                    "{device} ended the transfer (code {other}). Both devices may need a newer \
                     version of Baton."
                ),
            ),
        }
    }

    /// A transfer to `device` that could not be carried through.
    fn transfer_failed(error: &SendError, device: &str) -> Self {
        match error {
            SendError::GaveUp(Why::Connection | Why::Silent) => Self::new(
                ErrorKind::Unreachable,
                format!(
                    "Could not reach {device}. Baton has to be open on it, and both devices \
                     need to be online."
                ),
            ),
            SendError::GaveUp(Why::TimedOut | Why::Stalled) => Self::new(
                ErrorKind::Unreachable,
                format!(
                    "The connection to {device} stopped moving data, and Baton gave up. Try \
                     again when both devices are on a good connection."
                ),
            ),
            SendError::GaveUp(_) => Self::new(
                ErrorKind::Other,
                format!("{device} lost track of the transfer. Try sending the files again."),
            ),
            SendError::NoOutcome => Self::new(
                ErrorKind::Other,
                format!(
                    "{device} did not say whether it has the files. Look there before sending \
                     them again."
                ),
            ),
            SendError::Refused(close_code::NOT_PAIRED) => Self::new(
                ErrorKind::NotPaired,
                format!(
                    "{device} no longer has this device paired. Pair them again to send files."
                ),
            ),
            SendError::Refused(code) => Self::new(
                ErrorKind::Other,
                format!("{device} ended the connection (code {code})."),
            ),
            SendError::Protocol(what) | SendError::Internal(what) => Self::new(
                ErrorKind::Other,
                format!("Something went wrong with the transfer ({what})."),
            ),
            SendError::Stopped => Self::stopped(),
            _ => Self::new(
                ErrorKind::Other,
                format!("The files did not reach {device}."),
            ),
        }
    }
}

/// Why the folder for received files cannot be used, as one sentence.
pub(crate) fn unusable_folder(error: &OpenError) -> String {
    let message = match error {
        OpenError::NotAFolder => "The folder for received files is not a folder.".to_owned(),
        OpenError::Io(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            "Baton is not allowed to use the folder for received files.".to_owned()
        }
        OpenError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "The folder for received files is not there.".to_owned()
        }
        OpenError::Io(e) => format!("Baton cannot use the folder for received files ({e})."),
    };
    one_line(&message)
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// The name of the file at `path`, without the folders around it.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "one of its files".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `text` on one line, with nothing in it that could act on a screen, and not too long.
fn one_line(text: &str) -> String {
    sanitize_text(text, MAX_MESSAGE_CHARS).unwrap_or_default()
}

/// What the core said about `error`, short enough to sit inside a sentence of ours: when it
/// has to be cut, it is cut here and marked, so that the words after it survive.
fn detail(error: &Core) -> String {
    let said = error.to_string();
    let whole = sanitize_text(&said, usize::MAX).unwrap_or_default();
    if whole.chars().count() <= MAX_DETAIL_CHARS {
        return whole;
    }
    let mut kept = sanitize_text(&said, MAX_DETAIL_CHARS - 1).unwrap_or_default();
    kept.push('…');
    kept
}

#[cfg(test)]
mod tests {
    use baton_core::transfer::send::selection::LeftOut;

    use super::*;

    #[test]
    fn a_message_is_one_short_line_whatever_it_was_made_from() {
        let nasty = format!("first\nsecond\u{202e}\u{7}{}", "x".repeat(1000));
        let error = Error::new(ErrorKind::Other, nasty);
        assert!(!error.message().contains(['\n', '\r', '\u{202e}', '\u{7}']));
        assert!(error.message().starts_with("first second"));
        assert!(error.message().chars().count() <= MAX_MESSAGE_CHARS);
    }

    #[test]
    fn what_the_other_device_says_is_cleaned_before_it_is_shown() {
        let closed = Core::Closed {
            code: 7,
            reason: "go away\n\u{1b}[31mred".to_owned(),
        };
        let error = Error::reaching(&closed, "Kitchen iPad");
        assert_eq!(error.kind(), ErrorKind::Unreachable);
        assert!(!error.message().contains(['\n', '\u{1b}']), "{error}");
        assert!(error.message().contains("Kitchen iPad"));
    }

    #[test]
    fn a_long_reason_from_the_core_does_not_push_the_advice_out() {
        let long = Core::Closed {
            code: 7,
            reason: "x ".repeat(400),
        };
        for error in [
            Error::reaching(&long, "Kitchen iPad"),
            Error::pairing(&long),
        ] {
            assert_eq!(error.kind(), ErrorKind::Unreachable);
            assert!(
                error.message().contains("Baton has to be open on"),
                "{error}"
            );
            assert!(error.message().ends_with("online."), "{error}");
            assert!(error.message().chars().count() <= MAX_MESSAGE_CHARS);
            assert!(error.message().contains('…'), "{error}");
        }
        let short = Core::Closed {
            code: 7,
            reason: "no".to_owned(),
        };
        assert!(!Error::reaching(&short, "Mac").message().contains('…'));
    }

    #[test]
    fn a_window_that_cannot_be_offered_is_told_in_words_of_its_own() {
        let busy = Error::sharing(&Core::Busy, "Kitchen iPad");
        assert_eq!(busy.kind(), ErrorKind::Other);
        assert!(busy.message().contains("End it first"), "{busy}");

        let old = Error::sharing(&Core::Unsupported("window".to_owned()), "Kitchen iPad");
        assert_eq!(old.kind(), ErrorKind::Declined);
        assert!(old.message().contains("Kitchen iPad"), "{old}");
        assert!(old.message().contains("cannot show windows"), "{old}");
        assert!(old.message().contains("newer version"), "{old}");

        let bad = Error::sharing(&Core::BadItem("title".to_owned()), "Kitchen iPad");
        assert_eq!(bad.kind(), ErrorKind::BadItem);
        assert!(bad.message().contains("title or size"), "{bad}");
    }

    #[test]
    fn a_folder_that_others_can_change_is_called_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Core::WritableByOthers {
            path: dir.path().to_owned(),
            mode: 0o777,
        };
        let message = Error::local(&folder).message().to_owned();
        assert!(
            message.contains("the folder Baton keeps its files in"),
            "{message}"
        );
        assert!(
            !message.contains(&*dir.path().file_name().unwrap().to_string_lossy()),
            "{message}"
        );

        let file = Core::WritableByOthers {
            path: dir.path().join("config.toml"),
            mode: 0o666,
        };
        assert!(Error::local(&file).message().contains("config.toml"));
    }

    #[test]
    fn a_locked_folder_is_told_apart_from_a_damaged_one() {
        let locked = Core::Locked {
            path: "/data/lock".into(),
        };
        assert_eq!(Error::starting(&locked).kind(), ErrorKind::AlreadyRunning);
        let damaged = Core::Invalid {
            path: "/data/devices.json".into(),
            message: "not json".to_owned(),
        };
        assert_eq!(Error::starting(&damaged).kind(), ErrorKind::Storage);
    }

    #[test]
    fn a_message_never_tells_a_person_to_run_a_command() {
        let readable = Core::InsecurePermissions {
            path: "/data/identity.toml".into(),
            mode: 0o644,
        };
        let writable = Core::WritableByOthers {
            path: "/data".into(),
            mode: 0o777,
        };
        for core in [&readable, &writable] {
            assert!(core.to_string().contains("chmod"), "the core says: {core}");
            let error = Error::starting(core);
            assert_eq!(error.kind(), ErrorKind::Storage);
            assert!(!error.message().contains("chmod"), "{error}");
        }
        assert!(
            Error::starting(&readable)
                .message()
                .contains("identity.toml")
        );
        assert!(!Error::starting(&readable).message().contains("/data/"));
    }

    /// Checks the kind and a part of the sentence for each outcome, and that the sentence is one
    /// line.
    fn check_not_sent(cases: impl IntoIterator<Item = (Outcome, ErrorKind, &'static str)>) {
        for (outcome, kind, part) in cases {
            let error = Error::not_sent(&outcome, "Kitchen iPad");
            assert_eq!(error.kind(), kind, "{outcome:?}: {error}");
            assert!(error.message().contains(part), "{outcome:?}: {error}");
            assert!(!error.message().contains('\n'), "{error}");
        }
    }

    #[test]
    fn a_transfer_that_the_other_device_or_the_person_ended_says_why() {
        check_not_sent([
            (
                Outcome::Declined(DeclineReason::NoSpace),
                ErrorKind::Declined,
                "Kitchen iPad did not take the files: it has no room for it.",
            ),
            (
                Outcome::Declined(DeclineReason::Refused),
                ErrorKind::Declined,
                "nobody there accepted it",
            ),
            (
                Outcome::Stopped(close_code::CANCELLED),
                ErrorKind::Declined,
                "Kitchen iPad stopped the transfer.",
            ),
            (
                Outcome::Stopped(close_code::NO_SPACE),
                ErrorKind::Declined,
                "Kitchen iPad has no room for the files.",
            ),
            (
                Outcome::Stopped(close_code::NOT_PAIRED),
                ErrorKind::NotPaired,
                "no longer has this device paired",
            ),
            (
                Outcome::Stopped(close_code::TOO_LARGE),
                ErrorKind::Other,
                "(code 413)",
            ),
            (
                Outcome::Cancelled { told: true },
                ErrorKind::Cancelled,
                "You stopped sending.",
            ),
            (
                Outcome::Cancelled { told: false },
                ErrorKind::Cancelled,
                "You stopped sending.",
            ),
        ]);
    }

    #[test]
    fn a_transfer_that_could_not_be_carried_through_says_why() {
        check_not_sent([
            (
                Outcome::Failed(SendError::GaveUp(Why::Connection)),
                ErrorKind::Unreachable,
                "Could not reach Kitchen iPad. Baton has to be open on it",
            ),
            (
                Outcome::Failed(SendError::GaveUp(Why::Silent)),
                ErrorKind::Unreachable,
                "Could not reach Kitchen iPad.",
            ),
            (
                Outcome::Failed(SendError::GaveUp(Why::TimedOut)),
                ErrorKind::Unreachable,
                "stopped moving data",
            ),
            (
                Outcome::Failed(SendError::GaveUp(Why::Stalled)),
                ErrorKind::Unreachable,
                "stopped moving data",
            ),
            (
                Outcome::Failed(SendError::GaveUp(Why::Forgotten)),
                ErrorKind::Other,
                "lost track of the transfer",
            ),
            (
                Outcome::Failed(SendError::NoOutcome),
                ErrorKind::Other,
                "did not say whether it has the files",
            ),
            (
                Outcome::Failed(SendError::Refused(close_code::NOT_PAIRED)),
                ErrorKind::NotPaired,
                "no longer has this device paired",
            ),
            (
                Outcome::Failed(SendError::Refused(close_code::INTERNAL)),
                ErrorKind::Other,
                "ended the connection (code",
            ),
            (
                Outcome::Failed(SendError::Protocol("an odd answer")),
                ErrorKind::Other,
                "Something went wrong with the transfer (an odd answer).",
            ),
            (
                Outcome::Failed(SendError::Internal("no list")),
                ErrorKind::Other,
                "Something went wrong with the transfer (no list).",
            ),
            (
                Outcome::Failed(SendError::Stopped),
                ErrorKind::Stopped,
                "stopped",
            ),
        ]);
    }

    #[test]
    fn what_cannot_be_made_into_a_transfer_says_why() {
        let nothing = SelectionError::Empty(LeftOut::default());
        let all_left_out = SelectionError::Empty(LeftOut {
            links: 2,
            ..LeftOut::default()
        });
        let too_many = SelectionError::TooMany {
            what: "folders",
            max: 1000,
        };
        for (error, part) in [
            (nothing, "There is nothing to send in what you chose."),
            (all_left_out, "Nothing in what you chose can be sent."),
            (
                too_many,
                "There are more than 1000 folders in what you chose.",
            ),
            (SelectionError::ListTooLong, "Send it in parts."),
            (SelectionError::TooBig, "larger than one transfer can hold"),
        ] {
            let shown = Error::selection(&error);
            assert_eq!(shown.kind(), ErrorKind::BadItem, "{error}");
            assert!(shown.message().contains(part), "{shown}");
        }
    }

    #[test]
    fn a_folder_for_received_files_that_cannot_be_used_says_why_and_not_where() {
        use std::io::{Error as Io, ErrorKind as Of};

        for (error, said) in [
            (
                OpenError::NotAFolder,
                "The folder for received files is not a folder.",
            ),
            (
                OpenError::Io(Io::from(Of::PermissionDenied)),
                "Baton is not allowed to use the folder for received files.",
            ),
            (
                OpenError::Io(Io::from(Of::NotFound)),
                "The folder for received files is not there.",
            ),
        ] {
            assert_eq!(unusable_folder(&error), said);
        }
        let other = unusable_folder(&OpenError::Io(Io::other("disk on fire\nsecond line")));
        assert!(
            other.starts_with("Baton cannot use the folder for received files ("),
            "{other}"
        );
        assert!(!other.contains('\n'), "{other}");
    }
}
