# Security model

What Baton protects, who it defends against, and how. Each mechanism below says whether it is *implemented*, *partly implemented* or *planned* (designed but not built yet). Pull requests update this page as they land, so it never claims more than the code does.

To report a vulnerability, see [SECURITY.md](../SECURITY.md).

## What is being protected

- **The contents of handoff items** while they travel: clipboard text, links, and later files and tabs.
- **The device key.** Whoever holds it *is* that device to every device that paired with it.
- **The list of paired devices.** It decides who may send things to this device.
- **What this device does with an item.** Opening a link or replacing the clipboard acts on the person's behalf, so it must not happen because a stranger asked.

## Who is considered

| Adversary | Can do | Baton's answer |
| --- | --- | --- |
| Passive or active network attacker on the LAN or the internet | Read, drop, modify or replay packets | The connection is QUIC with TLS 1.3, mutually authenticated by device keys. Tampering fails the handshake; content is unreadable. |
| Relay operator | See which public keys talk, from which IP addresses, and how much | Relays forward ciphertext only. Metadata is visible to them (see limitations). Relays are replaceable and self-hostable. |
| Unpaired device that can reach this one | Dial this device's address or public key | After the handshake the connection is closed unless the peer's key is in the trust store. No application data is read first. |
| Someone who sees a pairing ticket (QR code over the shoulder, a leaked screenshot) | Try to pair | A ticket works once and expires quickly. The person on the offering device must approve the request, and does so by checking that a six-digit code shown on both devices is the same. The code depends on the session, so a stranger's device cannot show the one the person is waiting for. Declining a request ends the offer, so a ticket that leaked is dead. |
| A paired device that is lost, stolen or compromised | Send items as a trusted peer | The receiver's per-device policy limits the impact: items go to an inbox, and links and clipboard are not acted on automatically unless the person opted in. Unpairing removes the key and closes live connections. |
| Another local user or process on the same machine | Read key files, talk to the control socket | Key and state files are readable only by the owner. The control socket is private to the user. |

Out of scope: an attacker who controls an unlocked user session or the operating system, traffic analysis by a global observer, and hiding metadata from relays.

## Mechanisms

| Mechanism | Status |
| --- | --- |
| **Device identity.** An Ed25519 key per device, generated on first run. The public key is the device's identity everywhere. | Planned |
| **Authenticated, encrypted transport.** QUIC and TLS 1.3 through iroh; both ends are identified by their keys, so a connection to a given identity cannot be impersonated. | Planned |
| **Access control after the handshake.** The session protocol accepts only keys in the trust store. The pairing protocol accepts connections only while an offer is open. Everything else is closed with an error code before any message is read. | Planned |
| **Pairing.** One device creates a ticket holding its address and a random 256-bit secret, valid for five minutes and for one successful use. The joiner proves it knows the secret with a keyed hash bound to both device keys and to the TLS session, so the secret itself is not sent and a proof cannot be replayed on another connection. Both devices then work out a six-digit confirmation code from the same inputs and show it, and the offering person approves only if the codes are the same. A request without a valid proof is answered the same way whatever the reason, so a stranger learns nothing, and a request that is declined ends the offer. | Partly implemented: the ticket, proof and confirmation code are in `baton-proto`; the exchange comes with the transport |
| **Trust store.** Paired keys are stored with names and policies, written atomically, in a file only the owner can read. Removing a device closes its live connections. | Planned |
| **Handoff policy.** Per paired device, on the receiving side. Defaults: links and clipboard text go to an inbox. Auto-opening links and auto-applying the clipboard are opt-in. Only `http` and `https` links are ever opened. | Planned |
| **Strict message decoding.** A message is read completely as CBOR and rejected unless it is exactly one well-formed item, before any of it is interpreted. Sizes are checked before anything is allocated, nesting is limited, and unknown variants of fixed enums are errors. The decoders are covered by property tests, by a test that damages valid messages in every position and compares the decoder with an independent CBOR checker, and by fuzz targets. | Implemented in `baton-proto` |
| **Resource limits.** Maximum frame and item sizes checked before allocating, a limit on concurrent streams, timeouts on every read, a limit on concurrent pairing attempts, and a rate limit on items per device. | Partly implemented: size, depth and field limits are checked in `baton-proto`; the rest comes with the transport |
| **Display names.** Names that arrive from a peer have control characters and every invisible or text-reordering character removed (zero-width characters, direction overrides, variation selectors, tag characters, blank Hangul fillers) and are length-limited before they reach a prompt or a log. A name is a label and never a reason to trust a device: look-alike letters survive cleaning, so prompts show the key fingerprint beside it. | Partly implemented: the sanitiser is in `baton-proto`; the prompts and logs that use it come with pairing and the agent |
| **Private discovery.** On the local network a device announces a short value that changes every 15 minutes and that only its paired devices can recognise, instead of its permanent key. See the design goal below. | Planned, after milestone 1 |
| **Key storage in OS keystores** (Keychain, Credential Manager, Secret Service, Android Keystore). | Planned, later milestone |
| **Honoring "concealed" clipboard flags** that password managers set. | Planned, later milestone |

## Design goal: private discovery

A device that wants to be found on the local network announces its public key over mDNS, and with DNS discovery on it also publishes where to reach that key in a public DNS service. A key that never changes is a tracking handle. Anyone on the same café Wi-Fi learns it, and can then ask public DNS from anywhere, at any later time, whether the device is online and roughly which region it is in. Other tools in this space broadcast stable identifiers in the same way. Baton aims to do better: someone who is not paired with a device should learn nothing that links one sighting of it to another.

The sketch below is to be refined when it is built.

- At pairing, each device gives the other a random 32-byte *presence key*.
- On the local network a device announces only `BLAKE3-keyed(presence key, current 15-minute window)` cut to about eight bytes, plus its addresses. A paired device works out the value it expects from each peer, accepting the neighbouring windows to allow for clock skew. Everyone else sees a value that changes every window and links to nothing. It is the same idea as the resolvable private addresses in Bluetooth.
- iroh's discovery is pluggable, so this can be a discovery service of Baton's own rather than a fork.
- On the internet side, the trust store already remembers each peer's home relay, which is enough to dial it. A DNS lookup is needed only after that relay changes, so DNS discovery stays off by default until a record under a name derived for each pair can replace the one published under the device key.

Open questions: whether there is one presence key per device or one per pair (a key per pair makes the announcement larger and shows how many devices are paired, while a key per device has to be replaced and handed out again whenever a device is unpaired), and how a device that was offline for longer than a window catches up.

## Known limitations

- **Metadata is not hidden.** To find devices on the local network, Baton announces its public key and addresses over mDNS to everyone on that network. The service name is specific to Baton, but a person on the same Wi-Fi can see that a Baton device is present. Because the key never changes, it can also be used to recognise the same device on other networks. Private discovery, described above, is the planned fix. Relays see which keys talk to each other and from where. Turn LAN discovery off in settings if that matters to you.
- **A ticket is a bearer secret.** Anyone who gets it before it expires or is used can request pairing. The approval step is the safeguard: compare the six-digit code on both devices, and decline if the other device shows no code or a different one. Do not paste tickets into services that keep logs. A QR code swapped for a different one by someone with physical access would pair the joiner with their device, and the confirmation code cannot catch that, because it matches between the joiner and the device it really talked to. The device you meant to pair with never gets a request, so notice when it does not, and compare the fingerprint the joiner prints with the one on the device you meant.
- **The clipboard may contain secrets.** Automatic clipboard sync is off by default. Until Baton honors the "concealed" flags password managers set, turning it on for a device means everything copied on the other devices is sent to it.
- **The inbox is stored on disk unencrypted**, readable only by the owner. Anything you send to a device can be read from its inbox by anyone who can read that user's files.
- **Pairing is not atomic.** If the connection drops after the offering side approves but before the joiner stores the result, the two sides disagree about whether they are paired. Run pairing again.
- **Compromise of a paired device is not contained by cryptography**, only by policy. Remove the device from the others as soon as you know.
