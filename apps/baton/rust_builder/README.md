# rust_builder

Glue that builds the Rust library of the Baton app (`crates/baton-ffi`) as part of the
iOS and macOS builds. It is a Flutter plugin only so that CocoaPods runs
[Cargokit](cargokit/README) at the right moment; there is no Dart code in it.

Android is added here when the Android app is.

`cargokit/` is copied from [Cargokit](https://github.com/irondash/cargokit) by Matej Knopp
(MIT, see `cargokit/LICENSE`) as the `flutter_rust_bridge` integration template ships it,
with three small changes. To update it, run the code generator's `integrate` command in a scratch
project, copy the folder and make the changes again.

Two are one line each, in `cargokit/build_tool/lib/src`: `getArtifactNames` in
`artifacts_provider.dart`, and `libName` in `build_pod.dart`. Both turn each `-` of the package
name into `_`, because Cargo names the library of the package `baton-ffi` `libbaton_ffi.a`, and
the template looks for, and writes, `libbaton-ffi.a`. The crate in the template is called
`rust_lib_` followed by the name of the app, so it has no `-` to trip over.

The third is in `cargokit/build_pod.sh`. The template has Cargo build in the temporary folder that
Xcode makes for each configuration and platform, in DerivedData, under a name that depends on
where the project is. The script now builds in `build/cargokit` of the app, for all of them
(Cargo keeps the targets and the profiles apart in it). CI keeps that folder from one run to the
next, so the dependencies of the library are built once and not in every job.

`test/cargokit_test.dart` fails if any of the three lines is lost.

## The Swift that shows windows

The plugin holds more than the build of the Rust library: `ios/Classes` and `macos/Classes` hold the Swift with which an iPhone, an iPad or a Mac shows the windows of other devices. It is the platform side of the C interface of `crates/baton-stream-abi`, and `docs/architecture.md` says what it does.

- `BatonViewer.swift` has the functions that the Rust side calls, the reading of what it passes, and the rules for which picture is shown and when a decoder has failed. It imports nothing but Foundation, so that it builds anywhere.
- `BatonH264Decoder.swift` decodes H.264 with VideoToolbox.
- `BatonStreamPlugin.swift` is the plugin class that Flutter registers (`pluginClass` in `pubspec.yaml`), with the textures that windows are drawn from. It is the only file that imports Flutter.
- `baton_stream_abi.h` is a copy of `crates/baton-stream-abi/include/baton_stream_abi.h`. CocoaPods puts it in the module of the pod, which is how the Swift sees the C types.

A pod can only use files from its own folder, so the files are in both `ios/Classes` and `macos/Classes`, and `test/viewer_files_test.dart` fails if the two differ, or if the header is not the crate's. Copy a changed file to the other folder, and the header from the crate, before you commit.

`swift_tests/run.sh` builds `BatonViewer.swift` with its tests and runs them. Any computer with a Swift compiler can. The tests of the decoder, which decode a short video with VideoToolbox and compare every sample with what was coded (`PcmVideo.swift` writes the video), need a Mac. Give the script `-sanitize=thread` or `-sanitize=address` to build with a sanitizer, as CI does.
