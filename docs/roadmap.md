# Roadmap

A proposal, not a promise. Each milestone ends in something a person can use, and each is built as a series of small pull requests.

## Requirement that cuts across every milestone

**Baton keeps working while the device you left is locked.** The point of handing work off is to walk away from the first device, so nothing may depend on that device staying unlocked. This shapes the agent, the key storage, the transport and window streaming, and [architecture.md](architecture.md#locked-devices) lists the design rules that follow from it. [locked-devices.md](locked-devices.md) records what each operating system allows, the design rules that follow, and the behaviour that still has to be verified on real hardware.

## Milestone 1: core, handoff and fast file transfer

Scope: the Rust core, and a command-line client that hands off clipboard text, links and files between two desktop machines.

- Device identity and a persistent trust store.
- Pairing with a one-time ticket, approved on the offering device after comparing a six-digit code shown on both.
- Discovery on the local network (mDNS) and connection by public key, with the internet relay as a fallback.
- Handoff of links and clipboard text with per-device policies and an inbox.
- File and folder transfer in Baton's own protocol, built to fill the link. A folder is one offer with a manifest, files move over parallel resumable streams, and hashing happens while sending. A benchmark that anyone can rerun is part of the milestone, with four cases that each hit a different limit: one large file, thousands of small files, Wi-Fi under load, and a relayed connection. Each run also measures the raw throughput of the link, so the numbers mean the same on any network. The first two cases have targets, which the benchmark will confirm or change: one large file should reach at least 90% of the raw throughput (about 100 MB/s on gigabit Ethernet), and 10,000 files of 10 KiB should take no more than twice as long as one file of the same total size. The other two cases are reported without a target until there are real measurements. There is no compatibility with other tools' protocols.
- A small probe program that records lock, sleep and resume events and answers pings, so that the tests listed in [locked-devices.md](locked-devices.md#8-to-verify-on-real-hardware) can be run on real machines.
- Continuous integration on Linux, macOS and Windows; license checks on dependencies.

Done when: two machines on the same Wi-Fi pair with `baton`, a link sent from one is waiting in the other's inbox (or opens, if the person opted in), the benchmark meets the two targets above, and the same works across different networks through a relay.

## Milestone 2: apps

Flutter apps for macOS, Windows, Android, iOS and iPadOS that wrap the core. QR pairing, a device list, sending and receiving links, clipboard and files. Tray icon and notifications on desktop. The apps register the `baton1:` URL scheme, which pairing tickets already start with, so scanning a ticket's QR code with the phone's own camera opens Baton on the pairing screen.

## Milestone 3: tabs and sharing

Send the current browser tab. A browser extension, the phone share sheet, and iOS Shortcuts and App Intents ("Send this to my PC").

## Milestone 4: accounts

A self-hostable server that keeps a device roster, so devices signed in to the same account find and trust each other without scanning QR codes. It also runs signaling, a relay of its own and a push gateway for iOS. The relay is part of the server because the public relays are rate limited and meant for development and hobby use, which would cap file transfers on every connection that cannot go direct. Sign-in by email link or OpenID Connect. QR pairing stays for people who want no server.

## Milestone 5: window streaming

Stream a window from a Mac or Windows PC to an iPad or phone, with touch and keyboard control. Needs capture and encode on the source, decode and render on the viewer, and a transport choice for video (QUIC streams or WebRTC). No documented API captures a locked desktop (see [locked-devices.md](locked-devices.md)), so the first version needs the source unlocked while it streams and can lock it afterwards. A privacy mode or a separate session may follow.

## Later

- iPhone control kit: AirPlay video plus a Bluetooth keyboard and mouse emulator, with a proof of concept to settle pointer calibration.
- Mac bridge that exposes the iPhone Mirroring window to other devices.
- Linux screen capture, audio, multi-monitor.

## Open questions

- Video transport for milestone 5: QUIC datagrams and streams on the existing connection, or WebRTC?
- Where each OS should keep the device key (Keychain, Credential Manager, Secret Service, Android Keystore). The answer must stay readable at the lock screen.
- Who hosts the default relay and account server, and how that is paid for.
- Short typed pairing codes (a password-authenticated key exchange) for devices that cannot scan a QR code.
- Which of the things a person might continue (current tabs, the clipboard) a locked device may serve to a paired device, and under which policy.
- How long a hand-off may keep a device awake. The sleep tests in [locked-devices.md](locked-devices.md) should settle the bound.
