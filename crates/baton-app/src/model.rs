// SPDX-License-Identifier: Apache-2.0

//! The plain values the app layer hands to a screen.
//!
//! Everything here is data: strings, numbers and enums. A bridge to another language can
//! copy these without knowing anything about Baton's internals, and a screen can keep them
//! as long as it likes. Ids are text (64 hexadecimal characters) so that they can be kept
//! in a widget, a route or a test as they are.

use std::fmt;

use baton_core::{
    InboxEntry, InboxItem, PairedDevice,
    link::PathKind,
    node::{NodeStatus, PingReport},
};
use baton_proto::Platform as ProtoPlatform;

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

impl From<ProtoPlatform> for Platform {
    fn from(platform: ProtoPlatform) -> Self {
        match platform {
            ProtoPlatform::MacOs => Self::MacOs,
            ProtoPlatform::Windows => Self::Windows,
            ProtoPlatform::Linux => Self::Linux,
            ProtoPlatform::Ios => Self::Ios,
            ProtoPlatform::IpadOs => Self::IpadOs,
            ProtoPlatform::Android => Self::Android,
            ProtoPlatform::Other => Self::Other,
        }
    }
}

/// This device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    /// The device's id.
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
    /// The name to show for it: the name this device gave it, or else the name it calls
    /// itself. Cleaned for display, but chosen by the other device: show the
    /// fingerprint wherever a person has to be sure which device it is.
    pub name: String,
    /// The system it runs.
    pub platform: Platform,
    /// Its fingerprint, as `abcd-efgh-ijkl-mnop-qrst`.
    pub fingerprint: String,
    /// When it was paired, in seconds since the Unix epoch.
    pub paired_at: u64,
    /// When this device last heard from it, in seconds since the Unix epoch.
    pub last_seen_at: Option<u64>,
}

impl From<&PairedDevice> for Device {
    fn from(device: &PairedDevice) -> Self {
        Self {
            id: device.id.to_string(),
            name: device.display_name().to_owned(),
            platform: device.platform.into(),
            fingerprint: device.id.fingerprint().to_string(),
            paired_at: device.added_at,
            last_seen_at: device.last_seen_at,
        }
    }
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

impl From<PathKind> for Route {
    fn from(path: PathKind) -> Self {
        match path {
            PathKind::Lan => Self::Lan,
            PathKind::Direct => Self::Direct,
            PathKind::Relay => Self::Relay,
            _ => Self::Unknown,
        }
    }
}

/// The answer to a ping: the device is there, and this is how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reach {
    /// The round trip time, in milliseconds.
    pub round_trip_ms: u32,
    /// The route the traffic took.
    pub route: Route,
}

impl From<PingReport> for Reach {
    fn from(report: PingReport) -> Self {
        Self {
            round_trip_ms: u32::try_from(report.rtt.as_millis()).unwrap_or(u32::MAX),
            route: report.path.into(),
        }
    }
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

impl From<&NodeStatus> for Network {
    fn from(status: &NodeStatus) -> Self {
        Self {
            relay: status.relay.clone(),
            addresses: status
                .direct_addrs
                .iter()
                .map(ToString::to_string)
                .collect(),
            lan_discovery: status.lan_discovery,
        }
    }
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

// What was received may be a password or a link with a sign-in token in it, and values
// like this one end up in logs. Only sizes are printed.
impl fmt::Debug for ReceivedItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    /// [`App::remove_received`](crate::App::remove_received).
    pub number: u64,
    /// The id of the device that sent it.
    pub from_id: String,
    /// The name that device was shown under when this arrived.
    pub from_name: String,
    /// When it arrived, in seconds since the Unix epoch.
    pub received_at: u64,
    /// What it is.
    pub item: ReceivedItem,
}

impl From<&InboxEntry> for Received {
    fn from(entry: &InboxEntry) -> Self {
        Self {
            number: entry.number,
            from_id: entry.from.to_string(),
            from_name: entry.from_name.clone(),
            received_at: entry.received_at,
            item: match &entry.item {
                InboxItem::Link { url, title } => ReceivedItem::Link {
                    url: url.clone(),
                    title: title.clone(),
                },
                InboxItem::Text { text } => ReceivedItem::Text { text: text.clone() },
            },
        }
    }
}

/// An invitation to pair, as the person who made it sees it.
#[derive(Clone, PartialEq, Eq)]
pub struct Offer {
    /// The ticket to show as a QR code, or as text to copy, on this device and to enter on
    /// the other. It is a secret until it has been used: show it to the person, keep it out
    /// of logs, and do not send it anywhere but their other device.
    pub ticket: String,
    /// When the offer lapses, in seconds since the Unix epoch.
    pub expires_at: u64,
}

impl fmt::Debug for Offer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Offer")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// A device asking to pair, for the person to answer.
///
/// Show all of it, with the code most prominent, and say what the code is to be compared
/// with: the one the other device shows. The person accepts only if the two are the same.
/// Answer with [`App::answer_pairing`](crate::App::answer_pairing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequest {
    /// Names this request when answering it.
    pub id: u64,
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

/// How an [`Offer`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferOutcome {
    /// A device was paired.
    Paired(Device),
    /// The time ran out.
    Expired,
    /// It was withdrawn, or replaced by a newer offer.
    Cancelled,
    /// The person declined the request.
    Declined,
    /// The device that asked disconnected before the person answered.
    Abandoned,
    /// Something went wrong; the sentence says what.
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_received_is_not_printed() {
        let text = ReceivedItem::Text {
            text: "hunter2".to_owned(),
        };
        let link = ReceivedItem::Link {
            url: "https://example.com/reset?token=abc".to_owned(),
            title: Some("Reset your password".to_owned()),
        };
        for shown in [format!("{text:?}"), format!("{link:?}")] {
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(!shown.contains("token"), "{shown}");
            assert!(!shown.contains("Reset"), "{shown}");
        }
    }

    #[test]
    fn a_ticket_is_not_printed() {
        let offer = Offer {
            ticket: "BATON1:SECRET".to_owned(),
            expires_at: 1,
        };
        assert!(!format!("{offer:?}").contains("SECRET"));
    }
}
