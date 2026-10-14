// SPDX-License-Identifier: Apache-2.0

//! Framing and encoding.
//!
//! A frame is a 4-byte big-endian payload length followed by the payload, which
//! is one message in CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)).
//! This module does no I/O: a transport reads the header, calls [`payload_len`],
//! reads that many bytes and hands them to [`decode_payload`].

use crate::{limits::LimitError, message::Message};

/// Size of the length prefix.
pub const HEADER_LEN: usize = 4;

/// How deeply nested a CBOR item may be. Our messages are shallow; this only
/// bounds the work spent on hostile input.
const RECURSION_LIMIT: usize = 16;

/// Why a frame could not be produced or understood.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The declared or actual payload is larger than this message type allows.
    #[error("frame of {len} bytes is larger than the limit of {max}")]
    TooLarge {
        /// Declared or actual payload size.
        len: usize,
        /// Largest allowed payload size.
        max: usize,
    },
    /// The payload is empty. Every message encodes to at least one byte.
    #[error("frame has no payload")]
    Empty,
    /// The buffer ends before the frame does.
    #[error("frame is cut short")]
    Truncated,
    /// The payload is not exactly one message.
    #[error("{0} unexpected bytes after the message")]
    TrailingBytes(usize),
    /// The message breaks a protocol limit.
    #[error(transparent)]
    Limit(#[from] LimitError),
    /// The message could not be serialized.
    #[error("could not encode message: {0}")]
    Encode(String),
    /// The payload is not a valid message of the expected type.
    #[error("could not decode message: {0}")]
    Decode(String),
}

/// Validates `message` and encodes it as a complete frame, header included.
pub fn encode<M: Message>(message: &M) -> Result<Vec<u8>, FrameError> {
    message.validate()?;
    let mut frame = vec![0; HEADER_LEN];
    ciborium::into_writer(message, &mut frame).map_err(|e| FrameError::Encode(e.to_string()))?;
    let len = frame.len() - HEADER_LEN;
    if len > M::MAX_FRAME_LEN {
        return Err(FrameError::TooLarge {
            len,
            max: M::MAX_FRAME_LEN,
        });
    }
    let header = u32::try_from(len).map_err(|_| FrameError::TooLarge {
        len,
        max: M::MAX_FRAME_LEN,
    })?;
    frame[..HEADER_LEN].copy_from_slice(&header.to_be_bytes());
    Ok(frame)
}

/// Reads a frame header and returns how many payload bytes follow.
///
/// Call this before allocating a buffer for the payload.
pub fn payload_len<M: Message>(header: [u8; HEADER_LEN]) -> Result<usize, FrameError> {
    let len = usize::try_from(u32::from_be_bytes(header)).unwrap_or(usize::MAX);
    if len == 0 {
        return Err(FrameError::Empty);
    }
    if len > M::MAX_FRAME_LEN {
        return Err(FrameError::TooLarge {
            len,
            max: M::MAX_FRAME_LEN,
        });
    }
    Ok(len)
}

/// Decodes and validates one message from a payload (a frame without its header).
pub fn decode_payload<M: Message>(payload: &[u8]) -> Result<M, FrameError> {
    if payload.is_empty() {
        return Err(FrameError::Empty);
    }
    if payload.len() > M::MAX_FRAME_LEN {
        return Err(FrameError::TooLarge {
            len: payload.len(),
            max: M::MAX_FRAME_LEN,
        });
    }
    let mut rest = payload;
    let message: M = ciborium::de::from_reader_with_recursion_limit(&mut rest, RECURSION_LIMIT)
        .map_err(|e| FrameError::Decode(e.to_string()))?;
    if !rest.is_empty() {
        return Err(FrameError::TrailingBytes(rest.len()));
    }
    message.validate()?;
    Ok(message)
}

/// Decodes one complete frame, header included, from a buffer.
///
/// Meant for tests and in-memory use. A transport that reads from a stream
/// should use [`payload_len`] and [`decode_payload`] instead.
pub fn decode<M: Message>(frame: &[u8]) -> Result<M, FrameError> {
    let (header, payload) = frame
        .split_first_chunk::<HEADER_LEN>()
        .ok_or(FrameError::Truncated)?;
    let len = payload_len::<M>(*header)?;
    match payload.len().cmp(&len) {
        std::cmp::Ordering::Less => Err(FrameError::Truncated),
        std::cmp::Ordering::Greater => Err(FrameError::TrailingBytes(payload.len() - len)),
        std::cmp::Ordering::Equal => decode_payload(payload),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Ping, Request};

    #[test]
    fn ping_has_a_known_wire_encoding() {
        // The exact bytes are the protocol. If this fails, the wire format changed.
        // {"Ping": {"nonce": 1}} as CBOR, behind a 4-byte length of 14.
        let frame = encode(&Request::Ping(Ping { nonce: 1 })).unwrap();
        assert_eq!(hex(&frame), "0000000ea16450696e67a1656e6f6e636501");
    }

    fn hex(bytes: &[u8]) -> String {
        data_encoding::HEXLOWER.encode(bytes)
    }

    #[test]
    fn round_trip() {
        let request = Request::Ping(Ping { nonce: u64::MAX });
        let frame = encode(&request).unwrap();
        assert_eq!(decode::<Request>(&frame).unwrap(), request);
    }

    #[test]
    fn header_limits_are_enforced_before_reading_the_payload() {
        let max = u32::try_from(Request::MAX_FRAME_LEN).unwrap();
        assert_eq!(
            payload_len::<Request>(max.to_be_bytes()),
            Ok(Request::MAX_FRAME_LEN)
        );
        assert_eq!(
            payload_len::<Request>((max + 1).to_be_bytes()),
            Err(FrameError::TooLarge {
                len: Request::MAX_FRAME_LEN + 1,
                max: Request::MAX_FRAME_LEN
            })
        );
        assert_eq!(payload_len::<Request>([0; 4]), Err(FrameError::Empty));
        assert!(matches!(
            payload_len::<Request>([0xff; 4]),
            Err(FrameError::TooLarge { .. })
        ));
    }

    #[test]
    fn truncated_and_padded_frames_are_rejected() {
        let frame = encode(&Request::Ping(Ping { nonce: 5 })).unwrap();
        assert_eq!(decode::<Request>(&frame[..2]), Err(FrameError::Truncated));
        assert_eq!(
            decode::<Request>(&frame[..frame.len() - 1]),
            Err(FrameError::Truncated)
        );
        let mut padded = frame.clone();
        padded.push(0);
        assert_eq!(
            decode::<Request>(&padded),
            Err(FrameError::TrailingBytes(1))
        );
    }

    #[test]
    fn two_messages_in_one_payload_are_rejected() {
        let one = encode(&Request::Ping(Ping { nonce: 1 })).unwrap();
        let payload = [&one[HEADER_LEN..], &one[HEADER_LEN..]].concat();
        assert!(matches!(
            decode_payload::<Request>(&payload),
            Err(FrameError::TrailingBytes(_))
        ));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for payload in [
            &[0xff][..],
            &[0x00],
            &[0xa1],
            &[0x9f, 0x9f, 0x9f],
            b"not cbor at all",
        ] {
            assert!(decode_payload::<Request>(payload).is_err(), "{payload:?}");
        }
    }

    #[test]
    fn deeply_nested_input_is_rejected() {
        // 10,000 nested single-element arrays.
        let payload = vec![0x81; 10_000];
        assert!(decode_payload::<Request>(&payload).is_err());
    }

    #[test]
    fn declared_huge_lengths_do_not_allocate() {
        // A text string that claims to be 4 GiB long, with no bytes behind it.
        let payload = [0x7b, 0, 0, 0, 1, 0, 0, 0, 0];
        assert!(decode_payload::<Request>(&payload).is_err());
    }

    #[test]
    fn messages_over_their_limit_are_not_encoded() {
        use crate::id::ItemId;
        use crate::message::{ClipboardText, Handoff, Item};
        let too_big = Request::Handoff(Handoff {
            id: ItemId::from_bytes([0; 16]),
            item: Item::ClipboardText(ClipboardText {
                text: "x".repeat(crate::limits::MAX_CLIPBOARD_TEXT_BYTES + 1),
            }),
        });
        assert!(matches!(encode(&too_big), Err(FrameError::Limit(_))));
    }
}
