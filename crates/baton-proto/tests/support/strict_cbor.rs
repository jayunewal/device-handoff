// SPDX-License-Identifier: Apache-2.0

//! A small CBOR checker that shares no code with ciborium.
//!
//! It answers one question: is this byte string exactly one well-formed CBOR
//! item ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949) section 3 and
//! appendix C)? The tests and the fuzz target use it as an independent
//! reference: a decoder in `baton-proto` must never accept a payload that this
//! checker refuses, however the message types happen to read their input.
//!
//! Text strings must also be valid UTF-8, which is stricter than well-formedness
//! asks for. Being stricter is the safe direction for that use.

/// Deepest nesting the checker follows. The decoders allow fewer levels, so this
/// never makes the checker refuse something a decoder accepts.
const MAX_DEPTH: usize = 64;

/// Whether `bytes` is exactly one well-formed CBOR data item.
pub(crate) fn is_one_item(bytes: &[u8]) -> bool {
    let mut input = bytes;
    item(&mut input, 0).is_some() && input.is_empty()
}

fn take<'a>(input: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
    let (head, tail) = input.split_at_checked(n)?;
    *input = tail;
    Some(head)
}

fn is_break(input: &[u8]) -> bool {
    input.first() == Some(&0xff)
}

/// Reads an initial byte and its argument. The argument is `None` for the
/// indefinite-length marker (additional information 31).
fn head(input: &mut &[u8]) -> Option<(u8, u8, Option<u64>)> {
    let first = *take(input, 1)?.first()?;
    let (major, info) = (first >> 5, first & 0x1f);
    let arg = match info {
        0..=23 => Some(u64::from(info)),
        24 => Some(u64::from(*take(input, 1)?.first()?)),
        25 => Some(u64::from(u16::from_be_bytes(
            take(input, 2)?.try_into().ok()?,
        ))),
        26 => Some(u64::from(u32::from_be_bytes(
            take(input, 4)?.try_into().ok()?,
        ))),
        27 => Some(u64::from_be_bytes(take(input, 8)?.try_into().ok()?)),
        // Reserved by the RFC: never well-formed.
        28..=30 => return None,
        _ => None,
    };
    Some((major, info, arg))
}

fn string_body(input: &mut &[u8], major: u8, len: u64) -> Option<()> {
    let bytes = take(input, usize::try_from(len).ok()?)?;
    if major == 3 {
        std::str::from_utf8(bytes).ok()?;
    }
    Some(())
}

fn item(input: &mut &[u8], depth: usize) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let (major, info, arg) = head(input)?;
    match (major, arg) {
        // Integers have no indefinite form; that case is refused by the last arm.
        (0 | 1, Some(_)) => Some(()),
        (2 | 3, Some(len)) => string_body(input, major, len),
        // An indefinite string is a run of definite strings of the same type.
        (2 | 3, None) => loop {
            if is_break(input) {
                take(input, 1)?;
                return Some(());
            }
            let (chunk_major, _, chunk_len) = head(input)?;
            if chunk_major != major {
                return None;
            }
            string_body(input, major, chunk_len?)?;
        },
        (4, Some(count)) => {
            for _ in 0..count {
                item(input, depth + 1)?;
            }
            Some(())
        }
        (4, None) => loop {
            if is_break(input) {
                take(input, 1)?;
                return Some(());
            }
            item(input, depth + 1)?;
        },
        (5, Some(count)) => {
            for _ in 0..count {
                item(input, depth + 1)?;
                item(input, depth + 1)?;
            }
            Some(())
        }
        (5, None) => loop {
            if is_break(input) {
                take(input, 1)?;
                return Some(());
            }
            item(input, depth + 1)?;
            item(input, depth + 1)?;
        },
        // A tag is followed by exactly one item.
        (6, Some(_)) => item(input, depth + 1),
        // Simple values and floats. The one-byte form of a simple value must not
        // hold a value below 32.
        (7, Some(value)) => (info != 24 || value >= 32).then_some(()),
        // An indefinite length where none is allowed (integers, tags), and a break
        // code outside an indefinite item, which is not an item at all.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::is_one_item;

    fn hex(text: &str) -> Vec<u8> {
        data_encoding::HEXLOWER.decode(text.as_bytes()).unwrap()
    }

    #[test]
    fn accepts_the_examples_from_the_rfc() {
        for text in [
            "00",
            "17",
            "1818",
            "1903e8",
            "1a000f4240",
            "1b000000e8d4a51000",
            "20",
            "3863",
            "40",
            "4401020304",
            "60",
            "6161",
            "6449455446",
            "80",
            "83010203",
            "a0",
            "a201020304",
            "a26161016162820203",
            "8301820203820405",
            "5f42010243030405ff",
            "7f657374726561646d696e67ff",
            "9fff",
            "9f018202039f0405ffff",
            "bf61610161629f0203ffff",
            "f4",
            "f5",
            "f6",
            "f7",
            "f0",
            "f820",
            "f93c00",
            "fa47c35000",
            "fb3ff199999999999a",
            "c074323031332d30332d32315432303a30343a30305a",
            "d74401020304",
        ] {
            assert!(is_one_item(&hex(text)), "{text}");
        }
    }

    #[test]
    fn refuses_what_is_not_well_formed() {
        for (why, text) in [
            ("empty", ""),
            ("break at the top", "ff"),
            ("reserved additional information", "1c"),
            ("integer cut short", "19ff"),
            ("string cut short", "6261"),
            ("array cut short", "8301"),
            ("map with a key and no value", "a161"),
            ("trailing byte", "0000"),
            ("indefinite integer", "1f"),
            ("simple value below 32 in the long form", "f800"),
            (
                "indefinite string with a chunk of the wrong type",
                "5f6161ff",
            ),
            ("indefinite string with an indefinite chunk", "5f5fffff"),
            ("indefinite map with a key and no value", "bf6161ff"),
            ("tag without an item", "c0"),
            ("invalid UTF-8", "62c328"),
            ("missing break", "9f01"),
        ] {
            assert!(!is_one_item(&hex(text)), "{why}: {text}");
        }
    }

    #[test]
    fn the_malformed_payload_from_the_review_is_refused() {
        // {"Handoff": {"outcome": {"Accepted": "id"}, h'4242...': <missing>}}
        let payload = hex(
            "a16748616e646f6666a2676f7574636f6d65a16841636365707465646269645042424242424242424242424242424242",
        );
        assert!(!is_one_item(&payload));
    }
}
