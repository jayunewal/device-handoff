# Feasibility notes

A summary of the research done before the first line of code, October 2026. Platform behavior changes between OS releases, so re-check a specific claim before building on it.

The question: can one open source app let you leave one device and pick up the same work on another, across macOS, Windows, iOS, iPadOS and Android, over the local network first and the internet as a fallback?

**Yes, with one split.** Two different features get called "handoff", and the operating systems treat them very differently.

- **Live window streaming** (see and control the actual window from another device) works when the source is a Mac, a Windows PC or an Android device. Any device, including an iPad or iPhone, can be the viewer.
- **App-state handoff** (the same document opens natively in the app on the other device) only works for state that is portable: browser tabs and URLs, files, the clipboard, cloud documents reachable by link, and the app's own data. No OS lets a third-party app pull the internal state out of another app. Apple Handoff and Microsoft's Cross-device Resume work because the apps publish their own state.
- **iPhone and iPad as the source** is the hard wall. iOS can broadcast the whole screen, view-only, started by the user each time, and cannot be remotely controlled. Handoff out of iOS goes through the share sheet, Shortcuts and links.

So the realistic product is a hybrid: hand off context when it is portable, and stream the window when it is not.

## What each OS allows (as the source device)

| Source | Capture a window | Remote input | Main restriction |
| --- | --- | --- | --- |
| macOS | Per window (ScreenCaptureKit) | Yes (CGEvent injection) | Screen Recording and Accessibility permission prompts; macOS re-asks for screen recording consent periodically. |
| Windows | Per window (Windows.Graphics.Capture, DXGI) | Yes (SendInput) | Cannot inject into elevated windows unless the agent is elevated; the UAC secure desktop is not capturable. |
| Android | Screen or single app (MediaProjection) | Limited (AccessibilityService, or adb) | Consent every session; Google Play scrutinizes accessibility use; secure screens are blanked. |
| iPadOS, iOS | Whole screen, view only (ReplayKit broadcast extension) | No | User starts the broadcast from Control Center; the extension has a small memory cap; protected content is blanked. |
| Linux | Per window (PipeWire portal, X11) | Yes | Wayland requires the portal consent flow. |

Every platform can act as a viewer and controller of another device's stream.

## iPhone Mirroring cannot be reused

Apple's iPhone Mirroring is undocumented, tied to the Apple Account and Apple hardware identity, and runs over Apple's private peer-to-peer Wi-Fi. The phone side is an Apple system service, so a third-party device would have to impersonate a genuine Mac. No public open source project reimplements it, and similar efforts (Beeper Mini's iMessage bridge in 2023) were shut down by Apple within weeks. Apps using private APIs are rejected by the App Store.

What works on public APIs: a Mac agent that captures the iPhone Mirroring window and forwards input into it, so the phone can be viewed and controlled from other devices. It needs a Mac that is on, and iPhone Mirroring has not been available in the EU.

## Ideas for controlling an iPhone or iPad anyway

An open source license does not remove legal or terms-of-service risk, and publishing a tool can carry more risk than using one privately. This is not legal advice. The ideas below are ordered from firmest ground to riskiest.

| Idea | Gets you | Needs | Practicality |
| --- | --- | --- | --- |
| Fake Bluetooth keyboard and mouse | Control. iOS accepts Bluetooth keyboards and mice, and shows a pointer with AssistiveTouch on. | An ESP32 dongle, a Raspberry Pi, or an Android phone acting as a Bluetooth HID device. Pointer calibration against the video. | High. The best idea; needs a proof of concept. |
| AirPlay receiver (UxPlay) | Live picture from iOS Screen Mirroring. | The user starts mirroring from Control Center; same network. | High. With the item above it gives full wireless view and control. |
| Cable capture, PiKVM style | Lowest-latency picture plus control. | USB-C device, hub, HDMI capture stick, nearby Pi or PC. | Medium. A desk setup; DRM video shows black. |
| Always-on Mac as a bridge | iPhone control from anywhere via iPhone Mirroring. | A spare Mac signed in to the same Apple Account. | High, but costs a Mac. |
| Developer Mode tooling (WebDriverAgent with go-ios or pymobiledevice3) | View and tap over USB or Wi-Fi. | Developer Mode and a self-signed helper app (free accounts re-sign every 7 days). | Medium. Power users. |
| Shortcuts and App Intents | Better context handoff ("Send this to my PC"). | Nothing beyond Baton. | High. Not mirroring, but makes iOS feel close to native. |
| Jailbreak | Everything. | A jailbreakable iOS version. | Low, and breaks on every update. |
| Cloning Apple's protocols | In theory, iPhone Mirroring elsewhere. | Months of reverse engineering. | Very low, with the highest legal risk. Not a foundation. |

## Connectivity

Local first, internet as a fallback, is a solved problem on every platform:

1. Discover devices on the LAN (mDNS or DNS-SD; iOS needs the Local Network permission).
2. Pair once per device pair (QR code or one-time code, exchanging public keys).
3. Connect: direct LAN address, then NAT hole punching over the internet, then a relay.
4. Encrypt end to end with the paired keys, so a relay sees only ciphertext.

WebRTC gives hardware codecs and jitter handling for video for free but needs signaling and TURN servers. iroh dials by public key, tries direct paths first and falls back to self-hostable relays. Baton uses iroh for the control and data plane (see [ADR 0003](adr/0003-iroh-transport.md)); the video transport is a decision for the streaming milestone.

## Accounts and same-account sync

An account is simply a list of your devices and their public keys. Once a device is on the list it discovers and trusts the others without extra pairing. The account server only introduces devices to each other; content still travels peer to peer and stays end-to-end encrypted through a relay.

- **No server:** add a device by scanning a QR code from one already in the group. Fully private, but there is no "sign in on a new laptop" without another device at hand.
- **Self-hostable sync server (recommended):** sign in with an email link or OpenID Connect. One small server binary can also run signaling, the relay and push forwarding. Ship a hosted default and let people run their own.

Platform limits: iOS and iPadOS suspend apps in the background, so a phone cannot hold a live connection; it catches up through push notifications (self-hosted servers need a small shared push gateway for Apple's push service) or when the app opens. Android 10 and later block background clipboard reads, and iOS shows a paste banner, so clipboard sync from a phone happens while the app is open or through the share sheet.

## Open source projects to learn from

| Project | Why it matters | License |
| --- | --- | --- |
| RustDesk | Remote desktop on all five platforms with a self-hostable relay. Closest to the streaming half. | AGPL-3.0 |
| KDE Connect | Clipboard, file and URL sharing; model for pairing and plugins. | GPL |
| Sunshine and Moonlight | Low-latency GPU streaming. | GPL-3.0 |
| scrcpy | Android mirroring and control over adb. | Apache-2.0 |
| Syncthing | Discovery and relay pools. | MPL-2.0 |
| iroh | Connections by public key with relay fallback. | MIT or Apache-2.0 |

Baton learns from the copyleft projects but does not copy their code (see [ADR 0004](adr/0004-apache-2-license.md)).

## Biggest risks

- App Store and Play Store review of remote-control and accessibility features.
- Latency and battery cost of video encoding on phones.
- Relay hosting cost once people rely on the internet fallback.
- Push delivery for self-hosted servers, which needs a shared gateway for iOS.
