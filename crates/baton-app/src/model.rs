// SPDX-License-Identifier: Apache-2.0

//! The plain values the app layer hands to a screen.
//!
//! Everything here is data: strings, numbers and enums. A bridge to another language can
//! copy these without knowing anything about Baton's internals, and a screen can keep them
//! as long as it likes. Ids are text (64 hexadecimal characters) so that they can be kept
//! in a widget, a route or a test as they are.

use std::{fmt, path::PathBuf};

use baton_core::{
    InboxEntry, InboxItem, PairedDevice, StartPolicy,
    link::PathKind,
    node::{NodeStatus, PingReport},
    transfer::send::{Progress, Stage},
};
use baton_proto::Platform as ProtoPlatform;
use baton_stream::{
    control::SilentReason,
    gate::Reason,
    input::{Button, Modifiers, PointerAction},
    input_queue::Input,
};

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

/// What the person at this device allowed a paired device to do without being asked each time:
/// ask this device to start sharing its screen with it (`docs/adr/0010`). It is off for a device
/// that was just paired, and only the person at this device can change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartAccess {
    /// Nothing. A device that asks is told that the settings do not let it.
    #[default]
    Off,
    /// Look at the screen of this device.
    Look,
    /// Look at the screen of this device, and control it.
    Control,
}

impl From<StartPolicy> for StartAccess {
    fn from(policy: StartPolicy) -> Self {
        match policy {
            StartPolicy::Off => Self::Off,
            StartPolicy::Look => Self::Look,
            StartPolicy::Control => Self::Control,
        }
    }
}

impl From<StartAccess> for StartPolicy {
    fn from(access: StartAccess) -> Self {
        match access {
            StartAccess::Off => Self::Off,
            StartAccess::Look => Self::Look,
            StartAccess::Control => Self::Control,
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

impl ReceivedItem {
    /// The address to open, if this is a link that a screen may open in the web browser.
    /// See [`web_address`].
    pub fn web_address(&self) -> Option<String> {
        match self {
            Self::Link { url, .. } => web_address(url),
            Self::Text { .. } => None,
        }
    }
}

/// The address to open for a received link, if it is a web address, one that starts with
/// `http` or `https`.
///
/// The address of a received link was chosen by another device, and the protocol leaves it
/// to the receiver to decide which kinds to open. `file:`, `smb:`, `tel:`, `sms:` and the
/// schemes that other apps register can start programs, reach files, dial a number or send
/// a message, and a person who tapped "open" on an item in a list has not agreed to that.
/// So a screen opens a received link only if this returns an address, and offers to copy
/// the link otherwise. The `baton` command follows the same rule.
///
/// Open the address that comes back, not the text that was sent. It is the address as it
/// was checked, written out in full (scheme and host in lower case, no leading spaces, tabs
/// or line breaks, a backslash read as a slash, as web browsers read it), and the parser
/// that checked it is not the one that opens it. Other parsers can read the same text
/// differently, and then what is opened is not what was checked.
pub fn web_address(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.into())
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

/// A device asking to send files, for the person to answer.
///
/// Show who is asking and what is offered, and answer with
/// [`App::answer_transfer`](crate::App::answer_transfer). The names were written by the other
/// device. They are cleaned to be safe to show, and they say what the person is asked to take,
/// not what the files will be called here: a name that is taken in the folder gets a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    /// Names this request when answering it.
    pub id: u64,
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
    pub bytes: u64,
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
    pub bytes: u64,
}

// A file name can say more than the person wants in a log, so only sizes are printed.
impl fmt::Debug for PreviewName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreviewName")
            .field("name_bytes", &self.name.len())
            .field("folder", &self.folder)
            .field("files", &self.files)
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// How a transfer that this device took has ended.
#[derive(Clone, PartialEq, Eq)]
pub struct TransferEnd {
    /// The [`TransferRequest::id`] of the transfer.
    pub request: u64,
    /// The id of the device that sent it.
    pub device_id: String,
    /// The name this device shows for it.
    pub device_name: String,
    /// How many files the transfer held.
    pub files: u32,
    /// How many bytes they add up to.
    pub bytes: u64,
    /// The first of the items that were offered, as the request showed them.
    pub preview: Vec<PreviewName>,
    /// Where the items that arrived are: the files and folders at the top of what was offered, in
    /// the order they were offered, each at the full path it was put at. A name that was taken in
    /// the folder has a number, so these are the names to look for, and not the ones in
    /// `preview`. An item that could not be saved is not among them, and none are when the
    /// transfer was dropped. A screen can show the person where the files are with these.
    pub saved: Vec<PathBuf>,
    /// How it ended.
    pub result: TransferResult,
}

// A path has the name of a file in it, and the folder that the person keeps their files in, so
// only how many there are is printed.
impl fmt::Debug for TransferEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferEnd")
            .field("request", &self.request)
            .field("device_id", &self.device_id)
            .field("device_name", &self.device_name)
            .field("files", &self.files)
            .field("bytes", &self.bytes)
            .field("preview", &self.preview)
            .field("saved", &self.saved.len())
            .field("result", &self.result)
            .finish()
    }
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
    /// The transfer was given up, and nothing of it is kept: the sender stopped, the device was
    /// unpaired, or the sender did not come back.
    Dropped,
}

/// Whether this device takes files, and where they go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiving {
    /// The app gave no folder for files, so this device takes none.
    Off,
    /// Files are taken, after the person says yes to each transfer.
    On,
    /// The app gave a folder, and Baton cannot use it, so this device takes no files. The
    /// sentence says why.
    Unusable(String),
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
    pub bytes: u64,
    /// How many bytes have gone out in this attempt. An attempt that is tried again starts
    /// from 0, because it sends everything again.
    pub sent: u64,
}

impl From<Stage> for SendStage {
    fn from(stage: Stage) -> Self {
        match stage {
            Stage::Connecting => Self::Connecting,
            Stage::Asking => Self::Asking,
            Stage::Finishing => Self::Finishing,
            Stage::Waiting => Self::Waiting,
            Stage::Over => Self::Over,
            // `Sending` and any stage added later: the transfer is under way.
            _ => Self::Sending,
        }
    }
}

impl From<Progress> for FileProgress {
    fn from(progress: Progress) -> Self {
        Self {
            stage: progress.stage.into(),
            attempt: progress.attempt,
            files: progress.files,
            bytes: progress.bytes,
            sent: progress.sent,
        }
    }
}

/// A transfer that the other device answered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesSent {
    /// How many files the transfer held.
    pub files: u32,
    /// How many bytes they add up to.
    pub bytes: u64,
    /// How many of the files are not in place on the other device. The others are.
    pub failed: u32,
    /// How many things in what was chosen were left out and not offered at all: links inside a
    /// folder, files that could not be read, and the like.
    pub left_out: u32,
}

/// What a device does with windows, as a screen needs to know it to offer only what is possible.
// Four things a device can do, each of which it can or cannot on its own: a state that combined
// them would only hide which one a screen means.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowAbilities {
    /// This device shows the windows that paired devices offer.
    pub can_show: bool,
    /// This device shares its windows with paired devices.
    pub can_share: bool,
    /// The person can let another device control a window that this one shares.
    pub can_control: bool,
    /// This device can give its screen to a paired device that asks for it, with no picker, if the
    /// person here allowed that device ([`App::set_start_access`](crate::App::set_start_access)).
    /// A screen offers that setting only where this is so.
    pub can_give_screen: bool,
}

/// A window, or a whole display, that a paired device offers to show on this one, for the person
/// to answer.
///
/// Show what is offered and which device offers it, and whether that device lets this one
/// control it. The title and the name of the application were written by the other device and
/// are cleaned for display, but they are labels and nothing more. Answer with
/// [`App::answer_window`](crate::App::answer_window).
#[derive(Clone, PartialEq, Eq)]
pub struct WindowRequest {
    /// Names this request when answering it.
    pub id: u64,
    /// The id of the device that offers the window.
    pub device_id: String,
    /// The name this device shows for it.
    pub device_name: String,
    /// Whether what is offered is a whole display, with every window on it, and not one window.
    pub display: bool,
    /// The title of the window, or the name of the display, or `None` if it has none.
    pub title: Option<String>,
    /// The name of the application the window belongs to, or `None`.
    pub app: Option<String>,
    /// The width of the window in pixels.
    pub width: u32,
    /// The height of the window in pixels.
    pub height: u32,
    /// Whether the person at the other device lets this one control the window. It cannot be
    /// asked for and cannot be turned on later.
    pub control: bool,
}

// What a window is called can name a document, a person or a page.
impl fmt::Debug for WindowRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowRequest")
            .field("id", &self.id)
            .field("display", &self.display)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("control", &self.control)
            .finish_non_exhaustive()
    }
}

/// Which end of a window this device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    /// The window is on another device, and this one shows it.
    Viewing,
    /// The window is on this device, and another one shows it.
    Sharing,
}

/// A window, or a whole display, that is shown on this device or shared from it.
// Five facts about a session (a whole display, control, private, sound, leave), each of its own: a
// state that combined them would only hide which one a screen means.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, PartialEq, Eq)]
pub struct WindowSession {
    /// Names this session when ending it with [`App::end_window`](crate::App::end_window),
    /// and in [`Event::WindowEnded`](crate::Event::WindowEnded). Counts up from 1 while the app
    /// runs and is not reused.
    pub id: u64,
    /// Which end this device is.
    pub role: WindowRole,
    /// The id of the other device.
    pub device_id: String,
    /// The name this device shows for it.
    pub device_name: String,
    /// Whether it is a whole display, with every window on it, and not one window.
    pub display: bool,
    /// The title of the window, or the name of the display, or `None`.
    pub title: Option<String>,
    /// The name of the application the window belongs to, or `None`.
    pub app: Option<String>,
    /// The width of the window in pixels, as it was offered. The picture may differ from it, and
    /// change while the session runs. It is nought for a screen that this device asked another to
    /// start sharing: that screen is shown before its size is known, and the size comes with the
    /// first picture ([`Event::WindowPicture`](crate::Event::WindowPicture)).
    pub width: u32,
    /// The height of the window in pixels, as it was offered; nought in the same case.
    pub height: u32,
    /// Whether the viewer may control the window.
    pub control: bool,
    /// Whether the screen is kept private at the device that shares it: dark, and with the keys
    /// and the pointer there out of use, until the device that shows it says otherwise with
    /// [`App::window_lift`](crate::App::window_lift).
    pub private: bool,
    /// Whether the person here can ask to hear the sound of the screen with
    /// [`App::hear_window`](crate::App::hear_window): what is shown here is a whole screen, this
    /// device plays sound, and the device that shares the screen sends it. That device may still
    /// have none to send, which [`Event::WindowSound`](crate::Event::WindowSound) says. Sound is
    /// off when the session starts. `false` for a window that this device shares.
    pub sound: bool,
    /// Whether the person here can have the device that shares the screen left as it is when they
    /// end the session, and not locked, with
    /// [`App::end_window_leaving`](crate::App::end_window_leaving): the screen is kept private
    /// there, and the device said that it can end that without locking itself
    /// (`docs/adr/0014-lock-the-mac-or-leave-it.md`). The person is asked every time they end
    /// such a session, and the device is locked unless they say otherwise. `false` for a window
    /// that this device shares.
    pub leave: bool,
    /// What to draw the picture with, when the window is shown on this device: the number that the
    /// platform gave the screen. `None` when the window is shared from this device.
    pub screen: Option<i64>,
}

// What a window is called can name a document, a person or a page.
impl fmt::Debug for WindowSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowSession")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("display", &self.display)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("control", &self.control)
            .field("private", &self.private)
            .field("sound", &self.sound)
            .field("leave", &self.leave)
            .field("screen", &self.screen)
            .finish_non_exhaustive()
    }
}

/// How the sound of a screen that is shown here stands, as
/// [`Event::WindowSound`](crate::Event::WindowSound) tells it.
///
/// Sound is off until the person asks for it with [`App::hear_window`](crate::App::hear_window).
/// The other device may then begin to send it, or say why it cannot, and this device may find that
/// it cannot play it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSound {
    /// The other device sends the sound of its screen, and it plays on this one. The other device
    /// keeps its own speakers quiet for as long as it does.
    Playing,
    /// There is no sound because the person turned it off.
    Off,
    /// The other device sends none: what it shares is a window, and only a whole screen has sound.
    Window,
    /// The other device sends none: its system has not let Baton record the sound, and the person
    /// there has to allow it. The other device goes on by itself when they have.
    NoPermission,
    /// The other device sends none: something went wrong there, where the sound is recorded.
    Failed,
    /// The other device sends none, for a reason that this version does not know.
    Other,
    /// This device could not play the sound: what plays it could not be made or started, because a
    /// call holds the audio of a phone, say, or there is nothing to play on. The other device was
    /// told to stop, so it is not quiet for nothing. The person can try again with
    /// [`App::hear_window`](crate::App::hear_window).
    CannotPlay,
}

impl From<SilentReason> for WindowSound {
    fn from(reason: SilentReason) -> Self {
        match reason {
            SilentReason::Window => Self::Window,
            SilentReason::NoPermission => Self::NoPermission,
            SilentReason::Failed => Self::Failed,
            SilentReason::Other => Self::Other,
        }
    }
}

/// How a window session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowEnd {
    /// This device ended it, or the device was unpaired, or the app stopped.
    ThisDevice,
    /// The other device ended it.
    OtherDevice,
    /// The connection to the other device was lost, or the other device stopped answering.
    Lost,
    /// Something went wrong; the sentence says what.
    Failed(String),
}

/// A button of a pointer on the picture of a window. A finger is the left one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowButton {
    /// The primary button, or a touch.
    Left,
    /// The secondary button.
    Right,
    /// The middle button.
    Middle,
}

/// What a pointer did on the picture of a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindowPointer {
    /// It moved, with a button down or not.
    Move,
    /// A button went down.
    Down(WindowButton),
    /// A button came up.
    Up(WindowButton),
    /// A scroll, in points: positive `dx` moves the content to the right and positive `dy` down.
    Scroll {
        /// Along the x axis.
        dx: f32,
        /// Along the y axis.
        dy: f32,
    },
}

/// The modifier keys that are down.
// Five keys of a keyboard, each of which is down or not on its own: a state that combined them
// would only hide which one a screen means.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowModifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub control: bool,
    /// Option, also called Alt.
    pub option: bool,
    /// Command, also called the Windows key or Meta.
    pub command: bool,
    /// Caps lock.
    pub caps_lock: bool,
}

/// What the person did on the picture of a window that is shown here, for
/// [`App::window_input`](crate::App::window_input).
///
/// A position is a fraction of the picture as it is drawn, from 0 to 1 across and from 0 to 1
/// down from its top left corner. A key is a USB HID usage of the keyboard page (`a` is 4).
/// Flutter's `PhysicalKeyboardKey.usbHidUsage` carries the page above the usage, so a screen sends
/// its low 16 bits only when the page is 7, and sends no key for a usage of another page, such as
/// a media key, whose low bits would be taken for a key of the keyboard. Code 0 means text that
/// came with no key, such as dictation or the keyboard on the screen: it must go down once, carry
/// the text and not repeat.
///
/// Taking an event does not mean that the other device does it: it drops what does not belong to
/// the window, such as a touch outside it, and says so with
/// [`Event::WindowNotice`](crate::Event::WindowNotice).
#[derive(Clone, PartialEq)]
pub enum WindowInput {
    /// A finger or a pointer.
    Pointer {
        /// Across the picture, from 0 to 1.
        x: f32,
        /// Down the picture, from 0 to 1.
        y: f32,
        /// The modifier keys that are down.
        modifiers: WindowModifiers,
        /// What it did.
        action: WindowPointer,
    },
    /// A key, or text that came with no key.
    Key {
        /// A USB HID usage of the keyboard page, or 0 for text with no key.
        code: u16,
        /// Whether the key went down (true) or came up (false).
        down: bool,
        /// The modifier keys that are down after this event.
        modifiers: WindowModifiers,
        /// Whether the system repeats a key that is held.
        repeat: bool,
        /// The text that the key produced. Long text is cut into pieces, and characters that
        /// cannot be typed are left out.
        text: Option<String>,
    },
}

// What was typed stays out of every log, so a key is only said to be down or up.
impl fmt::Debug for WindowInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pointer {
                x,
                y,
                modifiers,
                action,
            } => f
                .debug_struct("Pointer")
                .field("x", x)
                .field("y", y)
                .field("modifiers", modifiers)
                .field("action", action)
                .finish(),
            Self::Key {
                down, repeat, text, ..
            } => f
                .debug_struct("Key")
                .field("down", down)
                .field("repeat", repeat)
                .field("text_bytes", &text.as_ref().map(String::len))
                .finish_non_exhaustive(),
        }
    }
}

impl From<WindowButton> for Button {
    fn from(button: WindowButton) -> Self {
        match button {
            WindowButton::Left => Self::Left,
            WindowButton::Right => Self::Right,
            WindowButton::Middle => Self::Middle,
        }
    }
}

impl From<WindowPointer> for PointerAction {
    fn from(action: WindowPointer) -> Self {
        match action {
            WindowPointer::Move => Self::Move,
            WindowPointer::Down(button) => Self::Down(button.into()),
            WindowPointer::Up(button) => Self::Up(button.into()),
            WindowPointer::Scroll { dx, dy } => Self::Scroll { dx, dy },
        }
    }
}

impl From<WindowModifiers> for Modifiers {
    fn from(keys: WindowModifiers) -> Self {
        [
            (keys.shift, Self::SHIFT),
            (keys.control, Self::CONTROL),
            (keys.option, Self::OPTION),
            (keys.command, Self::COMMAND),
            (keys.caps_lock, Self::CAPS_LOCK),
        ]
        .into_iter()
        .filter(|(down, _)| *down)
        .fold(Self::NONE, |all, (_, key)| all | key)
    }
}

impl From<WindowInput> for Input {
    fn from(input: WindowInput) -> Self {
        match input {
            WindowInput::Pointer {
                x,
                y,
                modifiers,
                action,
            } => Self::Pointer {
                x,
                y,
                modifiers: modifiers.into(),
                action: action.into(),
            },
            WindowInput::Key {
                code,
                down,
                modifiers,
                repeat,
                text,
            } => Self::Key {
                code,
                down,
                modifiers: modifiers.into(),
                repeat,
                text,
            },
        }
    }
}

/// Why the other device did not do some of what the person did on a window.
///
/// A device drops what it must not do to a window: an event that comes when the window is not
/// where the person thought it was, or when something else has the keyboard, or when it is
/// locked. The sentences for a screen are in the order of how often the reasons come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowNoticeReason {
    /// The person at the other device did not let this one control the window, or turned control
    /// off.
    ControlOff,
    /// The window is paused, for example because it was hidden.
    Paused,
    /// The other device is locked, or its display is off.
    Locked,
    /// The window is not on the screen of the other device.
    NotOnScreen,
    /// Events came faster than the other device takes them.
    TooFast,
    /// The picture that the person touched is too old: the window has changed since.
    OutOfDate,
    /// The other device does not know the picture that the person touched, so it cannot tell
    /// where on the window the touch was.
    PictureUnknown,
    /// The window changed its size since the picture that the person touched.
    Resized,
    /// The touch was outside the window.
    OutsideWindow,
    /// Another window was in front of the window where it was touched.
    Covered,
    /// The window does not have the keyboard.
    NoKeyboard,
    /// A window of the same application is in front of the window, so a key might go to it.
    AppWindowInFront,
    /// More keys are held down than a person has fingers.
    TooManyKeys,
    /// The release of a button or a key that was not pressed.
    NotPressed,
    /// A rule of a later version, or an event that the other device found not valid.
    Other,
}

impl From<Reason> for WindowNoticeReason {
    fn from(reason: Reason) -> Self {
        match reason {
            Reason::ControlOff => Self::ControlOff,
            Reason::Paused => Self::Paused,
            Reason::Locked => Self::Locked,
            Reason::NotOnScreen => Self::NotOnScreen,
            Reason::TooFast => Self::TooFast,
            Reason::FrameUnknown => Self::PictureUnknown,
            Reason::PictureOutOfDate => Self::OutOfDate,
            Reason::WindowResized => Self::Resized,
            Reason::OutsideWindow => Self::OutsideWindow,
            Reason::Covered => Self::Covered,
            Reason::NoKeyboard => Self::NoKeyboard,
            Reason::AppWindowInFront => Self::AppWindowInFront,
            Reason::TooManyHeld => Self::TooManyKeys,
            Reason::NotPressed => Self::NotPressed,
            Reason::Malformed | Reason::Other => Self::Other,
        }
    }
}

/// What came of asking a paired device to start sharing its screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenStarted {
    /// The other device offered its screen, this one took it without asking anybody, and it is
    /// shown. A screen that is started has no title and no application, and its size is not known
    /// until the first picture ([`Event::WindowPicture`](crate::Event::WindowPicture)). Its end
    /// comes as [`Event::WindowEnded`](crate::Event::WindowEnded).
    Started(WindowSession),
    /// The other device did not start it; the sentence says why, in words that name the device and
    /// tell the person what to do about it.
    NotStarted(String),
}

/// What came of offering a window to a paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowShared {
    /// The person gave up choosing a window, so nothing was offered.
    NothingChosen,
    /// The person at the other device said yes, and the window is shared. Its end comes as
    /// [`Event::WindowEnded`](crate::Event::WindowEnded).
    Started(WindowSession),
    /// The other device did not take the window; the sentence says why, in words that name the
    /// device.
    Declined(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_window_is_called_is_not_printed() {
        let request = WindowRequest {
            id: 1,
            device_id: "ab".repeat(32),
            device_name: "Studio Mac".to_owned(),
            display: false,
            title: Some("Salary 2027.xlsx".to_owned()),
            app: Some("Numbers".to_owned()),
            width: 1440,
            height: 900,
            control: false,
        };
        let session = WindowSession {
            id: 1,
            role: WindowRole::Viewing,
            device_id: "ab".repeat(32),
            device_name: "Studio Mac".to_owned(),
            display: true,
            title: Some("Salary 2027.xlsx".to_owned()),
            app: Some("Numbers".to_owned()),
            width: 1440,
            height: 900,
            control: false,
            private: false,
            sound: false,
            leave: false,
            screen: Some(7),
        };
        for shown in [format!("{request:?}"), format!("{session:?}")] {
            assert!(!shown.contains("Salary"), "{shown}");
            assert!(!shown.contains("Numbers"), "{shown}");
        }
        // Whether it is a window or a whole screen is not a name, and a log says which.
        assert!(format!("{request:?}").contains("display: false"));
        assert!(format!("{session:?}").contains("display: true"));
    }

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
    fn the_names_of_files_are_not_printed() {
        let name = PreviewName {
            name: "tax return 2026.pdf".to_owned(),
            folder: false,
            files: 0,
            bytes: 4096,
        };
        let request = TransferRequest {
            id: 1,
            device_id: "0".repeat(64),
            device_name: "Kitchen iPad".to_owned(),
            items: 1,
            files: 1,
            folders: 0,
            bytes: 4096,
            preview: vec![name.clone()],
        };
        let end = TransferEnd {
            request: 1,
            device_id: "0".repeat(64),
            device_name: "Kitchen iPad".to_owned(),
            files: 1,
            bytes: 4096,
            preview: vec![name.clone()],
            saved: vec![PathBuf::from(
                "/Users/someone/Downloads/tax return 2026.pdf",
            )],
            result: TransferResult::Received { failed: 0 },
        };
        for shown in [
            format!("{name:?}"),
            format!("{request:?}"),
            format!("{end:?}"),
        ] {
            assert!(!shown.contains("tax"), "{shown}");
            assert!(!shown.contains(".pdf"), "{shown}");
            assert!(!shown.contains("someone"), "{shown}");
            assert!(shown.contains("4096"), "{shown}");
        }
        // How many were saved is printed.
        assert!(format!("{end:?}").contains("saved: 1"), "{end:?}");
    }

    #[test]
    fn the_stages_of_a_transfer_are_copied_one_for_one() {
        for (stage, copy) in [
            (Stage::Connecting, SendStage::Connecting),
            (Stage::Asking, SendStage::Asking),
            (Stage::Sending, SendStage::Sending),
            (Stage::Finishing, SendStage::Finishing),
            (Stage::Waiting, SendStage::Waiting),
            (Stage::Over, SendStage::Over),
        ] {
            assert_eq!(SendStage::from(stage), copy, "{stage:?}");
        }
    }

    #[test]
    fn only_web_links_may_be_opened() {
        for url in [
            "https://example.com/a?b=c",
            "http://example.com",
            "HTTPS://EXAMPLE.COM/",
        ] {
            assert!(web_address(url).is_some(), "{url}");
        }
        for url in [
            "file:///etc/passwd",
            "smb://server/share",
            "tel:+15555550100",
            "sms:+15555550100?body=hi",
            "mailto:a@example.com",
            "javascript:alert(1)",
            "obsidian://open?vault=x",
            "example.com",
            "",
            "not a link",
        ] {
            assert_eq!(web_address(url), None, "{url}");
        }
        let web = ReceivedItem::Link {
            url: "https://example.com/".to_owned(),
            title: None,
        };
        let phone = ReceivedItem::Link {
            url: "tel:+15555550100".to_owned(),
            title: Some("Call me".to_owned()),
        };
        let text = ReceivedItem::Text {
            text: "https://example.com/".to_owned(),
        };
        assert_eq!(web.web_address().as_deref(), Some("https://example.com/"));
        assert_eq!(phone.web_address(), None);
        assert_eq!(
            text.web_address(),
            None,
            "text is not a link, whatever it holds"
        );
    }

    #[test]
    fn what_is_opened_is_the_address_that_was_checked() {
        // The `url` parser cleans these up, and another parser may not. The address that
        // comes back is the cleaned one, and reading it again changes nothing.
        for (sent, opened) in [
            ("HTTPS://EXAMPLE.COM", "https://example.com/"),
            ("  https://example.com/a  ", "https://example.com/a"),
            ("https://exam\tple.com/pa\nth", "https://example.com/path"),
            ("https:\\\\example.com\\path", "https://example.com/path"),
            ("https://example.com/a b", "https://example.com/a%20b"),
            (
                "http://user@example.com:8080/",
                "http://user@example.com:8080/",
            ),
        ] {
            let address = web_address(sent).unwrap_or_default();
            assert_eq!(address, opened, "{sent:?}");
            assert_eq!(web_address(&address).as_deref(), Some(opened), "{sent:?}");
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

    #[test]
    fn each_modifier_key_is_its_own_bit_of_the_format() {
        let none = WindowModifiers::default();
        assert_eq!(Modifiers::from(none), Modifiers::NONE);
        let one = |keys: WindowModifiers| Modifiers::from(keys);
        assert_eq!(
            one(WindowModifiers {
                shift: true,
                ..none
            }),
            Modifiers::SHIFT
        );
        assert_eq!(
            one(WindowModifiers {
                control: true,
                ..none
            }),
            Modifiers::CONTROL
        );
        assert_eq!(
            one(WindowModifiers {
                option: true,
                ..none
            }),
            Modifiers::OPTION
        );
        assert_eq!(
            one(WindowModifiers {
                command: true,
                ..none
            }),
            Modifiers::COMMAND
        );
        assert_eq!(
            one(WindowModifiers {
                caps_lock: true,
                ..none
            }),
            Modifiers::CAPS_LOCK
        );
        assert_eq!(
            one(WindowModifiers {
                shift: true,
                command: true,
                ..none
            }),
            Modifiers::SHIFT | Modifiers::COMMAND
        );
        let all = WindowModifiers {
            shift: true,
            control: true,
            option: true,
            command: true,
            caps_lock: true,
        };
        assert_eq!(one(all).bits(), 0b1_1111);
    }

    #[test]
    fn what_the_person_does_reaches_the_queue_as_it_was_given() {
        let modifiers = WindowModifiers {
            shift: true,
            ..WindowModifiers::default()
        };
        for (action, wanted) in [
            (WindowPointer::Move, PointerAction::Move),
            (
                WindowPointer::Down(WindowButton::Left),
                PointerAction::Down(Button::Left),
            ),
            (
                WindowPointer::Down(WindowButton::Right),
                PointerAction::Down(Button::Right),
            ),
            (
                WindowPointer::Up(WindowButton::Middle),
                PointerAction::Up(Button::Middle),
            ),
            (
                WindowPointer::Scroll { dx: 1.5, dy: -2.0 },
                PointerAction::Scroll { dx: 1.5, dy: -2.0 },
            ),
        ] {
            let input = WindowInput::Pointer {
                x: 0.25,
                y: 0.75,
                modifiers,
                action,
            };
            assert_eq!(
                Input::from(input),
                Input::Pointer {
                    x: 0.25,
                    y: 0.75,
                    modifiers: Modifiers::SHIFT,
                    action: wanted,
                },
                "{action:?}"
            );
        }
        let key = WindowInput::Key {
            code: 4,
            down: true,
            modifiers,
            repeat: true,
            text: Some("a".to_owned()),
        };
        assert_eq!(
            Input::from(key),
            Input::Key {
                code: 4,
                down: true,
                modifiers: Modifiers::SHIFT,
                repeat: true,
                text: Some("a".to_owned()),
            }
        );
    }

    #[test]
    fn what_was_typed_is_not_printed() {
        let key = WindowInput::Key {
            code: 4,
            down: true,
            modifiers: WindowModifiers::default(),
            repeat: false,
            text: Some("hunter2".to_owned()),
        };
        let shown = format!("{key:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(!shown.contains("code"), "{shown}");
        // Only how long it was.
        assert!(shown.contains("text_bytes: Some(7)"), "{shown}");
    }
}
