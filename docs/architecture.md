# Architecture

This document describes how Baton is put together. The trust model is in [security.md](security.md). The wire format will be specified in `protocol.md`, which arrives with the protocol crate.

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

Connection handling comes from [iroh](https://github.com/n0-computer/iroh) (see [ADR 0003](adr/0003-iroh-transport.md)). The rest of Baton talks to it through a small boundary in `baton-core` so the transport can change.

## Trust

A device only talks to devices it has paired with. Pairing is explicit: one device offers a one-time ticket (shown as a QR code or text), the other redeems it, and the person on the offering device approves the request. Both sides then remember each other's public key in a local trust store. Connections from any other key are refused after the handshake, before any application data is read.

There are no accounts in milestone 1. Accounts arrive in milestone 4 as an *introducer*: a server that tells your devices about each other. It is never in the data path and never holds the keys needed to read content.

## Handoff

Moving something to another device is a *handoff item*: a link, some clipboard text, and later files and tabs. The receiver does not blindly act on an item. What happens next is decided by a per-device policy on the receiving side. By default received links and clipboard text go to an inbox and nothing is opened or overwritten until the person chooses; auto-open and auto-apply are opt-in, per device. See [security.md](security.md) for why.

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
