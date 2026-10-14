# 0002: A shared Rust core with a Flutter UI

Status: accepted

## Context

Baton targets macOS, Windows, Linux, iOS, iPadOS and Android. Networking, cryptography, the protocol and the trust model must behave identically everywhere, and a bug in them is a security bug. Writing them five times is not an option.

## Decision

Write the shared logic once in Rust (`baton-proto`, `baton-core`) and build the user interface in Flutter. Flutter calls the core through an FFI layer (likely `flutter_rust_bridge`). Platform-specific work, such as screen capture, the clipboard and share sheets, lives in thin native layers next to the app.

## Alternatives considered

- **Native UIs on top of the Rust core** (SwiftUI, Jetpack Compose, WinUI, GTK). Best platform feel, but five UIs to build and keep consistent.
- **Kotlin Multiplatform.** Strong on Android and iOS, weaker for a networking and crypto core on desktop.
- **Electron or Tauri.** Not available on iOS and Android in a form we would want to ship.

## Consequences

- Contributors need Rust for the core. The UI is approachable to Flutter developers.
- The FFI boundary must stay narrow and made of simple types, so it is easy to wrap.
- RustDesk (Rust core, Flutter UI) and LocalSend (Flutter) show that this combination works on all five target platforms.
- Window capture still needs native code on each platform. That was always true.
