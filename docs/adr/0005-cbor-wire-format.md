# 0005: CBOR in length-prefixed frames for control messages

Status: accepted

## Context

Devices exchange small control messages: hello, ping, links, clipboard text, pairing. The format must tolerate versions skewing in the field (a phone app updates later than a desktop app), be easy to implement from other languages, and be easy to inspect when debugging. Bulk data, such as files, will use separate raw streams, so this choice only has to serve small messages.

## Decision

Each message is one CBOR item ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) behind a 4-byte big-endian length, encoded with `serde` and `ciborium`. Structs are maps with text keys and unknown keys are ignored. The encoding is pinned by golden tests, and the format is specified in [protocol.md](../protocol.md).

Decoding is strict. A receiver reads the whole item into a generic value first, which is where well-formedness, nesting depth and leftover bytes are checked, and only then turns the value into a message. The first version deserialised straight from the bytes, and review showed that this is not safe: when an enum variant is not in its expected form, `ciborium` can return without reading the value that belongs to it, so the rest of the item is misread and a payload that is not CBOR can come out as a valid message. Reading into a value first closes that whole class of mistake for every message type at once, for the price of one extra allocation per message, which at these sizes does not matter.

## Alternatives considered

- **JSON.** Universal, but binary fields need base64 and there is no standard compact form. CBOR carries the same data model with real byte strings.
- **Protocol Buffers.** Excellent evolution rules, but it adds a schema language, code generation and a toolchain to every platform build.
- **postcard or bincode.** Very compact and fast, but not self-describing: adding a field to a struct breaks old peers, and other languages have no good implementation.

## Consequences

- Messages are somewhat larger than a schema-based binary format, because field names travel with the data. At these sizes it does not matter.
- Adding an optional field is compatible without a version bump.
- Enums that newer versions may extend can only gain variants without data. An unknown variant is read as `Other` only when there is no value to skip, so a new variant that carries data would make older peers fail to decode the whole message. Such additions go behind a capability instead.
- Receivers must bound lengths and nesting themselves; the frame layer does. Property tests, a test that damages valid messages in every position and checks the decoder against an independent CBOR checker, and fuzz targets cover it.
- A serde or ciborium upgrade could in principle change the bytes or what is accepted. The golden and compatibility tests would catch it.
