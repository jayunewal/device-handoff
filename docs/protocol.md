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
| A piece of the list of files of a transfer (`ListChunk`, on a lane, section 8) | 2 MiB |
| A message on a control stream of a streaming session (`SourceMessage`, `ViewerMessage`, section 9) | 64 KiB |

CBOR encoding rules, as produced by the reference implementation:

- Structs are maps with text keys. Receivers ignore unknown keys, so adding an optional field is compatible. Optional fields that are absent are omitted.
- A unit enum variant (one without data) is its name as text (`"Accepted"`). A variant with data is a one-entry map from the name to the data (`{"Ping": {"nonce": 1}}`). Receivers also accept a unit variant as a one-entry map whose value is null (`{"Accepted": null}`), which is how some encoders write it, and reject any other value there.
- A number that can have a fraction (only the positions and scroll steps of section 9 are such) is a CBOR float of any width: half, single or double. A receiver does not accept an integer in its place, so an encoder that writes a whole number as an integer has to write `0.0` and not `0`, or the receiver cannot read the message, which section 9 makes a violation.
- Device ids, item ids, secrets and proofs are byte strings of fixed length.
- Enums that a newer version may extend (`Platform`, `DeclineReason`, `ErrorCode`, `PairRejection`, `FailReason`, `SourceKind`, `Codec`, `StopReason`, the reasons of a `Pause`, the thermal states of a `Report` and the reasons of a `Notice`) decode an unknown variant as `Other`. That works only for variants without data, so **new variants of these enums MUST be unit variants**: an older receiver cannot skip a variant that carries data, and fails to decode the message. Other enums do not tolerate unknown variants; a request with an unknown variant is answered with `Error { code: Malformed }`.

String and list sizes are limited as well (see section 6). A message that breaks a limit MUST NOT be sent, and MUST be rejected when received.

## 3. Session protocol (`baton/1`)

Each request uses its own bidirectional stream: the client opens a stream, writes one `Request` frame, and finishes its side; the server reads the frame, writes one `Response` frame, and finishes. Either side MAY have several streams open at once. A receiver SHOULD give a peer no more than 16 concurrent streams and SHOULD time out a stream that makes no progress for 10 seconds. These limits are for the control messages of this section, which are small. Bulk transfer (section 8) uses streams of its own with a separate budget, so that a long transfer neither counts against the 16 nor is cut off by a timer meant for a request. Peers send no datagrams, and open no unidirectional streams except the lanes of a transfer that the receiver has accepted (section 8) and the frames and control streams of a session of window streaming that it has accepted (section 9). A receiver allows a peer that is not paired no unidirectional stream at all, and one that is paired no more than the sum of the counts of the kinds of stream it takes (section 9, Taking a stream). The reference implementation allows none to anyone until it takes a kind of stream, and then raises the limit to that sum for a peer it knows to be paired, in the place where it raises the connection window. Each receiver also chooses its own QUIC flow-control windows, which cap how much data a sender may have in flight and so how fast one stream can go: a stream cannot carry more than one window per round trip. The reference implementation announces 8 MiB for each stream. It starts every connection with a connection window of only 64 KiB, which is room for fifteen of the largest pairing frame and all that a stranger on `baton-pair/1` ever gets, and raises the connection window to 16 MiB once it has seen that the peer is paired, in both directions, which QUIC lets a receiver do at any time and announces in a `MAX_DATA` frame. That lets one stream fill a path of 500 Mbit/s with a round trip of 100 ms and still bounds the unread data a peer can make a device hold. A sender with more than 64 KiB for a new connection waits about one round trip for the raise, which a client that opens with a `Hello` never notices. The QUIC library's default of about 1.25 MB would cap the same stream at 12.5 MB/s at that round trip.

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
| `transfer.files` | Takes part in file transfers: accepts the requests and the lanes of section 8 |
| `stream.share` | Can offer a window to a paired device, and takes part in the session that follows as the source; accepts a `Stop` for that session (section 9) |
| `stream.view` | Can show the window of a paired device: accepts an `Offer` (section 9), and takes part in the session that follows as the viewer; accepts a `Stop` for that session |
| `stream.control` | Takes input from a viewer, in a session where the person at the source granted control (section 9) |

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

`Accepted` means the receiver took the item. What it then does is up to its own policy for the sending device, and the sender is not told (see [security.md](security.md)). `Declined` carries a reason: `Unsupported`, `TooLarge`, `RateLimited`, `Blocked`, `Invalid` or `Other`. Transfers (section 8) use two more, `NoSpace` and `Refused`.

The item id is chosen at random by the sender and reused when it retries, for example because the connection dropped before the answer arrived. A receiver SHOULD remember the outcome of recently handled ids per peer and give a repeat the same outcome without acting on it again. If the first copy is still being handled, the repeat waits for its outcome. The reference implementation remembers 1024 ids for 10 minutes.

A receiver SHOULD check an item in this order: whether it supports the kind of item, whether it is within the size limit, whether it is a repeat, and only then the rate limit. A repeat is answered from memory and is not counted against the rate limit, so a sender that retries after a lost reply is never turned away for an item that was in fact taken. Refusals made before the item reaches the application (`Unsupported`, `TooLarge`, `RateLimited`) are not remembered, so a sender that has fixed the problem can try the same item again.

Links are untrusted strings. The protocol does not restrict the scheme; receivers decide what they will open (the reference implementation opens only `http` and `https`). A link can carry a secret in its query or path, so implementations SHOULD log no more of one than its scheme and host.

### Errors

A request that cannot be handled at all gets `{"Error": {"code": "Malformed", "message": "..."}}` when a response can still be written. `code` is `Malformed`, `Internal` or `NotFound` (the request names something the receiver does not have, such as a transfer it never saw, section 8), or `Other` from a newer version. The message is for logs and MUST NOT be shown to a person as is.

If the frame header itself is unacceptable (zero length or over the limit), the receiver resets the stream with code 400 instead, and with code 408 if the request does not arrive within the deadline.

### Close codes

The same codes are used to close a connection and to reset a stream.

| Code | Meaning |
| --- | --- |
| 0 | Normal |
| 400 | Protocol violation |
| 403 | Not paired |
| 408 | Timeout |
| 413 | What the peer sends is larger than this device accepts: a list of files over its limit (section 8), or the picture of a frame of video over its limit (section 9) |
| 429 | Too many connections or pairing attempts at once, or a pairing connection that was pushed out by a newer one. A sender that is too fast is told so in the response (`Declined(RateLimited)`), not by closing |
| 499 | A transfer was stopped on purpose, by the person or by the program. Used to stop or reset a lane (section 8) |
| 500 | Internal error or shutting down |
| 507 | This device has no room for what the peer is sending: used to stop a lane (section 8) |

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
- handle one valid request at a time. A second one with a valid proof, while the first is waiting for the person, is answered `Rejected(Busy)`. Only someone holding the secret can see this answer, so it tells everyone else nothing, and it tells the joiner that another holder of the ticket is being asked (see the joiner rules). The proof is checked before the approval slot is taken, so a request without it can never occupy the slot;
- close the offer after one successful pairing, after the person declines or does not answer, when the joiner disconnects while the person is deciding, after it expires, and when it is withdrawn. Declining ends the offer on purpose: the person has just seen a request they did not expect, so the ticket may have leaked, and it must not stay usable. A joiner that disconnects while the question is on the screen ends the offer for the same reason, because the person never got to decline it, and an offer that stayed open would let a holder of the ticket raise any number of prompts, each with a new code, which would defeat the argument for six digits. An offer therefore puts one request to the person. If the offer is withdrawn while a person is deciding, the question is dropped and the joiner is told `Rejected(Invalid)`.

A wrong proof does not close the offer, and wrong proofs are not counted. The secret has 256 bits, so guessing it is not a realistic attack, while letting a few wrong proofs void the offer would hand a cheap trick to anyone who knows the offerer's device id, which a device announces on its network: they could cancel every pairing as it starts.

What an offerer SHOULD limit is its own load: how many pairing connections it serves at once, how long each may last, and how often one peer key or address may ask. These limits protect the device and say nothing about the secret. The reference implementation serves 8 pairing connections at once. When all 8 places are taken, a new connection takes the place of the oldest one that has not shown a valid proof, which is closed with code 429; a new connection is refused with 429 only when every place holds a connection that has. A connection that has not delivered its request within 10 seconds of being made, which includes opening the stream, is closed with 408 whether or not any other connection is waiting. It closes a pairing connection with code 0 once the answer has been delivered, and with 400 if the frame header breaks the limits. Both sides SHOULD turn on QUIC keep-alives, because the exchange can sit idle for as long as a person takes to decide. A future way of pairing with a short code that a person types would be guessable, so it would need a password-authenticated key exchange and strict limits on attempts. It would be a new protocol (planned in section 10), not a change to this one.

`Rejected(Declined)` is sent only after a valid proof, when the person said no or did not answer in time.

### Joiner rules

A joiner MUST dial only the device id inside the ticket, MUST check that the peer's authenticated id equals it, and MUST store the offerer only after receiving `Accepted`. It MUST show the confirmation code while it waits, and SHOULD show the offerer's name and fingerprint so the person can check they paired with the device they meant to. It waits for the answer for as long as the offerer may wait for the person, plus time for the network (the reference implementation waits 2 minutes 30 seconds).

A joiner that is answered `Rejected(Busy)` MUST NOT try again with the same ticket, and SHOULD tell the person what the answer means. An offer puts one request to the person, and every way that request ends spends the offer, so a second try gets `Busy` again while the request waits and `Rejected(Invalid)` after it. `Busy` says that another device holding the ticket is being asked about right now. Unless the person started that request themselves, someone else has seen the ticket, so the person should decline the request on the offering device and make a new ticket.

Pairing is not atomic: if the connection fails after the offerer accepted, the two sides can disagree. An offerer SHOULD store the joiner before it sends `Accepted`, so that a failure leaves it trusting a device that does not trust it back, which is harmless, and never the other way round. The reference implementation does. Pairing again resolves it.

## 5. Compatibility

Compatible, and allowed within a major version:

- new optional struct fields;
- new capabilities, and new item kinds guarded by them;
- new unit variants of the extensible enums, and new reasons for skipping a file (a byte of a lane that an older version reads as `Other`);
- new ignorable ticket flags and extensions;
- raising a limit.

Critical ticket flags and extensions are allowed within a major version too, but an older reader refuses such a ticket, so they are only for things that must not be ignored.

Anything else, including a new variant of a non-extensible enum without a capability, a new variant with data in an extensible enum, lowering a limit, changing how a proof or a confirmation code is computed, or changing how the hash of a file or of a list is made, needs a new major version and ALPN. A new kind of unidirectional stream, or a new tag of a frame on a lane, needs a capability that the other side advertised before it is used: a receiver cannot know how long a frame with a tag it does not know is, so it treats one as a violation (section 8).

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
| Top-level items that an offer shows by name | 16 |
| Files in one transfer | 100,000 |
| Folders in one transfer | 100,000 |
| Entries in one piece of the list of files | 1024 |
| The list of files, in the form that is hashed | 16 MiB |
| Name of a file or folder | 255 bytes |
| Path in a transfer | 1024 bytes and 64 names |
| One file in a transfer | 2^50 bytes (1 PiB) |
| Lanes of one transfer | 8 |
| The picture of one frame of video, header excluded | 8 MiB |
| Title of a shared window | 512 bytes |
| Name of the application of a shared window | 256 bytes |
| Each edge of a shared window | 16384 pixels |
| Each edge of a picture, and of the largest picture that a viewer says it can take or wants | 3840 pixels |
| The frame rate that a viewer says it can take or wants, and that a source sends | 1 to 120 a second |
| Codecs in an `Accepted` | 8 |
| A message on a control stream | 64 KiB |
| Events in one batch of input | 64 |
| Events of input that a viewer sends | 240 a second |
| The text that a key produced | 20 UTF-16 units |
| The code of a key | 0 to 0xE7 |
| One step of a scroll | 10,000 points in either direction |
| Files that `Done` names | the first 256 that failed, and a count of the rest |
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
| Frame streams that a viewer reads at once. One beyond the count is stopped with code 499 and not read | 12 |
| Bytes of pictures that a viewer holds, those that streams are still delivering and those that wait to be decoded. A stream that would take it beyond the limit is stopped with code 499 and not read any further, because the bytes that a viewer has read no longer count against the connection window | 16 MiB, the connection window |
| Time that a frame stream may deliver nothing, and that the rest of a message of a control stream may take to arrive once its length has. A stream that is slower is stopped with code 408 | 10 seconds |
| Time that a source has to open its control stream once the viewer has accepted. A viewer that has not taken one by then ends the session | 10 seconds |
| Time between two reports of a viewer | 500 milliseconds |
| Connection attempts let through before a QUIC retry token is asked for, from addresses, and relayed keys that are not paired, that have not proven themselves | 64 a second, together |
| Connection attempts from an address that has answered a retry token (for IPv6, from one /64) | 20 every 10 seconds |
| Connection attempts through a relay from the key of one paired device | 20 every 10 seconds |
| Connection attempts through a relay from keys that are not paired and have answered a retry token | 20 every 10 seconds, all together |
| Time for a request to arrive once its stream is open. On a pairing connection the time runs from when the connection was made, so opening the stream counts | 10 seconds |
| Time for the start of a unidirectional stream to arrive once the receiver has accepted it | 10 seconds |
| Time to wait for the response to a request | 30 seconds |
| Handoff ids remembered | 1024, for 10 minutes |
| Items accepted from one paired device | 30 a minute (a setting, at most 600) |
| Clipboard text accepted | 256 KiB (a setting) |
| Connections kept open to or from one paired device | 8; the oldest are closed with 429 beyond that |
| Idle session connection closed after | 2 minutes without a request. A connection that a transfer or a window that is shown is using is not idle, and the time starts again when it lets go |
| Pairing connections served at once | 8; a newcomer pushes out the oldest one that has shown no valid proof |
| Time a person has to approve a pairing request | 2 minutes |
| Lifetime of a pairing ticket | 5 minutes (a setting, from 30 seconds to 15 minutes) |
| QUIC keep-alive and idle timeout | 10 seconds and 30 seconds |

The reference implementation takes part as a receiver once the application gives it a folder to receive into, and advertises `transfer.files` for as long as it has one. It sends files with `Node::send_files`, to the times below. It takes part as a viewer of windows once the application gives it a handler for them, and advertises `stream.view` for as long as it has one. It takes part as a source of windows once the application sets it up to show its own, and advertises `stream.share` for as long as it is set up, and `stream.control` as well when it is set up to take input; `Node::share` makes the offer. These defaults are not part of the wire format either.

| Behaviour | Default |
| --- | --- |
| Unidirectional streams a peer may open on a connection | 0, raised once the peer is known to be paired to the sum of the counts that the device allows for the kinds of stream it takes (8 for lanes) |
| Transfers a paired device may have going at once, counted across its connections | 2; a further offer is declined with `RateLimited` |
| Lanes granted to a transfer | 4 |
| Time a person has to answer an offer | 2 minutes |
| Time a sender waits for the answer to an offer, and for `Done` | 2 minutes 30 seconds, and 2 minutes |
| Time without a byte moving, after which a transfer is stopped | 30 seconds |
| Decision on an offer kept | for an accepted offer, as long as what was received for it is kept; for a refusal by the person or by policy, at least 10 minutes |

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

### File transfer

The frame of an `Offer` with transfer id `42`×16, 2 top-level items (the folder `Holiday` with 2 files and 1200 bytes, and the file `notes.txt` of 34 bytes), 3 files, 1 folder, 1234 bytes, and the list hash `00 01 02 … 1f`:

```
000000c2a1685472616e73666572a1654f66666572a462696450424242424242424242424242424242426773756d6d617279a4656974656d73026566696c65730367666f6c64657273016562797465731904d2677072657669657782a4646e616d6567486f6c6964617966666f6c646572f56566696c6573026562797465731904b0a2646e616d65696e6f7465732e7478746562797465731822696c6973745f686173685820000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f
```

| Item | Value |
| --- | --- |
| Header of a lane for transfer id `a0 a1 … af` and lane 3 | `01a0a1a2a3a4a5a6a7a8a9aaabacadaeaf03` |
| Head of a `List` frame | `01` |
| Head of a `Span` frame, file `0x01020304`, span `0x0a0b0c0d` | `02010203040a0b0c0d` |
| Head of an `End` frame, file 5, hash `00 01 02 … 1f` | `0300000005000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f` |
| Head of a `Skip` frame, file 6, reason `Changed` | `040000000602` |

The BLAKE3 hash of a file whose byte at place *i* is *i* mod 251, which is the value `b3sum` prints, for the sizes where the shape of the tree changes:

| Size in bytes | Hash |
| --- | --- |
| 0 | `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262` |
| 1 | `2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213` |
| 1048576 (one span) | `74cb441fd087764ca9c3694da742ebe30cbeb3060a17009ca81825c7a8d10343` |
| 1048577 (one span and one byte) | `2f053cd7472cf0cd2f9adaf45c1180255b91b9a865404a63671a0ee5f792ed33` |
| 2097155 (two spans and three bytes) | `891dfce8b8f6360c474d71183da59574306e74157a5ce6cbcd12933cc78c819f` |
| 5242881 (five spans and one byte) | `a2eed482c537df008175a978a474f0a2f257d3ef11f7ac1f6fa9f5e49af4d345` |

The list hash of four entries: the folder `Holiday`, the file `Holiday/a.txt` of 5 bytes modified at 1700000000000, the file `b` of 0 bytes with no time, and the file `Ärger.txt` (10 bytes in UTF-8) of 1024 bytes modified at -1. The 107 bytes that are hashed, one entry to a line (the spaces are for reading):

```
01 0000000000000000 0000000000000000 0007 486f6c69646179
02 0000000000000005 0000018bcfe56800 000d 486f6c696461792f612e747874
00 0000000000000000 0000000000000000 0001 62
02 0000000000000400 ffffffffffffffff 000a c384726765722e747874
```

Their hash, BLAKE3 in its key derivation mode with the context `baton 2026-10-15 transfer list v1`: `995bf65ca3365c2cfb21667b2099730a378325363410779c479a102f69eefb99`.

### Window streaming

| Item | Value |
| --- | --- |
| Start of a frame stream for session `a0 a1 … af` | `02a0a1a2a3a4a5a6a7a8a9aaabacadaeaf` |
| Header of key frame 5 (depends on itself), chain 1, epoch 0, captured at 200000 µs, flags `05` (key frame, referenced), for that session | the start above, then `0000000000000005` `0000000000000005` `00000001` `00000000` `0000000000030d40` `05` |
| Header of frame `0102030405060708`, depends on `0102030405060707`, chain `0a0b0c0d`, epoch `11121314`, captured at `2122232425262728` µs, flags `04` (referenced), for that session | the start above, then `0102030405060708` `0102030405060707` `0a0b0c0d` `11121314` `2122232425262728` `04` |

The frame of an `Offer` for session `a0 a1 … af` that shows the window `Report.pdf` of `Preview`, 1440 by 900 pixels, with control granted. Without control the key `control` and its value are left out, which makes the frame nine bytes shorter, and it starts `00000067`:

```
00000070a16653747265616da1654f66666572a76773657373696f6e50a0a1a2a3a4a5a6a7a8a9aaabacadaeaf66736f757263656657696e646f77657469746c656a5265706f72742e7064666361707067507265766965776577696474681905a06668656967687419038467636f6e74726f6cf5
```

The frame of the `Accepted` that answers it, for a viewer that takes pictures up to 2732 by 2048 at 60 frames a second and decodes H.264:

```
0000005ea16653747265616da1684163636570746564a56773657373696f6e50a0a1a2a3a4a5a6a7a8a9aaabacadaeaf696d61785f7769647468190aac6a6d61785f686569676874190800676d61785f667073183c66636f64656373816448323634
```

The frames of the messages on the control streams. Each is a frame of section 2, a length of 4 bytes and one CBOR item, and a control stream carries them behind the 17 bytes of its start:

| Message | Frame |
| --- | --- |
| `Config`: the source tells the viewer about a picture of 1440 by 900 pixels at up to 60 frames a second, epoch 1, H.264 | `00000034a166436f6e666967a56565706f63680165636f64656364483236346577696474681905a06668656967687419038463667073183c` |
| `Pause`: the Mac is locked | `00000016a1655061757365a166726561736f6e664c6f636b6564` |
| `Resume`: pictures go on (a unit variant is its name as text) | `0000000766526573756d65` |
| `Notice`: 3 events were dropped because another window was in front of them | `0000001fa1664e6f74696365a266726561736f6e67436f766572656465636f756e7403` |
| `Report`: frame 41 is shown, decoding takes 2.5 ms, 3 frames were dropped so far, the device is `Fair` | `00000034a1665265706f7274a46573686f776e1829696465636f64655f75731909c46764726f707065640367746865726d616c6446616972` |
| `Report` before the first picture: the same without `shown`, which is left out, so the frame is 8 bytes shorter | `0000002ca1665265706f7274a3696465636f64655f75731909c46764726f707065640367746865726d616c6446616972` |
| `Want`: the viewer can take pictures up to 2796 by 1290 pixels at up to 120 frames a second | `00000020a16457616e74a3657769647468190aec6668656967687419050a636670731878` |
| `KeyFrame`: the newest frame that the viewer handed to its decoder is 5 | `00000012a1684b65794672616d65a165747269656405` |
| `Input`: a left button went down at 0.5 across and 0.25 down the picture of frame 7, no modifier down | `00000048a165496e707574a1666576656e747381a167506f696e746572a5656672616d65076178f938006179f93400696d6f646966696572730066616374696f6ea164446f776e644c656674` |
| `Input`: the key `a` went down, no modifier, and no text was attached, so `text` is left out and is not null | `00000035a165496e707574a1666576656e747381a1634b6579a464636f64650464646f776ef5696d6f646966696572730066726570656174f4` |
| `Input`: the same key with the text `a` attached | `0000003ca165496e707574a1666576656e747381a1634b6579a564636f64650464646f776ef5696d6f646966696572730066726570656174f464746578746161` |

## 8. File transfer

A device sends files and folders to a paired device as one **transfer**: a single offer that the receiver decides once, and then the data, which travels on streams of their own called **lanes**. [ADR 0008](adr/0008-file-transfer.md) records why it is shaped this way. A device that takes part advertises the capability `transfer.files` (section 3). A sender MUST NOT make an offer to a device that did not advertise it, and MUST NOT open a lane to one.

A device that does not take part answers every valid transfer request (`Offer`, `Finish` and `Cancel`) with `Declined(Unsupported)`, the reply that this section defines for it. A transfer that a device took before it stopped taking part, because the person turned receiving off for example, is finished and cancelled as usual. A device of a version before this section cannot read the request and answers `Error { code: Malformed }` (section 3), so a sender that ignored the capability is still told, not cut off. Everything below applies to a device that advertised `transfer.files`. The reference implementation is one as a receiver, for as long as it has a folder to receive into, and it sends as this section says (see section 6 for the numbers).

### The list of files

A transfer is a list of **entries**, files and folders, each with a path relative to the folder the receiver puts the transfer in:

```
{"path": "Holiday", "folder": true}
{"path": "Holiday/a.jpg", "size": 1100, "modified_ms": 1700000000000}
```

`folder` is left out for a file, `size` for an empty file, and `modified_ms` when the sender does not know the time. The time is in milliseconds since 1970 and may be negative. A folder has no size and no time. A file is at most 2^50 bytes (1 PiB). Only files and folders are entries: a sender does not send links, devices or other special files, and does not follow a link inside a folder it was asked to send.

A path is untrusted text from another device. It has one or more names separated by `/`, on every platform. A name is 1 to 255 bytes, is not `.` or `..`, and has no NUL character. A path has at most 1024 bytes and 64 names. A path that is absolute (it starts with `/`), has an empty name, or breaks any other rule here is not accepted: in an offer, that makes the offer not valid, and in the list, it is a violation (see the rules for the receiver). What one particular file system does not allow (control characters, the reserved names of Windows, a trailing dot or space, `\` and `:`, names that differ only in case) is not checked here, because it depends on the receiver: the receiver puts it right when it chooses the name to create (see the rules for the receiver below).

The list is in a fixed order, so that a receiver can check it as it arrives without a table of the paths it has seen. Entries are sorted name by name, each name by its bytes, so that a folder comes directly before what is inside it, and every entry is inside a folder that was listed before it. A path therefore appears once, and nothing is inside a file. (Sorting each whole path as one string would be wrong: it puts `a-b` between `a` and `a/b`, because `-` sorts before `/`.) The **top-level items** are the entries whose path is one name. The **position** of an entry is its number in the list, counting folders; data frames name a file by it.

The list travels in pieces of 1 to 1024 entries, each a frame of section 2 that holds a `ListChunk` (`{"entries": [...]}`, at most 2 MiB).

The **list hash** is BLAKE3 in its key derivation mode, with the context `baton 2026-10-15 transfer list v1`, of every entry in the order of the list, each in this form:

```
flags u8 | size u64 | modified i64 | path length u16 | path
```

The numbers are big-endian. Bit 0 of `flags` is set for a folder, bit 1 when the entry has a time, and the other bits are 0. The size is 0 for a folder, and the time is 0 when there is none. An entry takes 19 bytes and its path, so entries need no separator, and the hash does not depend on how the list was cut into pieces. Everything an entry says is in it, time included. A list takes at most 16 MiB in this form, and holds at most 100,000 files and 100,000 folders, because a receiver holds the whole list before it writes anything. A sender whose list is larger offers it as more than one transfer.

### Offer

```
Request  {"Transfer": {"Offer": {"id": h'<16 random bytes>',
                                 "summary": {"items": 2, "files": 3, "folders": 1, "bytes": 1234},
                                 "preview": [{"name": "Holiday", "folder": true, "files": 2, "bytes": 1200},
                                             {"name": "notes.txt", "bytes": 34}],
                                 "list_hash": h'<32 bytes>'}}}
Response {"Transfer": {"Accepted": {"id": h'<same id>', "lanes": 4}}}
Response {"Transfer": {"Declined": {"id": h'<same id>', "reason": "Refused"}}}
```

The sender makes the offer after it has walked the whole tree, so that the list, the counts and the hash exist before anything moves. `id` is chosen at random by the sender and reused when it retries. `items` counts the top-level items (at least 1, and at most `files` plus `folders`), `files` and `folders` count every file and folder, and `bytes` adds up the sizes of all the files. `preview` shows the first top-level items by name, in the order of the list: exactly the first `min(items, 16)` of them, so its names are in increasing order and none is repeated. A folder in it says how many files are inside it, in every folder below it, and how many bytes they hold. A file says its size, and a file has no `files`. `list_hash` is the hash of the whole list, and an offer without it cannot be read.

The receiver decides once, and a person may be asked: the preview is what they see. A preview is text that another device wrote, so it is shown under the rules for a device name (`sanitize_display_name`, section 3). An offer that is not valid on its own, because a number or a name is beyond a limit of section 6, the parts do not add up, or the preview is not what it must be, breaks a limit and is answered with `Error { code: Malformed }` like any other request that does.

A receiver SHOULD check an offer in this order, as it does for an item:

1. whether it takes part at all (`Unsupported`);
2. whether the offer is a repeat of one it has decided on, see below;
3. whether it is within what it will ever take: more than the person allows (`TooLarge`), or more bytes than its storage has room for, less the reserve it keeps (`NoSpace`);
4. whether the peer already has as many transfers going as it may, or is sending too many items (`RateLimited`);
5. its policy for the sending device (`Blocked`), and then the person, who may say no or may not answer in time (`Refused`).

`Accepted` carries `lanes`, a number from 1 to 8: how many lanes the sender may open, numbered from 0. A whole transfer counts as one item against the receiver's limit on items, however much it holds, and as one entry in its inbox. Its lanes count neither against the limit of concurrent streams nor against the timer for requests (section 3): they have a budget of their own.

**A repeat.** A sender that did not get the answer repeats the offer with the same id, as with a handoff. A transfer is different from a handoff, because data follows the decision: a yes for three files must not be reusable under the same id for three thousand. So a repeat is the same offer only if the id, the summary, the preview and the list hash are all equal. The receiver answers the same offer as it did the first time without asking again, waiting for the first copy's decision if the person has not given it yet. It answers any other offer under that id with `Declined(Invalid)`. A receiver remembers the decision, an accepted offer included, for as long as it keeps what it received for the transfer, and a refusal by the person or by its policy for at least 10 minutes. Refusals made before it asks anyone (`Unsupported`, `TooLarge`, `NoSpace`, `RateLimited`) are not remembered, so a sender that has fixed the problem can offer the same transfer again.

A sender waits for the answer to an offer for as long as a person may take, and for the network. That is 2 minutes of the person (as in pairing) and a margin, 2 minutes 30 seconds in the reference implementation, instead of the 30 seconds of other requests. It repeats the offer on a new stream if it gives up.

### Lanes

After `Accepted` the sender opens up to `lanes` unidirectional streams to the receiver. Each starts with a header of 18 bytes, and then holds frames:

```
lane   = header frame*
header = kind u8 (1) | transfer id [16] | lane u8
frame  = tag u8 | head | bytes

tag 1  List  head: none                    bytes: a frame of section 2 holding a ListChunk
tag 2  Span  head: file u32 | span u32     bytes: as many as the list says for that span
tag 3  End   head: file u32 | hash [32]    bytes: none
tag 4  Skip  head: file u32 | reason u8    bytes: none
```

Numbers are big-endian. The `kind` byte says what a unidirectional stream carries: 1 is a lane of a transfer, and 2 and 3 are the streams of window streaming (section 9). `lane` is the number of this lane, below the `lanes` that were accepted. The receiver sends nothing back along a lane. A receiver that wants no more data stops a lane (QUIC's `STOP_SENDING`) with a code of the table in section 3, and QUIC delivers the bytes of a lane reliably and in order, so there is nothing to acknowledge.

**Spans.** A file is cut into spans of 1 MiB (2^20 bytes), the last one shorter. A file of 0 bytes has no span, and a file of 1 byte up to 1 MiB has one. `Span` carries the bytes of span `span` of the file at position `file`: exactly as many as the size of that file says (1 MiB, or what is left for the last span), and nothing else, no CBOR around the data. `file` MUST be the position of a file, not of a folder, and `span` MUST be below the number of spans of the file. The spans of one large file may go on different lanes, which is how a long path gets more than one window, and the spans of many small files go back to back on one lane. Nothing waits per file or per span. The receiver writes each span at its offset in a file whose size it knows from the list.

**The hash of a file** is its ordinary BLAKE3 hash, the value `b3sum` prints. It can be made from the spans in any order, because a span that starts on a 1 MiB boundary is a complete subtree of BLAKE3's tree of 1 KiB chunks. Each side makes the chaining value of each span it handles, and merges the values of a file at the end with the rule of the tree: the left subtree of a node holds the largest power of two of spans that is less than the number of spans it holds. A file of one span or less, an empty file included, is hashed as a root, so that the value of its only span is the hash of the file (the chaining value of a subtree is not the hash of a file, and cannot be made from empty input). The test vectors in section 7 include the sizes where this matters.

**End and Skip.** After it has read the last span of a file, the sender sends `End` with the hash of the whole file, on any lane. It may reach the receiver before the last spans of that file do, because lanes are independent, so the receiver finishes a file when it holds `End` and all its spans. A file of 0 bytes has only an `End`, with the hash of nothing. `Skip` says that the sender will not send a file, or no more of it: it could not read it (`Unreadable`, code 1), the file changed while it was being read (`Changed`, 2), it is gone (`Missing`, 3), or another reason that this version does not know (`Other`, 0, which is what a byte that is not one of these means). The receiver drops what it holds of that file. In each attempt (see Resuming) every file of the list gets one `End` or one `Skip` from the sender, and a folder gets neither. A frame with a tag that this version does not know is a violation (see the rules for the receiver), because the length of what follows it is not known.

**The list goes first.** The first frames on lane 0 are the list, in `List` frames. The list is complete when it holds `files` plus `folders` entries. A `List` frame is valid only on lane 0 and only before the list is complete, and nothing else is valid on lane 0 before then. The receiver needs the whole list before it writes anything, because it checks the sizes, the quota and the free space first, so until the list of a transfer is in, it leaves the other lanes of that transfer unread. Credit for data is taken when it is written, and lanes can together hold more than the connection window, so a sender that wrote spans while it was still writing the list could use up the window with spans that the receiver is not reading, and the rest of the list could not be sent: nothing would move until the timer ended the transfer. Two rules prevent it, and a sender MUST follow the first and a receiver the second. Both are made for each transfer, because a connection can carry two at once, and the data of one must never use up the credit that the list of the other is waiting for:

- The sender writes the whole list of a transfer to its lane 0 before it writes a span, an `End` or a `Skip` of that transfer to any lane.
- The receiver reads lane 0 of a transfer up to the end of its list before it reads any other lane of that transfer, and it does not hold back the lanes of a transfer whose list is in because the list of another transfer is not.

**Lane 0 is opened first.** A lane holds one of the receiver's unidirectional streams from the moment it is opened until it ends, whether or not anything is read from it. Until the list of a transfer is in, the receiver reads nothing from the other lanes of that transfer, so they hold their streams and move nothing. A sender therefore opens lane 0 of a transfer before it opens the others, so that a transfer that holds a stream holds one for its list, and a receiver grants no more lanes to the transfers that a peer may have going at once than its count of lanes allows (see the rules for the receiver). Without both, such lanes could take up the whole count of lanes before lane 0 of a transfer was opened, and nothing would move until the timer ended the transfer.

A sender SHOULD send a lane at a lower priority than the streams of requests, so that data buffered for a lane never goes out before data buffered for a request. That is all that priority gives. Flow-control credit is taken when data is written, so lanes that keep writing keep the credit used up, and a request written during a transfer waits for the next credit update, like a lane does. A receiver sends one after it has read an eighth of the connection window.

### Finish, Cancel and the outcome

```
Request  {"Transfer": {"Finish": {"id": h'...', "files": 3, "bytes": 1234}}}
Response {"Transfer": {"Done": {"id": h'...', "failed": [{"file": 7, "reason": "Mismatch"}], "more_failed": 0}}}
Request  {"Transfer": {"Cancel": {"id": h'...'}}}
Response {"Transfer": {"Cancelled": {"id": h'...'}}}
```

The sender makes `Finish` when it has written everything it will write and finished its lanes. `files` is how many files it sent in full (the files of the list less the ones it skipped), and `bytes` is what those add up to. The receiver compares them with its own count, and logs a difference, and a difference changes no file's outcome, because each file's outcome is decided by its own frames. `Finish` can reach the receiver before the last frames of the lanes do, so the receiver waits for them.

It answers with `Done` when every file of the list has been settled: checked and in place, failed, or skipped by the sender, and not before. A file that still has no `End` and no `Skip` when no byte has moved for 30 seconds (see the rules for the receiver) is `Incomplete`. `failed` names the files that are not in place, by their position in the list and a reason (`Incomplete`, `Mismatch`, `NoSpace`, `WriteFailed`, `Skipped` or `Other`): the first 256 of them at most, in the order of the list. `more_failed` counts the rest, and they all come after the last one named. So a file that is not named is **in place** if `more_failed` is 0, or if it comes before the last file that is named: it matched its hash and is under its final name. When `more_failed` is above 0, a file after the last one named may or may not be in place, and the sender cannot tell which, so it MUST treat it as not in place, and it sends the file again if the transfer is tried again. A `Done` whose positions are not in increasing order, or that has a `more_failed` above 0 with fewer than 256 files named, is not valid. Being in place does not say that the disk has stored the file. Flushing each of 10,000 small files would cost more than moving them, so a receiver does not wait for the disk per file, and a power cut just after `Done` can lose files that the sender has shown as sent. A sender shows "sent" when the other device has the files, which is what it knows.

`Done` takes longer than a reply to an ordinary request, as an offer does, so the sender waits for it for longer than 30 seconds (2 minutes in the reference implementation). A sender that gives up sends `Finish` again, and the receiver answers it with the same outcome for as long as it remembers the transfer. A `Finish` for a transfer that the receiver does not know, because it never accepted it, dropped it or forgot it, is answered with `Error { code: NotFound }`.

A sender whose connection went before it heard the answer to `Finish` MAY ask for the outcome on its new connection before it repeats the offer, with the same `Finish`, because the receiver may have it. Such a `Finish` comes on a connection where the receiver has begun no attempt of the transfer, and the receiver answers it only when it can end the transfer without giving up a file that the sender could still send: when it has the outcome, when it is putting the items in place (the answer comes once they are), or when every file has its `End` or `Skip` and only the `Finish` was lost. Otherwise it answers `Error { code: NotFound }` at once, and an attempt that was stopped for a stall is not ended by it: what the receiver waits for went with the old connection, and a `Finish` on a new one cannot bring it. A sender that is told so offers the transfer again and sends it again (see Resuming), as it does after any answer that is not an outcome, and one that gets no answer within a few seconds (10 in the reference implementation) does the same, because a receiver that waits for those files never answers. A `Finish` that did reach the receiver on the connection of the attempt is answered as above, when the files are settled or the 30 seconds are up, and a sender that comes back after that is given the same outcome, with the files that did not come as `Incomplete`.

`Cancel` gives a transfer up at any point, an offer that still waits for a person included: the receiver drops what it holds for the transfer and answers `Cancelled`. It answers the same way for a transfer it does not know, because then there is nothing left to cancel. A sender that cancels also resets its lanes with code 499, and a receiver that gives a transfer up, because the person did so, stops the lanes with 499. A sender treats a stop code on a lane of the attempt that is going as the receiver's answer, and does not send `Finish` for a transfer that was stopped, unless the code was 507 (see the rules for the sender).

### Rules for the receiver

- **Who and how many.** A receiver MUST check that the peer is still paired when it reads each transfer request and when it takes a lane, as section 3 asks for each request. A device may keep several connections to the same peer (section 6), and the budget of streams is per connection, so a receiver counts the transfers a peer has going itself, across the connections (an offer that waits for a person is going), and answers an offer beyond its limit with `Declined(RateLimited)`. The lanes that it grants to the transfers that a peer may have going at once add up to no more than the count of lanes that it allows (section 9, Taking a stream), which is the share of lanes in the limit on unidirectional streams that it announces (the reference implementation allows 2 at once and grants 4 lanes each, which comes to the 8 of its count of lanes). A stranger has no unidirectional stream: the receiver announces a limit of 0 for them, and raises it for a peer only once it knows the peer is paired.
- **Which lanes.** A stream of a kind other than 1 is stopped with 400, unless the receiver takes part in window streaming and the kind is one of section 9. A lane belongs to the transfer that has its id and that this peer offered, never to one found by the id alone, so that another paired device that learned an id cannot add to someone else's transfer. A lane for a transfer that this peer did not offer, or that is not going because it was declined, cancelled, finished or forgotten, is stopped with 499: it can arrive late, and that is not a fault. A lane whose number is not below the number granted, or that repeats a number that the same transfer already used on its connection, is a violation of the transfer.
- **A violation of a transfer** is a list that is not valid, is not in order, has an entry that is not inside a listed folder, has other counts or sizes than the summary, does not show the top-level items of the preview with the sizes the preview gave, or does not have the hash of the offer; a frame with a tag that is not known; a `Span` that names a folder, a file that is not in the list, or a span that is not in the file; an `End` or a `Skip` for a folder or for a position that is not in the list; a second `End` or `Skip` for a file within one attempt (see Resuming); a `List` frame where none may come; a lane that the sender finishes in the middle of a frame (a lane that it resets is not that: it gave the lane up); lane 0 finished before the list is complete. The receiver stops every lane of the transfer with code 400, drops the transfer, and keeps the connection (it MAY close the connection if a peer does it again). A list that takes more than 16 MiB in the form that is hashed is stopped with 413 in the same way. A duplicate is not a violation: a span that the receiver already holds (it was sent again after a reconnection, see below) or one of a file that was skipped is read to its end and dropped, and the first value is kept.
- **Nothing is written before the list is checked.** The receiver compares the counts and sizes of the list with the summary, the quota and the free space before the first byte is written. If it runs out of room later, it stops the lanes with 507, and files that were not placed are `NoSpace` in `Done`. If it has no room for the staging folder, nothing was received: it stops the lanes with 507 and gives the transfer up, and a `Finish` for it is answered with `Error` and `NotFound`.
- **Staging.** It writes into a staging folder that it makes on the same volume as the destination, and it moves each top-level item into place once the transfer has ended (`Finish`) and the files in it have been checked, leaving out any file that failed. A person never finds a folder that is still being received. The staging files are made with `create_new` and its folders with `create_dir`, never `create_dir_all`, so that a clash that slips through never mixes the bytes of two files or the contents of two folders: a file whose name is taken fails alone, and a folder whose name is taken stops the transfer before anything is written.
- **Nothing is replaced.** The move into place MUST refuse to replace what exists, with a system call that does so atomically (`renamex_np` with `RENAME_EXCL` on macOS, iOS and iPadOS, `renameat2` with `RENAME_NOREPLACE` on Linux and Android, `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING` on Windows). When a name is taken it takes the next number and tries again. On a volume that does not support the call, the item fails and is named in `Done`, and it is never moved over another.
- **Names are the receiver's to choose.** It chooses the name of every entry, not only of the top-level items, and compares names the way its file system does (not case-sensitively where the file system is not, and after Unicode normalisation where the file system does it). A name that would take the name of something that exists, or clash with another name in the same folder of the same transfer, gets a number before its extension: `notes.txt` becomes `notes 2.txt`. Characters that the file system does not allow are replaced, and so are the characters that only disguise a name, such as the text-direction controls that make `invoice`, U+202E and `fdp.app` look like a PDF. A link inside a folder is never followed, and the receiver never creates a link, a device or another special file.
- **Permissions.** Permission bits are not carried, so a received file is never executable. Where the system has a mark for files that came from elsewhere, a received file gets it, and on a Mac so does every folder, because an app is a folder and Gatekeeper reads the mark of the app's folder and not of the files in it. The receiver does not open or run anything it receives (see [security.md](security.md)).
- **Time.** A transfer that has moved no byte for 30 seconds is stopped, with code 408 on the lanes, and a long transfer is not stopped for being long. The stop ends the attempt and not the transfer: the receiver keeps what it received and its decision, and the sender MAY try again (see Resuming). A receiver SHOULD read each lane on its own, so that a slow write holds up that lane and no other.
- **After a stop.** What a receiver received stays in the staging folder for as long as it remembers the decision, so that the transfer can resume. A staging folder that is not resumed is removed.

### Rules for the sender

- A sender walks the whole tree before it offers, takes the size and the modification time of each file for the list, and makes the list hash with the list. It reads no file for that: the hash of the list needs no pass over the data, so the first byte leaves at once.
- It hashes each file as it reads it, and sends the hash after the last span.
- A file can change while it is read, and the hash cannot show that, because the hash covers the bytes the sender read. So the sender compares the size and the modification time of a file with the list before it reads the file, after the last read, and before it resumes the file, and a difference fails the file with `Skip(Changed)`. This catches what programs normally do, which is to change one of the two, and it does not see a change that keeps both.
- A file that fails fails alone. The others are kept, and `Done` names the failures.
- A sender that is closed or killed while a transfer goes, or that gives a transfer up when it cannot reach the receiver, leaves the receiver holding what it was sent, and the receiver counts the transfer as going until it lets go of it (see After a stop, and the limit on transfers going at once). A sender SHOULD write down which transfers it has offered and has not seen end, before it makes each offer, and send `Cancel` for each of them when it starts again, and as soon as it can reach the receiver after a transfer that it could not tell it about. The answer to a `Cancel` is the same whether the receiver held the transfer or had let go of it already, so a word that was not needed does no harm. A sender sends it to any paired device whatever its hello says about taking files, because a device that stopped taking files may still hold a transfer that it took before. It stops writing a transfer down once the receiver is known to hold nothing of it: it answered `Done`, `Declined` or `Cancelled`, it ended the transfer with a stop on a lane, or it ended the connection because it does not know this device. It does not drop one for being old, because the receiver counts its hour on a clock that stops while the device sleeps, and an app that is suspended counts none of it, so a sender cannot tell when a receiver has let go. It asks again after longer and longer pauses (the reference implementation has one pace for all devices, and takes half an hour at most once the last word about every transfer that is left is more than an hour old), and it keeps no more than a few times what the receiver can hold, the one that was left first being the first to go (the reference implementation keeps 16 for a device, where a receiver holds 2 at a time). A sender that cannot write the list loses at worst the hour that the receiver keeps a transfer, counted in the receiver's own time. The reference implementation keeps the list in a file of its state directory (see [architecture.md](architecture.md#state-on-disk)).
- When a lane of the attempt that is going stops with a code, the transfer ends: 499 is the receiver giving it up, 507 and 413 are that it has no room or that the list is too long, 403 that this device is not paired any more, and 400 that the transfer broke a rule. After 507 the sender sends `Finish` all the same and does not try again, because the receiver puts in place the files it had checked and names the others `NoSpace` in `Done`; after the other codes it sends no `Finish`. Once `Finish` is sent the answer to it decides, and a stop that arrives on a lane the sender has already finished changes nothing, whatever its code, because QUIC tells the sender of a stop for as long as the end of a lane has not been acknowledged. A stop with 408, which is a timeout, ends only the attempt, and so does a write on the sender's side that makes no progress for 30 seconds: whichever side's 30 seconds run out first, the sender MAY try again, see below. A sender that ends an attempt resets the lanes of that attempt, with 408, and a stop that arrives later on a lane of an attempt that has ended changes nothing, whatever its code. With the reset, those lanes are usually closed before the receiver accepts the repeat, so the 499 that the receiver sends for the lanes of the attempt before (see Resuming) is usually not sent at all.

### Resuming

A transfer outlives a connection. If the connection drops, or an attempt ends in a stall (see Time), the sender connects again and repeats the offer unchanged: it keeps the list and does not make a new one, so the repeat is the same offer. It makes the new connection even when the old one is still up, as it often is after a stall: a receiver whose disk has stopped answering, a USB drive or a network share say, still answers keep-alives. Lane numbers are counted per transfer on each connection, and the header of a lane names no attempt, so a lane of a second attempt on the old connection could not be told from a late lane of the first, and would repeat a number, which is a violation. A receiver that has kept what it received answers `Accepted` again, and the sender sends the transfer again on new lanes of the new connection, the list first. Each such send is an **attempt**. A receiver that accepts a repeat on a new connection ends the attempt before it: it stops the lanes of that attempt that are still open with 499 (a sender that has ended the attempt has usually reset them already, and a stop on a lane of an attempt that has ended changes nothing, see the rules for the sender), reads the list again from the new lane 0, and checks it as it did the first time. A receiver drops the spans it already holds when they arrive again, as above. The `End` of a file carries the hash of the bytes as they are now, and a resumed attempt sends an `End` for every file again, those that were complete before included. The receiver checks each against the values of the spans it kept and of those that arrived now, so a file that was edited in between fails its check, even one that was complete in staging, and no mix of two versions is put in place.

This revision has no way to say which spans the receiver holds, so a transfer that is resumed is sent again in full, which is correct and not fast. A later revision adds an optional field to `Accepted` that names the spans the receiver holds. A sender that ignores it keeps sending everything, which stays correct, so the field needs no new capability. The sender then reads those spans again and hashes them without sending them, so that the hash it sends is still that of the bytes as they are now and never a merge of old and new values.

## 9. Window streaming

A device can show one of its windows to a paired device. The **source** (a Mac) sends the picture of the window as a run of encoded **frames**, one unidirectional stream for each frame, and the **viewer** (an iPhone or an iPad) decodes them and shows the newest. A frame that is late is reset and replaced, and never sent again. [ADR 0009](adr/0009-window-streaming.md) records the design and why it is shaped this way. This section specifies the offer that starts a session and the request that ends it, the start of every unidirectional stream, how a receiver takes a stream, the header of a frame of video, and the messages on the control streams. A device takes part as a viewer when it advertises `stream.view`, and as a source when it advertises `stream.share`. A device that takes part in neither advertises none of `stream.share`, `stream.view` and `stream.control`, answers every valid request of this section with `Declined(Unsupported)`, the reply that this section defines for it, and stops a stream of kind 2 or 3 with code 400, like a stream of any other kind it does not take (see Taking a stream). A device of a version before this section cannot read the request and answers `Error { code: Malformed }` (section 3), as it does for a transfer, so a source that ignored the capability is still told, not cut off.

### The offer and the end of a session

A session starts at the source: the person chose a window, a paired device to show it to, and whether that device may also control the window. The source makes one request on a stream of its own, like any request of section 3, and the viewer answers once.

```
Request  {"Stream": {"Offer": {"session": h'<16 random bytes>', "source": "Window",
                               "title": "Report.pdf", "app": "Preview",
                               "width": 1440, "height": 900, "control": true}}}
Response {"Stream": {"Accepted": {"session": h'<same id>', "max_width": 2732, "max_height": 2048,
                                  "max_fps": 60, "codecs": ["H264"]}}}
Response {"Stream": {"Declined": {"session": h'<same id>', "reason": "Refused"}}}
```

`session` is chosen at random by the source for each offer and never used for another one, so that a request or a stream that arrives late cannot be taken for part of a later session. `source` says what is shared. This version knows `Window`, one window of one application, and a kind it does not know is read as `Other`. `title` and `app` are the title of the window and the name of its application, and either can be empty. `width` and `height` are the size of the window in pixels, each from 1 to 16384; the picture that is sent can be smaller, because the source scales it down. `control` says that the person at the source granted the viewer control of the window, and it is left out when they did not. A viewer cannot ask for control and cannot turn it on later, so a session that began without it becomes a controlled one only by a new offer. A source MUST NOT set `control` unless it advertised `stream.control`. The viewer takes the offer as it is: it sends input in a session whose offer has `control`, and in no other.

A source MUST NOT make an offer to a device that did not advertise `stream.view`. It has one session at a time in this version, so it makes no offer while one is waiting for an answer or running. A session lives and dies with its connection: if the connection closes the session is over, and the person offers again.

The viewer decides once, and by default a person is asked: the offer is what they see. `title` and `app` are text that another device wrote, so they are shown under the rules for a device name (`sanitize_display_name`, section 3) and are never written to a log. An offer that is not valid on its own, because a title or a name is beyond a limit of section 6 or a size is 0 or beyond it, breaks a limit and is answered with `Error { code: Malformed }` like any other request that does. A viewer SHOULD check an offer in this order:

1. whether it takes part at all, which is whether it advertised `stream.view`, and whether it knows the kind of source (`Unsupported`);
2. whether the session id is one that it has seen before on this connection (`Invalid`);
3. whether another offer is waiting for its person or a session is running, or the peer is sending too many items (`RateLimited`);
4. its policy for the sending device (`Blocked`), and then the person, who may say no or may not answer in time (`Refused`).

A session counts as one item against the receiver's limit on items, whatever it carries, and a peer can have one offer waiting for a person at a time, counted across its connections. The person has 2 minutes, as in pairing. The source waits for the answer for as long as the person may take and for the network, the time that section 6 gives for the offer of a transfer, instead of the 30 seconds of other requests.

`Accepted` says what the viewer can take. `max_width` and `max_height` are the largest picture in pixels, each from 1 to 3840. `max_fps` is the highest frame rate it can decode and show, from 1 to 120. `codecs` are the codecs it can decode, from 1 to 8 of them. This version knows `H264`, and a codec that the reader does not know is read as `Other` and is never chosen. The source chooses what it sends within these. A source that can send none of them ends the session with a `Stop` whose reason is `Failed`.

Nothing is repeated under a session id. A source that did not get the answer ends the offer with `Stop` and offers again under a new id. Either side ends a session, or withdraws an offer that has not been answered, with a request of its own:

```
Request  {"Stream": {"Stop": {"session": h'<same id>', "reason": "Ended"}}}
Response {"Stream": {"Stopped": {"session": h'<same id>'}}}
```

`reason` is `Ended` (the person stopped sharing or viewing), `WindowClosed` (the window that was shared is gone), `Failed` (something went wrong that the person did not choose, such as the permission for capture being taken away) or `Other` from a later version. Stopping changes nothing the second time: a device that never had the session, or has forgotten it, answers `Stopped` all the same, because there is nothing left to stop. A `Stop` that arrives while the offer waits for the person withdraws it, and the request of the offer is answered `Declined(Refused)`. A viewer that answers `Stopped` has let go of the offer or the session that the `Stop` names, which then no longer holds its place (step 3 above). A source that ends a session sends a `Stop` as well as finishing its control stream, and one that offers again to the same device straight after a `Stop`, because the person gave up an offer or ended a session, SHOULD wait for the answer first (a few seconds at most): until then the viewer may still hold its place and answer `RateLimited` (step 3 above).

### The start of a stream

Every unidirectional stream of a session starts with 17 bytes:

```
start = kind u8 | session [16]

kind 2  a frame of video   start | rest of the frame header | the encoded picture
kind 3  a control stream   start | messages, each a length and one CBOR item (section 2)
```

Numbers are big-endian. `kind` is the first byte of the stream, as in section 8, where kind 1 is a lane of a transfer and the 16 bytes after it are the id of the transfer. `session` is 16 random bytes chosen by the source when it makes the offer. Every stream of the session carries it, so that a receiver can recognise a stream of a session that is not running, which can arrive late, after the session has ended.

### Taking a stream

A receiver reads the 17 bytes of the start of a unidirectional stream before it does anything else with the stream, and the rules of this subsection hold for every kind. What a stream means beyond its start, which transfer or session it belongs to and whether this peer is part of it, is for the rules of its kind (section 8 for a lane, and the next subsection for a frame and a control stream).

- **Paired.** A receiver MUST check that the peer is still paired when it takes a stream, and not only when the connection was made, as it does for a request (section 3). If the peer is not paired it closes the connection with code 403.
- **In time.** A stream whose start has not arrived whole within the time that a receiver allows for it is stopped with code 408. The reference implementation allows 10 seconds from when it accepted the stream.
- **A kind that is not taken.** A stream of a kind that the receiver does not take, because it does not know the kind or does not take part in what the kind is for, is stopped with code 400 and not read any further.
- **The count of a kind.** QUIC has one limit for the unidirectional streams of every kind and does not know what they carry, so a receiver also counts the open streams of each kind, and stops one beyond the count of its kind, which would take the place of a stream of another kind. A frame of video beyond the count is stopped with code 499, because the sender cannot see when the receiver stops counting a stream that it reset, so a frame can arrive just as the count is reached and no one is at fault. A stream of any other kind beyond the count is a violation and is stopped with code 400. How many streams of a kind are allowed is for the rules of the kind (section 8 for lanes, ADR 0009 for the others), and the limit that a receiver announces to QUIC is the sum of them.
- **Letting go.** A receiver lets go of a stream's place in the count before it answers anything that ends what the stream belongs to: `Done` or `Cancelled` for the lanes of a transfer, the end of the session for a control stream. The limit that it announces has room for the other kinds, so QUIC no longer holds back a sender that opens the next stream of the kind as soon as the end of the last is acknowledged, and a place that is still taken would make that stream one beyond the count.

### The session of a stream

A frame and a control stream belong to the session that their start names. A device takes one only if it has that session, if the stream comes from the device that the session is with, and if it comes on the connection that the session began on, since a session lives and dies with its connection. Any other stream of kind 2 or 3 is stopped with code 499 and not read. That is no one's fault, because a frame can arrive after its session has ended.

### Frames

A frame of video is one unidirectional stream: a header of 50 bytes, the start included, and then the encoded picture up to the end of the stream.

```
header = start (kind 2) | number u64 | depends on u64 | chain u32 | epoch u32 | capture time u64 | flags u8
```

| Field | Meaning |
| --- | --- |
| `number` | Counts the frames of a session from 0, one for each frame that the encoder made, including those that were given up on later. |
| `depends on` | The newest frame that the viewer must have decoded before this one. For a key frame it is the frame's own number, which means that it needs no other frame. For any other frame it is the newest earlier frame that something may refer to, so that a frame that nothing refers to can be skipped. For a recovery frame it is the newest such frame that the source knows the viewer holds, and it can be in an earlier chain. |
| `chain` | Counts the chains of a session. A chain is a run of frames that are decoded one after the other. It begins at a key frame or at a recovery frame. |
| `epoch` | Counts the sizes of the picture. It changes, with a key frame, when the size changes. |
| `capture time` | When the picture was captured, in microseconds since the session began, on the source's clock. |
| `flags` | Bit 0 (value 1): a key frame, which can be decoded alone and makes the decoder forget everything before it. Bit 1 (value 2): a recovery frame, which refers to a long-term reference that the viewer is known to hold and begins a new chain without a key frame. Bit 2 (value 4): another frame may refer to this one, so it cannot be skipped. |

A receiver MUST treat a header as a violation (code 400) if it is cut short, if a bit of `flags` other than these three is set, if bits 0 and 1 are both set, if a key frame does not depend on itself, or if another frame depends on a frame whose number is not lower than its own. A receiver MUST also treat a frame as a violation (code 400) if it holds a frame with the same number, or has decoded one since the last key frame. A frame has a number of its own, and one that is reset is not sent again, so an honest source never repeats a number. A frame whose number the receiver neither holds nor has decoded since the last key frame is taken like any other, and is dropped as late if its number is below that of a frame that has been decoded. The picture is at most the limit of section 6, and a stream whose picture goes beyond it is stopped with code 413 and not read any further. What the picture holds is what the session agreed on: the first version carries H.264 in Annex B form, with the parameter sets in front of every key frame.

### Control streams

After the viewer has accepted, each side opens one unidirectional stream of kind 3 to the other and keeps it open for as long as the session runs. The source's stream carries what the source has to say about the picture, and the viewer's carries how the viewer is doing, what it needs and, when the person at the source granted control, what the person did. Behind the 17 bytes of its start a control stream is a run of messages, each framed as in section 2: a length and one CBOR item, at most 64 KiB. These are not requests. Nothing answers a message, and none counts against the limits on requests (the 16 streams and the 30 seconds of section 6).

The source opens its control stream first. A viewer MUST NOT open its own before it has taken the source's, because until then it cannot tell that the source has made its side of the session, and a stream that reached the source before that would name a session that the source does not have (see the session of a stream, above). The same holds the other way round: a viewer MUST make its side of the session before it sends `Accepted`, because the source opens its control stream and its first frames as soon as it reads the answer, and a stream that reached a viewer with no session yet would be stopped with code 499 and the session would be over before it began. A viewer allows the source a time to open it, 10 seconds from its `Accepted` in the reference implementation, and ends the session if the stream has not come by then. Each side has one control stream in a session, and a second one from the same side is a violation (code 400). The end of a control stream is the end of the session: a side that finishes or resets its control stream has ended it, and the other side ends its own. A side that ends the session because the other broke a rule resets its control stream with code 400, and a side that cannot go on because of a fault of its own resets it with code 500. Any other end finishes the stream.

The source writes messages of one family and the viewer messages of another, and a message of the other family on a stream is a violation:

```
Source: {"Config": {"epoch": 1, "codec": "H264", "width": 1440, "height": 900, "fps": 60}}
        {"Pause": {"reason": "Locked"}}
        "Resume"
        {"Notice": {"reason": "Covered", "count": 3}}
Viewer: {"Report": {"shown": 41, "decode_us": 2500, "dropped": 3, "thermal": "Fair"}}
        {"Want": {"width": 2796, "height": 1290, "fps": 120}}
        {"KeyFrame": {"tried": 5}}
        {"Input": {"events": [ ... ]}}
```

| Message | Meaning |
| --- | --- |
| `Config` | The picture that the source sends from now on. `epoch` is the epoch of the frames it describes (Frames, above). `codec` is `H264`, or `Other` for a codec that the reader does not know, which a viewer cannot decode. `width` and `height` are the size of the picture in pixels, each from 1 to 3840, and the picture shows the whole window scaled to that size. `fps` is the most frames a second that the source sends, from 1 to 120. A frame and a message travel on streams of their own and arrive in any order, so a frame of an epoch can arrive before the `Config` that describes it. A viewer decodes it all the same, because the parameter sets are in the key frame, and uses the `Config` for its layout. The source keeps within what the viewer said in `Accepted`, and a viewer treats a `Config` that goes beyond it as a violation (code 400): a codec that is not one of `codecs`, a width or a height above `max_width` or `max_height`, or a rate above `max_fps`. |
| `Pause` | The source has stopped sending pictures. `reason` is `Locked` (the Mac is locked or its display is off), `Hidden` (the shared window is minimised or not on screen), or `Other` from a later version. The source drops input until `Resume`. |
| `Resume` | The source sends pictures again. |
| `Notice` | Events of the viewer were dropped, so that a tap that did nothing has a reason that the person can read. `reason` is the rule that dropped the most of them since the last notice, and `count` is how many it dropped, at least 1. A source SHOULD send at most one in a second. A notice holds no title and no text that was typed. |
| `Report` | How the viewer is doing. `shown` is the number of the newest frame that was decoded and shown, and is left out before the first picture. `decode_us` is how long the viewer takes to decode a frame, in microseconds, as it smooths it. `dropped` is how many frames the viewer received and did not decode in this session, counted from the start so that a report that is not read changes nothing. `thermal` is the thermal state of the device: `Nominal`, `Fair`, `Serious`, `Critical`, or `Other` from a later version. |
| `Want` | The largest picture and the highest frame rate that the viewer can take now, which change when the phone is turned. `width` and `height` are each from 1 to 3840 and `fps` is from 1 to 120. The source never sends more than the viewer said in `Accepted`, whatever this says. |
| `KeyFrame` | The viewer cannot decode and needs a key frame. `tried` is the number of the newest frame that the viewer handed to its decoder, whether the decoder took it or not. A key frame numbered above it answers the request, and one at or below it does not. |
| `Input` | What the person did on the viewer, in the order in which it happened: `events`, from 1 to 64 of them. Only in a session whose offer set `control`: a source drops the events of any other session and says `ControlOff`. |

The reasons of a `Notice` are the rules that drop an event:

| `reason` | The event was dropped because |
| --- | --- |
| `ControlOff` | control is off for the session |
| `Paused` | the session is paused |
| `Locked` | the Mac is locked, or its display is off |
| `NotOnScreen` | the shared window is not on screen |
| `TooFast` | the viewer sent more events in a second than it may: 240, and one batch of 64 more for events that a stall on the way bunched |
| `FrameUnknown` | the frame that the event names is not one that the source kept |
| `PictureOutOfDate` | the picture of that frame went out of date more than 500 ms before the event arrived |
| `WindowResized` | the window has changed its size since that frame |
| `OutsideWindow` | the point is outside the window |
| `Covered` | another window is in front of the window at that point |
| `NoKeyboard` | the shared window does not have the keyboard |
| `AppWindowInFront` | a window of the same application lies in front of the shared window and overlaps it, or is an open menu |
| `TooManyHeld` | more than 32 keys are held down |
| `NotPressed` | the event releases a button or a key that this session did not press |
| `Malformed` | the event holds a value that is not allowed. A source never sends this reason, because a message with such an event is a violation (below) and is not read any further |
| `Other` | a rule of a later version |

An event is one of two kinds. A pointer event is `{"Pointer": {"frame", "x", "y", "modifiers", "action"}}`: `frame` is the number of the frame of video that the person was looking at, `x` and `y` are finite numbers from 0 to 1 across and down the picture of that frame (a viewer whose finger or pointer leaves the picture clamps the position to the picture while a button is down, and sends nothing outside the picture otherwise, because a position outside it is a violation), `modifiers` is a byte with bit 0 for Shift, bit 1 for Control, bit 2 for Option, bit 3 for Command and bit 4 for Caps Lock (the other bits MUST be 0), and `action` is `"Move"`, `{"Down": button}`, `{"Up": button}` or `{"Scroll": {"dx", "dy"}}`. A `button` is `Left` (which is also a touch), `Right` or `Middle`. A scroll step is in points, finite, and at most 10,000 in either direction. A key event is `{"Key": {"code", "down", "modifiers", "repeat", "text"}}`: `code` is a USB HID usage of the keyboard page (0x07), where `a` is 0x04, from 0 to 0xE7: it is what the viewer has, and each source translates it to its own codes. Code 0 means no key, for text that was typed with no key, as the on-screen keyboard and dictation give it. An event with code 0 MUST go down, MUST NOT repeat and MUST carry text; the source types the text and no release follows. `down` says that the key went down and not up, `modifiers` are those that are down after the event, `repeat` says that the system repeats a key that is held, and `text`, which is left out when there is none, is the text that the key produced, at most 20 UTF-16 units (one call of the Mac's function that attaches text to an event is reported to keep no more), with no control character other than tab, line feed and carriage return. A viewer splits a longer text, from dictation, a paste or a long sequence of emoji, at character boundaries across several key events.

A sender MUST NOT send a message that breaks a limit of this section, and a receiver MUST treat one as a violation (code 400), also a message that it cannot read and a message of a kind that it does not know. The rate of events is not a property of a message: the source counts events from the time they arrive, and drops those over 240 and one batch of 64 in a second (the viewer counts 240 by its own clock, and a stall on the way bunches events that it sent apart) with a `Notice` (`TooFast`) and without a violation, since a viewer that sends too fast has lost events and has not broken the format. A later version adds a kind of message only with a capability that the other side advertised (section 5), so a receiver has nothing to skip. A later version may add reasons and states as unit variants of the enums above, which an older reader takes as `Other`, and optional fields, which it ignores. Typed text and key codes are secrets, since a password is typed one key at a time: a device MUST NOT write them to a log, and the reference implementation prints a key event without its code and without its text.

A receiver waits as long as it takes for a message to begin, because a control stream can be quiet. Once the length of a message has arrived, the rest of it has the time that a receiver allows for a stream that delivers nothing (10 seconds in the reference implementation), and a control stream that is slower is stopped with code 408 and the session ends. That is no violation, since the time is a receiver's own choice: the side that stopped the stream finishes its own. A session whose other side has gone quiet is ended by a rule of its own. A viewer sends a `Report` at least every 2 seconds for as long as the session runs, which the reference implementation does every 500 milliseconds from when it has taken the source's control stream. A source that has had no report for 10 seconds stops encoding, so that it does not fill a link that nobody is watching, and one that has had none for 30 seconds ends the session.

## 10. Planned extensions

Nothing in this section is part of the protocol yet. It is written down so that the rules above leave room for it, and so that the reasons are on record before any of it is built.

- **Pull.** Besides pushing an item to a device, a device can ask a paired device what it has ready to continue (`ListActivities`) and fetch one of them (`Fetch`). That lets a device that was asleep or locked when something was sent catch up when it wakes, and lets a phone pick up what was on a laptop without the laptop pushing it. Both requests are guarded by a capability, so older peers never see them. A device answers them while it is locked with the items that were sent to the asking device, and with staged items only if the person has turned that on for the asking device, which is off by default. See [Locked devices](architecture.md#locked-devices).
- **A short code that a person types**, for pairing devices that can neither show nor scan a QR code. As explained in section 4, that needs a password-authenticated key exchange and a new ALPN.
- **Window streaming** (milestone 5). A Mac offers one window to a paired iPhone or iPad with a single request that the viewer answers once, and the picture then moves as one unidirectional stream for each frame of video, at a priority between requests and the lanes of a file transfer. Two small control streams stay open, one in each direction, and carry the viewer's reports and, when the person at the Mac granted control with the offer, its input. A frame that is late is reset and replaced, and never sent again. Every unidirectional stream begins with a byte that says what it is, which also tells a lane from a frame. It is guarded by the capabilities `stream.share`, `stream.view` and `stream.control`. [ADR 0009](adr/0009-window-streaming.md) records the design. Section 9 specifies the offer, the end of a session, the start of a stream, the header of a frame and the messages on the control streams so far, and the rest of this becomes a specification there, with the rules for the receiver, when the code that applies them is added.
