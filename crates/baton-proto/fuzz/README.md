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
| `stream` | The first byte picks a part of window streaming, the rest is its input: the start of a stream, the header of a frame of video, a message of the source, a message of the viewer, the picture of a frame as it comes off the wire, a sample from the encoder (the length of its length field, a parameter set that may be nonsense, and the sample), the sequence parameter set of a key frame, or the header of a packet of sound | The parts never panic. A start or a header that is read can be written again to the same bytes and read again to the same value. A message is accepted only if the independent checker agrees that its payload is one well-formed CBOR item, what decodes passes its own checks, and it can be written and read again to the same message. A picture that is read leaves a sample for the decoder, and the sample, made into a picture again, is read as the same sample. A sample that is made into a picture is read back as the units that were written. What is refused leaves the output as it was. A parameter set gives a size that is not nothing, and one that is cut short gives that size or none. |

The independent checker is the point of the first target: a crash search alone would not have found a decoder that accepts bytes that are not CBOR, because accepting them does not crash. The `transfer` target works the same way for what a person agrees to. It counts the list again from scratch, because a checker that accepted a list other than the one that was offered would not crash either.

For a list, bit 2 of the first byte says whether the hash in the offer is replaced by the hash of the list that follows. A mutated list almost never keeps a hash that is right, and everything the checker does after it looks at the hash would go unseen without that.

## Seeds

`seeds/decode_payload` holds one valid payload of every message, plus malformed ones that once got through a decoder. `seeds/ticket` holds valid tickets with and without a relay and addresses, tickets with ignorable and critical extensions and flags, the text form, and a text form with the prefix of a later version. `seeds/transfer` holds one input for each part and for each way it can be refused: a lane header with a wrong kind or a lane that is too high, the head of every kind of frame and one with a reason of a later version, a list that is right, one in two pieces, one out of order, one whose preview does not match, one whose hash is not the one in the offer, and one with a file in a folder that was never listed, and files of different sizes with all of their spans, a missing span, repeated spans and the largest file there can be. The first byte of each of them says which part it is for. `seeds/stream` holds one input for each message of the two control streams, each header, and the start of each kind of stream, and for the ways they are refused: a header of a lane, with a flag that is not defined, cut short, or naming a frame that is not an earlier one; a message with a picture that is out of range, a frame rate that is too high, a notice of no events, a batch of no events or of 65, a position that is outside the picture or not a number, a scroll that is too far, modifiers that are not defined, text that is too long (21 units, where 20 is the longest) or has a control character, a code above the keyboard page, and text typed with no key (right, without text, and coming up); the reasons, states and codecs of a later version, which read as `Other`; and a message that nobody knows or that belongs to the other direction. For sound it holds the start of a stream of packets, the header of a first packet and of a later one with the sound behind it, a header cut short, a header that is the header of a frame of video, a header with a flag that is not defined, the format of sound (right, with a codec or a field of a later version, with a rate that is too low, with three channels, and with too many samples), a silence with a reason of a later version, and a request to hear (on, off, and one that does not say which), also sent in the wrong direction. For the pictures it holds a key frame and a delta frame from a real encoder, start codes of three bytes with zeros around them, a key frame with no parameter sets, parameter sets alone, a picture with something before its first start code or with no start code, a unit with its forbidden bit set, and pictures of 256 and 257 units; for the samples, a key frame and a delta frame, lengths of two bytes, parameter sets inside the sample, a candidate parameter set that is a slice, a length field of 3 bytes, a sample cut short, an empty unit, a unit with its forbidden bit set, a unit with a start code inside it or a zero at its end, and parameter sets alone; for the sizes of pictures, sets from a real encoder (a main profile, a high one with its cropping, 4:2:2, 4:4:4, monochrome, a constrained baseline and a frame that may be two fields) and sets with scaling lists that were written by hand, and the ways they are refused: a set cut short, a picture parameter set, a bit depth of 15 bits, a chroma format that does not exist, a picture order type that does not exist, 256 offsets of picture order where 255 is the most, numbers of 32 zeros or with no end, a picture that is cropped away and one that is wider than 32 bits. They are small on purpose, so that the fuzzer starts from the interesting shapes instead of finding them. A test in `crates/baton-stream/tests/seeds.rs` lists every seed of `stream` with what its part must make of it (refuse it, read it, or read it and write the same bytes), so a rule that breaks a seed fails in CI and not silently in the fuzzer.

When the fuzzer or a bug report finds an input that breaks something, do three things in the same pull request as the fix:

1. Add the input to the matching `seeds/` directory with a name that says what it is.
2. Add a unit or integration test that fails without the fix. The fuzzer is a way to find bugs; the regression test is what keeps them fixed on machines that never run it.
3. Do not commit anything from `fuzz/corpus` or `fuzz/artifacts`.
