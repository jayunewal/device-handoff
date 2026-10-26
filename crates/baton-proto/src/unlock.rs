// SPDX-License-Identifier: Apache-2.0

//! Unlocking a locked screen from a paired device: the request for the key that a secret is
//! sealed to, and the byte strings of an unlock.
//!
//! The design is recorded in `docs/adr/0013-unlock-from-a-phone.md` and specified in section 9 of
//! `docs/protocol.md`. A device that can be unlocked (the *target*) asks a paired device that
//! unlocks with the face of its person (the *approver*) for the public key of a key pair whose
//! private half cannot leave the approver, seals a secret to it, and keeps the sealed form.
//! That is the one request in this module. What follows during a session travels on the control
//! streams and is in `baton-stream`; the byte strings of those messages are defined here so that
//! the encodings of all of them stay in one crate.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_bytes::{ByteArray, ByteBuf, Bytes};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    limits::{LimitError, MAX_SEALED_KEY_BYTES, MAX_SEALING_KEY_BYTES, check_non_empty_bytes},
    message::DeclineReason,
};

/// A request about unlocking.
///
/// It travels on a bidirectional stream like any other request, and only to a device that
/// advertised
/// [`capability::UNLOCK_APPROVE`](crate::message::capability::UNLOCK_APPROVE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnlockRequest {
    /// Ask the receiver for the public key that a secret is sealed to, so that only the receiver
    /// can open it, and only after its person's face is recognised.
    Key(KeyRequest),
}

/// The target asks the approver for its key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRequest {
    /// Whether the approver is to replace the key it made for the sender with a new one. The old
    /// key then opens nothing. Without it the approver gives the key it made before, and makes
    /// one if it has none. Left out when it is not asked.
    #[serde(default, skip_serializing_if = "is_default")]
    pub renew: bool,
}

/// The answer to an [`UnlockRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnlockReply {
    /// Answers [`UnlockRequest::Key`]: the key.
    Key(UnlockKey),
    /// Answers [`UnlockRequest::Key`]: the receiver will not give one. A device that takes no part
    /// answers every request this way, with [`Unsupported`](DeclineReason::Unsupported).
    Declined(DeclineReason),
}

/// How a secret is sealed to a public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sealing {
    /// The key is a point of P-256 in the uncompressed form of ANSI X9.63 (65 bytes, the first
    /// of them 0x04). A secret is sealed to it with ECIES: an ephemeral key, the X9.63 key
    /// derivation with SHA-256 and AES-GCM with a variable-size IV, which is the algorithm
    /// `eciesEncryptionStandardVariableIVX963SHA256AESGCM` of Security.framework. The sealed form
    /// is the 65 bytes of the ephemeral public key, the encrypted secret and the tag of 16 bytes.
    EciesP256,
    /// Any way this version does not know about. A target does not set up an unlock with it.
    #[serde(other)]
    Other,
}

/// The approver gives the key that a secret is sealed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnlockKey {
    /// How a secret is sealed to `public`.
    pub sealing: Sealing,
    /// The public key, in the form that `sealing` names.
    pub public: SealingKey,
}

impl UnlockReply {
    pub(crate) fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Key(key) => key.public.validate(),
            Self::Declined(_) => Ok(()),
        }
    }
}

/// For `skip_serializing_if`: a field that has its default (`false`) is left out of the
/// message, and a reader that finds it absent takes the same value.
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// The public key that a secret is sealed to. It is not a secret.
///
/// It is a byte string of the form that [`Sealing`] names, from 1 to
/// [`MAX_SEALING_KEY_BYTES`] bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct SealingKey(Vec<u8>);

impl SealingKey {
    /// Wraps the bytes of a public key.
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes of the key.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Checks the length against the limit of the protocol.
    pub fn validate(&self) -> Result<(), LimitError> {
        check_non_empty_bytes("sealing key", &self.0, MAX_SEALING_KEY_BYTES)
    }
}

impl fmt::Debug for SealingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SealingKey({} bytes)", self.0.len())
    }
}

impl Serialize for SealingKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Bytes::new(&self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SealingKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteBuf::deserialize(deserializer).map(|bytes| Self(bytes.into_vec()))
    }
}

/// The box key of an unlock, sealed to the key of the approver. Only the approver can open it.
///
/// It is a byte string of the form that [`Sealing`] names, from 1 to [`MAX_SEALED_KEY_BYTES`]
/// bytes. It is not printed, since it is the one thing the approver's face guards, although it
/// is useless without the private key.
#[derive(Clone, PartialEq, Eq)]
pub struct SealedKey(Vec<u8>);

impl SealedKey {
    /// Wraps the bytes of a sealed key.
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The bytes of the sealed key.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Checks the length against the limit of the protocol.
    pub fn validate(&self) -> Result<(), LimitError> {
        check_non_empty_bytes("sealed key", &self.0, MAX_SEALED_KEY_BYTES)
    }
}

impl fmt::Debug for SealedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SealedKey({} bytes)", self.0.len())
    }
}

impl Serialize for SealedKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Bytes::new(&self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SealedKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteBuf::deserialize(deserializer).map(|bytes| Self(bytes.into_vec()))
    }
}

/// A question that the target asks once, and that an answer has to repeat.
///
/// It is 16 random bytes made for one `UnlockSealed` and used for that attempt alone. It ties an
/// answer to one question, so that a late or repeated answer does nothing. It is not a secret and
/// protects nothing by itself, and it is not printed.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Challenge([u8; Self::LEN]);

impl Challenge {
    /// Length of a challenge in bytes.
    pub const LEN: usize = 16;

    /// Wraps raw bytes, which the caller must have drawn from a cryptographic random source.
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The raw bytes.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Debug for Challenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Challenge(..)")
    }
}

impl Serialize for Challenge {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        Bytes::new(&self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Challenge {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

/// The key of the box in which the target keeps the password of the screen.
///
/// The target makes it at random when the unlock is set up, encrypts the password with it, and
/// seals it to the key of the approver. The approver gives it back, after its person's face is
/// recognised, in an `UnlockOpened`. It is a bearer secret: whoever holds it and the box can open
/// the password. The target therefore uses a key once: before it types, it puts the password in a
/// new box under a new key, sealed to the same key of the approver, so that a copy of a key that
/// was used opens nothing that the target holds afterwards. It is wiped from memory when dropped
/// and never printed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct BoxKey([u8; Self::LEN]);

impl BoxKey {
    /// Length of the key in bytes.
    pub const LEN: usize = 32;

    /// Wraps secret bytes, which the caller must have drawn from a cryptographic random source.
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The secret bytes. Handle with care.
    pub const fn expose(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl PartialEq for BoxKey {
    fn eq(&self, other: &Self) -> bool {
        // `blake3::Hash` compares in constant time.
        blake3::Hash::from_bytes(self.0) == blake3::Hash::from_bytes(other.0)
    }
}

impl Eq for BoxKey {}

impl fmt::Debug for BoxKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BoxKey(..)")
    }
}

impl Serialize for BoxKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Borrowed, so that no copy of the key is made on the way.
        Bytes::new(&self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BoxKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

#[cfg(test)]
mod tests {
    use ciborium::cbor;

    use super::*;
    use crate::{
        frame,
        limits::{MAX_SEALED_KEY_BYTES, MAX_SEALING_KEY_BYTES},
        message::{Request, Response},
    };

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes.iter().fold(String::new(), |mut out, b| {
            write!(out, "{b:02x}").unwrap();
            out
        })
    }

    fn public() -> SealingKey {
        let mut point = vec![0x04];
        point.extend((0..64u8).map(|i| 0xc0 ^ i));
        SealingKey::new(point)
    }

    #[test]
    fn the_messages_are_read_back_as_they_were_written() {
        for request in [
            UnlockRequest::Key(KeyRequest { renew: false }),
            UnlockRequest::Key(KeyRequest { renew: true }),
        ] {
            let sent = Request::Unlock(request);
            let read = frame::decode::<Request>(&frame::encode(&sent).unwrap()).unwrap();
            assert_eq!(read, sent);
        }
        for reply in [
            UnlockReply::Key(UnlockKey {
                sealing: Sealing::EciesP256,
                public: public(),
            }),
            UnlockReply::Declined(DeclineReason::Blocked),
            UnlockReply::Declined(DeclineReason::Unsupported),
        ] {
            let sent = Response::Unlock(reply);
            let read = frame::decode::<Response>(&frame::encode(&sent).unwrap()).unwrap();
            assert_eq!(read, sent);
        }
    }

    #[test]
    fn the_vectors_of_protocol_md_are_what_the_code_writes() {
        // The exact bytes are the protocol (docs/protocol.md section 7). If this fails, the
        // wire format changed.
        let plain = frame::encode(&Request::Unlock(UnlockRequest::Key(KeyRequest {
            renew: false,
        })))
        .unwrap();
        assert_eq!(
            hex(&plain),
            concat!(
                "0000000e",         // the frame: 14 bytes follow
                "a166556e6c6f636b", // {"Unlock":
                "a1634b6579",       //   {"Key":
                "a0",               //     {}  (renew is false and left out)
            )
        );

        // With `renew` the map holds one key, which is seven bytes more.
        let renewed = frame::encode(&Request::Unlock(UnlockRequest::Key(KeyRequest {
            renew: true,
        })))
        .unwrap();
        assert_eq!(
            hex(&renewed),
            concat!(
                "00000015",         // the frame: 21 bytes follow
                "a166556e6c6f636b", // {"Unlock":
                "a1634b6579",       //   {"Key":
                "a1",               //     map of 1
                "65",
                "72656e6577", //     "renew":
                "f5",         //       true
            )
        );

        let key = frame::encode(&Response::Unlock(UnlockReply::Key(UnlockKey {
            sealing: Sealing::EciesP256,
            public: public(),
        })))
        .unwrap();
        assert_eq!(
            hex(&key),
            concat!(
                "0000006a",         // the frame: 106 bytes follow
                "a166556e6c6f636b", // {"Unlock":
                "a1634b6579",       //   {"Key":
                "a2",               //     map of 2
                "67",
                "7365616c696e67", //     "sealing":
                "69",
                "456369657350323536", //       "EciesP256",
                "66",
                "7075626c6963",                     //     "public":
                "5841",                             //       65 bytes:
                "04",                               //         the point is uncompressed
                "c0c1c2c3c4c5c6c7c8c9cacbcccdcecf", //         then 64 bytes
                "d0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
                "e0e1e2e3e4e5e6e7e8e9eaebecedeeef",
                "f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff",
            )
        );

        // A target that is declined is told why, in the words of any other request.
        let declined = frame::encode(&Response::Unlock(UnlockReply::Declined(
            DeclineReason::Blocked,
        )))
        .unwrap();
        assert_eq!(
            hex(&declined),
            concat!(
                "0000001a",             // the frame: 26 bytes follow
                "a166556e6c6f636b",     // {"Unlock":
                "a1684465636c696e6564", //   {"Declined":
                "67426c6f636b6564",     //     "Blocked"
            )
        );
    }

    #[test]
    fn the_limits_of_an_unlock_are_the_ones_that_protocol_md_gives() {
        // Section 6 of the protocol. A device that lowers one of them ends the sessions of the
        // devices that keep to the document.
        assert_eq!(MAX_SEALING_KEY_BYTES, 256);
        assert_eq!(MAX_SEALED_KEY_BYTES, 1024);
    }

    #[test]
    fn the_key_of_the_approver_is_bounded() {
        let reply = |bytes: Vec<u8>| {
            Response::Unlock(UnlockReply::Key(UnlockKey {
                sealing: Sealing::EciesP256,
                public: SealingKey::new(bytes),
            }))
        };
        assert!(frame::encode(&reply(vec![4; MAX_SEALING_KEY_BYTES])).is_ok());
        assert!(frame::encode(&reply(vec![4; MAX_SEALING_KEY_BYTES + 1])).is_err());
        assert!(frame::encode(&reply(Vec::new())).is_err());
    }

    #[test]
    fn a_sealed_key_is_bounded() {
        assert!(
            SealedKey::new(vec![1; MAX_SEALED_KEY_BYTES])
                .validate()
                .is_ok()
        );
        assert_eq!(
            SealedKey::new(vec![1; MAX_SEALED_KEY_BYTES + 1]).validate(),
            Err(LimitError::TooLong {
                field: "sealed key",
                len: MAX_SEALED_KEY_BYTES + 1,
                max: MAX_SEALED_KEY_BYTES
            })
        );
        assert_eq!(
            SealedKey::new(Vec::new()).validate(),
            Err(LimitError::Empty {
                field: "sealed key"
            })
        );
    }

    #[test]
    fn a_way_of_sealing_that_is_not_known_is_read_as_other() {
        // {"Unlock": {"Key": {"sealing": "Lattice", "public": h'01'}}}
        let mut body = Vec::new();
        ciborium::into_writer(
            &cbor!({
                "Unlock" => { "Key" => { "sealing" => "Lattice", "public" => ciborium::Value::Bytes(vec![1]) } }
            })
            .unwrap(),
            &mut body,
        )
        .unwrap();
        let read = frame::decode_payload::<Response>(&body).unwrap();
        assert_eq!(
            read,
            Response::Unlock(UnlockReply::Key(UnlockKey {
                sealing: Sealing::Other,
                public: SealingKey::new(vec![1]),
            }))
        );
    }

    #[test]
    fn a_challenge_and_a_box_key_have_exactly_their_length() {
        let bytes = |len: usize| {
            let mut out = Vec::new();
            ciborium::into_writer(&ByteBuf::from(vec![7; len]), &mut out).unwrap();
            out
        };
        for len in [0, 15, 17, 32] {
            assert!(
                ciborium::from_reader::<Challenge, _>(bytes(len).as_slice()).is_err(),
                "{len}"
            );
        }
        assert!(ciborium::from_reader::<Challenge, _>(bytes(16).as_slice()).is_ok());
        for len in [0, 31, 33, 64] {
            assert!(
                ciborium::from_reader::<BoxKey, _>(bytes(len).as_slice()).is_err(),
                "{len}"
            );
        }
        assert!(ciborium::from_reader::<BoxKey, _>(bytes(32).as_slice()).is_ok());
    }

    #[test]
    fn the_secrets_are_not_printed() {
        let key = BoxKey::from_bytes([0x5a; BoxKey::LEN]);
        assert_eq!(format!("{key:?}"), "BoxKey(..)");
        assert_eq!(
            format!("{:?}", Challenge::from_bytes([0x5a; Challenge::LEN])),
            "Challenge(..)"
        );
        let sealed = SealedKey::new(vec![0x5a; 113]);
        assert_eq!(format!("{sealed:?}"), "SealedKey(113 bytes)");
        assert_eq!(format!("{:?}", public()), "SealingKey(65 bytes)");
    }

    #[test]
    fn a_box_key_is_wiped_and_compared_whole() {
        let key = BoxKey::from_bytes([0x5a; BoxKey::LEN]);
        let mut copy = key.clone();
        assert_eq!(copy, key);
        let mut other = [0x5a; BoxKey::LEN];
        other[BoxKey::LEN - 1] ^= 1;
        assert_ne!(BoxKey::from_bytes(other), key);
        copy.zeroize();
        assert_eq!(copy.expose(), &[0; BoxKey::LEN]);
        assert_eq!(
            key.expose(),
            &[0x5a; BoxKey::LEN],
            "the original is its own"
        );
    }

    #[test]
    fn a_box_key_is_written_as_a_byte_string_of_its_own_length() {
        let key = BoxKey::from_bytes(std::array::from_fn(|i| u8::try_from(i).unwrap()));
        let mut out = Vec::new();
        ciborium::into_writer(&key, &mut out).unwrap();
        assert_eq!(out.get(..2), Some(&[0x58, 0x20][..]), "a byte string of 32");
        assert_eq!(out.len(), 34);
        let back: BoxKey = ciborium::from_reader(out.as_slice()).unwrap();
        assert_eq!(back, key);
    }
}
