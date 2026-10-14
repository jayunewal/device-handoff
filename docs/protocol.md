# Wire protocol

This document specifies what Baton devices say to each other. The reference implementation is the `baton-proto` crate; where this document and the code disagree, that is a bug in one of them.

Words such as MUST and SHOULD carry their usual meaning ([RFC 2119](https://www.rfc-editor.org/rfc/rfc2119)).

## 1. Transport

Devices talk over QUIC with TLS 1.3, provided by [iroh](https://github.com/n0-computer/iroh). Each device has an Ed25519 key pair; the public key is its **device id** (32 bytes) and the TLS handshake authenticates both ends by it. A connection is therefore always between two known device ids, and a relay, if one is involved, only sees ciphertext.

How a connection is found (local network, hole punching, relay) is the transport's business and does not appear in the protocol.

Two ALPN identifiers select the protocol on a connection:

| ALPN | Purpose | Who may connect |
| --- | --- | --- |
| `baton/1` | Session: hello, ping and handoff | Only paired devices |
| `baton-pair/1` | Pairing | Anyone, while the receiver has an open offer |

The number in the ALPN is the **major** protocol version. Peers with different majors cannot connect. The **minor** revision (currently 0) is exchanged in `Hello` and only informs diagnostics: compatible changes never need negotiation.

A device MUST close a connection whose peer is not allowed on that ALPN (not paired for `baton/1`, no open offer for `baton-pair/1`) before reading any data from it, using close code 403.

## 2. Frames and encoding

Everything after the handshake is a sequence of **frames**:

```
+---------------------------+-------------------------------+
| length: u32, big-endian   | payload: CBOR, `length` bytes |
+---------------------------+-------------------------------+
```

The payload is exactly one message encoded as CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)). Receivers MUST check the length against the limit for the message type before allocating, MUST reject a zero length, and MUST reject a payload that is not exactly one CBOR item. They SHOULD reject CBOR nested deeper than 16 levels.

| Message family | Largest payload |
| --- | --- |
| Session (`Request`, `Response`) | 2 MiB |
| Pairing (`PairRequest`, `PairResponse`) | 4 KiB |

CBOR encoding rules, as produced by the reference implementation:

- Structs are maps with text keys. Receivers ignore unknown keys, so adding an optional field is compatible. Optional fields that are absent are omitted.
- A unit enum variant is its name as text (`"Accepted"`). A variant with data is a one-entry map from the name to the data (`{"Ping": {"nonce": 1}}`).
- Device ids, item ids, secrets and proofs are byte strings of fixed length.
- Enums that a newer version may extend (`Platform`, `DeclineReason`, `ErrorCode`, `PairRejection`) decode unknown variants as `Other`. Other enums do not; a request with an unknown variant is answered with `Error { code: Malformed }`.

String and list sizes are limited as well (see section 6). A message that breaks a limit MUST NOT be sent, and MUST be rejected when received.

## 3. Session protocol (`baton/1`)

Each request uses its own bidirectional stream: the client opens a stream, writes one `Request` frame, and finishes its side; the server reads the frame, writes one `Response` frame, and finishes. Either side MAY have several streams open at once. A receiver SHOULD give a peer no more than 16 concurrent streams and SHOULD time out a stream that makes no progress for 10 seconds.

### Hello

Both sides describe themselves. A client SHOULD send `Hello` on a new connection before anything else, and use the capabilities in the reply to decide what it may send.

```
Request  {"Hello": {"protocol_minor": 0,
                    "device": {"name": "Kitchen laptop", "platform": "Linux", "app_version": "0.1.0"},
                    "capabilities": ["handoff.link", "handoff.clipboard.text"]}}
Response {"Hello": { ... same shape ... }}
```

A paired device can rename itself at any time, so the receiver SHOULD update the name it shows. Names are chosen by the peer and MUST be sanitised before display (`sanitize_display_name`).

Capabilities defined so far:

| Capability | Meaning |
| --- | --- |
| `handoff.link` | Accepts `Item::Link` |
| `handoff.clipboard.text` | Accepts `Item::ClipboardText` |

A sender MUST NOT send an item whose capability the receiver did not advertise. A receiver that gets one anyway answers `Declined(Unsupported)`.

### Ping

```
Request  {"Ping": {"nonce": 7}}
Response {"Pong": {"nonce": 7}}
```

### Handoff

```
Request  {"Handoff": {"id": h'<16 random bytes>',
                      "item": {"Link": {"url": "https://example.com/", "title": "Example"}}}}
Request  {"Handoff": {"id": h'...', "item": {"ClipboardText": {"text": "hello"}}}}
Response {"Handoff": {"id": h'<same id>', "outcome": "Accepted"}}
Response {"Handoff": {"id": h'...', "outcome": {"Declined": "TooLarge"}}}
```

`Accepted` means the receiver took the item. What it then does is up to its own policy for the sending device, and the sender is not told (see [security.md](security.md)). `Declined` carries a reason: `Unsupported`, `TooLarge`, `RateLimited`, `Blocked`, `Invalid` or `Other`.

The item id is chosen at random by the sender and reused on retry. A receiver SHOULD remember recently seen ids per peer and answer `Accepted` to a repeat without acting on it again.

Links are untrusted strings. The protocol does not restrict the scheme; receivers decide what they will open (the reference implementation opens only `http` and `https`).

### Errors

A request that cannot be handled at all gets `{"Error": {"code": "Malformed", "message": "..."}}` when a response can still be written. `code` is `Malformed` or `Internal` (or `Other` from a newer version). The message is for logs and MUST NOT be shown to a person as is.

If the frame header itself is unacceptable (zero length or over the limit), the receiver resets the stream with code 400 instead.

### Close codes

| Code | Meaning |
| --- | --- |
| 0 | Normal |
| 400 | Protocol violation |
| 403 | Not paired |
| 408 | Timeout |
| 429 | Too many streams, or sending too fast |
| 500 | Internal error or shutting down |

## 4. Pairing protocol (`baton-pair/1`)

Pairing makes two devices trust each other's keys. It needs a person on the *offering* device, and ties the two devices together with a one-time secret that travels out of band, as a QR code or as text.

### The ticket

The **offerer** makes a ticket:

| Field | Meaning |
| --- | --- |
| offerer | Its device id |
| secret | 32 random bytes, used once |
| expires_at | Unix time in seconds after which the offer is void; the reference implementation uses 5 minutes |
| relay | Optional URL of its home relay |
| addrs | Up to 16 direct addresses (IP and port) |

Binary layout, big-endian:

```
version u8 = 1 | flags u8 = 0 | offerer [32] | secret [32] | expires_at u64
relay_len u8 | relay [relay_len] (printable ASCII) | addr_count u8 (<= 16)
addr * addr_count: tag u8 (4 or 6) | ip [4 or 16] | port u16
checksum [4]  = first 4 bytes of BLAKE3 derive_key("baton 2026-10-14 pairing ticket checksum v1", everything above)
```

Text form: `baton1:` followed by base32 (RFC 4648 alphabet, no padding) of the bytes above. Parsers ignore case and whitespace. The checksum only catches typing and copying errors; it is not a security feature. IPv6 scope ids are not carried, so addresses that need one (link-local) are left out.

The ticket is a **bearer secret**. Anyone who obtains it before it is used or expires can request pairing.

### The exchange

```
Offerer                                          Joiner
   |  shows ticket (QR or text) --------------->  |
   |                                              |
   | <============ QUIC connect, ALPN baton-pair/1 |   joiner dials the offerer's id from the ticket
   |                                              |
   | <------------------- PairRequest ----------- |   {device, proof}
   |   verify proof, ask the person to approve    |
   | -------------------- PairResponse ---------> |   Accepted{device} | Rejected(reason)
   |   store joiner's id                          |   store offerer's id
```

The proof shows that the joiner holds the secret, without sending it:

```
channel_binding = TLS exporter (RFC 5705), label "baton-pair-v1", empty context, 32 bytes
proof = BLAKE3 keyed_hash(key = secret,
          "baton 2026-10-14 pairing proof v1" || offerer_id || joiner_id || channel_binding)
```

The joiner's id is the one authenticated by TLS; the offerer MUST use that value and not one claimed in a message. Binding to the TLS session means a proof recorded on one connection cannot be replayed on another.

### Offerer rules

An offerer MUST:

- accept `baton-pair/1` connections only while an offer is open, and close the offer after one successful pairing, after it expires, or after three invalid proofs;
- compare proofs in constant time;
- have a person approve each request, showing the joiner's sanitised name and key fingerprint; there is no setting that skips this, because a ticket is a bearer secret and the approval is the safeguard against one that leaked;
- handle one request at a time and answer others with `Rejected(Busy)`;
- answer a bad proof, an expired offer and a used offer all the same way, `Rejected(Invalid)`, so a caller who lacks the secret learns nothing.

`Rejected(Declined)` is sent only after a valid proof, when the person said no.

### Joiner rules

A joiner MUST dial only the device id inside the ticket, MUST check that the peer's authenticated id equals it, and MUST store the offerer only after receiving `Accepted`. It SHOULD show the offerer's name and fingerprint so the person can check they paired with the device they meant to.

Pairing is not atomic: if the connection fails after the offerer accepted, the two sides can disagree. Pairing again resolves it.

## 5. Compatibility

Compatible, and allowed within a major version:

- new optional struct fields;
- new capabilities, and new item kinds guarded by them;
- new variants of the extensible enums;
- raising a limit.

Anything else, including a new variant of a non-extensible enum without a capability, lowering a limit, or changing how a proof is computed, needs a new major version and ALPN.

## 6. Limits

| Field | Limit |
| --- | --- |
| Session frame payload | 2 MiB |
| Pairing frame payload | 4 KiB |
| Device name | 256 bytes (shown at most 64 characters) |
| App version | 64 bytes |
| Capabilities | 64 entries of at most 64 bytes |
| Link URL | 8 KiB |
| Link title | 1 KiB |
| Clipboard text | 1 MiB |
| Error message | 1 KiB |
| Ticket text | 2048 characters |

## 7. Test vectors

These are checked by the test suite and were reproduced independently with the reference BLAKE3 implementation.

| Item | Value |
| --- | --- |
| `Request::Ping{nonce: 1}` as a frame | `0000000ea16450696e67a1656e6f6e636501` |
| Fingerprint of device id `00` repeated 32 times | `k65a-25j6-7e2y-gi4f-awnl` |
| Pairing proof for secret `01`×32, offerer `02`×32, joiner `03`×32, binding `09`×32 | `fd5eaa8f3cbf99c3fca24b23d171929262050317c44c1b8a41356dae1616ece5` |
| Ticket for offerer `11`×32, secret `22`×32, `expires_at` 1791965000, relay `https://r/`, address `10.0.0.1:80` | `0100`+`11`×32+`22`×32+`000000006acf3748`+`0a68747470733a2f2f722f`+`01040a0000010050`+`9e781aeb` |
