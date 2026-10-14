// SPDX-License-Identifier: Apache-2.0

//! Identifiers: devices and handoff items.

use std::{fmt, str::FromStr};

use data_encoding::{BASE32_NOPAD, HEXLOWER, HEXLOWER_PERMISSIVE};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_bytes::ByteArray;

/// Domain separation for [`DeviceId::fingerprint`]. Changing it changes every fingerprint.
const FINGERPRINT_CONTEXT: &str = "baton 2026-10-14 device fingerprint v1";

/// Number of base32 characters shown in a fingerprint (5 bits each).
const FINGERPRINT_CHARS: usize = 20;

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
    pub fn short(&self) -> String {
        HEXLOWER.encode(&self.0[..5])
    }

    /// A short string two people can compare out loud or on two screens.
    ///
    /// It is derived from the identity with a hash, so an attacker cannot pick
    /// a key whose fingerprint merely starts like another one.
    pub fn fingerprint(&self) -> Fingerprint {
        let digest = blake3::derive_key(FINGERPRINT_CONTEXT, &self.0);
        let encoded = BASE32_NOPAD.encode(&digest[..13]).to_ascii_lowercase();
        Fingerprint(encoded[..FINGERPRINT_CHARS].to_owned())
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
    pub fn short(&self) -> &str {
        &self.0[..4]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> DeviceId {
        DeviceId::from_bytes([byte; 32])
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
}
