# Architecture

This document describes how Baton is put together. The trust model is in [security.md](security.md). The wire format is specified in [protocol.md](protocol.md).

## Layers

```
 device A                                        device B
┌──────────────────────────────┐                ┌──────────────────────────────┐
│ UI: command line, later      │                │ UI                           │
│ Flutter apps                 │                │                              │
├──────────────────────────────┤                ├──────────────────────────────┤
│ baton-agent                  │                │ baton-agent                  │
│ clipboard, opening links,    │                │                              │
│ policies, inbox              │                │                              │
├──────────────────────────────┤                ├──────────────────────────────┤
│ baton-core                   │  QUIC + TLS    │ baton-core                   │
│ identity, trust store,       │◄──────────────►│                              │
│ pairing, handoff service     │  (via iroh)    │                              │
├──────────────────────────────┤                ├──────────────────────────────┤
│ baton-proto                  │                │ baton-proto                  │
│ message types, framing       │                │                              │
└──────────────────────────────┘                └──────────────────────────────┘
```

| Layer | Responsibility | Platform code? |
| --- | --- | --- |
| `baton-proto` | Message types, framing and the pairing ticket format. No I/O, so it can be tested and fuzzed on its own. | No |
| `baton-core` | Device identity, the trust store, pairing, discovery, the encrypted transport and the handoff service. Everything that must behave identically on every platform. | No |
| `baton-agent` | The parts that touch the desktop: reading and writing the clipboard, opening links, per-device policies and the inbox of received items. | Yes (desktop) |
| UI | Presents devices, pairing and received items. Command line first, Flutter apps later. | Yes |

On mobile, the Flutter app calls `baton-core` through an FFI layer and implements the equivalent of `baton-agent` natively (share sheet, Shortcuts, Android intents), because those platforms do not let a background process read the clipboard or open links the way a desktop agent does.

## Connecting devices

Every device has an Ed25519 key pair. The public key is the device's identity. Devices dial each other by that key, and the QUIC/TLS handshake proves both sides hold the keys they claim, so a connection to a given identity cannot be impersonated.

Reaching a device uses the best path available, tried in this order:

1. **Local network.** Devices announce themselves on the LAN with mDNS and connect directly.
2. **Direct over the internet.** A relay helps both sides learn each other's public addresses and hole-punch through NAT.
3. **Relay.** If no direct path works, traffic flows through a relay. The relay forwards encrypted packets addressed to a public key. It cannot read or alter them.

The relays Baton uses by default are the iroh project's public ones. According to iroh's documentation they are rate limited and meant for development and hobby use, so a transfer that cannot go direct is only as fast as that shared limit. That is fine for trying Baton out. The self-hostable server runs a relay of its own with limits its owner sets, and the file transfer benchmark includes a relayed run so the cost shows up in the numbers.

Connection handling comes from [iroh](https://github.com/n0-computer/iroh) (see [ADR 0003](adr/0003-iroh-transport.md)). The rest of Baton talks to it through a small boundary in `baton-core` so the transport can change.

What a device knows about how to reach another is kept with the pairing: the ticket carries the offerer's relay and direct addresses, and after every connection each side remembers the relay and addresses that worked. A device that restarts can therefore reach its paired devices before discovery has found anything, and devices that cannot find each other any other way still connect through a relay.

## Inside baton-core

| Module | Job |
| --- | --- |
| `identity`, `trust`, `config`, `paths` | The device key, the list of paired devices, settings, and the state directory. |
| `node` | Starts and stops everything: the network endpoint, discovery and the router. A `Node` is what an application holds. |
| `gate` | Decides who may connect: `Admission` before the handshake, `Gate` after it, and a registry of open connections so that unpairing can close them. |
| `pairing` | Offers, tickets, approval, and joining another device's offer. |
| `session` | The `baton/1` protocol: serving requests, the pool of outgoing connections, and `Intake`, which checks every incoming item before the application sees it. |
| `transport` | The QUIC settings every connection uses: how many streams a peer may open, keep-alives, and the flow-control windows that set how fast one stream can go and how much unread data a peer can make a device hold. |
| `ratelimit`, `wire`, `link`, `clock`, `util` | Small shared parts: a sliding-window limiter, reading and writing frames with deadlines, describing which path a connection took, a clock that tests can control, and helpers that keep one failing task from taking others down. |

An incoming connection passes through these checks, in this order:

```
incoming packet
   │  Admission     Asks the sender to prove its address first when attempts arrive faster than
   │                the node wants to handle. Costs the node a small reply, no cryptography.
   ▼
QUIC and TLS handshake        both ends are now known by their keys
   │  Gate          baton/1 from a key that is not paired: closed with 403, nothing remembered.
   │                baton-pair/1: allowed, because the person is not paired yet.
   ▼
SessionServer or PairServer   each connection runs as a task of its own, so a bug reachable
   │                          from the network cannot stop the node accepting connections
   │  per request   Still paired? Then Intake: kind of item, size, repeat, rate limit.
   ▼
the application's HandoffHandler
```

Outgoing requests use a pool that keeps one connection per paired device. It opens the connection on first use and says `Hello`, reuses it for later requests, retries once on a new connection if a reused one turned out to be dead, and closes connections that have been idle for two minutes. Unpairing a device closes its connections, whichever side opened them.

## Trust

A device only talks to devices it has paired with. Pairing is explicit: one device offers a one-time ticket (shown as a QR code or text), the other redeems it, and the person on the offering device approves the request after checking that a six-digit code shown on both devices is the same. There is no way to switch the approval off. Both sides then remember each other's public key in a local trust store (`devices.json`, see [State on disk](#state-on-disk)). Connections from any other key are refused after the handshake, before any application data is read. The check is repeated for each request, so unpairing a device ends its access at once, including on connections that are already open.

There are no accounts in milestone 1. Accounts arrive in milestone 4 as an *introducer*: a server that tells your devices about each other. It is never in the data path and never holds the keys needed to read content.

### Same-account sync: a signed roster (design, not built)

Pairing every pair of devices by hand is how tools such as KDE Connect work, and it is tedious past three or four devices. The plan for milestone 4 is a *roster* instead: the list of a person's device keys, kept as a chain of membership changes.

- Each change, adding a device or removing one, is signed by a device that is already in the roster and carries the hash of the change before it. A device checks the whole chain from the first entry before it trusts anything in it.
- Pairing a new phone with any one device adds it to the roster. The others trust it because the chain is signed by a key they already trust, with no further pairing.
- The account server only relays roster changes. It holds no roster key, so it cannot add a device on its own. A compromised server could hold updates back or replay old ones, which the chain and a sequence number make visible, but it could not forge a device. That keeps the promise that the server is an introducer and never in the data path, even if it is compromised.
- A removal is a signed entry too. A device that sees it drops the key like a manual unpairing, including closing live connections.

Open questions: what to do when two devices change the roster at the same time (a fork), how a person recovers when every device is lost, and whether a device that was offline for a long time should accept a long chain without asking.

The trust store leaves room for this before its format is in use: each entry records how the device came to be trusted (`provenance`, today only `paired`), and a device trusted through the roster will also say who vouched for it.

## State on disk

Each device keeps its state in one directory, which belongs to a single device identity. By default that is the per-user *local* data directory (`~/.local/share/baton` on Linux, `~/Library/Application Support/baton` on macOS, `%LOCALAPPDATA%\baton\data` on Windows), never a roaming or synced one, because a copied device key would make two machines the same device. As a second line of defence the identity file records which machine made the key, and Baton refuses it on another one. Set `BATON_HOME` to use another directory, for example to run two instances on one machine.

| File | Contents | Notes |
| --- | --- | --- |
| `identity.toml` | The device's Ed25519 secret key. | Created on first run. Readable by its owner only; Baton refuses to use it if other users can read or change it. |
| `devices.json` | The trust store: paired devices, the name the person knows each by, how each came to be trusted, their last known addresses and their per-device policies. | Replaced atomically on every change. A file that cannot be read is an error, never an empty list. Refused if other users can read or change it. |
| `config.toml` | Settings a person edits by hand (device name, relays, discovery, limits). | Optional. Unknown keys are rejected so a typo cannot silently leave a setting on. Refused if other users can change it. |
| `inbox.json` | Items received and not yet acted on. | Added with the agent. |
| `baton.lock` | An advisory lock. | Only one process may use a state directory at a time. |

State files are written to a temporary file in the same directory, flushed, and renamed over the target, so a crash leaves the old file or the new one. On Unix, files and a directory that Baton creates are readable by their owner only. A state directory that already exists is not changed, because it may be used for other things (think `BATON_HOME=$HOME`), but it is refused if other users can write to it, since they could replace the files in it. `identity.toml` and `devices.json` are refused if other users can read or change them, and `config.toml` if they can change it; the check is made on the file that is then read. Files are read with a size cap, and a file written by a newer, incompatible version is refused rather than misread or overwritten.

## Handoff

Moving something to another device is a *handoff item*: a link, some clipboard text, and later files and tabs. The receiver does not blindly act on an item. What happens next is decided by a per-device policy on the receiving side. By default received links and clipboard text go to an inbox and nothing is opened or overwritten until the person chooses; auto-open and auto-apply are opt-in, per device. See [security.md](security.md) for why.

## Locked devices

Handing work off means walking away from the first device, so that device is usually locked and often asleep. Baton has to work in that state: an app that needs the source unlocked fails at its one job. The rules below follow from that requirement. They are design rules, not a statement of what every platform allows. What each operating system really permits while locked is still being researched, and the results will be written down separately with a list of behaviour to verify on real hardware.

- **The agent does not need an unlocked session.** On desktops it runs as a background service that does not depend on the screen being unlocked. Receiving items, answering pings and serving what is already prepared must work with the screen locked.
- **Capture while the person is there.** What a person might want to continue, such as the clipboard or the current tab, cannot be counted on to be readable once the device locks, and on phones it is readable only while the app is in the foreground. So it is captured while the device is in use and kept ready in a small outbox that paired devices can fetch from. When the device is about to lock or sleep, the agent pushes the latest of it to paired devices that are reachable.
- **Store and forward.** Items that cannot be delivered right now, because the other device is asleep, offline or locked itself, wait in an outbox and are delivered when it can be reached, instead of failing.
- **Keys stay readable at the lock screen.** The device key and the trust store live in storage that is available after the first unlock of the device, not in storage that is sealed whenever the screen locks. A keystore that locks with the screen would make the agent deaf exactly when it is needed.
- **Serving from a locked device is an explicit, per-device choice.** Letting a paired device fetch what was open on a locked machine changes who can see what, so it has its own policy and is off by default.
- **Sleeping devices need a way to be woken.** That is Wake-on-LAN or the platform's network wake on desktops, and push notifications through a gateway on phones (milestone 4). Where there is no wake path, the outbox covers the gap.
- **Limits stay visible.** Some things cannot be done on a locked device, such as capturing a window from a locked desktop without a privacy mode or a virtual display. Those limits are documented per platform, not hidden.

## Platform boundaries

Some things only native code can do, and they are kept out of the core:

| Capability | macOS | Windows | Linux | Android | iOS and iPadOS |
| --- | --- | --- | --- | --- | --- |
| Clipboard | Pasteboard | Win32 clipboard | X11 or Wayland | ClipboardManager (foreground only) | UIPasteboard (foreground only, paste banner) |
| Open a link | Launch Services | ShellExecute | xdg-open | Intents | UIApplication |
| Window capture (milestone 5) | ScreenCaptureKit | Windows.Graphics.Capture | PipeWire | MediaProjection | ReplayKit (view only) |
| Background connection | Yes | Yes | Yes | Foreground service | No; push notification wakes the app |

## Non-goals

- Extracting the internal state of arbitrary apps. No platform allows it.
- Remote control of iOS or iPadOS through Apple's private interfaces.
- Being a general remote desktop or VPN product.
