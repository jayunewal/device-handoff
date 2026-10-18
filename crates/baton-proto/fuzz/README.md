# Fuzz targets for `baton-proto` and `baton-stream`

These targets feed arbitrary bytes to the code that reads input from the network. They are outside the main workspace on purpose: they need a nightly toolchain and a C++ compiler, and nobody should need either to build Baton.

Install the tools once:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

Run a target from `crates/baton-proto`, starting from the committed seeds. The corpus the fuzzer grows stays in `fuzz/corpus`, which git ignores:

```sh
mkdir -p fuzz/corpus/decode_payload fuzz/corpus/ticket fuzz/corpus/transfer fuzz/corpus/stream
cargo +nightly fuzz run decode_payload fuzz/corpus/decode_payload fuzz/seeds/decode_payload -- -max_total_time=120
cargo +nightly fuzz run ticket fuzz/corpus/ticket fuzz/seeds/ticket -- -max_total_time=120
cargo +nightly fuzz run transfer fuzz/corpus/transfer fuzz/seeds/transfer -- -max_total_time=120
cargo +nightly fuzz run stream fuzz/corpus/stream fuzz/seeds/stream -- -max_total_time=120
```

## What they check

| Target | Input | Checked |
| --- | --- | --- |
| `decode_payload` | Raw bytes, tried as every message type and as a whole frame | The decoders never panic. A payload is accepted only if an independent CBOR checker (`tests/support/strict_cbor.rs`) agrees it is exactly one well-formed item. Whatever decodes can be encoded again and decodes to the same message. |
| `ticket` | Raw bytes as a binary ticket, and the same bytes as text | The parsers never panic. A ticket that parses can be written out again and read back as the same ticket. |
| `transfer` | The first byte picks a part of a file transfer, the rest is its input: the header of a lane, the head of a frame, an offer followed by pieces of its list of files, or the size of a file followed by the hashes of its spans | The parts never panic. A header or a head that is read can be written and read again to the same value. A list that is accepted is in the order of the protocol, every entry is inside a folder that came before it, it is what the offer said, with the same counts, the same sizes and the same top-level items as the preview, and it has the hash that the offer carried. A file whose spans are all there has a hash, and one with a gap says which span is missing. |
| `stream` | The first byte picks a part of window streaming, the rest is its input: the start of a stream, the header of a frame of video, a message of the source, or a message of the viewer | The parts never panic. A start or a header that is read can be written again to the same bytes and read again to the same value. A message is accepted only if the independent checker agrees that its payload is one well-formed CBOR item, what decodes passes its own checks, and it can be written and read again to the same message. |

The independent checker is the point of the first target: a crash search alone would not have found a decoder that accepts bytes that are not CBOR, because accepting them does not crash. The `transfer` target works the same way for what a person agrees to. It counts the list again from scratch, because a checker that accepted a list other than the one that was offered would not crash either.

For a list, bit 2 of the first byte says whether the hash in the offer is replaced by the hash of the list that follows. A mutated list almost never keeps a hash that is right, and everything the checker does after it looks at the hash would go unseen without that.

## Seeds

`seeds/decode_payload` holds one valid payload of every message, plus malformed ones that once got through a decoder. `seeds/ticket` holds valid tickets with and without a relay and addresses, tickets with ignorable and critical extensions and flags, the text form, and a text form with the prefix of a later version. `seeds/transfer` holds one input for each part and for each way it can be refused: a lane header with a wrong kind or a lane that is too high, the head of every kind of frame and one with a reason of a later version, a list that is right, one in two pieces, one out of order, one whose preview does not match, one whose hash is not the one in the offer, and one with a file in a folder that was never listed, and files of different sizes with all of their spans, a missing span, repeated spans and the largest file there can be. The first byte of each of them says which part it is for. `seeds/stream` holds one input for each message of the two control streams, each header, and the start of each kind of stream, and for the ways they are refused: a header of a lane, with a flag that is not defined, cut short, or naming a frame that is not an earlier one; a message with a picture that is out of range, a frame rate that is too high, a notice of no events, a batch of no events or of 65, a position that is outside the picture or not a number, a scroll that is too far, modifiers that are not defined, text that is too long (21 units, where 20 is the longest) or has a control character, a code above the keyboard page, and text typed with no key (right, without text, and coming up); the reasons, states and codecs of a later version, which read as `Other`; and a message that nobody knows or that belongs to the other direction. They are small on purpose, so that the fuzzer starts from the interesting shapes instead of finding them.

When the fuzzer or a bug report finds an input that breaks something, do three things in the same pull request as the fix:

1. Add the input to the matching `seeds/` directory with a name that says what it is.
2. Add a unit or integration test that fails without the fix. The fuzzer is a way to find bugs; the regression test is what keeps them fixed on machines that never run it.
3. Do not commit anything from `fuzz/corpus` or `fuzz/artifacts`.
