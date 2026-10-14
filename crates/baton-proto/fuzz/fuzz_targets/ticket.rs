// SPDX-License-Identifier: Apache-2.0

//! Feeds arbitrary bytes and text to the pairing ticket parsers. A ticket comes
//! from a scanned QR code, so it is untrusted input.

#![no_main]

use baton_proto::ticket::PairTicket;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(ticket) = PairTicket::from_bytes(data) {
        // A ticket that parses can be written out again and read back as the same ticket.
        let bytes = ticket.to_bytes().expect("a ticket that parsed can be encoded");
        let again = PairTicket::from_bytes(&bytes).expect("an encoded ticket parses");
        assert_eq!(again.to_bytes().unwrap(), bytes);
    }
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = text.parse::<PairTicket>();
    }
});
