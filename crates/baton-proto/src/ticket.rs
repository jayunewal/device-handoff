// SPDX-License-Identifier: Apache-2.0

//! The pairing ticket: everything a joiner needs to find and prove itself to an offerer.
//!
//! A ticket is shown as a QR code or copied as text. Its text form is
//! `baton1:` followed by the base32 encoding (RFC 4648, no padding, any case)
//! of this binary layout, with multi-byte integers in big-endian order:
//!
//! ```text
//! version     u8        1
//! flags       u8        0, reserved
//! offerer     32 bytes  the offering device's identity
//! secret      32 bytes  one-time secret
//! expires_at  u64       seconds since the Unix epoch
//! relay_len   u8        0 when there is no relay hint
//! relay       bytes     ASCII URL of the offerer's home relay
//! addr_count  u8        at most 16
//! addr        repeated: tag u8 (4 or 6), IP address (4 or 16 bytes), port u16
//! checksum    4 bytes   first bytes of a BLAKE3 digest of everything above
//! ```
//!
//! Addresses carry an IP and a port only. IPv6 scope ids and flow labels are not
//! carried, so a creator should leave out addresses that cannot be used without
//! one (link-local IPv6).
//!
//! The checksum is not a security feature. It turns a mistyped or truncated
//! ticket into a clear error instead of a failed connection.

use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    str::FromStr,
};

use data_encoding::BASE32_NOPAD;

use crate::{id::DeviceId, pair::PairSecret};

/// Text form prefix.
pub const PREFIX: &str = "baton1:";

/// Most direct addresses a ticket can carry.
pub const MAX_ADDRS: usize = 16;

/// Longest relay hint, in bytes.
pub const MAX_RELAY_BYTES: usize = 255;

/// Longest text form accepted, to bound work on hostile input.
const MAX_TEXT_LEN: usize = 2048;

const FORMAT_VERSION: u8 = 1;
const CHECKSUM_LEN: usize = 4;
const CHECKSUM_CONTEXT: &str = "baton 2026-10-14 pairing ticket checksum v1";

/// An invitation to pair with one specific device.
#[derive(Debug, Clone)]
pub struct PairTicket {
    /// The device that made the offer.
    pub offerer: DeviceId,
    /// Proves the joiner was handed this ticket.
    pub secret: PairSecret,
    /// When the offer lapses, in seconds since the Unix epoch.
    pub expires_at: u64,
    /// URL of the offerer's home relay, so the joiner can reach it across networks.
    pub relay: Option<String>,
    /// Addresses the offerer can be reached at directly.
    pub addrs: Vec<SocketAddr>,
}

/// Why a ticket could not be produced or read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TicketError {
    /// The text does not start with `baton1:`.
    #[error("not a Baton pairing ticket (it should start with \"{PREFIX}\")")]
    BadPrefix,
    /// The text is longer than any ticket can be.
    #[error("the ticket is too long")]
    TooLong,
    /// The text is not valid base32.
    #[error("the ticket contains characters that do not belong in one")]
    BadEncoding,
    /// The checksum does not match: the ticket was mistyped or cut off.
    #[error("the ticket is damaged (checksum mismatch); copy it again")]
    BadChecksum,
    /// The ticket ends before it should.
    #[error("the ticket is cut short")]
    Truncated,
    /// The ticket was made by a newer, incompatible version.
    #[error("unsupported ticket version {0}")]
    UnsupportedVersion(u8),
    /// A field has a value this version does not accept.
    #[error("invalid ticket: {0}")]
    Invalid(&'static str),
}

impl PairTicket {
    /// Whether the offer has lapsed at `now`, in seconds since the Unix epoch.
    pub const fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    /// Encodes the ticket in its binary layout.
    pub fn to_bytes(&self) -> Result<Vec<u8>, TicketError> {
        let relay = self.relay.as_deref().unwrap_or("");
        if relay.len() > MAX_RELAY_BYTES {
            return Err(TicketError::Invalid("relay url is too long"));
        }
        if !relay.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(TicketError::Invalid("relay url must be printable ASCII"));
        }
        if self.addrs.len() > MAX_ADDRS {
            return Err(TicketError::Invalid("too many addresses"));
        }

        let mut out = Vec::with_capacity(96 + relay.len() + self.addrs.len() * 19);
        out.push(FORMAT_VERSION);
        out.push(0);
        out.extend_from_slice(self.offerer.as_bytes());
        out.extend_from_slice(self.secret.expose());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        // Both lengths were bounded above, so the narrowing casts cannot truncate.
        out.push(u8::try_from(relay.len()).map_err(|_| TicketError::Invalid("relay url"))?);
        out.extend_from_slice(relay.as_bytes());
        out.push(u8::try_from(self.addrs.len()).map_err(|_| TicketError::Invalid("addresses"))?);
        for addr in &self.addrs {
            match addr.ip() {
                IpAddr::V4(ip) => {
                    out.push(4);
                    out.extend_from_slice(&ip.octets());
                }
                IpAddr::V6(ip) => {
                    out.push(6);
                    out.extend_from_slice(&ip.octets());
                }
            }
            out.extend_from_slice(&addr.port().to_be_bytes());
        }
        let checksum = checksum(&out);
        out.extend_from_slice(&checksum);
        Ok(out)
    }

    /// Decodes a ticket from its binary layout.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, TicketError> {
        let body_len = bytes
            .len()
            .checked_sub(CHECKSUM_LEN)
            .ok_or(TicketError::Truncated)?;
        let (body, given) = bytes.split_at(body_len);
        if checksum(body) != given {
            return Err(TicketError::BadChecksum);
        }

        let mut reader = Reader(body);
        let version = reader.u8()?;
        if version != FORMAT_VERSION {
            return Err(TicketError::UnsupportedVersion(version));
        }
        if reader.u8()? != 0 {
            return Err(TicketError::Invalid("reserved flags are set"));
        }
        let offerer = DeviceId::from_bytes(reader.array()?);
        let secret = PairSecret::from_bytes(reader.array()?);
        let expires_at = u64::from_be_bytes(reader.array()?);

        let relay_len = usize::from(reader.u8()?);
        let relay = if relay_len == 0 {
            None
        } else {
            let raw = reader.take(relay_len)?;
            if !raw.iter().all(u8::is_ascii_graphic) {
                return Err(TicketError::Invalid("relay url must be printable ASCII"));
            }
            // Printable ASCII is always valid UTF-8.
            Some(String::from_utf8_lossy(raw).into_owned())
        };

        let addr_count = usize::from(reader.u8()?);
        if addr_count > MAX_ADDRS {
            return Err(TicketError::Invalid("too many addresses"));
        }
        let mut addrs = Vec::with_capacity(addr_count);
        for _ in 0..addr_count {
            let ip = match reader.u8()? {
                4 => IpAddr::V4(Ipv4Addr::from(reader.array::<4>()?)),
                6 => IpAddr::V6(Ipv6Addr::from(reader.array::<16>()?)),
                _ => return Err(TicketError::Invalid("unknown address type")),
            };
            let port = u16::from_be_bytes(reader.array()?);
            addrs.push(SocketAddr::new(ip, port));
        }
        if !reader.0.is_empty() {
            return Err(TicketError::Invalid(
                "unexpected data after the last address",
            ));
        }

        Ok(Self {
            offerer,
            secret,
            expires_at,
            relay,
            addrs,
        })
    }
}

impl fmt::Display for PairTicket {
    /// The text form. A ticket that cannot be encoded prints as `<invalid ticket>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_bytes() {
            Ok(bytes) => write!(
                f,
                "{PREFIX}{}",
                BASE32_NOPAD.encode(&bytes).to_ascii_lowercase()
            ),
            Err(_) => f.write_str("<invalid ticket>"),
        }
    }
}

impl FromStr for PairTicket {
    type Err = TicketError;

    /// Parses the text form. Case, surrounding whitespace and line breaks
    /// introduced by copying are ignored.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > MAX_TEXT_LEN {
            return Err(TicketError::TooLong);
        }
        let compact: String = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let encoded = compact.strip_prefix(PREFIX).ok_or(TicketError::BadPrefix)?;
        let bytes = BASE32_NOPAD
            .decode(encoded.to_ascii_uppercase().as_bytes())
            .map_err(|_| TicketError::BadEncoding)?;
        Self::from_bytes(&bytes)
    }
}

fn checksum(body: &[u8]) -> [u8; CHECKSUM_LEN] {
    let digest = blake3::derive_key(CHECKSUM_CONTEXT, body);
    let mut out = [0; CHECKSUM_LEN];
    out.copy_from_slice(&digest[..CHECKSUM_LEN]);
    out
}

/// A cursor over a byte slice that never panics.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], TicketError> {
        let (head, tail) = self.0.split_at_checked(n).ok_or(TicketError::Truncated)?;
        self.0 = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], TicketError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, TicketError> {
        Ok(self.array::<1>()?[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> PairTicket {
        PairTicket {
            offerer: DeviceId::from_bytes([0x11; 32]),
            secret: PairSecret::from_bytes([0x22; 32]),
            expires_at: 1_791_965_000,
            relay: Some("https://relay.example/".into()),
            addrs: vec![
                "192.168.1.20:4433".parse().unwrap(),
                "[2001:db8::1]:4433".parse().unwrap(),
            ],
        }
    }

    #[test]
    fn text_form_round_trips() {
        let text = ticket().to_string();
        assert!(text.starts_with(PREFIX));
        let parsed: PairTicket = text.parse().unwrap();
        assert_eq!(parsed.offerer, ticket().offerer);
        assert_eq!(parsed.secret.expose(), ticket().secret.expose());
        assert_eq!(parsed.expires_at, ticket().expires_at);
        assert_eq!(parsed.relay, ticket().relay);
        assert_eq!(parsed.addrs, ticket().addrs);
    }

    #[test]
    fn minimal_ticket_round_trips() {
        let minimal = PairTicket {
            relay: None,
            addrs: vec![],
            ..ticket()
        };
        let parsed: PairTicket = minimal.to_string().parse().unwrap();
        assert_eq!(parsed.relay, None);
        assert!(parsed.addrs.is_empty());
    }

    #[test]
    fn binary_layout_is_stable() {
        // Layout and checksum were reproduced independently with the reference BLAKE3 implementation.
        let bytes = PairTicket {
            relay: Some("https://r/".into()),
            addrs: vec!["10.0.0.1:80".parse().unwrap()],
            ..ticket()
        }
        .to_bytes()
        .unwrap();
        assert_eq!(
            data_encoding::HEXLOWER.encode(&bytes),
            "010011111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222000000006acf37480a68747470733a2f2f722f01040a00000100509e781aeb"
        );
    }

    #[test]
    fn parsing_ignores_case_and_copy_artifacts() {
        let text = ticket().to_string();
        let shouty = text.to_uppercase();
        assert!(shouty.parse::<PairTicket>().is_ok());
        let wrapped = format!("  {}\n{}\r\n", &text[..20], &text[20..]);
        assert!(wrapped.parse::<PairTicket>().is_ok());
    }

    #[test]
    fn expiry_is_exclusive_of_the_deadline() {
        let t = ticket();
        assert!(!t.is_expired(t.expires_at - 1));
        assert!(t.is_expired(t.expires_at));
        assert!(t.is_expired(u64::MAX));
    }

    #[test]
    fn wrong_prefix_and_junk_are_rejected() {
        assert_eq!(
            "".parse::<PairTicket>().unwrap_err(),
            TicketError::BadPrefix
        );
        assert_eq!(
            "hello".parse::<PairTicket>().unwrap_err(),
            TicketError::BadPrefix
        );
        assert_eq!(
            "baton2:aaaa".parse::<PairTicket>().unwrap_err(),
            TicketError::BadPrefix
        );
        assert_eq!(
            "baton1:!!!!".parse::<PairTicket>().unwrap_err(),
            TicketError::BadEncoding
        );
        assert_eq!(
            "baton1:".parse::<PairTicket>().unwrap_err(),
            TicketError::Truncated
        );
        let long = format!("baton1:{}", "a".repeat(MAX_TEXT_LEN));
        assert_eq!(
            long.parse::<PairTicket>().unwrap_err(),
            TicketError::TooLong
        );
    }

    #[test]
    fn every_single_character_change_is_caught() {
        let text = ticket().to_string();
        let body = &text[PREFIX.len()..];
        for i in 0..body.len() {
            let mut chars: Vec<char> = body.chars().collect();
            chars[i] = if chars[i] == 'a' { 'b' } else { 'a' };
            let damaged = format!("{PREFIX}{}", chars.into_iter().collect::<String>());
            assert!(
                damaged.parse::<PairTicket>().is_err(),
                "change at {i} was accepted"
            );
        }
    }

    #[test]
    fn truncation_is_caught_at_every_length() {
        let bytes = ticket().to_bytes().unwrap();
        for len in 0..bytes.len() {
            assert!(
                PairTicket::from_bytes(&bytes[..len]).is_err(),
                "{len} bytes were accepted"
            );
        }
    }

    #[test]
    fn structural_errors_are_reported_even_with_a_valid_checksum() {
        // Re-seal a modified body with a correct checksum to exercise the parser itself.
        fn reseal(mut body: Vec<u8>) -> Vec<u8> {
            let sum = checksum(&body);
            body.extend_from_slice(&sum);
            body
        }
        let mut body = ticket().to_bytes().unwrap();
        body.truncate(body.len() - CHECKSUM_LEN);

        let mut future_version = body.clone();
        future_version[0] = 2;
        assert_eq!(
            PairTicket::from_bytes(&reseal(future_version)).unwrap_err(),
            TicketError::UnsupportedVersion(2)
        );

        let mut flags = body.clone();
        flags[1] = 1;
        assert!(matches!(
            PairTicket::from_bytes(&reseal(flags)).unwrap_err(),
            TicketError::Invalid(_)
        ));

        let mut trailing = body.clone();
        trailing.push(0);
        assert!(matches!(
            PairTicket::from_bytes(&reseal(trailing)).unwrap_err(),
            TicketError::Invalid(_)
        ));

        let mut bad_tag = PairTicket {
            relay: None,
            addrs: vec!["1.2.3.4:5".parse().unwrap()],
            ..ticket()
        }
        .to_bytes()
        .unwrap();
        bad_tag.truncate(bad_tag.len() - CHECKSUM_LEN);
        let tag_at = bad_tag.len() - 7;
        bad_tag[tag_at] = 5;
        assert!(matches!(
            PairTicket::from_bytes(&reseal(bad_tag)).unwrap_err(),
            TicketError::Invalid(_)
        ));
    }

    #[test]
    fn oversized_fields_are_not_encoded() {
        let long_relay = PairTicket {
            relay: Some("h".repeat(MAX_RELAY_BYTES + 1)),
            ..ticket()
        };
        assert!(long_relay.to_bytes().is_err());
        let spaced_relay = PairTicket {
            relay: Some("has space".into()),
            ..ticket()
        };
        assert!(spaced_relay.to_bytes().is_err());
        let many = PairTicket {
            addrs: vec!["1.2.3.4:5".parse().unwrap(); MAX_ADDRS + 1],
            ..ticket()
        };
        assert!(many.to_bytes().is_err());
    }
}
