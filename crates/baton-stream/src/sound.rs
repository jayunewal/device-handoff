// SPDX-License-Identifier: Apache-2.0

//! Packets of sound: the header, and the rules for sending and receiving them.
//!
//! A packet of sound is one unidirectional stream: the 17 bytes that start every stream of a
//! session ([`PREFIX_LEN`], with the kind [`KIND_SOUND`]), the rest of the header
//! ([`PacketHeader`], 34 bytes in all) and then the packet of the codec, up to the end of the
//! stream. All numbers are big-endian. The design is in `docs/adr/0012-sound.md`.
//!
//! - [`send`] holds the source's accounting of packets in flight, and when to give up on one.
//! - [`receive`] holds the viewer's choice of which packet to play next, what to do about the
//!   ones that do not come, and its limits on the packet streams it reads.
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 1 | kind, [`KIND_SOUND`] |
//! | 1 | 16 | session |
//! | 17 | 8 | packet number |
//! | 25 | 8 | capture time, in microseconds |
//! | 33 | 1 | flags |

use baton_proto::id::SessionId;

use crate::{
    KIND_SOUND,
    header::{PREFIX_LEN, decode_prefix, encode_prefix},
};

pub mod receive;
pub mod send;
#[cfg(test)]
mod sim;

/// The flags of a packet of sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PacketFlags(u8);

impl PacketFlags {
    /// No flag set.
    pub const NONE: Self = Self(0);
    /// The first packet after the source began to send sound, or began again: the decoder
    /// forgets what it held before this packet.
    pub const FIRST: Self = Self(0b1);

    const ALL: u8 = 0b1;

    /// Reads flags from a byte. A byte with bits that are not defined is refused: a sender only
    /// sets what the receiver advertised, so a bit that is not defined is a broken sender.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// The byte that goes on the wire.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every flag of `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Why a header was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PacketError {
    /// There are fewer bytes than a header has.
    #[error("the header is cut short")]
    Truncated,
    /// The stream is not a stream of sound.
    #[error("a stream of kind {found} does not carry sound")]
    WrongKind {
        /// The kind byte that was read.
        found: u8,
    },
    /// A bit is set that no version of the protocol defines.
    #[error("the flags {0:#04x} use bits that are not defined")]
    ReservedFlags(u8),
}

/// The header of a packet of sound: what the viewer needs to put it in order and to play it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PacketHeader {
    /// The session the packet belongs to.
    pub session: SessionId,
    /// Counts the packets of a session from 0, one for each packet that the encoder made,
    /// including those that were reset later and those that the source had no room to write. A
    /// packet that is reset is not sent again, so a number is never used twice, and a number that
    /// never arrives tells the viewer that a packet is missing.
    pub number: u64,
    /// When the sound of the packet was captured, in microseconds since the session began, on
    /// the source's clock: the clock that [`FrameHeader::capture_time`](crate::header::FrameHeader)
    /// uses. It is the time of the first sample of the packet.
    pub capture_time: u64,
    /// Whether the packet begins a run of sound.
    pub flags: PacketFlags,
}

impl PacketHeader {
    /// Length of the header in bytes, the kind byte included.
    pub const LEN: usize = PREFIX_LEN + 8 + 8 + 1;

    /// Whether the packet begins a run of sound, so that the decoder starts afresh.
    pub const fn is_first(&self) -> bool {
        self.flags.contains(PacketFlags::FIRST)
    }

    /// Writes the header.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::LEN);
        out.extend_from_slice(&encode_prefix(KIND_SOUND, self.session));
        out.extend_from_slice(&self.number.to_be_bytes());
        out.extend_from_slice(&self.capture_time.to_be_bytes());
        out.push(self.flags.bits());
        out
    }

    /// Reads and checks a header that has been read from a stream in full.
    pub fn decode(bytes: &[u8; Self::LEN]) -> Result<Self, PacketError> {
        let (header, _) = Self::parse(bytes)?;
        Ok(header)
    }

    /// Reads and checks a header from the start of `bytes`, and returns what follows it.
    pub fn parse(bytes: &[u8]) -> Result<(Self, &[u8]), PacketError> {
        let (prefix, rest) = bytes
            .split_first_chunk::<PREFIX_LEN>()
            .ok_or(PacketError::Truncated)?;
        let (kind, session) = decode_prefix(prefix);
        if kind != KIND_SOUND {
            return Err(PacketError::WrongKind { found: kind });
        }
        let (number, rest) = rest
            .split_first_chunk::<8>()
            .ok_or(PacketError::Truncated)?;
        let (capture_time, rest) = rest
            .split_first_chunk::<8>()
            .ok_or(PacketError::Truncated)?;
        let (flags, rest) = rest.split_first().ok_or(PacketError::Truncated)?;
        let flags = PacketFlags::from_bits(*flags).ok_or(PacketError::ReservedFlags(*flags))?;
        let header = Self {
            session,
            number: u64::from_be_bytes(*number),
            capture_time: u64::from_be_bytes(*capture_time),
            flags,
        };
        Ok((header, rest))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn session() -> SessionId {
        SessionId::from_bytes(std::array::from_fn(|i| u8::try_from(i + 0xa0).unwrap()))
    }

    fn first() -> PacketHeader {
        PacketHeader {
            session: session(),
            number: 0x0102_0304_0506_0708,
            capture_time: 0x2122_2324_2526_2728,
            flags: PacketFlags::FIRST,
        }
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::new(), |mut out, b| {
            write!(out, "{b:02x}").unwrap();
            out
        })
    }

    #[test]
    fn a_header_has_a_known_wire_encoding() {
        // The exact bytes are the protocol. If this fails, the wire format changed.
        let bytes = first().encode();
        assert_eq!(bytes.len(), PacketHeader::LEN);
        assert_eq!(PacketHeader::LEN, 34);
        assert_eq!(
            hex(&bytes),
            concat!(
                "04",                               // kind: sound
                "a0a1a2a3a4a5a6a7a8a9aaabacadaeaf", // session
                "0102030405060708",                 // packet number
                "2122232425262728",                 // capture time
                "01",                               // flags: first
            )
        );
    }

    #[test]
    fn the_vectors_of_protocol_md_are_what_the_code_writes() {
        // The start of a packet stream for the session of these tests.
        assert_eq!(
            hex(&encode_prefix(KIND_SOUND, session())),
            "04a0a1a2a3a4a5a6a7a8a9aaabacadaeaf"
        );
        // The first packet of a session, captured at the start, and the one after it, 21333
        // microseconds later (1024 samples at 48000 Hz).
        let packet = |number, capture_time, flags| PacketHeader {
            session: session(),
            number,
            capture_time,
            flags,
        };
        assert_eq!(
            hex(&packet(0, 0, PacketFlags::FIRST).encode()),
            concat!(
                "04a0a1a2a3a4a5a6a7a8a9aaabacadaeaf",
                "0000000000000000",
                "0000000000000000",
                "01",
            )
        );
        assert_eq!(
            hex(&packet(1, 21_333, PacketFlags::NONE).encode()),
            concat!(
                "04a0a1a2a3a4a5a6a7a8a9aaabacadaeaf",
                "0000000000000001",
                "0000000000005355",
                "00",
            )
        );
    }

    #[test]
    fn a_header_reads_back_and_leaves_the_packet_alone() {
        let header = first();
        let mut bytes = header.encode();
        bytes.extend_from_slice(b"packet");
        let (read, rest) = PacketHeader::parse(&bytes).unwrap();
        assert_eq!(read, header);
        assert_eq!(rest, b"packet");
        assert!(read.is_first());
        assert!(
            !PacketHeader {
                flags: PacketFlags::NONE,
                ..header
            }
            .is_first()
        );
    }

    #[test]
    fn a_header_of_exactly_the_length_is_read_with_the_same_checks() {
        let header = first();
        let bytes: [u8; PacketHeader::LEN] = header.encode().try_into().unwrap();
        assert_eq!(PacketHeader::decode(&bytes), Ok(header));
        let mut other = bytes;
        *other.first_mut().unwrap() = 2;
        assert_eq!(
            PacketHeader::decode(&other),
            Err(PacketError::WrongKind { found: 2 })
        );
        let mut flagged = bytes;
        *flagged.last_mut().unwrap() = 0b10;
        assert_eq!(
            PacketHeader::decode(&flagged),
            Err(PacketError::ReservedFlags(0b10))
        );
    }

    #[test]
    fn a_header_that_is_cut_short_is_refused_at_every_length() {
        let bytes = first().encode();
        for len in 0..PacketHeader::LEN {
            assert_eq!(
                PacketHeader::parse(bytes.get(..len).unwrap()),
                Err(PacketError::Truncated),
                "{len} bytes"
            );
        }
    }

    #[test]
    fn another_kind_of_stream_is_not_sound() {
        for kind in [0_u8, 1, 2, 3, 5, 255] {
            let mut bytes = first().encode();
            *bytes.first_mut().unwrap() = kind;
            assert_eq!(
                PacketHeader::parse(&bytes),
                Err(PacketError::WrongKind { found: kind })
            );
        }
    }

    #[test]
    fn flags_that_are_not_defined_are_refused() {
        for bits in [0b10_u8, 0x80, 0xff, 0b11] {
            let mut bytes = first().encode();
            *bytes.last_mut().unwrap() = bits;
            assert_eq!(
                PacketHeader::parse(&bytes),
                Err(PacketError::ReservedFlags(bits))
            );
        }
        assert_eq!(PacketFlags::from_bits(0), Some(PacketFlags::NONE));
        assert_eq!(PacketFlags::from_bits(1), Some(PacketFlags::FIRST));
        assert_eq!(PacketFlags::FIRST.bits(), 1);
    }

    #[test]
    fn the_session_of_a_packet_is_found_as_it_is_for_every_stream_of_a_session() {
        // It is the 16 bytes after the kind, so a receiver that reads the start of a stream
        // before it knows the kind finds it with the same function.
        let bytes = first().encode();
        let (kind, session) = decode_prefix(bytes.first_chunk::<PREFIX_LEN>().unwrap());
        assert_eq!((kind, session), (KIND_SOUND, first().session));
    }

    fn arbitrary_header() -> impl Strategy<Value = PacketHeader> {
        (
            any::<[u8; 16]>(),
            any::<u64>(),
            any::<u64>(),
            prop_oneof![Just(PacketFlags::NONE), Just(PacketFlags::FIRST)],
        )
            .prop_map(|(session, number, capture_time, flags)| PacketHeader {
                session: SessionId::from_bytes(session),
                number,
                capture_time,
                flags,
            })
    }

    proptest! {
        #[test]
        fn a_valid_header_reads_back_as_it_was_written(header in arbitrary_header()) {
            let bytes = header.encode();
            prop_assert_eq!(bytes.len(), PacketHeader::LEN);
            let (read, rest) = PacketHeader::parse(&bytes).unwrap();
            prop_assert_eq!(read, header);
            prop_assert!(rest.is_empty());
        }

        #[test]
        fn bytes_that_are_not_a_header_are_an_error_and_not_a_panic(
            bytes in proptest::collection::vec(any::<u8>(), 0..80)
        ) {
            if let Ok((header, rest)) = PacketHeader::parse(&bytes) {
                // Whatever is accepted is a header that writes back to the same bytes.
                let again = header.encode();
                prop_assert_eq!(bytes.get(..PacketHeader::LEN), Some(&again[..]));
                prop_assert_eq!(rest.len(), bytes.len() - PacketHeader::LEN);
            }
        }

        // Random bytes start with the kind of a packet of sound once in 256 cases, so the
        // property above rarely gets past the first byte. This one starts with the kind, makes
        // the flags mostly defined and the length mostly whole, which puts the cut, the flag
        // check, and the writing back of what is accepted, under a property too.
        #[test]
        fn bytes_that_start_like_a_header_are_read_or_refused_and_never_a_panic(
            body in any::<[u8; 32]>(),
            flags in prop_oneof![any::<u8>(), 0_u8..2],
            tail in proptest::collection::vec(any::<u8>(), 0..8),
            len in prop_oneof![Just(PacketHeader::LEN), 0..=PacketHeader::LEN + 8],
        ) {
            let mut bytes = vec![KIND_SOUND];
            bytes.extend_from_slice(&body);
            bytes.push(flags);
            bytes.extend_from_slice(&tail);
            bytes.truncate(len);
            match PacketHeader::parse(&bytes) {
                Ok((header, rest)) => {
                    let again = header.encode();
                    prop_assert_eq!(bytes.get(..PacketHeader::LEN), Some(&again[..]));
                    prop_assert_eq!(rest.len(), bytes.len() - PacketHeader::LEN);
                }
                Err(PacketError::Truncated) => prop_assert!(bytes.len() < PacketHeader::LEN),
                Err(_) => prop_assert!(bytes.len() >= PacketHeader::LEN),
            }
        }
    }
}
