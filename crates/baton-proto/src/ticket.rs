// SPDX-License-Identifier: Apache-2.0

//! The pairing ticket: everything a joiner needs to find and prove itself to an offerer.
//!
//! A ticket is shown as a QR code or copied as text. Its text form is `BATON1:`
//! followed by the base32 encoding (RFC 4648, no padding) of this binary layout, with
//! multi-byte integers in big-endian order. Capital letters are what is written,
//! because a QR code made of capitals and digits fits a denser mode than one with
//! lower case, which makes the code smaller and easier to scan off a screen. Parsers
//! accept any case.
//!
//! ```text
//! version     u8        1
//! flags       u8        low four bits ignorable, high four bits critical; 0 for now
//! offerer     32 bytes  the offering device's identity
//! secret      32 bytes  one-time secret
//! expires_at  u64       seconds since the Unix epoch
//! relay_len   u8        0 when there is no relay hint
//! relay       bytes     ASCII URL of the offerer's home relay
//! addr_count  u8        at most 16
//! addr        repeated: tag u8 (4 or 6), IP address (4 or 16 bytes), port u16
//! extension   repeated until the checksum: type u8, len u8, value (len bytes)
//! checksum    4 bytes   first bytes of a BLAKE3 digest of everything above
//! ```
//!
//! Addresses carry an IP and a port only. IPv6 scope ids and flow labels are not
//! carried, so a creator should leave out addresses that cannot be used without
//! one (link-local IPv6).
//!
//! # Room to grow
//!
//! Tickets are printed, photographed and shown on screens, so a phone running last
//! month's version will meet tickets made by this month's. Two mechanisms let a
//! ticket gain fields without breaking it for older readers:
//!
//! - The extensions after the last address are skipped by a reader that does not know
//!   their type, unless the top bit of the type (0x80) is set. That bit marks a
//!   field the reader must understand, and a reader that does not rejects the
//!   ticket with [`TicketError::NeedsNewerVersion`].
//! - The same split applies to `flags`: the low four bits may be ignored, and a reader
//!   rejects a ticket that sets any of the high four.
//!
//! No extension and no flag is defined yet. A change that older readers cannot do
//! without a loss, for example to how the secret is used, still needs a new version
//! number.
//!
//! The checksum is not a security feature. It turns a mistyped or truncated
//! ticket into a clear error instead of a failed connection.
//!
//! # Handling
//!
//! The ticket holds the secret that lets someone request pairing, so the types
//! here keep it out of logs and wipe their copies. The text form is produced by
//! [`PairTicket::to_text`] on purpose and there is no `Display`: writing the secret
//! somewhere should be a visible decision.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    str::FromStr,
};

use data_encoding::BASE32_NOPAD;
use zeroize::Zeroizing;

use crate::{id::DeviceId, pair::PairSecret};

/// Text form prefix, as written. Parsers accept it in any case.
pub const PREFIX: &str = "BATON1:";

/// Most direct addresses a ticket can carry.
pub const MAX_ADDRS: usize = 16;

/// Longest relay hint, in bytes.
pub const MAX_RELAY_BYTES: usize = 255;

/// Most extensions a ticket may carry. A reader skips the ones it does not know, so this
/// only bounds the work spent on hostile input.
pub const MAX_EXTENSIONS: usize = 16;

/// Longest text form accepted, to bound work on hostile input.
const MAX_TEXT_LEN: usize = 2048;

/// How far a joiner's clock may differ from the offerer's before the joiner refuses a
/// ticket on its own judgement: 10 minutes. See [`PairTicket::is_long_expired`].
pub const CLOCK_SKEW_ALLOWANCE_SECS: u64 = 10 * 60;

const FORMAT_VERSION: u8 = 1;
const CHECKSUM_LEN: usize = 4;
const CHECKSUM_CONTEXT: &str = "baton 2026-10-14 pairing ticket checksum v1";

/// Set in an extension type, or in the high half of `flags`, when a reader must
/// understand the field to use the ticket.
const CRITICAL_EXTENSION_TYPE: u8 = 0x80;
const CRITICAL_FLAGS: u8 = 0xf0;

/// An invitation to pair with one specific device.
///
/// There is deliberately no `Display`: the text form contains the secret, so it comes
/// from [`to_text`](Self::to_text), where writing it out is a visible choice.
///
/// ```compile_fail
/// use baton_proto::ticket::PairTicket;
/// fn needs_display<T: std::fmt::Display>() {}
/// needs_display::<PairTicket>();
/// ```
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
    /// The ticket needs a feature this version does not have. Updating the app may help.
    #[error("this ticket needs a newer version of Baton")]
    NeedsNewerVersion,
    /// A field has a value this version does not accept.
    #[error("invalid ticket: {0}")]
    Invalid(&'static str),
}

impl PairTicket {
    /// Whether `expires_at` has passed at `now`, in seconds since the Unix epoch, by this
    /// device's clock.
    ///
    /// The offerer alone decides whether an offer is still open. It tracks the offer with
    /// a monotonic clock, so a jump of the wall clock after sleep neither extends the
    /// offer nor ends it early. A joiner's clock may be minutes off, so a joiner should
    /// not treat this as final and should use [`is_long_expired`](Self::is_long_expired).
    pub const fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    /// Whether the ticket expired so long ago, by this device's clock, that trying it is
    /// pointless even if the clocks of the two devices disagree by up to
    /// [`CLOCK_SKEW_ALLOWANCE_SECS`].
    ///
    /// A joiner can use this to give a clear message instead of dialing an offer that
    /// ended long ago. It must not refuse more eagerly than this: the offerer's answer
    /// is the one that counts.
    pub const fn is_long_expired(&self, now: u64) -> bool {
        now >= self.expires_at.saturating_add(CLOCK_SKEW_ALLOWANCE_SECS)
    }

    /// Encodes the ticket in its binary layout. The result holds the secret and is wiped
    /// when dropped.
    pub fn to_bytes(&self) -> Result<Zeroizing<Vec<u8>>, TicketError> {
        let relay = self.relay.as_deref().unwrap_or("");
        if !relay.is_empty() {
            check_relay_hint(relay)?;
        }
        if self.addrs.len() > MAX_ADDRS {
            return Err(TicketError::Invalid("too many addresses"));
        }

        let mut out = Zeroizing::new(Vec::with_capacity(96 + relay.len() + self.addrs.len() * 19));
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
        let (body, given) = bytes
            .split_at_checked(body_len)
            .ok_or(TicketError::Truncated)?;
        if checksum(body) != given {
            return Err(TicketError::BadChecksum);
        }

        let mut reader = Reader(body);
        let version = reader.u8()?;
        if version != FORMAT_VERSION {
            return Err(TicketError::UnsupportedVersion(version));
        }
        if reader.u8()? & CRITICAL_FLAGS != 0 {
            return Err(TicketError::NeedsNewerVersion);
        }
        let offerer = DeviceId::from_bytes(reader.array()?);
        let secret = PairSecret::from_bytes(reader.array()?);
        let expires_at = u64::from_be_bytes(reader.array()?);

        let relay_len = usize::from(reader.u8()?);
        let relay = if relay_len == 0 {
            None
        } else {
            let raw = reader.take(relay_len)?;
            // Printable ASCII is always valid UTF-8, and the check below insists on it.
            let relay = String::from_utf8_lossy(raw).into_owned();
            check_relay_hint(&relay)?;
            Some(relay)
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

        // Nothing is defined here yet. Skip what is optional, refuse what is required.
        let mut extensions = 0;
        while !reader.0.is_empty() {
            let kind = reader.u8()?;
            let len = usize::from(reader.u8()?);
            reader.take(len)?;
            if kind & CRITICAL_EXTENSION_TYPE != 0 {
                return Err(TicketError::NeedsNewerVersion);
            }
            extensions += 1;
            if extensions > MAX_EXTENSIONS {
                return Err(TicketError::Invalid("too many extensions"));
            }
        }

        Ok(Self {
            offerer,
            secret,
            expires_at,
            relay,
            addrs,
        })
    }

    /// The text form, `BATON1:` and capital-letter base32, for a QR code or for copying.
    ///
    /// The text contains the secret. The result is wiped when dropped, and there is
    /// deliberately no `Display` for tickets, so that a stray `{ticket}` in a log line
    /// cannot leak one.
    pub fn to_text(&self) -> Result<Zeroizing<String>, TicketError> {
        let bytes = self.to_bytes()?;
        let mut text = Zeroizing::new(String::with_capacity(
            PREFIX.len() + BASE32_NOPAD.encode_len(bytes.len()),
        ));
        text.push_str(PREFIX);
        BASE32_NOPAD.encode_append(&bytes, &mut text);
        Ok(text)
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
        // The copy of the text made here holds the secret, so it is wiped as well.
        let mut compact = Zeroizing::new(String::with_capacity(text.len()));
        compact.extend(
            text.chars()
                .filter(|c| !c.is_whitespace())
                .map(|c| c.to_ascii_uppercase()),
        );
        let encoded = compact.strip_prefix(PREFIX).ok_or(TicketError::BadPrefix)?;
        let bytes = Zeroizing::new(
            BASE32_NOPAD
                .decode(encoded.as_bytes())
                .map_err(|_| TicketError::BadEncoding)?,
        );
        Self::from_bytes(&bytes)
    }
}

/// Checks the relay hint: it is an address the joiner will connect to, and a ticket
/// comes from a scanned code, so it is untrusted input.
///
/// This is a structural check, not a URL parser. It requires `http` or `https`, a host,
/// and no credentials, and nothing it lets through is a reason for the joiner to skip
/// parsing the URL properly before it connects.
fn check_relay_hint(relay: &str) -> Result<(), TicketError> {
    if relay.len() > MAX_RELAY_BYTES {
        return Err(TicketError::Invalid("relay url is too long"));
    }
    if !relay.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(TicketError::Invalid("relay url must be printable ASCII"));
    }
    let after_scheme = ["https://", "http://"]
        .into_iter()
        .find_map(|scheme| {
            let (head, tail) = relay.split_at_checked(scheme.len())?;
            head.eq_ignore_ascii_case(scheme).then_some(tail)
        })
        .ok_or(TicketError::Invalid(
            "relay url must start with http:// or https://",
        ))?;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.is_empty() {
        return Err(TicketError::Invalid("relay url has no host"));
    }
    if authority.contains('@') {
        return Err(TicketError::Invalid(
            "relay url must not contain credentials",
        ));
    }
    let host_chars_only = authority
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']'));
    if !host_chars_only {
        return Err(TicketError::Invalid("relay url has an invalid host"));
    }
    Ok(())
}

/// Whether `relay` would be accepted as the relay hint of a ticket.
///
/// A relay that a device remembers about a peer, and will connect to later, is the same
/// kind of untrusted address as one in a scanned ticket, so it is held to the same rule.
#[must_use]
pub fn is_valid_relay_hint(relay: &str) -> bool {
    check_relay_hint(relay).is_ok()
}

fn checksum(body: &[u8]) -> [u8; CHECKSUM_LEN] {
    let digest = blake3::derive_key(CHECKSUM_CONTEXT, body);
    let [a, b, c, d, ..] = digest;
    [a, b, c, d]
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
        let (head, tail) = self
            .0
            .split_first_chunk::<N>()
            .ok_or(TicketError::Truncated)?;
        self.0 = tail;
        Ok(*head)
    }

    fn u8(&mut self) -> Result<u8, TicketError> {
        let [byte] = self.array::<1>()?;
        Ok(byte)
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

    /// The ticket's body (everything but the checksum), for building damaged variants.
    fn body_of(ticket: &PairTicket) -> Vec<u8> {
        let mut body = ticket.to_bytes().unwrap().to_vec();
        body.truncate(body.len() - CHECKSUM_LEN);
        body
    }

    /// Seals a body with a correct checksum, to exercise the parser itself.
    fn reseal(mut body: Vec<u8>) -> Vec<u8> {
        let sum = checksum(&body);
        body.extend_from_slice(&sum);
        body
    }

    #[test]
    fn text_form_round_trips() {
        let text = ticket().to_text().unwrap();
        assert!(text.starts_with(PREFIX));
        let parsed: PairTicket = text.parse().unwrap();
        assert_eq!(parsed.offerer, ticket().offerer);
        assert_eq!(parsed.secret.expose(), ticket().secret.expose());
        assert_eq!(parsed.expires_at, ticket().expires_at);
        assert_eq!(parsed.relay, ticket().relay);
        assert_eq!(parsed.addrs, ticket().addrs);
    }

    #[test]
    fn text_form_uses_only_the_characters_of_a_qr_alphanumeric_code() {
        // Digits, capital letters and ':' fit the denser alphanumeric mode of a QR code.
        let text = ticket().to_text().unwrap();
        assert!(
            text.bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase() || b == b':'),
            "{}",
            text.as_str()
        );
    }

    #[test]
    fn minimal_ticket_round_trips() {
        let minimal = PairTicket {
            relay: None,
            addrs: vec![],
            ..ticket()
        };
        let parsed: PairTicket = minimal.to_text().unwrap().parse().unwrap();
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
    fn text_and_extension_vectors_are_stable() {
        // Built independently with the reference BLAKE3 implementation and an RFC 4648 base32
        // encoder. The ticket is the one of `binary_layout_is_stable`.
        const BODY: &str = "010011111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222000000006acf37480a68747470733a2f2f722f01040a00000100";
        let ticket_from = |hex: String| data_encoding::HEXLOWER.decode(hex.as_bytes()).unwrap();

        let plain = PairTicket {
            relay: Some("https://r/".into()),
            addrs: vec!["10.0.0.1:80".parse().unwrap()],
            ..ticket()
        };
        assert_eq!(
            plain.to_text().unwrap().as_str(),
            "BATON1:AEABCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEJCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIRCEIAAAAAANLHTOSAKNB2HI4DTHIXS64RPAECAUAAAAEAFBHTYDLVQ"
        );

        // With an ignorable extension (type 0x01, value aabbcc) before the checksum, the
        // ticket is read and the extension is dropped.
        let optional = ticket_from(format!("{BODY}50{}{}", "0103aabbcc", "535c7c97"));
        let parsed = PairTicket::from_bytes(&optional).unwrap();
        assert_eq!(parsed.to_bytes().unwrap(), plain.to_bytes().unwrap());

        // A critical extension (type 0x80) is refused by a reader that does not know it.
        let critical = ticket_from(format!("{BODY}50{}{}", "8002aabb", "2a0dfea9"));
        assert_eq!(
            PairTicket::from_bytes(&critical).unwrap_err(),
            TicketError::NeedsNewerVersion
        );

        // So is a critical flag (0x10), while an ignorable flag (0x01) is read.
        let critical_flag = ticket_from(format!("0110{}50{}", &BODY[4..], "bcbeeb70"));
        assert_eq!(
            PairTicket::from_bytes(&critical_flag).unwrap_err(),
            TicketError::NeedsNewerVersion
        );
        let ignorable_flag = ticket_from(format!("0101{}50{}", &BODY[4..], "9a37ebee"));
        assert!(PairTicket::from_bytes(&ignorable_flag).is_ok());
    }

    #[test]
    fn parsing_ignores_case_and_copy_artifacts() {
        let text = ticket().to_text().unwrap();
        assert!(text.to_lowercase().parse::<PairTicket>().is_ok());
        let wrapped = format!("  {}\n{}\r\n", &text[..20], &text[20..]);
        assert!(wrapped.parse::<PairTicket>().is_ok());
    }

    #[test]
    fn debug_output_hides_the_secret() {
        let debug = format!("{:?}", ticket());
        assert!(debug.contains("PairSecret(..)"), "{debug}");
        assert!(!debug.contains("2222"), "{debug}");
        assert!(!debug.to_uppercase().contains("EIRCEIRC"), "{debug}");
    }

    #[test]
    fn expiry_is_exclusive_of_the_deadline() {
        let t = ticket();
        assert!(!t.is_expired(t.expires_at - 1));
        assert!(t.is_expired(t.expires_at));
        assert!(t.is_expired(u64::MAX));
    }

    #[test]
    fn a_joiner_gives_the_offerer_the_benefit_of_a_clock_difference() {
        let t = ticket();
        let late = t.expires_at + CLOCK_SKEW_ALLOWANCE_SECS;
        assert!(!t.is_long_expired(t.expires_at));
        assert!(!t.is_long_expired(late - 1));
        assert!(t.is_long_expired(late));
        assert!(t.is_long_expired(u64::MAX));
        let forever = PairTicket {
            expires_at: u64::MAX,
            ..ticket()
        };
        assert!(!forever.is_long_expired(u64::MAX - 1));
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
            "BATON1:".parse::<PairTicket>().unwrap_err(),
            TicketError::Truncated
        );
        let long = format!("BATON1:{}", "A".repeat(MAX_TEXT_LEN));
        assert_eq!(
            long.parse::<PairTicket>().unwrap_err(),
            TicketError::TooLong
        );
    }

    #[test]
    fn every_single_character_change_is_caught() {
        let text = ticket().to_text().unwrap();
        let body = &text[PREFIX.len()..];
        for i in 0..body.len() {
            let mut chars: Vec<char> = body.chars().collect();
            chars[i] = if chars[i] == 'A' { 'B' } else { 'A' };
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
        let body = body_of(&ticket());

        let mut future_version = body.clone();
        future_version[0] = 2;
        assert_eq!(
            PairTicket::from_bytes(&reseal(future_version)).unwrap_err(),
            TicketError::UnsupportedVersion(2)
        );

        let mut bad_tag = body_of(&PairTicket {
            relay: None,
            addrs: vec!["1.2.3.4:5".parse().unwrap()],
            ..ticket()
        });
        let tag_at = bad_tag.len() - 7;
        bad_tag[tag_at] = 5;
        assert!(matches!(
            PairTicket::from_bytes(&reseal(bad_tag)).unwrap_err(),
            TicketError::Invalid(_)
        ));
    }

    #[test]
    fn low_flag_bits_may_be_ignored_and_high_ones_may_not() {
        let body = body_of(&ticket());
        for flags in [0x01, 0x08, 0x0f] {
            let mut with_flag = body.clone();
            with_flag[1] = flags;
            assert!(
                PairTicket::from_bytes(&reseal(with_flag)).is_ok(),
                "flags {flags:#04x}"
            );
        }
        for flags in [0x10, 0x80, 0xf0, 0xff] {
            let mut with_flag = body.clone();
            with_flag[1] = flags;
            assert_eq!(
                PairTicket::from_bytes(&reseal(with_flag)).unwrap_err(),
                TicketError::NeedsNewerVersion,
                "flags {flags:#04x}"
            );
        }
    }

    #[test]
    fn extensions_an_older_reader_does_not_know_are_skipped_unless_they_are_critical() {
        let body = body_of(&ticket());

        // type 0x01, three bytes of value
        let mut optional = body.clone();
        optional.extend_from_slice(&[0x01, 3, 0xaa, 0xbb, 0xcc]);
        // type 0x7f, empty
        optional.extend_from_slice(&[0x7f, 0]);
        let parsed = PairTicket::from_bytes(&reseal(optional)).unwrap();
        assert_eq!(parsed.offerer, ticket().offerer);
        assert_eq!(parsed.secret.expose(), ticket().secret.expose());
        assert_eq!(parsed.addrs, ticket().addrs);

        for kind in [0x80, 0x81, 0xff] {
            let mut critical = body.clone();
            critical.extend_from_slice(&[0x01, 1, 0]);
            critical.extend_from_slice(&[kind, 2, 0xaa, 0xbb]);
            assert_eq!(
                PairTicket::from_bytes(&reseal(critical)).unwrap_err(),
                TicketError::NeedsNewerVersion,
                "type {kind:#04x}"
            );
        }
    }

    #[test]
    fn damaged_extensions_are_errors() {
        let body = body_of(&ticket());

        let mut no_length = body.clone();
        no_length.push(0x01);
        assert_eq!(
            PairTicket::from_bytes(&reseal(no_length)).unwrap_err(),
            TicketError::Truncated
        );

        let mut value_cut_short = body.clone();
        value_cut_short.extend_from_slice(&[0x01, 5, 0xaa]);
        assert_eq!(
            PairTicket::from_bytes(&reseal(value_cut_short)).unwrap_err(),
            TicketError::Truncated
        );

        let mut too_many = body.clone();
        for _ in 0..=MAX_EXTENSIONS {
            too_many.extend_from_slice(&[0x01, 0]);
        }
        assert_eq!(
            PairTicket::from_bytes(&reseal(too_many)).unwrap_err(),
            TicketError::Invalid("too many extensions")
        );

        let mut at_the_limit = body;
        for _ in 0..MAX_EXTENSIONS {
            at_the_limit.extend_from_slice(&[0x01, 0]);
        }
        assert!(PairTicket::from_bytes(&reseal(at_the_limit)).is_ok());
    }

    #[test]
    fn oversized_fields_are_not_encoded() {
        let long_relay = PairTicket {
            relay: Some(format!("https://{}", "h".repeat(MAX_RELAY_BYTES))),
            ..ticket()
        };
        assert!(long_relay.to_bytes().is_err());
        let spaced_relay = PairTicket {
            relay: Some("https://has space/".into()),
            ..ticket()
        };
        assert!(spaced_relay.to_bytes().is_err());
        let many = PairTicket {
            addrs: vec!["1.2.3.4:5".parse().unwrap(); MAX_ADDRS + 1],
            ..ticket()
        };
        assert!(many.to_bytes().is_err());
    }

    #[test]
    fn relay_hints_must_be_http_urls_with_a_host_and_no_credentials() {
        for good in [
            "https://relay.example/",
            "http://localhost:3340",
            "https://203.0.113.5:3340/",
            "https://[2001:db8::1]:3340/path?x=1",
            "HTTPS://UPPER.example",
            "https://euw1-1.relay.n0.iroh.link./",
        ] {
            assert_eq!(check_relay_hint(good), Ok(()), "{good}");
            assert!(is_valid_relay_hint(good), "{good}");
        }
        for bad in [
            "relay.example",
            "ftp://relay.example/",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
            "https:///path",
            "https://?x=1",
            "https://user:secret@relay.example/",
            "https://relay.example@evil.example/",
            "https://relay.example\\@evil.example/",
            "https://ex%61mple/",
            "https://exa mple/",
            "https://relay.example/\u{e9}",
        ] {
            assert!(
                matches!(check_relay_hint(bad), Err(TicketError::Invalid(_))),
                "{bad}"
            );
            assert!(!is_valid_relay_hint(bad), "{bad}");
        }
    }

    #[test]
    fn a_bad_relay_hint_is_refused_when_reading_a_ticket_too() {
        let with_relay = |relay: &str| {
            let mut ticket = ticket();
            ticket.relay = Some("https://relay.example/".into());
            let mut body = body_of(&ticket);
            // The relay length byte sits after version, flags, offerer, secret and expires_at.
            let at = 2 + 32 + 32 + 8;
            let old_len = usize::from(body[at]);
            body.splice(
                at..=at + old_len,
                [u8::try_from(relay.len()).unwrap()]
                    .into_iter()
                    .chain(relay.bytes()),
            );
            reseal(body)
        };
        assert!(PairTicket::from_bytes(&with_relay("https://other.example/")).is_ok());
        for bad in [
            "http://",
            "https://u:p@relay.example/",
            "gopher://relay.example/",
        ] {
            assert!(
                matches!(
                    PairTicket::from_bytes(&with_relay(bad)),
                    Err(TicketError::Invalid(_))
                ),
                "{bad}"
            );
        }
    }
}
