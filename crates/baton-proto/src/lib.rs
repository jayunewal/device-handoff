// SPDX-License-Identifier: Apache-2.0

//! Wire protocol for Baton.
//!
//! This crate is deliberately free of networking and platform code so that it
//! can be tested and fuzzed in isolation, and so that other implementations of
//! the protocol have a single reference to compare against.

/// Major version of the Baton wire protocol.
///
/// A peer that speaks a different major version cannot talk to us at all: the
/// version is part of the ALPN identifiers below, so the mismatch is detected
/// during the TLS handshake. Compatible additions (new optional fields, new
/// capabilities) do not change it.
pub const PROTOCOL_MAJOR: u16 = 1;

/// ALPN identifier for the main session protocol between paired devices.
pub const ALPN_SESSION: &[u8] = b"baton/1";

/// ALPN identifier for the one-time pairing protocol.
pub const ALPN_PAIR: &[u8] = b"baton-pair/1";

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
