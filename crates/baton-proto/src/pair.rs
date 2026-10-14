// SPDX-License-Identifier: Apache-2.0

//! The pairing protocol, spoken on [`ALPN_PAIR`](crate::ALPN_PAIR).
//!
//! One device, the *offerer*, creates a [`PairTicket`](crate::ticket::PairTicket)
//! and waits. The other, the *joiner*, redeems it. The joiner opens a stream,
//! sends a [`PairRequest`], and reads one [`PairResponse`].
//!
//! The transport has already authenticated both ends by their device keys, so
//! the joiner knows it reached the device named in the ticket. What the proof
//! adds is the other direction: it shows the offerer that the joiner holds the
//! ticket's secret, without sending the secret.

use serde::{Deserialize, Serialize};
use serde_bytes::ByteArray;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    device::DeviceInfo,
    id::DeviceId,
    limits::{LimitError, MAX_PAIR_FRAME_LEN},
    message::Message,
};

/// TLS exporter label (RFC 5705) both sides use to derive the channel binding.
///
/// Mixing the exporter output into the proof ties it to one TLS session, so a
/// proof captured on one connection is useless on any other.
pub const EXPORTER_LABEL: &[u8] = b"baton-pair-v1";

/// Length of the channel binding derived from the TLS session.
pub const CHANNEL_BINDING_LEN: usize = 32;

/// Domain separation for the proof. Changing it changes every proof.
const PROOF_CONTEXT: &[u8] = b"baton 2026-10-14 pairing proof v1";

/// The one-time secret inside a pairing ticket.
///
/// It is a bearer credential: whoever has it can ask to pair until it expires
/// or is used. It is wiped from memory when dropped and never printed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct PairSecret([u8; Self::LEN]);

impl PairSecret {
    /// Length of the secret in bytes.
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

impl std::fmt::Debug for PairSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairSecret(..)")
    }
}

/// Shows that the joiner knows the ticket's secret.
///
/// It is a BLAKE3 keyed hash, keyed with the secret, over a fixed context
/// string, the offerer's identity, the joiner's identity and the channel
/// binding. `PairProof` deliberately has no `PartialEq`: compare with
/// [`verify`](Self::verify), which takes constant time.
#[derive(Clone, Copy)]
pub struct PairProof([u8; Self::LEN]);

impl PairProof {
    /// Length of a proof in bytes.
    pub const LEN: usize = 32;

    /// Computes the proof the joiner must send.
    pub fn compute(
        secret: &PairSecret,
        offerer: &DeviceId,
        joiner: &DeviceId,
        channel_binding: &[u8; CHANNEL_BINDING_LEN],
    ) -> Self {
        let mut hasher = blake3::Hasher::new_keyed(secret.expose());
        hasher.update(PROOF_CONTEXT);
        hasher.update(offerer.as_bytes());
        hasher.update(joiner.as_bytes());
        hasher.update(channel_binding);
        Self(*hasher.finalize().as_bytes())
    }

    /// Whether this is the proof the secret and the session would produce.
    pub fn verify(
        &self,
        secret: &PairSecret,
        offerer: &DeviceId,
        joiner: &DeviceId,
        channel_binding: &[u8; CHANNEL_BINDING_LEN],
    ) -> bool {
        let expected = Self::compute(secret, offerer, joiner, channel_binding);
        // `blake3::Hash` compares in constant time.
        blake3::Hash::from_bytes(self.0) == blake3::Hash::from_bytes(expected.0)
    }

    /// The raw bytes.
    pub const fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl std::fmt::Debug for PairProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairProof(..)")
    }
}

impl Serialize for PairProof {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ByteArray::new(self.0).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PairProof {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ByteArray::<{ Self::LEN }>::deserialize(deserializer).map(|bytes| Self(bytes.into_array()))
    }
}

/// The joiner's request to be paired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    /// Who the joiner is. Shown to the person approving.
    pub device: DeviceInfo,
    /// Shows the joiner holds the ticket's secret.
    pub proof: PairProof,
}

/// The offerer's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairResponse {
    /// Paired. Both sides now trust each other's keys.
    Accepted {
        /// Who the offerer is.
        device: DeviceInfo,
    },
    /// Not paired.
    Rejected(PairRejection),
}

/// Why pairing did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairRejection {
    /// The proof was wrong, or the ticket is unknown, expired or already used.
    ///
    /// These cases are deliberately not told apart, so a caller who does not
    /// hold the secret learns nothing about the offer.
    Invalid,
    /// The proof was right, but the person on the offering device said no.
    Declined,
    /// The offerer is busy with another request. Try again.
    Busy,
    /// Any reason this version does not know about.
    #[serde(other)]
    Other,
}

impl Message for PairRequest {
    const MAX_FRAME_LEN: usize = MAX_PAIR_FRAME_LEN;

    fn validate(&self) -> Result<(), LimitError> {
        self.device.validate()
    }
}

impl Message for PairResponse {
    const MAX_FRAME_LEN: usize = MAX_PAIR_FRAME_LEN;

    fn validate(&self) -> Result<(), LimitError> {
        match self {
            Self::Accepted { device } => device.validate(),
            Self::Rejected(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(byte: u8) -> PairSecret {
        PairSecret::from_bytes([byte; 32])
    }

    fn id(byte: u8) -> DeviceId {
        DeviceId::from_bytes([byte; 32])
    }

    #[test]
    fn proof_verifies_only_with_the_same_inputs() {
        let binding = [9; CHANNEL_BINDING_LEN];
        let proof = PairProof::compute(&secret(1), &id(2), &id(3), &binding);
        assert!(proof.verify(&secret(1), &id(2), &id(3), &binding));
        assert!(
            !proof.verify(&secret(0), &id(2), &id(3), &binding),
            "wrong secret"
        );
        assert!(
            !proof.verify(&secret(1), &id(4), &id(3), &binding),
            "wrong offerer"
        );
        assert!(
            !proof.verify(&secret(1), &id(2), &id(4), &binding),
            "wrong joiner"
        );
        assert!(
            !proof.verify(&secret(1), &id(2), &id(3), &[8; 32]),
            "other session"
        );
        // Offerer and joiner are not interchangeable.
        assert!(
            !proof.verify(&secret(1), &id(3), &id(2), &binding),
            "roles swapped"
        );
    }

    #[test]
    fn proof_has_a_known_answer() {
        // Computed independently with the reference BLAKE3 implementation (Python `blake3`):
        // keyed_hash(key=[1;32], PROOF_CONTEXT || [2;32] || [3;32] || [9;32])
        let proof = PairProof::compute(&secret(1), &id(2), &id(3), &[9; 32]);
        assert_eq!(
            data_encoding::HEXLOWER.encode(proof.as_bytes()),
            "fd5eaa8f3cbf99c3fca24b23d171929262050317c44c1b8a41356dae1616ece5"
        );
    }

    #[test]
    fn secrets_and_proofs_are_not_printed() {
        assert_eq!(format!("{:?}", secret(0xaa)), "PairSecret(..)");
        let proof = PairProof::compute(&secret(1), &id(2), &id(3), &[0; 32]);
        assert_eq!(format!("{proof:?}"), "PairProof(..)");
    }
}
