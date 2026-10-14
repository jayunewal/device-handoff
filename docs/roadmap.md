# Roadmap

A proposal, not a promise. Each milestone ends in something a person can use, and each is built as a series of small pull requests.

## Milestone 1: core and command-line handoff

Scope: the Rust core, and a command-line client that hands off clipboard text and links between two desktop machines.

- Device identity and a persistent trust store.
- Pairing with a one-time ticket, approved on the offering device.
- Discovery on the local network (mDNS) and connection by public key, with the internet relay as a fallback.
- Handoff of links and clipboard text with per-device policies and an inbox.
- Continuous integration on Linux, macOS and Windows; license checks on dependencies.

Done when: two machines on the same Wi-Fi pair with `baton`, a link sent from one is waiting in the other's inbox (or opens, if the person opted in), and the same works across different networks through a relay.

## Milestone 2: apps

Flutter apps for macOS, Windows, Android, iOS and iPadOS that wrap the core. QR pairing, a device list, sending and receiving links and clipboard. Tray icon and notifications on desktop.

## Milestone 3: files and tabs

Send files and the current browser tab. A browser extension, the phone share sheet, and iOS Shortcuts and App Intents ("Send this to my PC").

## Milestone 4: accounts

A self-hostable server that keeps a device roster, so devices signed in to the same account find and trust each other without scanning QR codes. It also runs signaling, a relay and a push gateway for iOS. Sign-in by email link or OpenID Connect. QR pairing stays for people who want no server.

## Milestone 5: window streaming

Stream a window from a Mac or Windows PC to an iPad or phone, with touch and keyboard control. Needs capture and encode on the source, decode and render on the viewer, and a transport choice for video (QUIC streams or WebRTC).

## Later

- iPhone control kit: AirPlay video plus a Bluetooth keyboard and mouse emulator, with a proof of concept to settle pointer calibration.
- Mac bridge that exposes the iPhone Mirroring window to other devices.
- Linux screen capture, audio, multi-monitor.

## Open questions

- Video transport for milestone 5: QUIC datagrams and streams on the existing connection, or WebRTC?
- Where each OS should keep the device key (Keychain, Credential Manager, Secret Service, Android Keystore).
- Who hosts the default relay and account server, and how that is paid for.
- Short typed pairing codes (a password-authenticated key exchange) for devices that cannot scan a QR code.
