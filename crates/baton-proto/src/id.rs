// SPDX-License-Identifier: Apache-2.0

//! Identifiers: devices, handoff items, transfers and streaming sessions.

use std::{fmt, str::FromStr};

use data_encoding::{BASE32_NOPAD, HEXLOWER, HEXLOWER_PERMISSIVE};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_bytes::ByteArray;

/// Domain separation for [`DeviceId::fingerprint`]. Changing it changes every fingerprint.
const FINGERPRINT_CONTEXT: &str = "baton 2026-10-14 device fingerprint v1";

/// Number of base32 characters shown in a fingerprint (5 bits each).
const FINGERPRINT_CHARS: usize = 20;

/// Bytes of the digest needed to fill [`FINGERPRINT_CHARS`] characters: 100 bits, rounded up.
const FINGERPRINT_BYTES: usize = 13;

/// A device's identity: its Ed25519 public key.
///
/// Devices dial each other by this key, and the transport handshake proves that
/// the peer holds the matching private key. It is public information.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId([u8; Self::LEN]);

impl DeviceId {
    /// Length of an identity in bytes.
    pub const LEN: usize = 32;

    /// Wraps the raw public key bytes.
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The raw public key bytes.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }

    /// The first five bytes in hex, for logs and compact listings.
    ///
    /// Forty bits, taken straight from the key: enough to tell a few devices apart in a
    /// log, and nothing a person should rely on. Use [`fingerprint`](Self::fingerprint)
    /// wherever someone has to check that a device is the one they mean.
    pub fn short(&self) -> String {
        let [b0, b1, b2, b3, b4, ..] = self.0;
        HEXLOWER.encode(&[b0, b1, b2, b3, b4])
    }

    /// A short string two people can compare out loud or on two screens.
    ///
    /// It is 100 bits of a hash of the identity, so an attacker cannot pick a key whose
    /// fingerprint merely starts like another one.
    pub fn fingerprint(&self) -> Fingerprint {
        let digest = blake3::derive_key(FINGERPRINT_CONTEXT, &self.0);
        let bytes: Vec<u8> = digest.iter().copied().take(FINGERPRINT_BYTES).collect();
        let text = BASE32_NOPAD.encode(&bytes).to_ascii_lowercase();
        Fingerprint(text.chars().take(FINGERPRINT_CHARS).collect())
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({}…)", self.short())
    }
}

/// The text is not a valid [`DeviceId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a device id is {} hexadecimal characters", DeviceId::LEN * 2)]
pub struct ParseDeviceIdError;

impl FromStr for DeviceId {
    type Err = ParseDeviceIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = HEXLOWER_PERMISSIVE
            .decode(s.as_bytes())
            .map_err(|_| ParseDeviceIdError)?;
        let bytes: [u8; Self::LEN] = bytes.try_into().map_err(|_| ParseDeviceIdError)?;
        Ok(Self(bytes))
    }
}

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ByteArray::new(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

/// A human-comparable digest of a [`DeviceId`], shown as `abcd-efgh-ijkl-mnop-qrst`.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// The first group of four characters, for places where space is tight.
    ///
    /// Twenty bits, which anyone can match by trying a million keys. It is for telling
    /// entries apart in a compact list that also gives a way to see the whole
    /// fingerprint. **Never use it where a person verifies a device**, such as a pairing
    /// prompt: show the full [`Display`](fmt::Display) form there.
    pub fn short(&self) -> &str {
        self.0.get(..4).unwrap_or_default()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, group) in self.0.as_bytes().chunks(4).enumerate() {
            if i > 0 {
                f.write_str("-")?;
            }
            // The fingerprint is ASCII by construction.
            f.write_str(std::str::from_utf8(group).map_err(|_| fmt::Error)?)?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

/// Identifies one handoff item, so a retry is not delivered twice.
///
/// Chosen at random by the sender.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemId([u8; Self::LEN]);

impl ItemId {
    /// Length of an item id in bytes.
    pub const LEN: usize = 16;

    /// Wraps raw bytes, which the caller must have drawn from a random source.
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The raw bytes.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ItemId({self})")
    }
}

impl Serialize for ItemId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ByteArray::new(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ItemId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

/// Identifies one transfer of files, so a retry is recognised and its lanes find their offer.
///
/// Chosen at random by the sender. It is a type of its own, and not an [`ItemId`], so that
/// the two can never be mixed up in a table that is keyed by one of them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferId([u8; Self::LEN]);

impl TransferId {
    /// Length of a transfer id in bytes.
    pub const LEN: usize = 16;

    /// Wraps raw bytes, which the caller must have drawn from a random source.
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The raw bytes.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Display for TransferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for TransferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransferId({self})")
    }
}

impl Serialize for TransferId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ByteArray::new(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TransferId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

/// Identifies one streaming session between a source and a viewer.
///
/// Chosen at random by the device that offers the session, and carried by every stream that
/// belongs to it so that a stream for a session that is not running can be refused.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId([u8; Self::LEN]);

impl SessionId {
    /// Length of a session id in bytes.
    pub const LEN: usize = 16;

    /// Wraps raw bytes, which the caller must have drawn from a random source.
    pub const fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// The raw bytes.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&HEXLOWER.encode(&self.0))
    }
}

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({self})")
    }
}

impl Serialize for SessionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ByteArray::new(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SessionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> DeviceId {
        DeviceId::from_bytes([byte; 32])
    }

    #[test]
    fn a_transfer_id_prints_as_hex_and_is_not_an_item_id() {
        let id = TransferId::from_bytes(std::array::from_fn(|i| u8::try_from(i).unwrap()));
        assert_eq!(id.to_string(), "000102030405060708090a0b0c0d0e0f");
        assert_eq!(format!("{id:?}"), format!("TransferId({id})"));
        assert_eq!(id.as_bytes().len(), TransferId::LEN);
    }

    #[test]
    fn device_id_round_trips_through_text() {
        let original = DeviceId::from_bytes(std::array::from_fn(|i| u8::try_from(i).unwrap()));
        let text = original.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(text.parse::<DeviceId>().unwrap(), original);
        assert_eq!(text.to_uppercase().parse::<DeviceId>().unwrap(), original);
    }

    #[test]
    fn device_id_rejects_malformed_text() {
        for bad in ["", "zz", &"a".repeat(63), &"a".repeat(65), &"g".repeat(64)] {
            assert!(bad.parse::<DeviceId>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn debug_output_is_short() {
        assert_eq!(format!("{:?}", id(0xab)), "DeviceId(ababababab…)");
    }

    #[test]
    fn fingerprint_is_stable_and_grouped() {
        // Known answer: guards the construction in `fingerprint`.
        let fp = id(0).fingerprint();
        assert_eq!(fp.to_string(), "k65a-25j6-7e2y-gi4f-awnl");
        assert_eq!(fp.short(), "k65a");
    }

    #[test]
    fn fingerprints_differ_between_devices() {
        assert_ne!(id(1).fingerprint(), id(2).fingerprint());
    }

    #[test]
    fn every_fingerprint_is_twenty_lowercase_base32_characters() {
        for byte in 0..=u8::MAX {
            let fp = id(byte).fingerprint();
            let text = fp.to_string();
            assert_eq!(
                text.len(),
                24,
                "twenty characters in five groups, joined by four dashes"
            );
            assert_eq!(text.matches('-').count(), 4);
            assert!(
                text.chars()
                    .all(|c| c == '-' || c.is_ascii_lowercase() || ('2'..='7').contains(&c)),
                "{text}"
            );
            assert_eq!(fp.short().len(), 4);
        }
    }

    #[test]
    fn the_short_form_of_an_id_is_its_first_five_bytes() {
        let id = DeviceId::from_bytes(std::array::from_fn(|i| u8::try_from(i).unwrap()));
        assert_eq!(id.short(), "0001020304");
    }

    #[test]
    fn session_id_is_sixteen_bytes_in_cbor_and_shows_as_hex() {
        let id = SessionId::from_bytes(std::array::from_fn(|i| u8::try_from(i).unwrap()));
        assert_eq!(id.to_string(), "000102030405060708090a0b0c0d0e0f");
        assert_eq!(
            format!("{id:?}"),
            "SessionId(000102030405060708090a0b0c0d0e0f)"
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&id, &mut bytes).unwrap();
        // A byte string of 16 bytes: major type 2, length 16.
        assert_eq!(bytes.first(), Some(&0x50));
        assert_eq!(bytes.len(), 17);
        assert_eq!(
            ciborium::from_reader::<SessionId, _>(&bytes[..]).unwrap(),
            id
        );
    }

    #[test]
    fn session_id_rejects_a_byte_string_of_the_wrong_length() {
        for len in [0_usize, 15, 17, 32] {
            let mut bytes = Vec::new();
            ciborium::into_writer(&serde_bytes::ByteBuf::from(vec![7; len]), &mut bytes).unwrap();
            assert!(
                ciborium::from_reader::<SessionId, _>(&bytes[..]).is_err(),
                "{len} bytes"
            );
        }
    }
}
