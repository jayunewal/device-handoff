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
