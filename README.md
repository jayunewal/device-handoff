# Baton

**Pick up where you left off, on any device.**

Baton is an open source app for moving your work between your own devices: send a link, a browser tab, a file or your clipboard from one device to another, and later stream a window from your Mac or PC to your iPad or phone. It is meant to work between macOS, Windows, Linux, iOS, iPadOS and Android, in every direction. Devices talk directly over your local network when they can and fall back to the internet when they can't. Everything between devices is end-to-end encrypted.

> **Status: pre-alpha.** Milestone 1 (the Rust core and a command-line client) is under construction. Nothing here is ready to use yet. "Baton" is a working name and may change.

## What Baton can and cannot do

"Switch devices" is really two different features, and operating systems treat them very differently:

1. **Hand off context.** Links, browser tabs, files and the clipboard can be moved to another device and opened there. This works on every platform.
2. **Stream a live window.** Show a window from your Mac, Windows PC or Android device on another device, with touch or keyboard control. This is planned for a later milestone.

What no third-party app can do on any platform is reach into another app and pull out its internal state, such as an unsaved document. Apple's Handoff and Microsoft's Cross-device Resume only work because the apps take part. Baton hands off what is portable and streams the rest. iPhones and iPads can send content and view streams, but they cannot be remotely controlled by another device, because iOS does not allow it.

The research behind these limits, and the ideas for working around them, is in [docs/feasibility.md](docs/feasibility.md).

## Principles

- **Local first.** Devices on the same network connect directly. The internet, through a relay, is the fallback.
- **End-to-end encrypted.** Devices authenticate each other with keys that never leave them. A relay only forwards ciphertext.
- **No account required.** You pair devices with a QR code or a one-time code. Optional, self-hostable accounts will make pairing automatic later.
- **Open protocol.** The wire format is specified in `docs/protocol.md` (it arrives with the protocol crate), so other implementations are possible.
- **Works while locked.** You hand work off so that you can walk away, so Baton is designed to keep working while the device you left is locked. What each operating system allows is still being researched; see [docs/architecture.md](docs/architecture.md#locked-devices).
- **Fast.** File transfer is Baton's own protocol, built to fill the link, and its speed is measured by a benchmark that lives in the repository.
- **Small trusted core.** Networking, cryptography and the protocol live in one Rust library. Each platform adds a thin layer for what only it can do, such as the clipboard or screen capture.

## Roadmap

| Milestone | Scope | State |
| --- | --- | --- |
| 1 | Rust core: device identity, pairing, LAN discovery, encrypted transport with relay fallback. Clipboard, link and file handoff between two desktop clients from a command-line app, with Baton's own fast file transfer and a benchmark. Designed to work while devices are locked. CI on Linux, macOS and Windows. | In progress |
| 2 | Flutter apps for desktop and mobile that wrap the core. Pairing by QR code, send and receive links, clipboard and files. | Planned |
| 3 | Browser tabs and sharing. Browser extension, phone share sheet, iOS Shortcuts. | Planned |
| 4 | Self-hostable account server: automatic device list, signaling and push notification gateway, so devices on the same account find each other. | Planned |
| 5 | Window streaming from Mac and Windows to an iPad or phone, with input. | Planned |
| Later | iPhone control kit, Mac bridge for iPhone Mirroring, Linux screen capture. | Ideas |

The order is a proposal and can change. See [docs/roadmap.md](docs/roadmap.md) for the detail behind each milestone.

## Repository layout

| Path | What lives there |
| --- | --- |
| `crates/baton-proto` | Wire protocol: message types, framing, pairing ticket format. No networking. |
| `docs/` | Architecture, protocol and security documents, and architecture decision records in `docs/adr/`. |

More crates are added as milestone 1 lands (`baton-core`, `baton-agent`, `baton-cli`). Flutter apps will live under `apps/`.

## Building

You need a recent stable Rust toolchain (the minimum supported version is in `Cargo.toml`).

```sh
cargo build --workspace
cargo test --workspace
```

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) first. Report security problems privately, as described in [SECURITY.md](SECURITY.md).

## License

Apache License 2.0. See [LICENSE](LICENSE).

Baton is informed by the design of [RustDesk](https://github.com/rustdesk/rustdesk), [KDE Connect](https://kdeconnect.kde.org/) and [Syncthing](https://syncthing.net/), and builds on [iroh](https://github.com/n0-computer/iroh). It does not contain code from projects whose licenses are incompatible with Apache-2.0.
