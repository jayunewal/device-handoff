# Baton

**Pick up where you left off, on any device.**

Baton is an open source app for moving your work between your own devices. Send a link, some text or files from one device to another, and look at and control your Mac from your iPhone or iPad, with its sound. Devices talk directly over your local network when they can and fall back to the internet when they can't. Everything between devices is end-to-end encrypted. "Baton" is a working name and may change.

> **Status: pre-alpha.** Baton is built for one Mac with iPhones and iPads, installed from source with Xcode. Most of it has been tried on real devices only in part, and the newest pieces (unlocking the Mac with Face ID, and the question "Lock the Mac?" at the end of a session) are built and tested in CI but have not yet been tried on real devices. The table below says where each piece stands.

## What works today

| Feature | Mac | iPhone and iPad | Windows and Linux | Android | Where it stands |
| --- | --- | --- | --- | --- | --- |
| Pair two devices (QR code or pasted code, six-digit check) | App | App | `baton` program | Not built | Tried on devices |
| Send links and text, with an inbox | App | App | `baton` program | Not built | Tried on devices |
| Send files and folders (the other side says yes first) | App, files and folders | App, files only | Not built | Not built | Tried between a Mac and an iPhone |
| Look at a window or the whole screen of the Mac | Shares (macOS 14 or newer) | Looks | Not built | Not built | Tried on an iPhone and an iPad |
| Control the Mac by touch and keyboard | Is controlled (macOS 15.2 or newer) | Controls | Not built | Not built | Built; not fully tried on devices |
| Privacy mode: the Mac stays dark, its own keyboard and trackpad are held, and it locks when the session ends | Yes (macOS 15.2 or newer) | Starts it | Not built | Not built | Built; not fully tried on devices |
| Hear the Mac's sound on the phone, with the Mac quiet meanwhile | Sends (macOS 14.2 or newer) | Plays | Not built | Not built | First try worked on an iPad |
| "Lock the Mac?" when a private session ends at the phone (Lock, Leave it, Cancel) | Yes (macOS 15.2 or newer) | Asks | Not built | Not built | Built, not yet tried on devices |
| Unlock the Mac from the phone with Face ID or Touch ID | Is unlocked (macOS 15.2 or newer) | Unlocks | Not built | Not built | Built, not yet tried on devices |
| Receive while the app is closed or the phone is locked | Not built | Not built | `baton listen` in a window | Not built | Needs the background agent and push |

An iPhone or iPad can look at and control a Mac. It cannot be controlled itself, which iOS does not allow, and showing its screen is not built. Windows and Linux have only the `baton` command-line program for now, which pairs and sends links and text. The Android app is on hold until the Apple apps are released. A Windows PC as a screen to look at is planned.

## Run it on each device

- **Mac**: build the app with Flutter and Xcode, or download the build CI makes for every change merged into `main`. [docs/install-the-app.md](docs/install-the-app.md#the-mac-app) has both.
- **iPhone and iPad**: build on your Mac and install with a cable. A free Apple ID is enough, but an app installed that way stops opening after seven days and has to be installed again. [docs/install-the-app.md](docs/install-the-app.md#the-iphone-or-ipad).
- **Look at and control the Mac, hear it, unlock it**: [docs/install-the-app.md](docs/install-the-app.md#look-at-and-control-the-mac) says which permissions the Mac needs and how to start a session from either device.
- **Windows, Linux and Mac from the command line**: download the `baton` program from CI or build it with Cargo. [docs/try-it.md](docs/try-it.md).
- **Android**: not built.

If you rebuild the Mac app, macOS forgets the permissions it gave the old build (Accessibility, and Screen & System Audio Recording), and sharing or control stops working without saying why. Run `tccutil reset All io.github.jayunewal.baton`, start Baton, allow the permissions again when it asks, then quit Baton and start it once more.

## What Baton can and cannot do

"Switch devices" is really two different features, and operating systems treat them very differently:

1. **Hand off context.** Links, text and files can be moved to another device and opened there. Browser tabs, and the clipboard in the apps, are planned.
2. **Stream a live screen.** Show a window or the whole screen of your Mac on your iPhone or iPad, with touch and keyboard control and the Mac's sound.

What no third-party app can do on any platform is reach into another app and pull out its internal state, such as an unsaved document. Apple's Handoff and Microsoft's Cross-device Resume only work because the apps take part. Baton hands off what is portable and streams the rest.

The research behind these limits, and the ideas for working around them, is in [docs/feasibility.md](docs/feasibility.md).

## Principles

- **Local first.** Devices on the same network connect directly. The internet, through a relay, is the fallback.
- **End-to-end encrypted.** Devices authenticate each other with keys that never leave them. A relay only forwards ciphertext.
- **No account required.** You pair devices with a QR code or a one-time ticket you copy across, and confirm with a six-digit code that both devices show. Optional, self-hostable accounts will make pairing automatic later.
- **Open protocol.** The wire format is specified in [docs/protocol.md](docs/protocol.md), so other implementations are possible.
- **Works while locked.** You hand work off so that you can walk away, so Baton is designed to keep working while the device you left is locked. What each operating system allows, with the evidence and the tests still to run on real hardware, is in [docs/locked-devices.md](docs/locked-devices.md); [docs/architecture.md](docs/architecture.md#locked-devices) has the short form of the rules.
- **Fast.** File transfer is Baton's own protocol, built to fill the link, and its speed is measured by a benchmark that lives in the repository.
- **Small trusted core.** Networking, cryptography and the protocol live in one Rust library. Each platform adds a thin layer for what only it can do, such as the clipboard or screen capture.

## Roadmap

| Milestone | Scope | State |
| --- | --- | --- |
| 1 | Rust core: device identity, pairing, LAN discovery, encrypted transport with relay fallback. Clipboard, link and file handoff between two desktop clients from a command-line app, with Baton's own fast file transfer and a benchmark. Designed to work while devices are locked. CI on Linux, macOS and Windows. | In progress: pairing, links and text in the `baton` program; files, the background agent and the benchmark still to build |
| 2 | Flutter apps for desktop and mobile that wrap the core. Pairing by QR code, send and receive links, clipboard and files. | In progress: pairing, links, text and files on iPhone, iPad and Mac |
| 3 | Browser tabs and sharing. Browser extension, phone share sheet, iOS Shortcuts. | Planned |
| 4 | Self-hostable account server: automatic device list, signaling and push notification gateway, so devices on the same account find each other. | Planned |
| 5 | Screen streaming from a Mac to an iPad or phone, with control, privacy mode, sound and unlock from the phone. A Windows PC as the source follows. | Built for the Mac; trying on real devices |
| Later | Android app, iPhone control kit, Mac bridge for iPhone Mirroring, Linux screen capture, microphone and camera. | Ideas, Android on hold |

The order is a proposal and can change. See [docs/roadmap.md](docs/roadmap.md) for the detail behind each milestone.

## Repository layout

| Path | What lives there |
| --- | --- |
| `crates/baton-proto` | Wire protocol: message types, framing, the pairing handshake and ticket format. No networking. |
| `crates/baton-core` | Device identity, the trust store, per-device policy and settings, the state directory, and the encrypted transport: discovery, pairing, admission control and the session protocol between paired devices. |
| `crates/baton-fs` | File system calls that the standard library does not have: moving a file or a folder to a new name without replacing what has that name, marking a received file as one that came from elsewhere, hiding a folder, asking how much room a volume has, and making an open file wait for its data. They need `unsafe` code, so they have a crate of their own. |
| `crates/baton-stream` | Screen streaming without any input or output: the start of a stream, frames of video and sound, the accounting of frames in flight, what a viewer that controls the screen may do and the rules that confine it, the sound, and the unlock. See [ADR 0009](docs/adr/0009-window-streaming.md) and the records after it. |
| `crates/baton-stream-abi` | The small C interface through which the Swift code of an Apple device captures, shows, plays and controls a screen, and the checks between it and the Rust side. |
| `crates/baton-app` | What a graphical app calls: start Baton, pair devices, send, read the inbox and hear what happens, all as plain values. No protocol logic of its own. |
| `crates/baton-cli` | The `baton` command-line program: pair devices, send links and text, read the inbox, and listen for what other devices send. |
| `crates/baton-ffi` | The bridge to Dart: copies of `baton-app`'s values and calls, and the glue [flutter_rust_bridge](https://github.com/fzyzcjy/flutter_rust_bridge) generates around them. |
| `apps/baton` | The Flutter app for iPhone, iPad and Mac (Android is on hold), its Swift code for the screen, sound and unlock, and the generated Dart side of the bridge. See [apps/baton/README.md](apps/baton/README.md). |
| `docs/` | Architecture, protocol and security documents, the locked-device page, and architecture decision records in `docs/adr/`. The research notes behind the locked-device page, privacy mode and sound are in `docs/research/`. |

More crates are added as milestone 1 lands (`baton-agent`).

## Building

You need a recent stable Rust toolchain (the minimum supported version is in `Cargo.toml`).

```sh
cargo build --workspace
cargo test --workspace
cargo run -p baton-cli -- --help
```

The last command prints the help of the `baton` program.

The app needs Flutter on top of that; [apps/baton/README.md](apps/baton/README.md) says what to install and how to run its tests, and [docs/install-the-app.md](docs/install-the-app.md) how to build it and put it on a device.

[docs/try-it.md](docs/try-it.md) walks through pairing two computers and sending a link and some text. CI builds the `baton` program for macOS, Windows and Linux for every change that is merged into `main`; the guide says where to download it.

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) first. Report security problems privately, as described in [SECURITY.md](SECURITY.md).

## License

Apache License 2.0. See [LICENSE](LICENSE).

Baton is informed by the design of [RustDesk](https://github.com/rustdesk/rustdesk), [KDE Connect](https://kdeconnect.kde.org/) and [Syncthing](https://syncthing.net/), and builds on [iroh](https://github.com/n0-computer/iroh). It does not contain code from projects whose licenses are incompatible with Apache-2.0.
