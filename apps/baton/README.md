# Baton app

The Flutter app for iPhone, iPad and Mac. Android follows. Everything it does goes through
[`crates/baton-ffi`](../../crates/baton-ffi), the bridge to the Rust core; the app holds no
protocol logic of its own. [docs/architecture.md](../../docs/architecture.md#the-bridge) says how
the bridge is put together.

What is in this folder:

| Path | What it is |
| --- | --- |
| `lib/` | The app's own Dart code. |
| `lib/src/rust/` | The Dart side of the bridge, written by `flutter_rust_bridge_codegen`. Do not edit it; it is committed so that the app builds without the code generator. |
| `rust_builder/` | A Flutter plugin that builds `crates/baton-ffi` during an iOS or macOS build, by way of [Cargokit](https://github.com/irondash/cargokit). |
| `test/` | Tests. `bridge_test.dart` starts two real Batons on this computer and has them pair and hand each other things, so it covers the generated code, the Rust library and the way they are called together. |

## What you need

- [Flutter](https://docs.flutter.dev/get-started/install) 3.47 or newer. CI uses 3.47.7.
- A stable Rust toolchain, as for the rest of the repository.

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

## Writing the bridge again

Change the functions in `crates/baton-ffi/src/api` and the Dart side has to follow. The code
generator must be the same version as the `flutter_rust_bridge` crate and package, 2.13.0:

```sh
cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked
cd apps/baton
flutter pub get
flutter_rust_bridge_codegen generate
```

That rewrites `lib/src/rust` and `crates/baton-ffi/src/frb_generated.rs`. Commit both. CI runs
the same command and fails when it changes anything.
