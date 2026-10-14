# 0003: iroh for connections between devices

Status: accepted

## Context

Devices must find and connect to each other on the same network first and over the internet otherwise, with end-to-end encryption and without requiring everyone to run a server. People will connect devices behind home routers and mobile networks, so NAT traversal and a relay fallback are required.

## Decision

Use [iroh](https://github.com/n0-computer/iroh) 1.x for connections. It dials by public key, uses QUIC with TLS 1.3 so that both ends are authenticated by their keys, hole-punches when it can and falls back to relays when it cannot. iroh and its mDNS address lookup crate are licensed MIT or Apache-2.0.

Baton talks to iroh through one module in `baton-core`, so the transport can be replaced without touching the protocol or the agent.

Defaults:

- LAN discovery uses mDNS with a Baton-specific service name, so Baton devices do not show up to unrelated iroh applications.
- The internet fallback uses iroh's default public relays, which are run by n0. Relays are configurable, and the relay server (`iroh-relay`) is open source and self-hostable.
- Publishing a device's relay address to n0's DNS lookup service is optional and publishes only the relay address, never IP addresses.

## Alternatives considered

- **WebRTC** (webrtc-rs or libwebrtc). Excellent for video, with hardware codecs and congestion control, but it needs signaling and TURN servers for any connection, and a heavier native dependency on every platform. It stays an option for the streaming milestone.
- **libp2p.** Broader than we need and more moving parts to configure for a two-device use case.
- **A VPN overlay** (Tailscale, Headscale, WireGuard). Needs the user to run or sign in to a network product first.
- **Custom QUIC with Noise.** We would own NAT traversal, relays and discovery, which is the hard part.

## Consequences

- LAN-first and relay fallback come with the dependency.
- By default, internet fallback depends on infrastructure we do not control. Self-hosting is supported, and the later account server will include a relay.
- iroh is a large dependency tree, so `cargo deny` checks every dependency's license in CI.
- The mDNS crate is still 0.x, so its API may change faster than iroh's.
- Video may use the same QUIC connection (streams and datagrams) or WebRTC. That is decided in milestone 5.
