# Baton app

The Flutter app for iPhone, iPad and Mac. Android is on hold. It asks for a name for the device the
first time, then starts Baton and shows two places. One is the devices paired with it, where a new
one is paired (one device shows a QR code, the other scans it or has it pasted in, and the person
compares six digits) and where a tap on a device opens a screen to send it files, a folder (on a
Mac), a link or some text. The other is an inbox of what other devices sent, with a link to open
and an item to copy or remove. A device that asks to send files is asked about at the bottom of both
places, and the person says yes or no. A Mac also shares a window or its screen with a phone, which
looks at it, controls it, plays its sound and can unlock it with Face ID; the Swift code for that is
in `rust_builder/macos/Classes` and `rust_builder/ios/Classes`. Everything it does goes through
[`crates/baton-ffi`](../../crates/baton-ffi), the bridge to the Rust core; the app holds no
protocol logic of its own. [docs/architecture.md](../../docs/architecture.md#the-bridge) says how
the bridge is put together, and [the app](../../docs/architecture.md#the-app) how the screens sit
on it.

To put the app on an iPhone, an iPad or a Mac, follow
[docs/install-the-app.md](../../docs/install-the-app.md). It needs an Apple ID, which can be a free
one, and nothing else.

What is in this folder:

| Path | What it is |
| --- | --- |
| `lib/` | The app's own Dart code: `main.dart` starts it, `lib/src` has the controllers and the two interfaces the screens are written against (`backend.dart`), and `lib/src/screens` the screens. `rust_backend.dart` is the only file that calls the bridge. |
| `lib/src/rust/` | The Dart side of the bridge, written by `flutter_rust_bridge_codegen`. Do not edit it; it is committed so that the app builds without the code generator. |
| `ios/`, `macos/` | The Xcode projects. The Mac app is sandboxed; both apps leave Baton's folder out of backups. |
| `rust_builder/` | A Flutter plugin that builds `crates/baton-ffi` during an iOS or macOS build, by way of [Cargokit](https://github.com/irondash/cargokit). |
| `test/` | Tests. The widget tests (`app_test.dart`, `home_screen_test.dart`, `devices_screen_test.dart`, and one for each pairing, send and inbox screen, and for the controllers and the panel for files) run the screens against a fake Baton (`fake_backend.dart`) and a fake camera, so they need no Rust library. `pairing_flow_test.dart` runs two real Batons through the pairing controllers, handing the ticket over as a camera would, `send_flow_test.dart` has two paired real Batons send to each other through the send and inbox controllers, and `qr_test.dart` draws tickets as QR codes and reads them back. `bridge_test.dart` starts two real Batons on this computer and has them pair and hand each other things, files included, so it covers the generated code, the Rust library and the way they are called together; `rust_backend_test.dart` does the same through `RustBackend`. `platform_projects_test.dart` and `cargokit_test.dart` hold the few settings in the Xcode projects and in the copy of Cargokit that nothing on a Linux computer would notice losing. |
| `integration_test/` | The app started for real, with the real library, on a Mac and in an iPhone simulator, where CI runs it. It names a device, checks where received files go, pairs it with a second Baton through the real screens, receives a text from that Baton and sends it a link. |
| `tool/` | `make_icons.py` draws the app icon at every size. The pictures are committed; nothing at build time runs it. |

## What you need

- [Flutter](https://docs.flutter.dev/get-started/install) 3.47 or newer. CI uses 3.47.7.
- A stable Rust toolchain, as for the rest of the repository.
- To build for an iPhone, an iPad or a Mac: a Mac with Xcode and CocoaPods.
  [docs/install-the-app.md](../../docs/install-the-app.md) has the whole list.

## Running the tests

The tests load the Rust library built for the computer they run on, so build it first. From the
top of the repository:

```sh
cargo build -p baton-ffi
cd apps/baton
flutter pub get
flutter analyze
flutter test
```

The tests look for the library in `target/debug` (`libbaton_ffi.so` on Linux, `libbaton_ffi.dylib`
on a Mac, `baton_ffi.dll` on Windows). To use one built somewhere else, put its path in the
`BATON_FFI_LIB` environment variable.

CI also checks the formatting, with the command below, which leaves out the files the code
generator wrote:

```sh
dart format --output=none --set-exit-if-changed lib/main.dart lib/src/*.dart lib/src/screens test integration_test
```

## Running the app

On a Mac, with the tools from [docs/install-the-app.md](../../docs/install-the-app.md):

```sh
flutter run -d macos
```

That builds the Rust library as part of the build, which takes a while the first time. The same
document says how to put the app on an iPhone or an iPad. The integration test starts the app and
names a device, and CI runs it on a Mac and in an iPhone simulator:

```sh
flutter test integration_test -d macos
```

## Writing the bridge again

Change the functions in `crates/baton-ffi/src/api` and the Dart side has to follow. The code
generator must be the same version as the `flutter_rust_bridge` crate and package, 2.13.0:

```sh
cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked
cd apps/baton
flutter pub get
flutter_rust_bridge_codegen generate
cargo fmt --all
```

That rewrites `lib/src/rust` and `crates/baton-ffi/src/frb_generated.rs`; the last command puts
the Rust in the form the rest of the repository has. Commit both. CI runs the same commands and
fails when they change anything.
