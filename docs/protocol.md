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
| `baton-pair/1` | Pairing | Anyone, but only a request that proves it holds the ticket's secret is acted on |

The number in the ALPN is the **major** protocol version. Peers with different majors cannot connect. The **minor** revision (currently 0) is exchanged in `Hello` and only informs diagnostics: compatible changes never need negotiation.

A device MUST close a `baton/1` connection from a peer that is not paired, using close code 403, before reading any data from it. A `baton-pair/1` connection cannot be refused on that basis, because a device that asks to pair is by definition not paired yet; section 4 says how such a connection is handled instead. A connection that negotiates any other ALPN is closed with code 400.

## 2. Frames and encoding

Everything after the handshake is a sequence of **frames**:

```
+---------------------------+-------------------------------+
| length: u32, big-endian   | payload: CBOR, `length` bytes |
+---------------------------+-------------------------------+
```

The payload is exactly one message encoded as CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)). Receivers MUST check the length against the limit for the message type before allocating, MUST reject a zero length, and MUST reject a payload that is not exactly one well-formed CBOR item: it may not be cut short and nothing may follow it. They MUST NOT act on any part of a payload they reject. They SHOULD reject CBOR nested deeper than 16 levels.

A receiver reads the whole item before it interprets any of it. That is a requirement, not a matter of style: a decoder that interprets while it reads can be led to accept bytes that are not CBOR at all. (The CBOR library the reference implementation uses did exactly that for an enum variant that was not in its expected form, which is how this rule came to be written down. The regression tests and the fuzz targets in `crates/baton-proto` cover it.)

| Message family | Largest payload |
| --- | --- |
| Session (`Request`, `Response`) | 2 MiB |
| Pairing (`PairRequest`, `PairResponse`) | 4 KiB |

CBOR encoding rules, as produced by the reference implementation:

- Structs are maps with text keys. Receivers ignore unknown keys, so adding an optional field is compatible. Optional fields that are absent are omitted.
- A unit enum variant (one without data) is its name as text (`"Accepted"`). A variant with data is a one-entry map from the name to the data (`{"Ping": {"nonce": 1}}`). Receivers also accept a unit variant as a one-entry map whose value is null (`{"Accepted": null}`), which is how some encoders write it, and reject any other value there.
- Device ids, item ids, secrets and proofs are byte strings of fixed length.
- Enums that a newer version may extend (`Platform`, `DeclineReason`, `ErrorCode`, `PairRejection`) decode an unknown variant as `Other`. That works only for variants without data, so **new variants of these enums MUST be unit variants**: an older receiver cannot skip a variant that carries data, and fails to decode the message. Other enums do not tolerate unknown variants; a request with an unknown variant is answered with `Error { code: Malformed }`.

String and list sizes are limited as well (see section 6). A message that breaks a limit MUST NOT be sent, and MUST be rejected when received.

## 3. Session protocol (`baton/1`)

Each request uses its own bidirectional stream: the client opens a stream, writes one `Request` frame, and finishes its side; the server reads the frame, writes one `Response` frame, and finishes. Either side MAY have several streams open at once. A receiver SHOULD give a peer no more than 16 concurrent streams and SHOULD time out a stream that makes no progress for 10 seconds. These limits are for the control messages of this section, which are small. Bulk transfer (section 8) uses streams of its own with a separate budget, so that a long transfer neither counts against the 16 nor is cut off by a timer meant for a request. Peers do not open unidirectional streams or send datagrams, and a receiver SHOULD NOT allow them. Each receiver also chooses its own QUIC flow-control windows, which cap how much data a sender may have in flight and so how fast one stream can go: a stream cannot carry more than one window per round trip. The reference implementation announces 8 MiB for each stream. It starts every connection with a connection window of only 64 KiB, which is room for fifteen of the largest pairing frame and all that a stranger on `baton-pair/1` ever gets, and raises the connection window to 16 MiB once it has seen that the peer is paired, in both directions, which QUIC lets a receiver do at any time and announces in a `MAX_DATA` frame. That lets one stream fill a path of 500 Mbit/s with a round trip of 100 ms and still bounds the unread data a peer can make a device hold. A sender with more than 64 KiB for a new connection waits about one round trip for the raise, which a client that opens with a `Hello` never notices. The QUIC library's default of about 1.25 MB would cap the same stream at 12.5 MB/s at that round trip.

Both directions use the same protocol: a device that has paired with another may open a connection to it, and the other may open one back. A connection is not tied to one direction of traffic.

A receiver MUST check that the peer is still paired when it reads each request, and not only when the connection was made. Unpairing a device therefore takes effect on connections that are already open: the receiver closes them with code 403, and a request already in flight is not answered.

### Hello

Both sides describe themselves. A client SHOULD send `Hello` on a new connection before anything else, and use the capabilities in the reply to decide what it may send.

```
Request  {"Hello": {"protocol_minor": 0,
                    "device": {"name": "Kitchen laptop", "platform": "Linux", "app_version": "0.1.0"},
                    "capabilities": ["handoff.link", "handoff.clipboard.text"]}}
Response {"Hello": { ... same shape ... }}
```

A paired device can rename itself at any time. The receiver SHOULD remember the name the device reports now, but SHOULD keep showing the name the person chose for it when they paired, and tell the person about the change so they can accept the new name or ignore it. Following a rename silently would let a paired device present itself as another one. Names are chosen by the peer and MUST be sanitised before display (`sanitize_display_name`).

Sanitising removes control characters and every code point with the Unicode property `Default_Ignorable_Code_Point` (zero-width and joiner characters, text-direction controls, variation selectors, the tag characters, the blank Hangul fillers and similar) and the three interlinear annotation characters U+FFF9 to U+FFFB (format characters that are not default ignorable, between which a renderer may hide text), turns the braille blank into a space, collapses runs of whitespace, shortens the result to 64 characters, and gives `unnamed device` if nothing is left. Doing the same twice changes nothing more. A sanitised name is safe to print and to write to a log. It is still only a label chosen by the peer: letters from different scripts can look alike, so a name must never be what a person relies on to tell devices apart. Wherever the difference matters, show the key fingerprint next to it.

Capabilities defined so far:

| Capability | Meaning |
| --- | --- |
| `handoff.link` | Accepts `Item::Link` |
| `handoff.clipboard.text` | Accepts `Item::ClipboardText` |

A sender MUST NOT send an item whose capability the receiver did not advertise. A receiver that gets one anyway answers `Declined(Unsupported)`. A kind of item the receiver does not know at all cannot be decoded, and is answered with `Error { code: Malformed }`: the capability exchange is how new kinds are introduced, not an unannounced request.

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

The item id is chosen at random by the sender and reused when it retries, for example because the connection dropped before the answer arrived. A receiver SHOULD remember the outcome of recently handled ids per peer and give a repeat the same outcome without acting on it again. If the first copy is still being handled, the repeat waits for its outcome. The reference implementation remembers 1024 ids for 10 minutes.

A receiver SHOULD check an item in this order: whether it supports the kind of item, whether it is within the size limit, whether it is a repeat, and only then the rate limit. A repeat is answered from memory and is not counted against the rate limit, so a sender that retries after a lost reply is never turned away for an item that was in fact taken. Refusals made before the item reaches the application (`Unsupported`, `TooLarge`, `RateLimited`) are not remembered, so a sender that has fixed the problem can try the same item again.

Links are untrusted strings. The protocol does not restrict the scheme; receivers decide what they will open (the reference implementation opens only `http` and `https`). A link can carry a secret in its query or path, so implementations SHOULD log no more of one than its scheme and host.

### Errors

A request that cannot be handled at all gets `{"Error": {"code": "Malformed", "message": "..."}}` when a response can still be written. `code` is `Malformed` or `Internal` (or `Other` from a newer version). The message is for logs and MUST NOT be shown to a person as is.

If the frame header itself is unacceptable (zero length or over the limit), the receiver resets the stream with code 400 instead, and with code 408 if the request does not arrive within the deadline.

### Close codes

The same codes are used to close a connection and to reset a stream.

| Code | Meaning |
| --- | --- |
| 0 | Normal |
| 400 | Protocol violation |
| 403 | Not paired |
| 408 | Timeout |
| 429 | Too many connections or pairing attempts at once, or a pairing connection that was pushed out by a newer one. A sender that is too fast is told so in the response (`Declined(RateLimited)`), not by closing |
| 500 | Internal error or shutting down |

## 4. Pairing protocol (`baton-pair/1`)

Pairing makes two devices trust each other's keys. It needs a person on the *offering* device, and ties the two devices together with a one-time secret that travels out of band, as a QR code or as text. The person then confirms that the request comes from the device in their hand by comparing a six-digit code that both devices show.

### The ticket

The **offerer** makes a ticket:

| Field | Meaning |
| --- | --- |
| offerer | Its device id |
| secret | 32 random bytes, used once |
| expires_at | Unix time in seconds from which the ticket no longer works; the reference implementation uses 5 minutes by default |
| relay | Optional URL of its home relay |
| addrs | Up to 16 direct addresses (IP and port) |
| extensions | Optional further fields; none are defined yet |

Binary layout, big-endian:

```
version u8 = 1 | flags u8 | offerer [32] | secret [32] | expires_at u64
relay_len u8 | relay [relay_len] (printable ASCII) | addr_count u8 (<= 16)
addr * addr_count: tag u8 (4 or 6) | ip [4 or 16] | port u16
extension * n, up to the checksum: type u8 | len u8 | value [len]
checksum [4]  = first 4 bytes of BLAKE3 derive_key("baton 2026-10-14 pairing ticket checksum v1", everything above)
```

Text form: `BATON1:` followed by base32 (RFC 4648 alphabet, no padding) of the bytes above, written in capital letters. Digits, capitals and `:` are the characters a QR code can store in its dense alphanumeric mode, so a ticket in this form makes a smaller code that is easier to scan off a screen than one in lower case. Parsers accept any case and ignore whitespace. The checksum only catches typing and copying errors; it is not a security feature. IPv6 scope ids are not carried, so addresses that need one (link-local) are left out.

**Growing the format.** Tickets are printed, photographed and shown on screens, so a phone running last month's version will meet tickets made this month. Fields can therefore be added without a new version number. The low four bits of `flags` and extension types below 0x80 are *ignorable*: a reader that does not know one skips it. The high four bits of `flags` and extension types from 0x80 up are *critical*: a reader that does not know one MUST refuse the ticket and tell the person to update the app. A ticket has at most 16 extensions. A change that an older reader cannot ignore without harm, such as a different use of the secret, needs a new version number and a new prefix. No flag and no extension is defined yet.

A reader that meets a later version MUST tell the person to update the app, and MUST NOT say that the ticket is damaged or is not a Baton ticket. It recognises a later version in either of two places. The prefix is `BATON` followed by a version number above 1, written without leading zeros, and a colon (`BATON2:`, `BATON10:`); nothing after the colon is read, because a later version may encode it differently. The first byte of the binary layout is a version above 1, in a ticket whose checksum is right. The second only works if every version ends with the same four checksum bytes, made the same way with the context above, so that an older reader can check the checksum of a ticket it cannot otherwise read. A version that changes how the checksum is made MUST also change the prefix, because the prefix is the only part an older reader can recognise without the checksum. A checksum that does not match is damage, whatever the version byte says. Version 0 has never been used and is invalid.

**The relay hint.** The relay URL is an address the joiner will connect to and it comes from a scanned code, so it is untrusted. It MUST be an `http` or `https` URL of at most 255 bytes with a host, without a user name or password, in ASCII, and a reader MUST reject a ticket in which it is not. That is a structural check only: a joiner still parses the URL properly before it connects. A relay that a device remembers about a paired peer, and will connect to later, is held to the same rule.

**Expiry.** The offerer alone decides whether an offer is still open. It measures time with a monotonic clock, so a jump of its wall clock neither extends an offer nor ends it early. For the joiner, `expires_at` is a hint. A joiner whose clock says the ticket ran out more than 10 minutes ago MAY tell the person so instead of connecting. Otherwise it MUST try, because the clocks of two devices can differ by minutes and the offerer's answer is the one that counts. The deadline decides whether a request may be put to the person. A request that arrived before it is answered after it, within the time the person is given, and the offerer MUST NOT take the question away from them when the deadline passes. Requests that arrive after the deadline get `Rejected(Invalid)`.

The ticket is a **bearer secret**. Anyone who obtains it before it is used or expires can request pairing. Implementations SHOULD keep it out of logs and wipe their copies of it from memory.

### The exchange

```
Offerer                                          Joiner
   |  shows ticket (QR or text) --------------->  |
   |                                              |
   | <============ QUIC connect, ALPN baton-pair/1 |   joiner dials the offerer's id from the ticket
   |                                              |
   | <------------------- PairRequest ----------- |   {device, proof}
   |   verify proof; show the person the joiner,  |   shows the confirmation code
   |   its fingerprint and the confirmation code, |   while it waits
   |   ask them to approve                        |
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

### The confirmation code

A proof tells the offerer that the joiner has the ticket. It cannot tell the person who has to approve the request whether that joiner is the device in their hand or a stranger who copied the ticket, because both prove it equally well. The joiner's name means nothing, since a stranger can choose any name, and its key fingerprint means nothing to a person who has never seen it. So both devices also work out a six-digit code and show it, and the person approves only if the two codes are the same:

```
code = (first 4 bytes, big-endian, of
          BLAKE3 derive_key("baton 2026-10-14 pairing confirmation v1",
                            secret || offerer_id || joiner_id || channel_binding))
       mod 1,000,000
```

It is shown as two groups of three digits with leading zeros kept, for example `042 917`. Nothing about the code is sent. It depends on the secret, on both keys and on the TLS session, so a stranger's request produces a different code from the one on the person's own device, and nobody can work it out before the session exists. A stranger has one chance in a million of matching it by guessing, and the attempt shows the person a prompt they did not expect.

The offerer MUST show the code with the request, and tell the person to approve only if the same code is shown on the device they are pairing. The joiner MUST show the code while it waits for the answer. A person who sees no code on their own device, or a different one, declines.

Six digits are enough because each offer allows a single approval prompt (see the rules below). They would not be enough against an attacker who could try a million sessions unobserved, and nothing in the design allows that.

### Offerer rules

An offerer MUST:

- accept `baton-pair/1` connections from anyone, whether or not an offer is open, and read at most one request from each. Whether an offer exists is not something a stranger may learn;
- act on a request only if an offer is open and has not expired and the proof is valid. Proofs are compared in constant time;
- answer every other request in the same way, `Rejected(Invalid)`: a wrong proof, an expired or used offer, no offer at all, or a payload that cannot be decoded. It SHOULD take the same time to do so, for example by always waiting a fixed short moment before answering, so that a caller who lacks the secret learns nothing (the reference implementation holds the answer back for 250 ms);
- have a person approve each request that carries a valid proof, showing the joiner's sanitised name, its key fingerprint and the confirmation code. There is no setting that skips this, because a ticket is a bearer secret and the approval is the safeguard against one that leaked. A request the person does not answer in time counts as declined (the reference implementation waits 2 minutes);
- handle one valid request at a time. A second one with a valid proof, while the first is waiting for the person, is answered `Rejected(Busy)`. Only someone holding the secret can see this answer, so it tells everyone else nothing. The proof is checked before the approval slot is taken, so a request without it can never occupy the slot;
- close the offer after one successful pairing, after the person declines or does not answer, when the joiner disconnects while the person is deciding, after it expires, and when it is withdrawn. Declining ends the offer on purpose: the person has just seen a request they did not expect, so the ticket may have leaked, and it must not stay usable. A joiner that disconnects while the question is on the screen ends the offer for the same reason, because the person never got to decline it, and an offer that stayed open would let a holder of the ticket raise any number of prompts, each with a new code, which would defeat the argument for six digits. An offer therefore puts one request to the person. If the offer is withdrawn while a person is deciding, the question is dropped and the joiner is told `Rejected(Invalid)`.

A wrong proof does not close the offer, and wrong proofs are not counted. The secret has 256 bits, so guessing it is not a realistic attack, while letting a few wrong proofs void the offer would hand a cheap trick to anyone who knows the offerer's device id, which a device announces on its network: they could cancel every pairing as it starts.

What an offerer SHOULD limit is its own load: how many pairing connections it serves at once, how long each may last, and how often one peer key or address may ask. These limits protect the device and say nothing about the secret. The reference implementation serves 8 pairing connections at once. When all 8 places are taken, a new connection takes the place of the oldest one that has not shown a valid proof, which is closed with code 429; a new connection is refused with 429 only when every place holds a connection that has. A connection that has not delivered its request within 10 seconds of being made, which includes opening the stream, is closed with 408 whether or not any other connection is waiting. It closes a pairing connection with code 0 once the answer has been delivered, and with 400 if the frame header breaks the limits. Both sides SHOULD turn on QUIC keep-alives, because the exchange can sit idle for as long as a person takes to decide. A future way of pairing with a short code that a person types would be guessable, so it would need a password-authenticated key exchange and strict limits on attempts. It would be a new protocol (section 8), not a change to this one.

`Rejected(Declined)` is sent only after a valid proof, when the person said no or did not answer in time.

### Joiner rules

A joiner MUST dial only the device id inside the ticket, MUST check that the peer's authenticated id equals it, and MUST store the offerer only after receiving `Accepted`. It MUST show the confirmation code while it waits, and SHOULD show the offerer's name and fingerprint so the person can check they paired with the device they meant to. It waits for the answer for as long as the offerer may wait for the person, plus time for the network (the reference implementation waits 2 minutes 30 seconds).

Pairing is not atomic: if the connection fails after the offerer accepted, the two sides can disagree. An offerer SHOULD store the joiner before it sends `Accepted`, so that a failure leaves it trusting a device that does not trust it back, which is harmless, and never the other way round. The reference implementation does. Pairing again resolves it.

## 5. Compatibility

Compatible, and allowed within a major version:

- new optional struct fields;
- new capabilities, and new item kinds guarded by them;
- new unit variants of the extensible enums;
- new ignorable ticket flags and extensions;
- raising a limit.

Critical ticket flags and extensions are allowed within a major version too, but an older reader refuses such a ticket, so they are only for things that must not be ignored.

Anything else, including a new variant of a non-extensible enum without a capability, a new variant with data in an extensible enum, lowering a limit, or changing how a proof or a confirmation code is computed, needs a new major version and ALPN.

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
| Ticket relay hint | 255 bytes |
| Ticket addresses | 16 |
| Ticket extensions | 16 |

The reference implementation also applies the following by default. They are not part of the wire format and a receiver may choose differently; they are listed so that implementers know what a peer is likely to expect.

| Behaviour | Default |
| --- | --- |
| Concurrent streams a peer may open on a connection | 16 |
| Unread data a stream may hold, and so how much a sender may have in flight on it | 8 MiB |
| Unread data a connection may hold | 64 KiB, raised to 16 MiB once the peer is known to be paired |
| Connection attempts let through before a QUIC retry token is asked for, from addresses, and relayed keys that are not paired, that have not proven themselves | 64 a second, together |
| Connection attempts from an address that has answered a retry token (for IPv6, from one /64) | 20 every 10 seconds |
| Connection attempts through a relay from the key of one paired device | 20 every 10 seconds |
| Connection attempts through a relay from keys that are not paired and have answered a retry token | 20 every 10 seconds, all together |
| Time for a request to arrive once its stream is open. On a pairing connection the time runs from when the connection was made, so opening the stream counts | 10 seconds |
| Time to wait for the response to a request | 30 seconds |
| Handoff ids remembered | 1024, for 10 minutes |
| Items accepted from one paired device | 30 a minute (a setting, at most 600) |
| Clipboard text accepted | 256 KiB (a setting) |
| Connections kept open to or from one paired device | 8; the oldest are closed with 429 beyond that |
| Idle session connection closed after | 2 minutes |
| Pairing connections served at once | 8; a newcomer pushes out the oldest one that has shown no valid proof |
| Time a person has to approve a pairing request | 2 minutes |
| Lifetime of a pairing ticket | 5 minutes (a setting, from 30 seconds to 15 minutes) |
| QUIC keep-alive and idle timeout | 10 seconds and 30 seconds |

## 7. Test vectors

These are checked by the test suite and were reproduced independently with the reference BLAKE3 implementation.

| Item | Value |
| --- | --- |
| `Request::Ping{nonce: 1}` as a frame | `0000000ea16450696e67a1656e6f6e636501` |
| Fingerprint of device id `00` repeated 32 times | `k65a-25j6-7e2y-gi4f-awnl` |
| Pairing proof for secret `01`×32, offerer `02`×32, joiner `03`×32, binding `09`×32 | `fd5eaa8f3cbf99c3fca24b23d171929262050317c44c1b8a41356dae1616ece5` |
| Confirmation code for the same secret, offerer, joiner and binding | `196 465` |
| Confirmation code with joiner `04`×32 and everything else the same | `213 543` |
| Confirmation code for secret `2b`×32, offerer `02`×32, joiner `03`×32, binding `fb`×32 | `000 045` |
| Ticket for offerer `11`×32, secret `22`×32, `expires_at` 1791965000, relay `https://r/`, address `10.0.0.1:80` | `0100`+`11`×32+`22`×32+`000000006acf3748`+`0a68747470733a2f2f722f`+`01040a0000010050`+`9e781aeb` |
| The same ticket with the ignorable extension type `01`, value `aabbcc`, before the checksum | the bytes above up to `…0050`, then `0103aabbcc`, then `535c7c97`; read as the ticket above, the extension skipped |
| The same ticket with the critical extension type `80`, value `aabb` | the bytes above up to `…0050`, then `8002aabb`, then `2a0dfea9`; refused: needs a newer version |
| The same ticket with flags `10` (critical) | as the ticket above with `0110` at the start and checksum `bcbeeb70`; refused: needs a newer version |
| The text form of the ticket above, with `BATON2:` in place of `BATON1:` | refused: needs a newer version, whatever follows the colon |

## 8. Planned extensions

Nothing in this section is part of the protocol yet. It is written down so that the rules above leave room for it, and so that the reasons are on record before any of it is built.

- **File transfer** (milestone 1). Files and folders move on streams of their own, with their own flow-control budget and a lower send priority than the control streams of section 3, so that a transfer never delays a ping or a handoff. The sender offers a whole file or folder at once, with a manifest of names, sizes and hashes, and the receiver decides once; the chunks that follow do not each count against the rate limit on items or the limit on streams. Liveness is judged on progress over time, not on a fixed deadline for each stream. The sender hashes while it sends, so a large file needs no extra pass first. It is guarded by its own capability.
- **Pull.** Besides pushing an item to a device, a device can ask a paired device what it has ready to continue (`ListActivities`) and fetch one of them (`Fetch`). That lets a device that was asleep or locked when something was sent catch up when it wakes, and lets a phone pick up what was on a laptop without the laptop pushing it. Both requests are guarded by a capability, so older peers never see them. A device answers them while it is locked only if the person has turned that on for the asking device; it is off by default. See [Locked devices](architecture.md#locked-devices).
- **A short code that a person types**, for pairing devices that can neither show nor scan a QR code. As explained in section 4, that needs a password-authenticated key exchange and a new ALPN.
