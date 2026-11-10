// SPDX-License-Identifier: Apache-2.0

//! Messages exchanged between paired devices on the session protocol.
//!
//! Each request travels on its own bidirectional stream and gets exactly one
//! response, much like a call over HTTP. The data of a file transfer travels on streams of its
//! own, described in [`transfer`](crate::transfer).

use serde::{Deserialize, Serialize};

use crate::{
    device::DeviceInfo,
    id::ItemId,
    limits::{
        LimitError, MAX_CAPABILITIES, MAX_CAPABILITY_BYTES, MAX_CLIPBOARD_TEXT_BYTES,
        MAX_ERROR_MESSAGE_BYTES, MAX_FRAME_LEN, MAX_TITLE_BYTES, MAX_URL_BYTES, check_len,
        check_non_empty_len,
    },
    stream::{StreamReply, StreamRequest},
    transfer::{TransferReply, TransferRequest},
    unlock::{UnlockReply, UnlockRequest},
};

/// Capability identifiers a device can advertise in [`Hello`].
///
/// A device only sends what the peer advertised, so new kinds of items can be
/// added without breaking older peers.
pub mod capability {
    /// Accepts [`Item::Link`](super::Item::Link).
    pub const LINK: &str = "handoff.link";
    /// Accepts [`Item::ClipboardText`](super::Item::ClipboardText).
    pub const CLIPBOARD_TEXT: &str = "handoff.clipboard.text";
    /// Accepts [`Request::Transfer`](super::Request::Transfer), and with it the lanes of
    /// section 8 of `docs/protocol.md`: offers of files and folders.
    pub const TRANSFER_FILES: &str = "transfer.files";
    /// Can offer a window of its own to a paired device: makes a
    /// [`StreamRequest::Offer`](crate::stream::StreamRequest::Offer) and takes part in the
    /// session that follows, as the source (section 9 of `docs/protocol.md`).
    pub const STREAM_SHARE: &str = "stream.share";
    /// Can show the window of a paired device: accepts
    /// [`Request::Stream`](super::Request::Stream) and takes part in the session that follows,
    /// as the viewer.
    pub const STREAM_VIEW: &str = "stream.view";
    /// Takes input from a viewer, in a session where the person at the source granted control.
    pub const STREAM_CONTROL: &str = "stream.control";
    /// Starts a session when a paired device asks it to: accepts
    /// [`StreamRequest::Start`](crate::stream::StreamRequest::Start), answers it, and then makes
    /// the offer, as the source. Only a device that advertises [`STREAM_SHARE`] advertises it
    /// (section 9 of `docs/protocol.md`).
    pub const STREAM_START: &str = "stream.start";
    /// Can send the sound of the screen it shares, as the source of a session of a `Display`:
    /// takes a `Hear` from the viewer, and writes `Sound` and `Silent` and the streams of kind 4
    /// (section 9 of `docs/protocol.md`). Only a device that advertises [`STREAM_SHARE`]
    /// advertises it.
    pub const STREAM_SOUND: &str = "stream.sound";
    /// Plays the sound of a screen, as the viewer: takes `Sound` and `Silent` from the source and
    /// the streams of kind 4, and sends a `Hear`. Only a device that advertises [`STREAM_VIEW`]
    /// advertises it.
    pub const STREAM_HEAR: &str = "stream.hear";
    /// Keeps its own screen private while a paired device controls it, dark and with its keys and
    /// pointer out of use, until the viewer asks otherwise: sets `private` in an offer and takes
    /// `Lift` on the control stream of that session (section 9 of `docs/protocol.md`). Only a
    /// device that advertises [`STREAM_CONTROL`] advertises it.
    pub const STREAM_PRIVATE: &str = "stream.private";
    /// Can be unlocked by a paired device that its person set up for it, as the target of an
    /// unlock: sets `unlock` in an offer, writes `UnlockSealed` and `UnlockDone` on the control
    /// stream of that session, takes `Unlock`, `UnlockOpened` and `UnlockCancelled`, and sends
    /// [`UnlockRequest::Key`](crate::unlock::UnlockRequest::Key) (section 9 of
    /// `docs/protocol.md`). Only a device that advertises [`STREAM_PRIVATE`] advertises it.
    pub const UNLOCK_TARGET: &str = "unlock.target";
    /// Unlocks a paired device with the face of its person, as the approver of an unlock: answers
    /// [`UnlockRequest::Key`](crate::unlock::UnlockRequest::Key), takes `UnlockSealed` and
    /// `UnlockDone`, and sends `Unlock`, `UnlockOpened` and `UnlockCancelled` (section 9 of
    /// `docs/protocol.md`). Only a device that advertises [`STREAM_VIEW`] advertises it.
    pub const UNLOCK_APPROVE: &str = "unlock.approve";
    /// Gives a paired device what its person is working on, when that device asks and the person
    /// allowed it: the page that is in front in a browser, and the text on the clipboard. Accepts
    /// [`Request::Continue`](super::Request::Continue) (section 3 of `docs/protocol.md`).
    pub const CONTINUE_SERVE: &str = "continue.serve";
}

/// A message with a defined maximum size and field limits.
///
/// [`frame::encode`](crate::frame::encode) and
/// [`frame::decode_payload`](crate::frame::decode_payload) call [`validate`](Self::validate)
/// on both sides, so a message that breaks a limit is neither sent nor accepted.
pub trait Message: Serialize + for<'de> Deserialize<'de> {
    /// Largest frame, header excluded, that may carry this message.
    const MAX_FRAME_LEN: usize;

    /// Checks every field against the protocol's limits.
    fn validate(&self) -> Result<(), LimitError>;
}

/// A request from one paired device to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Introduce yourself and learn what the peer supports.
    Hello(Hello),
    /// Check that the peer is reachable.
    Ping(Ping),
    /// Hand something to the peer.
    Handoff(Handoff),
    /// Offer files and folders to the peer, or follow up on an offer. Only to a peer that
    /// advertised [`capability::TRANSFER_FILES`].
    Transfer(TransferRequest),
    /// Offer a window to the peer, end a session, or ask the peer to start one. An offer goes
    /// only to a peer that advertised [`capability::STREAM_VIEW`], a start only to one that
    /// advertised [`capability::STREAM_START`], and a stop to the other end of the session.
    Stream(StreamRequest),
    /// Ask a paired device for the key that an unlock is sealed to. Only to a peer that
    /// advertised [`capability::UNLOCK_APPROVE`].
    Unlock(UnlockRequest),
    /// Ask a paired device what its person is working on, to continue it here. Only to a peer
    /// that advertised [`capability::CONTINUE_SERVE`].
    Continue(ContinueRequest),
}

/// A response to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// Answers [`Request::Hello`].
    Hello(Hello),
    /// Answers [`Request::Ping`].
    Pong(Ping),
    /// Answers [`Request::Handoff`].
    Handoff(HandoffReply),
    /// Answers [`Request::Transfer`].
    Transfer(TransferReply),
    /// Answers [`Request::Stream`].
    Stream(StreamReply),
    /// Answers [`Request::Unlock`].
    Unlock(UnlockReply),
    /// Answers [`Request::Continue`].
    Continue(ContinueReply),
    /// The request could not be handled at all.
    Error(ErrorReply),
}

/// Sent by both sides when a connection starts, as a request and again as its reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Minor revision of the protocol the sender implements.
    pub protocol_minor: u16,
    /// Who the sender is. A paired device may rename itself at any time.
    pub device: DeviceInfo,
    /// What the sender is willing to receive. See [`capability`].
    pub capabilities: Vec<String>,
}

/// Payload of [`Request::Ping`] and [`Response::Pong`]; the pong echoes the nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    /// Any value; the reply repeats it.
    pub nonce: u64,
}

/// Something one person is handing from one of their devices to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    /// Chosen at random by the sender. A retry reuses it.
    pub id: ItemId,
    /// What is being handed over.
    pub item: Item,
}

/// The kinds of things that can be handed off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Item {
    /// A web link to open.
    Link(Link),
    /// Text for the clipboard.
    ClipboardText(ClipboardText),
}

impl Item {
    /// The [`capability`] a receiver must advertise to be sent this item.
    pub const fn required_capability(&self) -> &'static str {
        match self {
            Self::Link(_) => capability::LINK,
            Self::ClipboardText(_) => capability::CLIPBOARD_TEXT,
        }
    }
}

/// A web link.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The address. The receiver decides which schemes it will open.
    pub url: String,
    /// The page title, if the sender knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

// Links carry secrets all the time: sign-in and password-reset tokens in the query,
// credentials before the host, document ids in the path, and titles that name the
// page. A request is logged with `{:?}` when something goes wrong, so only what is
// needed to tell links apart in a log is printed: the scheme, the host, and sizes.
impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("origin", &format_args!("{}", origin_of(&self.url)))
            .field("url_bytes", &self.url.len())
            .field("title_bytes", &self.title.as_ref().map(String::len))
            .finish()
    }
}

/// The longest host name, in bytes, that [`origin_of`] prints.
const MAX_LOGGED_HOST_BYTES: usize = 253;

/// The longest scheme, in bytes, that [`origin_of`] prints.
const MAX_LOGGED_SCHEME_BYTES: usize = 32;

/// What of a link is safe to put in a log: the scheme and the host, never anything else.
///
/// Returns `https://example.com` for `https://user:pw@example.com:8443/path?q#f`, and
/// `mailto:` for `mailto:someone@example.com`. This is a log aid, not a URL parser, so it
/// prefers printing too little to printing something odd: a scheme that is not plain ASCII
/// letters, digits, `+`, `-` and `.`, or a host that is not plain ASCII (which also
/// rules out control characters and text-direction tricks), is replaced by `?`.
fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once(':') else {
        return "?".to_owned();
    };
    let scheme_ok = scheme.len() <= MAX_LOGGED_SCHEME_BYTES
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok {
        return "?".to_owned();
    }
    let scheme = scheme.to_ascii_lowercase();
    let Some(after_slashes) = rest.strip_prefix("//") else {
        return format!("{scheme}:");
    };

    // The authority ends at the first `/`, `?` or `#`. Everything before the last `@`
    // in it is credentials, which are never printed.
    let authority = after_slashes
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host_and_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host_and_port.starts_with('[') {
        // An IPv6 address in brackets, possibly followed by a port.
        host_and_port
            .find(']')
            .map(|end| host_and_port.get(..=end).unwrap_or_default())
            .unwrap_or_default()
    } else {
        host_and_port.split(':').next().unwrap_or_default()
    };
    let host_ok = !host.is_empty()
        && host.len() <= MAX_LOGGED_HOST_BYTES
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
    if host_ok {
        format!("{scheme}://{}", host.to_ascii_lowercase())
    } else {
        format!("{scheme}://?")
    }
}

/// Plain text from the clipboard.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardText {
    /// The text.
    pub text: String,
}

// The text may be a password, so it stays out of logs.
impl std::fmt::Debug for ClipboardText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClipboardText({} bytes)", self.text.len())
    }
}

/// The receiver's answer to a [`Handoff`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffReply {
    /// The id of the item this answers.
    pub id: ItemId,
    /// What the receiver did with it.
    pub outcome: Outcome,
}

/// What the receiver did with a handed-off item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// The receiver took the item. What happens to it next is the receiver's
    /// policy, and the sender is not told.
    Accepted,
    /// The receiver took no action, for the given reason.
    Declined(DeclineReason),
}

/// Why a receiver declined an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeclineReason {
    /// The receiver does not handle this kind of item.
    Unsupported,
    /// The item is larger than the receiver accepts.
    TooLarge,
    /// The sender is sending too fast.
    RateLimited,
    /// The receiver's settings for this device do not allow it.
    Blocked,
    /// The content is not valid for its kind, for example a malformed link.
    Invalid,
    /// The receiver has no room for it: its storage is full, or what it would hold from this
    /// device would pass the limit the person set.
    NoSpace,
    /// The person using the receiver said no, or did not answer in time.
    Refused,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
}

/// What a device asks of a paired device to continue its person's work on itself (`docs/protocol.md`,
/// section 3, Continue).
///
/// A request is not extensible: a device asks only what the capabilities say is understood, so a
/// kind that this version does not know is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContinueRequest {
    /// Ask what the person there is working on: the page that is in front in the browser, and the
    /// text on the clipboard.
    Ask(ContinueAsk),
}

/// The question of [`ContinueRequest::Ask`].
///
/// It has no fields yet. It is a struct, and not a unit, so that a later version can add some
/// that an older device ignores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContinueAsk {}

/// A device's answer to a [`ContinueAsk`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContinueReply {
    /// What its person has ready. It may be nothing: the device is locked, or there is no page
    /// and no text.
    Items(ContinueItems),
    /// The device gave nothing, and says why: [`DeclineReason::Blocked`] where its person has not
    /// allowed the asking device, [`DeclineReason::Unsupported`] where it cannot read what its
    /// person works on, [`DeclineReason::RateLimited`] where the asking device asks too often or
    /// a question is being read already, and [`DeclineReason::Other`] where it could not read it
    /// just now.
    Declined(DeclineReason),
}

/// What the person at a device has ready to continue somewhere else.
///
/// A field that is missing is something that there is none of, and says nothing of why.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContinueItems {
    /// The page that is in front in the browser.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Link>,
    /// The text on the clipboard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<ClipboardText>,
}

/// A request that could not be handled at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReply {
    /// A stable, machine-readable code.
    pub code: ErrorCode,
    /// A short explanation for logs. Never shown as-is to a person.
    pub message: String,
}

/// Machine-readable reasons for [`ErrorReply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The request could not be decoded or broke a limit.
    Malformed,
    /// The receiver is shutting down or failed internally.
    Internal,
    /// The request names something the receiver does not have, such as a transfer it never
    /// saw or has forgotten.
    NotFound,
    /// Any code this version does not know about.
    #[serde(other)]
    Other,
}

fn validate_capabilities(capabilities: &[String]) -> Result<(), LimitError> {
    if capabilities.len() > MAX_CAPABILITIES {
        return Err(LimitError::TooMany {
            field: "capabilities",
            count: capabilities.len(),
            max: MAX_CAPABILITIES,
        });
    }
    capabilities
        .iter()
        .try_for_each(|c| check_non_empty_len("capability", c, MAX_CAPABILITY_BYTES))
}

impl Hello {
    fn validate(&self) -> Result<(), LimitError> {
        self.device.validate()?;
        validate_capabilities(&self.capabilities)
    }

    /// Whether the sender advertised `capability`.
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

impl Link {
    /// Checks the address and the title against the protocol's limits: what a link may be, whether
    /// it is handed over or given back.
    ///
    /// # Errors
    ///
    /// The address is empty or too long, or the title is too long.
    pub fn validate(&self) -> Result<(), LimitError> {
        check_non_empty_len("link url", &self.url, MAX_URL_BYTES)?;
        self.title
            .as_deref()
            .map_or(Ok(()), |t| check_len("link title", t, MAX_TITLE_BYTES))
    }
}

impl ClipboardText {
    /// Checks the text against the protocol's limit: what clipboard text may be, whether it is
    /// handed over or given back.
    ///
    /// # Errors
    ///
    /// The text is empty or too long.
    pub fn validate(&self) -> Result<(), LimitError> {
        check_non_empty_len("clipboard text", &self.text, MAX_CLIPBOARD_TEXT_BYTES)
    }
}

impl Item {
    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Link(link) => link.validate(),
            Self::ClipboardText(clip) => clip.validate(),
        }
    }
}

impl ContinueReply {
    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Items(items) => {
                items.page.as_ref().map_or(Ok(()), Link::validate)?;
                items
                    .clipboard
                    .as_ref()
                    .map_or(Ok(()), ClipboardText::validate)
            }
            Self::Declined(_) => Ok(()),
        }
    }
}

impl Message for Request {
    const MAX_FRAME_LEN: usize = MAX_FRAME_LEN;

    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Hello(hello) => hello.validate(),
            Self::Ping(_) | Self::Unlock(_) | Self::Continue(_) => Ok(()),
            Self::Handoff(handoff) => handoff.item.validate(),
            Self::Transfer(request) => request.validate(),
            Self::Stream(request) => request.validate(),
        }
    }
}

impl Message for Response {
    const MAX_FRAME_LEN: usize = MAX_FRAME_LEN;

    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Hello(hello) => hello.validate(),
            Self::Pong(_) | Self::Handoff(_) => Ok(()),
            Self::Transfer(reply) => reply.validate(),
            Self::Stream(reply) => reply.validate(),
            Self::Unlock(reply) => reply.validate(),
            Self::Continue(reply) => reply.validate(),
            Self::Error(error) => {
                check_len("error message", &error.message, MAX_ERROR_MESSAGE_BYTES)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Platform;

    fn device() -> DeviceInfo {
        DeviceInfo {
            name: "Test".into(),
            platform: Platform::Linux,
            app_version: "0.1.0".into(),
        }
    }

    fn link(url: &str) -> Request {
        Request::Handoff(Handoff {
            id: ItemId::from_bytes([7; 16]),
            item: Item::Link(Link {
                url: url.into(),
                title: None,
            }),
        })
    }

    #[test]
    fn limits_are_enforced_on_every_field() {
        assert!(link("https://example.com").validate().is_ok());
        assert!(link("").validate().is_err());
        assert!(link(&"x".repeat(MAX_URL_BYTES + 1)).validate().is_err());

        let long_title = Request::Handoff(Handoff {
            id: ItemId::from_bytes([0; 16]),
            item: Item::Link(Link {
                url: "https://example.com".into(),
                title: Some("t".repeat(MAX_TITLE_BYTES + 1)),
            }),
        });
        assert!(long_title.validate().is_err());

        let clip = |text: String| {
            Request::Handoff(Handoff {
                id: ItemId::from_bytes([0; 16]),
                item: Item::ClipboardText(ClipboardText { text }),
            })
        };
        assert!(
            clip("x".repeat(MAX_CLIPBOARD_TEXT_BYTES))
                .validate()
                .is_ok()
        );
        assert!(
            clip("x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1))
                .validate()
                .is_err()
        );
        assert!(clip(String::new()).validate().is_err());
    }

    #[test]
    fn hello_capabilities_are_bounded() {
        let hello = |capabilities: Vec<String>| Hello {
            protocol_minor: 0,
            device: device(),
            capabilities,
        };
        assert!(
            Request::Hello(hello(vec![capability::LINK.into()]))
                .validate()
                .is_ok()
        );
        assert!(
            Request::Hello(hello(vec![String::new()]))
                .validate()
                .is_err()
        );
        assert!(
            Request::Hello(hello(vec!["c".repeat(MAX_CAPABILITY_BYTES + 1)]))
                .validate()
                .is_err()
        );
        let too_many = (0..=MAX_CAPABILITIES).map(|i| format!("c{i}")).collect();
        assert!(Request::Hello(hello(too_many)).validate().is_err());
    }

    #[test]
    fn supports_checks_the_advertised_list() {
        let hello = Hello {
            protocol_minor: 0,
            device: device(),
            capabilities: vec![capability::LINK.into()],
        };
        assert!(hello.supports(capability::LINK));
        assert!(!hello.supports(capability::CLIPBOARD_TEXT));
    }

    #[test]
    fn items_name_the_capability_they_need() {
        let link = Item::Link(Link {
            url: "https://example.com".into(),
            title: None,
        });
        let clip = Item::ClipboardText(ClipboardText { text: "x".into() });
        assert_eq!(link.required_capability(), capability::LINK);
        assert_eq!(clip.required_capability(), capability::CLIPBOARD_TEXT);
    }

    #[test]
    fn clipboard_text_is_not_printed() {
        let clip = ClipboardText {
            text: "hunter2".into(),
        };
        assert!(!format!("{clip:?}").contains("hunter2"));
    }

    #[test]
    fn a_link_prints_only_its_scheme_host_and_sizes() {
        let link = Link {
            url: "https://user:hunter2@Example.com:8443/reset/abc123?token=s3cr3t#frag".into(),
            title: Some("Reset the password for Jay".into()),
        };
        let shown = format!("{link:?}");
        assert!(shown.contains("https://example.com"), "{shown}");
        for secret in [
            "user", "hunter2", "8443", "reset", "abc123", "token", "s3cr3t", "frag", "password",
            "Jay",
        ] {
            assert!(!shown.contains(secret), "{secret} leaked in {shown}");
        }

        // The same holds when the link sits inside a request that is logged whole.
        let request = Request::Handoff(Handoff {
            id: ItemId::from_bytes([7; 16]),
            item: Item::Link(link),
        });
        let shown = format!("{request:?}");
        assert!(shown.contains("https://example.com"), "{shown}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("s3cr3t"),
            "{shown}"
        );
    }

    fn ready(page: Option<Link>, clipboard: Option<ClipboardText>) -> Response {
        Response::Continue(ContinueReply::Items(ContinueItems { page, clipboard }))
    }

    fn page(url: &str, title: Option<&str>) -> Link {
        Link {
            url: url.into(),
            title: title.map(Into::into),
        }
    }

    fn text_of(text: &str) -> ClipboardText {
        ClipboardText { text: text.into() }
    }

    /// Checks that a message is written as exactly the CBOR that `expected` says, built by hand,
    /// key by key, and that the frame it makes is `frame` in hexadecimal.
    fn assert_wire<M: Message>(message: &M, expected: &ciborium::Value, frame: &str) {
        let mut cbor = Vec::new();
        ciborium::into_writer(expected, &mut cbor).unwrap();
        let written = crate::frame::encode(message).unwrap();
        assert_eq!(written[crate::frame::HEADER_LEN..], cbor);
        assert_eq!(data_encoding::HEXLOWER.encode(&written), frame);
    }

    #[test]
    fn the_messages_of_continuing_have_known_wire_encodings() {
        use ciborium::cbor;
        // The exact CBOR is the protocol. It is built here by hand so that a change in the order
        // of the fields, or in how a part that is not there is left out, shows up.
        assert_wire(
            &Request::Continue(ContinueRequest::Ask(ContinueAsk {})),
            &cbor!({ "Continue" => { "Ask" => {} } }).unwrap(),
            "00000010a168436f6e74696e7565a16341736ba0",
        );
        assert_wire(
            &ready(
                Some(page("https://example.com/report", Some("The report"))),
                Some(text_of("hello")),
            ),
            &cbor!({
                "Continue" => {
                    "Items" => {
                        "page" => { "url" => "https://example.com/report", "title" => "The report" },
                        "clipboard" => { "text" => "hello" },
                    },
                },
            })
            .unwrap(),
            "0000005fa168436f6e74696e7565a1654974656d73a26470616765a26375726c781a68747470733a2f2f6578616d706c652e636f6d2f7265706f7274657469746c656a546865207265706f727469636c6970626f617264a164746578746568656c6c6f",
        );
        // Nothing at all is an empty map: a part that is not there is not written.
        assert_wire(
            &ready(None, None),
            &cbor!({ "Continue" => { "Items" => {} } }).unwrap(),
            "00000012a168436f6e74696e7565a1654974656d73a0",
        );
        assert_wire(
            &Response::Continue(ContinueReply::Declined(DeclineReason::Blocked)),
            &cbor!({ "Continue" => { "Declined" => "Blocked" } }).unwrap(),
            "0000001ca168436f6e74696e7565a1684465636c696e656467426c6f636b6564",
        );
    }

    #[test]
    fn a_device_that_gives_what_its_person_works_on_names_the_capability_for_it() {
        assert_eq!(capability::CONTINUE_SERVE, "continue.serve");
    }

    #[test]
    fn what_a_device_has_ready_may_be_nothing_a_page_text_or_both() {
        assert!(ready(None, None).validate().is_ok());
        assert!(
            ready(Some(page("https://example.com", None)), None)
                .validate()
                .is_ok()
        );
        assert!(ready(None, Some(text_of("hello"))).validate().is_ok());
        assert!(
            ready(
                Some(page("https://example.com", Some("Example"))),
                Some(text_of("hello"))
            )
            .validate()
            .is_ok()
        );
        // A refusal carries nothing to check, and a request nothing at all.
        for reason in [
            DeclineReason::Blocked,
            DeclineReason::Unsupported,
            DeclineReason::RateLimited,
            DeclineReason::Other,
        ] {
            assert!(
                Response::Continue(ContinueReply::Declined(reason))
                    .validate()
                    .is_ok()
            );
        }
        assert!(
            Request::Continue(ContinueRequest::Ask(ContinueAsk {}))
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn what_a_device_has_ready_follows_the_limits_of_a_link_and_of_text() {
        // The page is a link and the text is clipboard text, so they are held to the same limits as
        // when they are handed over.
        assert!(ready(Some(page("", None)), None).validate().is_err());
        assert!(
            ready(Some(page(&"x".repeat(MAX_URL_BYTES), None)), None)
                .validate()
                .is_ok()
        );
        assert!(
            ready(Some(page(&"x".repeat(MAX_URL_BYTES + 1), None)), None)
                .validate()
                .is_err()
        );
        let title = |n: usize| {
            ready(
                Some(page("https://example.com", Some(&"t".repeat(n)))),
                None,
            )
        };
        assert!(title(MAX_TITLE_BYTES).validate().is_ok());
        assert!(title(MAX_TITLE_BYTES + 1).validate().is_err());
        assert!(ready(None, Some(text_of(""))).validate().is_err());
        assert!(
            ready(None, Some(text_of(&"x".repeat(MAX_CLIPBOARD_TEXT_BYTES))))
                .validate()
                .is_ok()
        );
        assert!(
            ready(
                None,
                Some(text_of(&"x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1)))
            )
            .validate()
            .is_err()
        );
        // A good page does not make up for text that is too long.
        assert!(
            ready(
                Some(page("https://example.com", None)),
                Some(text_of(&"x".repeat(MAX_CLIPBOARD_TEXT_BYTES + 1)))
            )
            .validate()
            .is_err()
        );
    }

    #[test]
    fn what_a_device_has_ready_stays_out_of_a_log() {
        let shown = format!(
            "{:?}",
            ready(
                Some(page(
                    "https://user:hunter2@Example.com/reset/abc123?token=s3cr3t",
                    Some("Reset the password for Jay"),
                )),
                Some(text_of("correct horse battery staple")),
            )
        );
        assert!(shown.contains("https://example.com"), "{shown}");
        for secret in [
            "user", "hunter2", "reset", "abc123", "s3cr3t", "Jay", "horse",
        ] {
            assert!(!shown.contains(secret), "{secret} leaked in {shown}");
        }
    }

    #[test]
    fn the_origin_of_a_link_is_only_ever_a_plain_scheme_and_host() {
        for (url, expected) in [
            ("https://example.com", "https://example.com"),
            ("HTTP://EXAMPLE.com/a", "http://example.com"),
            ("https://example.com:8443", "https://example.com"),
            ("https://[2001:db8::1]:8443/x", "https://[2001:db8::1]"),
            // The last `@` ends the credentials, however many come before it.
            ("https://u:p@h1:p2@example.com/", "https://example.com"),
            ("https://example.com?token=x", "https://example.com"),
            ("https://example.com#frag", "https://example.com"),
            ("mailto:someone@example.com", "mailto:"),
            ("javascript:alert(document.cookie)", "javascript:"),
            ("data:text/plain;base64,c2VjcmV0", "data:"),
            ("https:\\\\example.com", "https:"),
            // An empty host.
            ("file:///home/jay/secret.txt", "file://?"),
            // Hosts that are not plain ASCII are not printed: control characters could
            // forge log lines, and look-alike letters or direction overrides could
            // make a host read as another one.
            ("https://exa\nmple.com", "https://?"),
            ("https://ex\u{430}mple.com", "https://?"),
            ("https://\u{202e}moc.elpmaxe", "https://?"),
            ("https://[::1", "https://?"),
            ("https://exa mple.com", "https://?"),
            // No usable scheme.
            ("", "?"),
            ("no scheme here", "?"),
            ("1http://x", "?"),
            ("ht tp://x", "?"),
            ("\nhttps://x", "?"),
            (":", "?"),
        ] {
            assert_eq!(origin_of(url), expected, "{url:?}");
        }
        assert_eq!(origin_of(&format!("{}://x", "a".repeat(33))), "?");
        assert_eq!(
            origin_of(&format!("https://{}", "a".repeat(254))),
            "https://?"
        );
        assert_eq!(
            origin_of(&format!("https://{}", "a".repeat(253))),
            format!("https://{}", "a".repeat(253))
        );
    }
}
