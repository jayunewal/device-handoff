# 0005: CBOR in length-prefixed frames for control messages

Status: accepted

## Context

Devices exchange small control messages: hello, ping, links, clipboard text, pairing. The format must tolerate versions skewing in the field (a phone app updates later than a desktop app), be easy to implement from other languages, and be easy to inspect when debugging. Bulk data, such as files, will use separate raw streams, so this choice only has to serve small messages.

## Decision

Each message is one CBOR item ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) behind a 4-byte big-endian length, encoded with `serde` and `ciborium`. Structs are maps with text keys and unknown keys are ignored. The encoding is pinned by golden tests, and the format is specified in [protocol.md](../protocol.md).

## Alternatives considered

- **JSON.** Universal, but binary fields need base64 and there is no standard compact form. CBOR carries the same data model with real byte strings.
- **Protocol Buffers.** Excellent evolution rules, but it adds a schema language, code generation and a toolchain to every platform build.
- **postcard or bincode.** Very compact and fast, but not self-describing: adding a field to a struct breaks old peers, and other languages have no good implementation.

## Consequences

- Messages are somewhat larger than a schema-based binary format, because field names travel with the data. At these sizes it does not matter.
- Adding an optional field is compatible without a version bump.
- Receivers must bound lengths and nesting themselves; the frame layer does, and fuzz-style property tests cover it.
- A serde or ciborium upgrade could in principle change the bytes. The golden tests would catch it.
