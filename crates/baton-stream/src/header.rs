// SPDX-License-Identifier: Apache-2.0

//! The first bytes of the streams that carry video, and the header of a frame.
//!
//! Every unidirectional stream of window streaming starts with the same 17 bytes: one byte for
//! the kind of stream and 16 for the session it belongs to ([`PREFIX_LEN`]). A stream of video
//! goes on with the rest of the frame header ([`FrameHeader`], 50 bytes in all) and then the
//! encoded picture, up to the end of the stream. All numbers are big-endian.
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 1 | kind, [`KIND_VIDEO`] |
//! | 1 | 16 | session |
//! | 17 | 8 | frame number |
//! | 25 | 8 | depends on |
//! | 33 | 4 | chain |
//! | 37 | 4 | epoch |
//! | 41 | 8 | capture time, in microseconds |
//! | 49 | 1 | flags |

use baton_proto::id::SessionId;

use crate::KIND_VIDEO;

/// Length of the start that every stream of window streaming has: the kind and the session.
pub const PREFIX_LEN: usize = 1 + SessionId::LEN;

/// Writes the start of a stream: its kind, then the session it belongs to.
pub fn encode_prefix(kind: u8, session: SessionId) -> [u8; PREFIX_LEN] {
    std::array::from_fn(|i| match i.checked_sub(1) {
        None => kind,
        Some(at) => session.as_bytes().get(at).copied().unwrap_or(0),
    })
}

/// Reads the start of a stream, and returns its kind and its session.
///
/// Whether the kind is one that this peer allows, and whether the session is running, are for
/// the caller to decide.
pub fn decode_prefix(bytes: &[u8; PREFIX_LEN]) -> (u8, SessionId) {
    let kind = bytes.first().copied().unwrap_or(0);
    let session = std::array::from_fn(|i| bytes.get(i + 1).copied().unwrap_or(0));
    (kind, SessionId::from_bytes(session))
}

/// What a frame is, as far as the viewer's decoder is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameKind {
    /// Can be decoded alone, and makes the decoder forget everything before it.
    Key,
    /// Refers to a long-term reference that the viewer is known to hold. It begins a new chain
    /// without a key frame.
    Recovery,
    /// Refers to frames of its own chain.
    Delta,
}

/// The flags of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Flags(u8);

impl Flags {
    /// No flag set: a frame that nothing refers to.
    pub const NONE: Self = Self(0);
    /// A key frame.
    pub const KEY: Self = Self(0b001);
    /// A recovery frame.
    pub const RECOVERY: Self = Self(0b010);
    /// Another frame may refer to this one, so it cannot be skipped.
    pub const REFERENCED: Self = Self(0b100);

    const ALL: u8 = 0b111;

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

impl std::ops::BitOr for Flags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Why a header was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    /// There are fewer bytes than a header has.
    #[error("the header is cut short")]
    Truncated,
    /// The stream is not a stream of video.
    #[error("a stream of kind {found} does not carry video")]
    WrongKind {
        /// The kind byte that was read.
        found: u8,
    },
    /// A bit is set that no version of the protocol defines.
    #[error("the flags {0:#04x} use bits that are not defined")]
    ReservedFlags(u8),
    /// Both the key and the recovery flag are set.
    #[error("a frame cannot be a key frame and a recovery frame at once")]
    KeyAndRecovery,
    /// The frame depends on a frame that cannot come before it.
    #[error("frame {number} cannot depend on frame {depends_on}")]
    BadDependency {
        /// The number of the frame.
        number: u64,
        /// The number it depends on.
        depends_on: u64,
    },
}

/// The header of a frame of video: what the viewer needs to decode it and to put it in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHeader {
    /// The session the frame belongs to.
    pub session: SessionId,
    /// Counts the frames of a session from 0, one for each frame that was encoded, including
    /// those that were abandoned later.
    pub number: u64,
    /// The newest frame that the viewer must have decoded before this one. It is the frame's own
    /// number for a key frame, which means that a key frame needs no other frame. For any other
    /// frame it is the newest frame before this one that something may refer to, so frames that
    /// nothing refers to can be skipped. For a recovery frame it is the newest such frame that
    /// the source knows the viewer holds, and it can be in an earlier chain.
    pub depends_on: u64,
    /// Counts the chains of a session. A chain is a run of frames that are decoded one after the
    /// other, and it begins at a key frame or at a recovery frame.
    pub chain: u32,
    /// Counts the sizes of the picture. It changes, with a key frame, when the size changes.
    pub epoch: u32,
    /// When the picture was captured, in microseconds since the session began, on the source's
    /// clock.
    pub capture_time: u64,
    /// Key frame, recovery frame, and whether something refers to this frame.
    pub flags: Flags,
}

impl FrameHeader {
    /// Length of the header in bytes, the kind byte included.
    pub const LEN: usize = PREFIX_LEN + 8 + 8 + 4 + 4 + 8 + 1;

    /// What the frame is.
    pub const fn kind(&self) -> FrameKind {
        if self.flags.contains(Flags::KEY) {
            FrameKind::Key
        } else if self.flags.contains(Flags::RECOVERY) {
            FrameKind::Recovery
        } else {
            FrameKind::Delta
        }
    }

    /// Whether the frame is a key frame.
    pub const fn is_key(&self) -> bool {
        matches!(self.kind(), FrameKind::Key)
    }

    /// Whether another frame may refer to this one.
    pub const fn is_referenced(&self) -> bool {
        self.flags.contains(Flags::REFERENCED)
    }

    /// Checks the rules that do not depend on any other frame.
    pub fn validate(&self) -> Result<(), HeaderError> {
        if self.flags.contains(Flags::KEY) && self.flags.contains(Flags::RECOVERY) {
            return Err(HeaderError::KeyAndRecovery);
        }
        let fine = match self.kind() {
            FrameKind::Key => self.depends_on == self.number,
            FrameKind::Recovery | FrameKind::Delta => self.depends_on < self.number,
        };
        if fine {
            Ok(())
        } else {
            Err(HeaderError::BadDependency {
                number: self.number,
                depends_on: self.depends_on,
            })
        }
    }

    /// Writes the header, after checking it.
    pub fn encode(&self) -> Result<Vec<u8>, HeaderError> {
        self.validate()?;
        let mut out = Vec::with_capacity(Self::LEN);
        out.extend_from_slice(&encode_prefix(KIND_VIDEO, self.session));
        out.extend_from_slice(&self.number.to_be_bytes());
        out.extend_from_slice(&self.depends_on.to_be_bytes());
        out.extend_from_slice(&self.chain.to_be_bytes());
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&self.capture_time.to_be_bytes());
        out.push(self.flags.bits());
        Ok(out)
    }

    /// Reads and checks a header that has been read from a stream in full.
    pub fn decode(bytes: &[u8; Self::LEN]) -> Result<Self, HeaderError> {
        let (header, _) = Self::parse(bytes)?;
        Ok(header)
    }

    /// Reads and checks a header from the start of `bytes`, and returns what follows it.
    pub fn parse(bytes: &[u8]) -> Result<(Self, &[u8]), HeaderError> {
        let (prefix, rest) = bytes
            .split_first_chunk::<PREFIX_LEN>()
            .ok_or(HeaderError::Truncated)?;
        let (kind, session) = decode_prefix(prefix);
        if kind != KIND_VIDEO {
            return Err(HeaderError::WrongKind { found: kind });
        }
        let (number, rest) = take_u64(rest)?;
        let (depends_on, rest) = take_u64(rest)?;
        let (chain, rest) = take_u32(rest)?;
        let (epoch, rest) = take_u32(rest)?;
        let (capture_time, rest) = take_u64(rest)?;
        let (flags, rest) = rest.split_first().ok_or(HeaderError::Truncated)?;
        let flags = Flags::from_bits(*flags).ok_or(HeaderError::ReservedFlags(*flags))?;
        let header = Self {
            session,
            number,
            depends_on,
            chain,
            epoch,
            capture_time,
            flags,
        };
        header.validate()?;
        Ok((header, rest))
    }
}

fn take_u64(bytes: &[u8]) -> Result<(u64, &[u8]), HeaderError> {
    let (head, rest) = bytes
        .split_first_chunk::<8>()
        .ok_or(HeaderError::Truncated)?;
    Ok((u64::from_be_bytes(*head), rest))
}

fn take_u32(bytes: &[u8]) -> Result<(u32, &[u8]), HeaderError> {
    let (head, rest) = bytes
        .split_first_chunk::<4>()
        .ok_or(HeaderError::Truncated)?;
    Ok((u32::from_be_bytes(*head), rest))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn session() -> SessionId {
        SessionId::from_bytes(std::array::from_fn(|i| u8::try_from(i + 0xa0).unwrap()))
    }

    fn delta() -> FrameHeader {
        FrameHeader {
            session: session(),
            number: 0x0102_0304_0506_0708,
            depends_on: 0x0102_0304_0506_0707,
            chain: 0x0a0b_0c0d,
            epoch: 0x1112_1314,
            capture_time: 0x2122_2324_2526_2728,
            flags: Flags::REFERENCED,
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
        let bytes = delta().encode().unwrap();
        assert_eq!(bytes.len(), FrameHeader::LEN);
        assert_eq!(FrameHeader::LEN, 50);
        assert_eq!(
            hex(&bytes),
            concat!(
                "02",                               // kind: video
                "a0a1a2a3a4a5a6a7a8a9aaabacadaeaf", // session
                "0102030405060708",                 // frame number
                "0102030405060707",                 // depends on
                "0a0b0c0d",                         // chain
                "11121314",                         // epoch
                "2122232425262728",                 // capture time
                "04",                               // flags: referenced
            )
        );
    }

    #[test]
    fn the_vectors_of_protocol_md_are_what_the_code_writes() {
        // The start of a frame stream for the session of these tests.
        assert_eq!(
            hex(&encode_prefix(KIND_VIDEO, session())),
            "02a0a1a2a3a4a5a6a7a8a9aaabacadaeaf"
        );
        // Key frame 5, which depends on itself, in chain 1 and epoch 0, captured at 200000 us.
        let key = FrameHeader {
            session: session(),
            number: 5,
            depends_on: 5,
            chain: 1,
            epoch: 0,
            capture_time: 200_000,
            flags: Flags::KEY | Flags::REFERENCED,
        };
        assert_eq!(
            hex(&key.encode().unwrap()),
            concat!(
                "02a0a1a2a3a4a5a6a7a8a9aaabacadaeaf",
                "0000000000000005",
                "0000000000000005",
                "00000001",
                "00000000",
                "0000000000030d40",
                "05",
            )
        );
        // The frame of `a_header_has_a_known_wire_encoding`, as the document gives it.
        assert_eq!(
            hex(&delta().encode().unwrap()),
            concat!(
                "02a0a1a2a3a4a5a6a7a8a9aaabacadaeaf",
                "0102030405060708",
                "0102030405060707",
                "0a0b0c0d",
                "11121314",
                "2122232425262728",
                "04",
            )
        );
    }

    #[test]
    fn a_header_reads_back_and_leaves_the_picture_alone() {
        let header = delta();
        let mut bytes = header.encode().unwrap();
        bytes.extend_from_slice(b"picture");
        let (read, rest) = FrameHeader::parse(&bytes).unwrap();
        assert_eq!(read, header);
        assert_eq!(rest, b"picture");
    }

    #[test]
    fn the_kind_of_a_frame_follows_its_flags() {
        let key = FrameHeader {
            number: 5,
            depends_on: 5,
            flags: Flags::KEY | Flags::REFERENCED,
            ..delta()
        };
        let recovery = FrameHeader {
            flags: Flags::RECOVERY | Flags::REFERENCED,
            ..delta()
        };
        assert_eq!(key.kind(), FrameKind::Key);
        assert!(key.is_key() && key.is_referenced());
        assert_eq!(recovery.kind(), FrameKind::Recovery);
        assert_eq!(delta().kind(), FrameKind::Delta);
        assert!(
            !FrameHeader {
                flags: Flags::NONE,
                ..delta()
            }
            .is_referenced()
        );
    }

    #[test]
    fn a_key_frame_depends_on_itself_and_nothing_else_does() {
        let key = FrameHeader {
            flags: Flags::KEY,
            ..delta()
        };
        assert!(matches!(
            key.validate(),
            Err(HeaderError::BadDependency { .. })
        ));
        let ok = FrameHeader {
            depends_on: key.number,
            ..key
        };
        assert_eq!(ok.validate(), Ok(()));
        for depends_on in [delta().number, delta().number + 1, u64::MAX] {
            let bad = FrameHeader {
                depends_on,
                ..delta()
            };
            assert!(
                matches!(bad.validate(), Err(HeaderError::BadDependency { .. })),
                "{depends_on}"
            );
        }
        let recovery = FrameHeader {
            flags: Flags::RECOVERY,
            depends_on: 0,
            ..delta()
        };
        assert_eq!(recovery.validate(), Ok(()));
    }

    #[test]
    fn a_frame_is_not_a_key_frame_and_a_recovery_frame() {
        let both = FrameHeader {
            flags: Flags::KEY | Flags::RECOVERY,
            depends_on: delta().number,
            ..delta()
        };
        assert_eq!(both.validate(), Err(HeaderError::KeyAndRecovery));
        assert_eq!(both.encode(), Err(HeaderError::KeyAndRecovery));
    }

    #[test]
    fn a_header_that_is_cut_short_is_refused_at_every_length() {
        let bytes = delta().encode().unwrap();
        for len in 0..FrameHeader::LEN {
            assert_eq!(
                FrameHeader::parse(bytes.get(..len).unwrap()),
                Err(HeaderError::Truncated),
                "{len} bytes"
            );
        }
    }

    #[test]
    fn another_kind_of_stream_is_not_video() {
        for kind in [0_u8, 1, 3, 4, 255] {
            let mut bytes = delta().encode().unwrap();
            *bytes.first_mut().unwrap() = kind;
            assert_eq!(
                FrameHeader::parse(&bytes),
                Err(HeaderError::WrongKind { found: kind })
            );
        }
    }

    #[test]
    fn flags_that_are_not_defined_are_refused() {
        for bits in [0b1000_u8, 0x80, 0xff, 0b1_0100] {
            let mut bytes = delta().encode().unwrap();
            *bytes.last_mut().unwrap() = bits;
            assert_eq!(
                FrameHeader::parse(&bytes),
                Err(HeaderError::ReservedFlags(bits))
            );
        }
    }

    #[test]
    fn the_start_of_a_stream_holds_its_kind_and_its_session() {
        let bytes = encode_prefix(crate::KIND_CONTROL, session());
        assert_eq!(bytes.len(), PREFIX_LEN);
        assert_eq!(bytes.first(), Some(&crate::KIND_CONTROL));
        assert_eq!(decode_prefix(&bytes), (crate::KIND_CONTROL, session()));
    }

    fn arbitrary_header() -> impl Strategy<Value = FrameHeader> {
        (
            any::<[u8; 16]>(),
            any::<u64>(),
            any::<u64>(),
            any::<u32>(),
            any::<u32>(),
            any::<u64>(),
            0_u8..8,
        )
            .prop_filter_map(
                "must be a valid header",
                |(session, a, b, chain, epoch, capture_time, flags)| {
                    let flags = Flags::from_bits(flags)?;
                    let (number, depends_on) = if flags.contains(Flags::KEY) {
                        (a, a)
                    } else {
                        (a.max(b), a.min(b))
                    };
                    let header = FrameHeader {
                        session: SessionId::from_bytes(session),
                        number,
                        depends_on,
                        chain,
                        epoch,
                        capture_time,
                        flags,
                    };
                    header.validate().ok().map(|()| header)
                },
            )
    }

    proptest! {
        #[test]
        fn a_valid_header_reads_back_as_it_was_written(header in arbitrary_header()) {
            let bytes = header.encode().unwrap();
            prop_assert_eq!(bytes.len(), FrameHeader::LEN);
            let (read, rest) = FrameHeader::parse(&bytes).unwrap();
            prop_assert_eq!(read, header);
            prop_assert!(rest.is_empty());
        }

        #[test]
        fn bytes_that_are_not_a_header_are_an_error_and_not_a_panic(
            bytes in proptest::collection::vec(any::<u8>(), 0..80)
        ) {
            if let Ok((header, rest)) = FrameHeader::parse(&bytes) {
                // Whatever is accepted is a header that writes back to the same bytes.
                let again = header.encode().unwrap();
                prop_assert_eq!(bytes.get(..FrameHeader::LEN), Some(&again[..]));
                prop_assert_eq!(rest.len(), bytes.len() - FrameHeader::LEN);
            }
        }

        // Random bytes start with the kind of a frame of video once in 256 cases and almost
        // never end in defined flags, so the property above rarely gets past the first byte.
        // This one starts with the kind, makes the flags mostly defined and the length mostly
        // whole, which puts the cut, the flag and the dependency checks, and the writing back of
        // what is accepted, under a property too.
        #[test]
        fn bytes_that_start_like_a_header_are_read_or_refused_and_never_a_panic(
            body in any::<[u8; 48]>(),
            flags in prop_oneof![any::<u8>(), 0_u8..8],
            tail in proptest::collection::vec(any::<u8>(), 0..8),
            len in prop_oneof![Just(FrameHeader::LEN), 0..=FrameHeader::LEN + 8],
        ) {
            let mut bytes = vec![crate::KIND_VIDEO];
            bytes.extend_from_slice(&body);
            bytes.push(flags);
            bytes.extend_from_slice(&tail);
            bytes.truncate(len);
            match FrameHeader::parse(&bytes) {
                Ok((header, rest)) => {
                    let again = header.encode().unwrap();
                    prop_assert_eq!(bytes.get(..FrameHeader::LEN), Some(&again[..]));
                    prop_assert_eq!(rest.len(), bytes.len() - FrameHeader::LEN);
                }
                Err(HeaderError::Truncated) => prop_assert!(bytes.len() < FrameHeader::LEN),
                Err(_) => prop_assert!(bytes.len() >= FrameHeader::LEN),
            }
        }
    }
}
