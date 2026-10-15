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
// this one end up in logs. Only sizes are printed.
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
