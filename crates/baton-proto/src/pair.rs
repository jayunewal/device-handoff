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
//!
//! A proof cannot tell the person who approves the request whether it comes
//! from the device in their hand or from someone who got hold of the ticket.
//! For that, both devices work out the same [`ConfirmationCode`] and show it,
//! and the person approves only if the two codes match.

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

/// Domain separation for the confirmation code. Changing it changes every code.
const CONFIRMATION_CONTEXT: &str = "baton 2026-10-14 pairing confirmation v1";

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
        let proof = Self(*hasher.finalize().as_bytes());
        // The hasher holds a copy of the secret as its key.
        hasher.zeroize();
        proof
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

/// A six-digit code that both people see while a pairing request waits for approval.
///
/// The offering device and the joining device each work it out from the ticket's
/// secret, both device ids and the TLS session, and show it. Nothing about it is
/// sent. The person on the offering device approves the request only if the code
/// on the joining device, the one in their hand, is the same.
///
/// That is what lets approval catch a ticket that leaked. Someone who holds a copy
/// of the ticket can send a request with a valid proof, but their device key and
/// their TLS session are their own, so their code differs from the one on the
/// device the person is holding. A fingerprint of the joiner cannot do this, because
/// it never changes and the person has nothing to compare it with. This code is new
/// for every session, so it cannot be worked out ahead of time, and a wrong guess
/// costs the attacker a live connection and a prompt that the person sees.
///
/// Six digits are enough for that: the attacker cannot see the code on the other
/// screen, so a request has one chance in a million of matching it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationCode(u32);

impl ConfirmationCode {
    /// Number of digits shown.
    pub const DIGITS: usize = 6;

    const MODULUS: u32 = 1_000_000;

    /// Works out the code for one pairing session.
    pub fn compute(
        secret: &PairSecret,
        offerer: &DeviceId,
        joiner: &DeviceId,
        channel_binding: &[u8; CHANNEL_BINDING_LEN],
    ) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key(CONFIRMATION_CONTEXT);
        hasher.update(secret.expose());
        hasher.update(offerer.as_bytes());
        hasher.update(joiner.as_bytes());
        hasher.update(channel_binding);
        let digest = hasher.finalize();
        // The hasher holds a copy of the secret.
        hasher.zeroize();
        let [a, b, c, d, ..] = *digest.as_bytes();
        Self(u32::from_be_bytes([a, b, c, d]) % Self::MODULUS)
    }

    /// The code as a number from 0 to 999,999.
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Six digits in two groups of three, such as `042 917`, with leading zeros kept.
impl std::fmt::Display for ConfirmationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:03} {:03}", self.0 / 1000, self.0 % 1000)
    }
}

impl std::fmt::Debug for ConfirmationCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConfirmationCode({self})")
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
    /// The proof was right, but the person on the offering device said no, or did
    /// not answer in time. The ticket is spent.
    Declined,
    /// The proof was right, but the offerer is already waiting for its person to
    /// answer another request. Only a holder of the secret ever sees this. Try again.
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
    fn confirmation_code_has_known_answers() {
        // Computed independently with the reference BLAKE3 implementation (Python `blake3`):
        // derive_key(CONFIRMATION_CONTEXT, secret || offerer || joiner || binding), first four
        // bytes as a big-endian u32, modulo 1,000,000.
        let code = ConfirmationCode::compute(&secret(1), &id(2), &id(3), &[9; 32]);
        assert_eq!(code.value(), 196_465);
        assert_eq!(code.to_string(), "196 465");
        let other_joiner = ConfirmationCode::compute(&secret(1), &id(2), &id(4), &[9; 32]);
        assert_eq!(other_joiner.to_string(), "213 543");
        // Leading zeros are kept.
        let padded = ConfirmationCode::compute(&secret(43), &id(2), &id(3), &[251; 32]);
        assert_eq!(padded.value(), 45);
        assert_eq!(padded.to_string(), "000 045");
    }

    #[test]
    fn confirmation_code_depends_on_every_input() {
        let base = ConfirmationCode::compute(&secret(1), &id(2), &id(3), &[9; 32]);
        for (what, changed) in [
            (
                "secret",
                ConfirmationCode::compute(&secret(0), &id(2), &id(3), &[9; 32]),
            ),
            (
                "offerer",
                ConfirmationCode::compute(&secret(1), &id(4), &id(3), &[9; 32]),
            ),
            (
                "joiner",
                ConfirmationCode::compute(&secret(1), &id(2), &id(4), &[9; 32]),
            ),
            (
                "session",
                ConfirmationCode::compute(&secret(1), &id(2), &id(3), &[8; 32]),
            ),
            (
                "roles",
                ConfirmationCode::compute(&secret(1), &id(3), &id(2), &[9; 32]),
            ),
        ] {
            assert_ne!(base, changed, "the code ignores the {what}");
        }
        // Both devices get the same code from the same inputs.
        assert_eq!(
            base,
            ConfirmationCode::compute(&secret(1), &id(2), &id(3), &[9; 32])
        );
    }

    #[test]
    fn secrets_and_proofs_are_not_printed() {
        assert_eq!(format!("{:?}", secret(0xaa)), "PairSecret(..)");
        let proof = PairProof::compute(&secret(1), &id(2), &id(3), &[0; 32]);
        assert_eq!(format!("{proof:?}"), "PairProof(..)");
    }
}
