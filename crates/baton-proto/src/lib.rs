// SPDX-License-Identifier: Apache-2.0

//! Wire protocol for Baton.
//!
//! This crate is deliberately free of networking and platform code so that it
//! can be tested and fuzzed in isolation, and so that other implementations of
//! the protocol have a single reference to compare against. The protocol is
//! specified in `docs/protocol.md`; the code here is the reference.
//!
//! - [`message`] holds the requests and responses between paired devices.
//! - [`pair`] holds the pairing handshake and its proof of possession.
//! - [`ticket`] holds the pairing ticket that is shown as a QR code or text.
//! - [`frame`] turns messages into length-prefixed frames and back.
//! - [`limits`] holds the size limits every message is checked against.

// This crate parses bytes from strangers. An index or a slice range that is out of bounds
// panics, and a panic on input from the network is a remote crash, so the code spells out
// what happens when the data is short (`get`, `split_first_chunk` and friends).
// Tests are exempt through `clippy.toml`.
#![warn(clippy::indexing_slicing)]

pub mod device;
pub mod frame;
pub mod id;
pub mod limits;
pub mod message;
pub mod pair;
pub mod ticket;

pub use device::{DeviceInfo, Platform};
pub use id::{DeviceId, Fingerprint, ItemId};

/// Major version of the Baton wire protocol.
///
/// A peer that speaks a different major version cannot talk to us at all: the
/// version is part of the ALPN identifiers below, so the mismatch is detected
/// during the TLS handshake. Compatible additions (new optional fields, new
/// capabilities) do not change it.
pub const PROTOCOL_MAJOR: u16 = 1;

/// Minor revision within [`PROTOCOL_MAJOR`], advertised in [`message::Hello`].
pub const PROTOCOL_MINOR: u16 = 0;

/// ALPN identifier for the main session protocol between paired devices.
pub const ALPN_SESSION: &[u8] = b"baton/1";

/// ALPN identifier for the one-time pairing protocol.
pub const ALPN_PAIR: &[u8] = b"baton-pair/1";

/// Application error codes sent when closing a connection or resetting a stream.
///
/// They follow HTTP status numbers loosely, because people already know them.
pub mod close_code {
    /// Orderly close; nothing went wrong.
    pub const NORMAL: u32 = 0;
    /// The peer broke the protocol, for example with an oversized frame.
    pub const PROTOCOL_VIOLATION: u32 = 400;
    /// The peer is not paired with this device.
    pub const NOT_PAIRED: u32 = 403;
    /// The peer was too slow.
    pub const TIMEOUT: u32 = 408;
    /// The peer holds more connections, or more pairing attempts at once, than this
    /// device allows.
    pub const OVERLOADED: u32 = 429;
    /// This device failed internally or is shutting down.
    pub const INTERNAL: u32 = 500;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpn_identifiers_carry_the_major_version() {
        let suffix = format!("/{PROTOCOL_MAJOR}");
        for alpn in [ALPN_SESSION, ALPN_PAIR] {
            let alpn = std::str::from_utf8(alpn).expect("ALPN identifiers are ASCII");
            assert!(alpn.ends_with(&suffix), "{alpn} must end with {suffix}");
        }
    }

    #[test]
    fn alpn_identifiers_are_distinct() {
        assert_ne!(ALPN_SESSION, ALPN_PAIR);
    }
}
