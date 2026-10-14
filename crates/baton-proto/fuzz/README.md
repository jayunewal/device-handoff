# Fuzz targets for `baton-proto`

These targets feed arbitrary bytes to the code that reads input from the network. They are outside the main workspace on purpose: they need a nightly toolchain and a C++ compiler, and nobody should need either to build Baton.

Install the tools once:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

Run a target from `crates/baton-proto`, starting from the committed seeds. The corpus the fuzzer grows stays in `fuzz/corpus`, which git ignores:

```sh
cargo +nightly fuzz run decode_payload fuzz/corpus/decode_payload fuzz/seeds/decode_payload -- -max_total_time=120
cargo +nightly fuzz run ticket fuzz/corpus/ticket fuzz/seeds/ticket -- -max_total_time=120
```

## What they check

| Target | Input | Checked |
| --- | --- | --- |
| `decode_payload` | Raw bytes, tried as every message type and as a whole frame | The decoders never panic. A payload is accepted only if an independent CBOR checker (`tests/support/strict_cbor.rs`) agrees it is exactly one well-formed item. Whatever decodes can be encoded again and decodes to the same message. |
| `ticket` | Raw bytes as a binary ticket, and the same bytes as text | The parsers never panic. A ticket that parses can be written out again and read back as the same ticket. |

The independent checker is the point of the first target: a crash search alone would not have found a decoder that accepts bytes that are not CBOR, because accepting them does not crash.

## Seeds

`seeds/decode_payload` holds one valid payload of every message, plus malformed ones that once got through a decoder. `seeds/ticket` holds valid tickets with and without a relay and addresses, tickets with ignorable and critical extensions and flags, and the text form. They are small on purpose, so that the fuzzer starts from the interesting shapes instead of finding them.

When the fuzzer or a bug report finds an input that breaks something, do three things in the same pull request as the fix:

1. Add the input to the matching `seeds/` directory with a name that says what it is.
2. Add a unit or integration test that fails without the fix. The fuzzer is a way to find bugs; the regression test is what keeps them fixed on machines that never run it.
3. Do not commit anything from `fuzz/corpus` or `fuzz/artifacts`.
