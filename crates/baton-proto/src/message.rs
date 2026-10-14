// SPDX-License-Identifier: Apache-2.0

//! Messages exchanged between paired devices on the session protocol.
//!
//! Each request travels on its own bidirectional stream and gets exactly one
//! response, much like a call over HTTP.

use serde::{Deserialize, Serialize};

use crate::{
    device::DeviceInfo,
    id::ItemId,
    limits::{
        LimitError, MAX_CAPABILITIES, MAX_CAPABILITY_BYTES, MAX_CLIPBOARD_TEXT_BYTES,
        MAX_ERROR_MESSAGE_BYTES, MAX_FRAME_LEN, MAX_TITLE_BYTES, MAX_URL_BYTES, check_len,
        check_non_empty_len,
    },
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The address. The receiver decides which schemes it will open.
    pub url: String,
    /// The page title, if the sender knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
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

impl Item {
    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Link(link) => {
                check_non_empty_len("link url", &link.url, MAX_URL_BYTES)?;
                link.title
                    .as_deref()
                    .map_or(Ok(()), |t| check_len("link title", t, MAX_TITLE_BYTES))
            }
            Self::ClipboardText(clip) => {
                check_non_empty_len("clipboard text", &clip.text, MAX_CLIPBOARD_TEXT_BYTES)
            }
        }
    }
}

impl Message for Request {
    const MAX_FRAME_LEN: usize = MAX_FRAME_LEN;

    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Hello(hello) => hello.validate(),
            Self::Ping(_) => Ok(()),
            Self::Handoff(handoff) => handoff.item.validate(),
        }
    }
}

impl Message for Response {
    const MAX_FRAME_LEN: usize = MAX_FRAME_LEN;

    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Hello(hello) => hello.validate(),
            Self::Pong(_) | Self::Handoff(_) => Ok(()),
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
}
