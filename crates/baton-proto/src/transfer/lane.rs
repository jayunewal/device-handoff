// SPDX-License-Identifier: Apache-2.0

//! Lanes: the unidirectional streams that carry the data of a transfer.
//!
//! A lane starts with a [`LaneHeader`] and then holds frames, each of which starts with a tag
//! byte and a fixed-size head ([`FrameHead`]). Some frames are followed by bytes whose number
//! the head or the list of files gives. This module reads and writes the fixed-size parts and
//! knows nothing about streams: the transport reads as many bytes as [`LaneHeader::LEN`] or
//! [`FrameHead::rest_len`] says, and hands them over.
//!
//! ```text
//! lane   = header frame*
//! header = kind u8 (1) | transfer id [16] | lane u8
//! frame  = tag u8 | head | bytes
//!
//! tag 1  list    head: none                      bytes: a frame of section 2 holding a ListChunk
//! tag 2  span    head: file u32 | span u32       bytes: as many as the list says for that span
//! tag 3  end     head: file u32 | hash [32]      bytes: none
//! tag 4  skip    head: file u32 | reason u8      bytes: none
//! ```
//!
//! Numbers are big-endian, as everywhere in the protocol. A `file` is the position of an entry
//! in the list (folders count), and a `span` is a number as in [`hash`](super::hash).

use blake3::Hash;

use crate::{id::TransferId, limits::MAX_LANES};

/// The first byte of a unidirectional stream says what the stream carries. This is the only
/// kind there is: a lane of a transfer.
pub const KIND_LANE: u8 = 1;

/// Tag of a frame that carries a piece of the list of files.
pub const TAG_LIST: u8 = 1;
/// Tag of a frame that carries a span of a file.
pub const TAG_SPAN: u8 = 2;
/// Tag of a frame that says a file has been sent in full, and gives its hash.
pub const TAG_END: u8 = 3;
/// Tag of a frame that says a file will not be sent.
pub const TAG_SKIP: u8 = 4;

/// Why a lane or a frame head was not understood.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LaneError {
    /// The stream is of a kind this version does not know.
    #[error("a stream of kind {0} is not a lane")]
    BadKind(u8),
    /// The lane number is more than a transfer may use.
    #[error("lane {0} is more than a transfer may use")]
    BadLane(u8),
    /// The tag is not that of a frame this version knows.
    #[error("{0} is not the tag of a frame")]
    BadTag(u8),
    /// The head has a different number of bytes than its tag needs.
    #[error("the head of frame {tag} has {got} bytes, not {expected}")]
    WrongLength {
        /// The tag of the frame.
        tag: u8,
        /// How many bytes its head has.
        expected: usize,
        /// How many were given.
        got: usize,
    },
}

/// The start of a lane: which transfer it belongs to, and which lane of it this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneHeader {
    /// The transfer whose offer was accepted.
    pub transfer: TransferId,
    /// Number of the lane, below the number of lanes the receiver allowed. Lane 0 carries the
    /// list of files.
    pub lane: u8,
}

impl LaneHeader {
    /// Length of the header in bytes.
    pub const LEN: usize = 1 + TransferId::LEN + 1;

    /// The header as it is written at the start of the lane.
    pub fn to_bytes(&self) -> [u8; Self::LEN] {
        let mut out = [0; Self::LEN];
        let [kind, transfer @ .., lane] = &mut out;
        *kind = KIND_LANE;
        *transfer = *self.transfer.as_bytes();
        *lane = self.lane;
        out
    }

    /// Reads the header from the first bytes of a stream.
    pub fn from_bytes(bytes: &[u8; Self::LEN]) -> Result<Self, LaneError> {
        let [kind, transfer @ .., lane] = *bytes;
        if kind != KIND_LANE {
            return Err(LaneError::BadKind(kind));
        }
        if lane >= MAX_LANES {
            return Err(LaneError::BadLane(lane));
        }
        Ok(Self {
            transfer: TransferId::from_bytes(transfer),
            lane,
        })
    }
}

/// Why the sender will not send a file. Sent as one byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The file could not be read.
    Unreadable,
    /// The file changed while it was being read, so what was read is not one version of it.
    Changed,
    /// The file is gone.
    Missing,
    /// Any reason this version does not know about.
    Other,
}

impl SkipReason {
    /// The byte that stands for the reason on the wire.
    pub const fn code(self) -> u8 {
        match self {
            Self::Other => 0,
            Self::Unreadable => 1,
            Self::Changed => 2,
            Self::Missing => 3,
        }
    }

    /// The reason a byte stands for. A code this version does not know is [`Other`](Self::Other).
    pub const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Unreadable,
            2 => Self::Changed,
            3 => Self::Missing,
            _ => Self::Other,
        }
    }
}

/// The fixed-size start of a frame on a lane, after its tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameHead {
    /// A piece of the list of files. A frame as in section 2, holding a
    /// [`ListChunk`](super::list::ListChunk), follows. Only lane 0 carries these.
    List,
    /// Span `span` of file `file`. Its bytes follow, as many as
    /// [`span_size`](super::hash::span_size) gives for the size of the file in the list.
    Span {
        /// Position of the file in the list.
        file: u32,
        /// Number of the span in the file.
        span: u32,
    },
    /// Every span of file `file` has been sent, and the BLAKE3 hash of its bytes is `hash`.
    End {
        /// Position of the file in the list.
        file: u32,
        /// The hash of the whole file.
        hash: Hash,
    },
    /// File `file` will not be sent, and nothing more is sent for it.
    Skip {
        /// Position of the file in the list.
        file: u32,
        /// Why not.
        reason: SkipReason,
    },
}

impl FrameHead {
    /// The most bytes a head can have after its tag.
    pub const MAX_REST_LEN: usize = 4 + blake3::OUT_LEN;

    /// The tag that starts a frame with this head.
    pub const fn tag(&self) -> u8 {
        match self {
            Self::List => TAG_LIST,
            Self::Span { .. } => TAG_SPAN,
            Self::End { .. } => TAG_END,
            Self::Skip { .. } => TAG_SKIP,
        }
    }

    /// How many bytes of head follow a tag, to be read before [`decode`](Self::decode).
    pub const fn rest_len(tag: u8) -> Result<usize, LaneError> {
        match tag {
            TAG_LIST => Ok(0),
            TAG_SPAN => Ok(8),
            TAG_END => Ok(4 + blake3::OUT_LEN),
            TAG_SKIP => Ok(5),
            other => Err(LaneError::BadTag(other)),
        }
    }

    /// Reads a head from the bytes after its tag.
    pub fn decode(tag: u8, rest: &[u8]) -> Result<Self, LaneError> {
        let expected = Self::rest_len(tag)?;
        let wrong = LaneError::WrongLength {
            tag,
            expected,
            got: rest.len(),
        };
        if rest.len() != expected {
            return Err(wrong);
        }
        match tag {
            TAG_LIST => Ok(Self::List),
            TAG_SPAN => {
                let (file, rest) = take_u32(rest).ok_or(wrong)?;
                let (span, _) = take_u32(rest).ok_or(wrong)?;
                Ok(Self::Span { file, span })
            }
            TAG_END => {
                let (file, rest) = take_u32(rest).ok_or(wrong)?;
                let hash = <[u8; blake3::OUT_LEN]>::try_from(rest).map_err(|_| wrong)?;
                Ok(Self::End {
                    file,
                    hash: Hash::from_bytes(hash),
                })
            }
            TAG_SKIP => {
                let (file, rest) = take_u32(rest).ok_or(wrong)?;
                let [reason] = rest else {
                    return Err(wrong);
                };
                Ok(Self::Skip {
                    file,
                    reason: SkipReason::from_code(*reason),
                })
            }
            other => Err(LaneError::BadTag(other)),
        }
    }

    /// Appends the tag and the head to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.tag());
        match self {
            Self::List => {}
            Self::Span { file, span } => {
                out.extend_from_slice(&file.to_be_bytes());
                out.extend_from_slice(&span.to_be_bytes());
            }
            Self::End { file, hash } => {
                out.extend_from_slice(&file.to_be_bytes());
                out.extend_from_slice(hash.as_bytes());
            }
            Self::Skip { file, reason } => {
                out.extend_from_slice(&file.to_be_bytes());
                out.push(reason.code());
            }
        }
    }
}

fn take_u32(bytes: &[u8]) -> Option<(u32, &[u8])> {
    let (number, rest) = bytes.split_first_chunk::<4>()?;
    Some((u32::from_be_bytes(*number), rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        data_encoding::HEXLOWER.encode(bytes)
    }

    fn encoded(head: &FrameHead) -> Vec<u8> {
        let mut out = Vec::new();
        head.encode_into(&mut out);
        out
    }

    #[test]
    fn a_lane_header_has_a_known_wire_encoding() {
        let header = LaneHeader {
            transfer: TransferId::from_bytes(std::array::from_fn(|i| {
                u8::try_from(i).unwrap() + 0xa0
            })),
            lane: 3,
        };
        let bytes = header.to_bytes();
        assert_eq!(hex(&bytes), "01a0a1a2a3a4a5a6a7a8a9aaabacadaeaf03");
        assert_eq!(LaneHeader::from_bytes(&bytes), Ok(header));
    }

    #[test]
    fn a_lane_header_of_another_kind_or_beyond_the_lanes_is_refused() {
        let good = LaneHeader {
            transfer: TransferId::from_bytes([7; 16]),
            lane: MAX_LANES - 1,
        }
        .to_bytes();
        assert!(LaneHeader::from_bytes(&good).is_ok());

        for kind in [0, 2, 0x80, 0xff] {
            let mut other = good;
            other[0] = kind;
            assert_eq!(
                LaneHeader::from_bytes(&other),
                Err(LaneError::BadKind(kind))
            );
        }
        for lane in [MAX_LANES, MAX_LANES + 1, 255] {
            let mut other = good;
            other[LaneHeader::LEN - 1] = lane;
            assert_eq!(
                LaneHeader::from_bytes(&other),
                Err(LaneError::BadLane(lane))
            );
        }
    }

    #[test]
    fn frame_heads_have_known_wire_encodings() {
        let hash = Hash::from_bytes(std::array::from_fn(|i| u8::try_from(i).unwrap()));
        for (head, expected) in [
            (FrameHead::List, "01"),
            (
                FrameHead::Span {
                    file: 0x0102_0304,
                    span: 0x0a0b_0c0d,
                },
                "02010203040a0b0c0d",
            ),
            (
                FrameHead::End { file: 5, hash },
                "0300000005000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            ),
            (
                FrameHead::Skip {
                    file: 6,
                    reason: SkipReason::Changed,
                },
                "040000000602",
            ),
        ] {
            let bytes = encoded(&head);
            assert_eq!(hex(&bytes), expected, "{head:?}");
            let (tag, rest) = bytes.split_first().unwrap();
            assert_eq!(FrameHead::rest_len(*tag), Ok(rest.len()));
            assert_eq!(FrameHead::decode(*tag, rest), Ok(head));
            assert!(rest.len() <= FrameHead::MAX_REST_LEN);
        }
    }

    #[test]
    fn an_unknown_tag_or_a_head_of_the_wrong_length_is_refused() {
        for tag in [0, 5, 6, 0x80, 0xff] {
            assert_eq!(FrameHead::rest_len(tag), Err(LaneError::BadTag(tag)));
            assert_eq!(FrameHead::decode(tag, &[]), Err(LaneError::BadTag(tag)));
        }
        for (tag, expected) in [(TAG_LIST, 0), (TAG_SPAN, 8), (TAG_END, 36), (TAG_SKIP, 5)] {
            for got in [0, 1, 4, 35, 36, 37, 100] {
                if got == expected {
                    continue;
                }
                assert_eq!(
                    FrameHead::decode(tag, &vec![0; got]),
                    Err(LaneError::WrongLength { tag, expected, got }),
                    "tag {tag}, {got} bytes"
                );
            }
        }
    }

    #[test]
    fn a_reason_this_version_does_not_know_is_other() {
        assert_eq!(
            FrameHead::decode(TAG_SKIP, &[0, 0, 0, 1, 77]),
            Ok(FrameHead::Skip {
                file: 1,
                reason: SkipReason::Other
            })
        );
        for reason in [
            SkipReason::Unreadable,
            SkipReason::Changed,
            SkipReason::Missing,
            SkipReason::Other,
        ] {
            assert_eq!(SkipReason::from_code(reason.code()), reason);
        }
    }
}
