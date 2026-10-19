// SPDX-License-Identifier: Apache-2.0

//! The plain values the app receives. They mirror `baton_app::model`, with one difference:
//! numbers are signed 64-bit (`i64`), which is the integer a Dart program has.

use super::error::BatonError;

/// The operating system a device runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// iOS.
    Ios,
    /// iPadOS.
    IpadOs,
    /// Android.
    Android,
    /// Anything this version does not know about.
    Other,
}

/// This device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    /// The device's id: 64 hexadecimal characters.
    pub id: String,
    /// The name other devices show for it.
    pub name: String,
    /// The system it runs.
    pub platform: Platform,
    /// A short string, as `abcd-efgh-ijkl-mnop-qrst`, that two people can compare on two
    /// screens to be sure they mean the same device.
    pub fingerprint: String,
}

/// A device that is paired with this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The device's id. Pass it to the calls that act on a device.
    pub id: String,
    /// The name to show for it. Chosen by the other device, so show the fingerprint
    /// wherever a person has to be sure which device it is.
    pub name: String,
    /// The system it runs.
    pub platform: Platform,
    /// Its fingerprint, as `abcd-efgh-ijkl-mnop-qrst`.
    pub fingerprint: String,
    /// When it was paired, in seconds since the Unix epoch.
    pub paired_at: i64,
    /// When this device last heard from it, in seconds since the Unix epoch.
    pub last_seen_at: Option<i64>,
}

/// How traffic to a device gets there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Directly over the local network.
    Lan,
    /// Directly over the internet.
    Direct,
    /// Through a relay server: slower, but it works when nothing else does.
    Relay,
    /// Not known yet.
    Unknown,
}

/// The answer to a ping: the device is there, and this is how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reach {
    /// The round trip time, in milliseconds.
    pub round_trip_ms: u32,
    /// The route the traffic took.
    pub route: Route,
}

/// Where this device can be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    /// The relay other devices can reach this one through, once it is connected to one.
    pub relay: Option<String>,
    /// Addresses on this device's networks that other devices can try directly.
    pub addresses: Vec<String>,
    /// Whether this device announces itself and looks for others on the local network.
    pub lan_discovery: bool,
}

/// What was handed over.
#[derive(Clone, PartialEq, Eq)]
pub enum ReceivedItem {
    /// A web link.
    Link {
        /// The address, in the plain ASCII form a browser would send.
        url: String,
        /// The page title, if the sender knew one.
        title: Option<String>,
    },
    /// Text the sender copied or typed.
    Text {
        /// The text, exactly as sent.
        text: String,
    },
}

// What was received may be a password or a link with a sign-in token in it, and values like
// this one end up in logs. Only sizes are printed. That does not carry over to Dart, where
// the generated class prints the link or the text: see "The bridge" in docs/architecture.md.
impl std::fmt::Debug for ReceivedItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Link { url, title } => f
                .debug_struct("Link")
                .field("url_bytes", &url.len())
                .field("title_bytes", &title.as_ref().map(String::len))
                .finish(),
            Self::Text { text } => write!(f, "Text({} bytes)", text.len()),
        }
    }
}

/// One item in the inbox, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// Counts up from 1 in this inbox and is never reused. Pass it to
    /// `Baton.removeReceived`.
    pub number: i64,
    /// The id of the device that sent it.
    pub from_id: String,
    /// The name that device was shown under when this arrived.
    pub from_name: String,
    /// When it arrived, in seconds since the Unix epoch.
    pub received_at: i64,
    /// What it is.
    pub item: ReceivedItem,
}

/// An invitation to pair, as the person who made it sees it.
#[derive(Clone, PartialEq, Eq)]
pub struct Offer {
    /// The ticket to show as a QR code, or as text to copy, on this device and to enter on
    /// the other. It is a secret until it has been used: show it to the person, keep it out
    /// of logs, and do not send it anywhere but their other device.
    pub ticket: String,
    /// When the offer lapses, in seconds since the Unix epoch.
    pub expires_at: i64,
}

impl std::fmt::Debug for Offer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Offer")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// A device asking to pair, for the person to answer.
///
/// Show all of it, with the code most prominent, and say what the code is to be compared
/// with: the one the other device shows. The person accepts only if the two are the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequest {
    /// Names this request when answering it with `Baton.answerPairing`.
    pub id: i64,
    /// The id of the device asking.
    pub device_id: String,
    /// The name it gives itself. Chosen by the other device, so never what tells a person
    /// which device it is: the fingerprint is.
    pub name: String,
    /// The system it says it runs.
    pub platform: Platform,
    /// Its fingerprint, as `abcd-efgh-ijkl-mnop-qrst`.
    pub fingerprint: String,
    /// Six digits, as `514 643`, which the other device is showing too.
    pub code: String,
}

/// How an offer ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferOutcome {
    /// A device was paired.
    Paired {
        /// The device that is now paired.
        device: Device,
    },
    /// The time ran out.
    Expired,
    /// It was withdrawn, or replaced by a newer offer.
    Cancelled,
    /// The person declined the request.
    Declined,
    /// The device that asked disconnected before the person answered.
    Abandoned,
    /// Something went wrong.
    Failed {
        /// One sentence that says what.
        message: String,
    },
}

/// A device asking to send files, for the person to answer.
///
/// Show who is asking and what is offered, and answer with `Baton.answerTransfer`. The names
/// were written by the other device. They are cleaned to be safe to show, and they say what
/// the person is asked to take, not what the files will be called here: a name that is taken
/// in the folder gets a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    /// Names this request when answering it with `Baton.answerTransfer`.
    pub id: i64,
    /// The id of the device asking.
    pub device_id: String,
    /// The name this device shows for it.
    pub device_name: String,
    /// How many files and folders were chosen to be sent: the items that are not inside
    /// another one.
    pub items: u32,
    /// How many files there are in all, in every folder.
    pub files: u32,
    /// How many folders there are in all, in every folder.
    pub folders: u32,
    /// How many bytes the files add up to.
    pub bytes: i64,
    /// The first of the items, in order. It holds fewer than `items` when there are many.
    pub preview: Vec<PreviewName>,
}

/// One of the files and folders that were chosen to be sent.
#[derive(Clone, PartialEq, Eq)]
pub struct PreviewName {
    /// The name, cleaned to be safe to show.
    pub name: String,
    /// Whether it is a folder.
    pub folder: bool,
    /// For a folder, how many files are in it, in every folder inside it. 0 for a file.
    pub files: u32,
    /// The size of the file, or of all the files in the folder added up.
    pub bytes: i64,
}

// A file name can say more than the person wants in a log, so only sizes are printed. That
// does not carry over to Dart, where the generated class prints the name: see "The bridge" in
// docs/architecture.md.
impl std::fmt::Debug for PreviewName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewName")
            .field("name_bytes", &self.name.len())
            .field("folder", &self.folder)
            .field("files", &self.files)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// How a transfer that this device took has ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferEnd {
    /// The `TransferRequest.id` of the transfer.
    pub request: i64,
    /// The id of the device that sent it.
    pub device_id: String,
    /// The name this device shows for it.
    pub device_name: String,
    /// How many files the transfer held.
    pub files: u32,
    /// How many bytes they add up to.
    pub bytes: i64,
    /// The first of the items that were offered, as the request showed them.
    pub preview: Vec<PreviewName>,
    /// How it ended.
    pub result: TransferResult,
}

/// How a transfer that this device took has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferResult {
    /// The sender is done. Every file is in the folder except `failed` of them, and those are
    /// not there at all, never half of one.
    Received {
        /// How many files did not arrive.
        failed: u32,
    },
    /// The transfer was given up, and nothing of it is kept: the sender stopped, the device
    /// was unpaired, or the sender did not come back.
    Dropped,
}

/// Whether this device takes files, and where they go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiving {
    /// The app gave no folder for files, so this device takes none.
    Off,
    /// Files are taken, after the person says yes to each transfer.
    On,
    /// The app gave a folder, and Baton cannot use it, so this device takes no files.
    Unusable {
        /// One sentence that says why.
        message: String,
    },
}

/// What a transfer that is being sent is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStage {
    /// Making a connection to the other device.
    Connecting,
    /// Waiting for the answer to the offer. A person may be deciding.
    Asking,
    /// The files are going out.
    Sending,
    /// Everything is sent, and the other device is putting it in place.
    Finishing,
    /// The connection failed or the other device stalled, and Baton waits a moment before it
    /// tries again.
    Waiting,
    /// The transfer is over, whichever way it ended.
    Over,
}

/// How far a transfer that is being sent has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileProgress {
    /// What the transfer is doing now.
    pub stage: SendStage,
    /// The attempt that is going, from 1.
    pub attempt: u32,
    /// How many files the transfer holds.
    pub files: u32,
    /// How many bytes they add up to.
    pub bytes: i64,
    /// How many bytes have gone out in this attempt. An attempt that is tried again starts
    /// from 0, because it sends everything again.
    pub sent: i64,
}

/// A transfer that the other device answered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesSent {
    /// How many files the transfer held.
    pub files: u32,
    /// How many bytes they add up to.
    pub bytes: i64,
    /// How many of the files are not in place on the other device. The others are.
    pub failed: u32,
    /// How many things in what was chosen were left out and not offered at all: links inside
    /// a folder, files that could not be read, and the like.
    pub left_out: u32,
}

/// Something a screen may want to react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A device handed this one something. It is in the inbox already.
    Received {
        /// What arrived.
        received: Received,
    },
    /// The list of paired devices changed: one was added, renamed, forgotten, or heard from.
    /// Read it again with `Baton.devices`.
    DevicesChanged,
    /// A device wants to pair, using the offer that is showing. Show the request and answer
    /// it with `Baton.answerPairing`.
    PairingRequested {
        /// The request.
        request: PairingRequest,
    },
    /// A request no longer needs an answer: the offer ended, or the device gave up. Take the
    /// question off the screen.
    PairingRequestEnded {
        /// The `PairingRequest.id` that ended.
        id: i64,
    },
    /// The offer that was showing ended.
    OfferEnded {
        /// How.
        outcome: OfferOutcome,
    },
    /// A device wants to send files. Show what it offers, with who is asking, and answer
    /// with `Baton.answerTransfer`: the person has two minutes, and no answer is a no.
    TransferRequested {
        /// The request.
        request: TransferRequest,
    },
    /// A request for files no longer needs an answer: the other device gave up, was
    /// unpaired, or the time ran out. Take the question off the screen.
    TransferRequestEnded {
        /// The `TransferRequest.id` that ended.
        id: i64,
    },
    /// A transfer that the person said yes to has ended. The files that arrived are in the
    /// folder already.
    TransferEnded {
        /// How it ended.
        end: TransferEnd,
    },
    /// Events were missed because they came faster than they were read. Read the devices and
    /// the inbox again.
    Resync,
}

/// What happens while this device joins a pairing: the code to show, and then the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinProgress {
    /// The other device has been reached. Show the code: the person on the other device
    /// checks that it shows the same one, and says yes there.
    Code {
        /// Six digits, as `514 643`.
        code: String,
    },
    /// The other device said yes. This is the last item.
    Paired {
        /// The device that is now paired.
        device: Device,
    },
    /// The pairing did not happen. This is the last item.
    Failed {
        /// Why.
        error: BatonError,
    },
}
