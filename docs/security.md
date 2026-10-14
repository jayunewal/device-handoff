# Security model

What Baton protects, who it defends against, and how. Mechanisms marked *planned* are designed but not built yet; each pull request that builds one flips its status here, so this page never claims more than the code does.

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
| Someone who sees a pairing ticket (QR code over the shoulder, a leaked screenshot) | Try to pair | A ticket is single-use, short-lived and limited to a few attempts, and the person on the offering device must approve the request. |
| A paired device that is lost, stolen or compromised | Send items as a trusted peer | The receiver's per-device policy limits the impact: items go to an inbox, and links and clipboard are not acted on automatically unless the person opted in. Unpairing removes the key and closes live connections. |
| Another local user or process on the same machine | Read key files, talk to the control socket | Key and state files are readable only by the owner. The control socket is private to the user. |

Out of scope: an attacker who controls an unlocked user session or the operating system, traffic analysis by a global observer, and hiding metadata from relays.

## Mechanisms

| Mechanism | Status |
| --- | --- |
| **Device identity.** An Ed25519 key per device, generated on first run. The public key is the device's identity everywhere. | Planned |
| **Authenticated, encrypted transport.** QUIC and TLS 1.3 through iroh; both ends are identified by their keys, so a connection to a given identity cannot be impersonated. | Planned |
| **Access control after the handshake.** The session protocol accepts only keys in the trust store. The pairing protocol accepts connections only while an offer is open. Everything else is closed with an error code before any message is read. | Planned |
| **Pairing.** One device creates a ticket holding its address and a random 256-bit secret, valid for five minutes and for one successful use. The joiner proves it knows the secret with a keyed hash bound to both device keys and to the TLS session, so the secret itself is not sent and a proof cannot be replayed on another connection. After three wrong proofs the ticket is void. The offering person approves the request, seeing the joiner's name and key fingerprint. | Planned |
| **Trust store.** Paired keys are stored with names and policies, written atomically, in a file only the owner can read. Removing a device closes its live connections. | Planned |
| **Handoff policy.** Per paired device, on the receiving side. Defaults: links and clipboard text go to an inbox. Auto-opening links and auto-applying the clipboard are opt-in. Only `http` and `https` links are ever opened. | Planned |
| **Resource limits.** Maximum frame and item sizes checked before allocating, a limit on concurrent streams, timeouts on every read, a limit on concurrent pairing attempts, and a rate limit on items per device. | Planned |
| **Display names.** Names that arrive from a peer have control and bidirectional-override characters removed and are length-limited before they reach a prompt or a log. | Planned |
| **Key storage in OS keystores** (Keychain, Credential Manager, Secret Service, Android Keystore). | Planned, later milestone |
| **Honoring "concealed" clipboard flags** that password managers set. | Planned, later milestone |

## Known limitations

- **Metadata is not hidden.** To find devices on the local network, Baton announces its public key and addresses over mDNS to everyone on that network. The service name is specific to Baton, but a person on the same Wi-Fi can see that a Baton device is present. Relays see which keys talk to each other and from where. Turn LAN discovery off in settings if that matters to you.
- **A ticket is a bearer secret.** Anyone who gets it before it expires or is used can request pairing. The approval step is the safeguard, so read the name and fingerprint you are asked to approve. Do not paste tickets into services that keep logs. A QR code swapped for a different one by someone with physical access would pair you with them; the joiner prints the fingerprint of the device it paired with so it can be compared.
- **The clipboard may contain secrets.** Automatic clipboard sync is off by default. Until Baton honors the "concealed" flags password managers set, turning it on for a device means everything copied on the other devices is sent to it.
- **The inbox is stored on disk unencrypted**, readable only by the owner. Anything you send to a device can be read from its inbox by anyone who can read that user's files.
- **Pairing is not atomic.** If the connection drops after the offering side approves but before the joiner stores the result, the two sides disagree about whether they are paired. Run pairing again.
- **Compromise of a paired device is not contained by cryptography**, only by policy. Remove the device from the others as soon as you know.
