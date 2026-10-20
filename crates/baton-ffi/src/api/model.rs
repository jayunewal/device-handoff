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

// A file name can say more than the person wants in a log, so only sizes are printed. The Dart
// class has no `toString` to do the same, so the name prints wherever a screen prints it: see
// "Dart prints what it holds" in docs/architecture.md.
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
#[derive(Clone, PartialEq, Eq)]
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
    /// Where the items that arrived are: the files and folders at the top of what was offered, in
    /// the order they were offered, each at the full path it was put at. A name that was taken in
    /// the folder has a number, so these are the names to look for, and not the ones in
    /// `preview`. An item that could not be saved is not among them, and none are when the
    /// transfer was dropped.
    pub saved: Vec<String>,
    /// How it ended.
    pub result: TransferResult,
}

// A path has the name of a file in it, and the folder that the person keeps their files in, so
// only how many there are is printed. The Dart class has no `toString` to do the same, so the
// paths print wherever a screen prints `saved`: see "Dart prints what it holds" in
// docs/architecture.md.
impl std::fmt::Debug for TransferEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// What this device does with windows, so that a screen offers only what is possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowAbilities {
    /// This device shows the windows that paired devices offer.
    pub can_show: bool,
    /// This device shares its windows with paired devices.
    pub can_share: bool,
    /// The person can let another device control a window that this one shares.
    pub can_control: bool,
}

/// A window, or a whole display, that a paired device offers to show on this one, for the person
/// to answer.
///
/// Show what is offered and which device offers it, and whether that device lets this one
/// control it. The title and the name of the application were written by the other device: they
/// are labels, and nothing else. Answer with `Baton.answerWindow`.
#[derive(Clone, PartialEq, Eq)]
pub struct WindowRequest {
    /// Names this request when answering it with `Baton.answerWindow`.
    pub id: i64,
    /// The id of the device that offers the window.
    pub device_id: String,
    /// The name this device shows for it.
    pub device_name: String,
    /// Whether what is offered is a whole display, with every window on it, and not one window.
    pub display: bool,
    /// The title of the window, or the name of the display, or `null` if it has none.
    pub title: Option<String>,
    /// The name of the application the window belongs to, or `null`.
    pub app: Option<String>,
    /// The width of the window in pixels.
    pub width: u32,
    /// The height of the window in pixels.
    pub height: u32,
    /// Whether the person at the other device lets this one control the window.
    pub control: bool,
}

// What a window is called can name a document, a person or a page, and values like this one end
// up in logs, so only sizes are printed.
impl std::fmt::Debug for WindowRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
#[derive(Clone, PartialEq, Eq)]
pub struct WindowSession {
    /// Names this session when ending it with `Baton.endWindow`, and in `Event.windowEnded`.
    /// Counts up from 1 while Baton runs and is not reused.
    pub id: i64,
    /// Which end this device is.
    pub role: WindowRole,
    /// The id of the other device.
    pub device_id: String,
    /// The name this device shows for it.
    pub device_name: String,
    /// Whether it is a whole display, with every window on it, and not one window.
    pub display: bool,
    /// The title of the window, or the name of the display, or `null`.
    pub title: Option<String>,
    /// The name of the application the window belongs to, or `null`.
    pub app: Option<String>,
    /// The width of the window in pixels, as it was offered. The picture may differ from it, and
    /// change while the session runs.
    pub width: u32,
    /// The height of the window in pixels, as it was offered.
    pub height: u32,
    /// Whether the viewer may control the window.
    pub control: bool,
    /// What to draw the picture with when the window is shown on this device: the number of
    /// the screen, which on Flutter is the id of a `Texture`. `null` when the window is shared
    /// from this device.
    pub screen: Option<i64>,
}

// See `WindowRequest`.
impl std::fmt::Debug for WindowSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowSession")
            .field("id", &self.id)
            .field("role", &self.role)
            .field("display", &self.display)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("control", &self.control)
            .field("screen", &self.screen)
            .finish_non_exhaustive()
    }
}

/// How a window session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowEnd {
    /// This device ended it, or the device was unpaired, or Baton stopped.
    ThisDevice,
    /// The other device ended it.
    OtherDevice,
    /// The connection to the other device was lost, or the other device stopped answering.
    Lost,
    /// Something went wrong.
    Failed {
        /// One sentence that says what.
        message: String,
    },
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
    Down {
        /// Which.
        button: WindowButton,
    },
    /// A button came up.
    Up {
        /// Which.
        button: WindowButton,
    },
    /// A scroll, in points: positive `dx` moves the content to the right and positive `dy`
    /// down, as a finger that drags it does.
    Scroll {
        /// Along the x axis.
        dx: f32,
        /// Along the y axis.
        dy: f32,
    },
}

/// The modifier keys that are down.
// Five keys of a keyboard, each of which is down or not on its own.
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
/// `Baton.windowInput`.
///
/// A position is a fraction of the picture as it is drawn, from 0 to 1 across and from 0 to 1
/// down from its top left corner. A key is a USB HID usage of the keyboard page (`a` is 4).
/// Flutter's `PhysicalKeyboardKey.usbHidUsage` carries the page above the usage, so a screen sends
/// its low 16 bits only when the page is 7, and sends no key for a usage of another page, such as
/// a media key, whose low bits would be taken for a key of the keyboard. Code 0 means text that
/// came with no key, such as dictation or the keyboard on the screen: it must go down once, carry
/// the text and not repeat.
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
        /// The text that the key produced, or `null`. Long text is cut into pieces, and
        /// characters that cannot be typed are left out.
        text: Option<String>,
    },
}

// What was typed stays out of every log, so a key is only said to be down or up.
impl std::fmt::Debug for WindowInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

/// Why the other device did not do some of what the person did on a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowNoticeReason {
    /// The person at the other device did not let this one control the window, or turned
    /// control off.
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

/// What came of offering a window to a paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowShared {
    /// The person gave up choosing a window, so nothing was offered.
    NothingChosen,
    /// The person at the other device said yes, and the window is shared. Its end comes as
    /// `Event.windowEnded`.
    Started {
        /// The session.
        session: WindowSession,
    },
    /// The other device did not take the window.
    Declined {
        /// One sentence that says why, in words that name the device.
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
    /// A paired device offers a window to show on this one. Show the request and answer it with
    /// `Baton.answerWindow`.
    WindowRequested {
        /// The request.
        request: WindowRequest,
    },
    /// A window request no longer needs an answer: the other device withdrew it, the time ran
    /// out, or the connection ended. Take the question off the screen. The person's own answer
    /// is not announced with this event: a screen takes the question down when the person
    /// answers, and draws the window when `Event.windowStarted` says so. A yes that comes as the
    /// offer is withdrawn, or whose session then fails to begin, ends with no event at all.
    WindowRequestEnded {
        /// The `WindowRequest.id` that ended.
        id: i64,
        /// One sentence to show when the person said yes and this device could not show the
        /// window after all; `null` otherwise.
        failure: Option<String>,
    },
    /// A window is shown on this device or shared from it. When it is shown here,
    /// `WindowSession.screen` names the screen to draw it with.
    WindowStarted {
        /// The session.
        session: WindowSession,
    },
    /// The picture of a window that is shown here is this many pixels wide and high. The source
    /// says so before its first picture and again whenever it changes the size, so a screen
    /// lays the picture out in this shape and not in that of the window that was offered. It
    /// comes after the `Event.windowStarted` of the same window, and not for a window that is
    /// shared from this device.
    WindowPicture {
        /// The `WindowSession.id` whose picture this is.
        id: i64,
        /// The width of the picture in pixels, at least 1.
        width: u32,
        /// The height of the picture in pixels, at least 1.
        height: u32,
    },
    /// The other device did not do some of what the person did on a window that is shown here:
    /// it dropped events, for the reason named, and `count` is how many. Say so in a sentence,
    /// so that a tap that did nothing is not a mystery. The other device sends at most one a
    /// second, naming the reason that cost the most events since the last. It comes after the
    /// `Event.windowStarted` of the same window, and not for a window that is shared from this
    /// device.
    WindowNotice {
        /// The `WindowSession.id` whose events were dropped.
        id: i64,
        /// The rule that dropped the most of them.
        reason: WindowNoticeReason,
        /// How many events it dropped, at least 1.
        count: u32,
    },
    /// A window that was shown or shared is over.
    WindowEnded {
        /// The `WindowSession.id` that ended.
        id: i64,
        /// How.
        end: WindowEnd,
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
