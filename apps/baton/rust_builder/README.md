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

## The Swift that shows and shares windows

The plugin holds more than the build of the Rust library: `ios/Classes` and `macos/Classes` hold the Swift with which an iPhone, an iPad or a Mac shows the windows of other devices, and with which a Mac shares a window of its own. It is the platform side of the C interface of `crates/baton-stream-abi`, and `docs/architecture.md` says what it does.

- `BatonViewer.swift` has the functions that the Rust side calls, the reading of what it passes, the rules for which picture is shown and when a decoder has failed, and the places that windows are drawn on, with the rule for asking again until Flutter gives them. It imports nothing but Foundation, so that it builds anywhere.
- `BatonH264Decoder.swift` decodes H.264 with VideoToolbox.
- `BatonSpeaker.swift` is the sound of a screen on the device that shows it: the speaker that the calls `sound`, `play` and `stopped` of the Rust side reach, and the ring that holds the sound until the device plays it. It imports nothing but Foundation either. `BatonAacDecoder.swift` decodes AAC-LC with AudioToolbox, and `BatonEngineOutput.swift` plays the ring with an audio engine of the system.
- `BatonSource.swift` is the same for the window that this device shares: the functions that the Rust side calls, the reading of what it passes, the choice of the picture, the pump that decides when a picture is made, and the sentences that the person is told when a capture stops. It imports nothing but Foundation either.
- `BatonH264Encoder.swift` encodes H.264 with VideoToolbox, and `BatonCapture.swift` is the picker of the system and the capture with ScreenCaptureKit (macOS 14 and later; the file has nothing in it on an iPhone or an iPad). It also gives the main display with no picker, for a paired device that the person at the Mac allowed beforehand to ask for it; the system wants the permission to record the screen for that, and does not for a window or a display that the person picked.
- `BatonSound.swift` is the same for the sound of a whole display that this device shares: the pump that starts the tap when the other device wants to hear, cuts what it reads into packets, keeps the Mac quiet and stops the tap, the conversion of a rate that changed, and the look for a Mac that has not let the app record its sound. It imports nothing but Foundation either. `BatonSoundMac.swift` is the tap of Core Audio that reads the sound of the Mac and keeps it from the speakers (macOS 14.2 and later; the file has nothing in it on an iPhone or an iPad), and `BatonAacEncoder.swift` encodes AAC-LC with AudioToolbox.
- `BatonStreamPlugin.swift` is the plugin class that Flutter registers (`pluginClass` in `pubspec.yaml`), with the textures that windows are drawn from. It is the only file that imports Flutter. Flutter on a Mac gives no texture until its engine runs, which is after the plugins are registered, so the plugin asks again until it has them. A texture with no window draws a blank picture (`makeBlankPicture` in `BatonH264Decoder.swift`), since Flutter on an iPhone goes on drawing the last picture of a texture that answers with none. It registers the sharer too, on a Mac from macOS 14. Its channel `io.github.jayunewal.baton/stream` answers `status` (what the registration of the viewer and of the sharer returned, with 1 for a sharer that was not registered because the device cannot share, and how many textures it has) for `integration_test/app_test.dart`; the app does not use it.
- `baton_stream_abi.h` is a copy of `crates/baton-stream-abi/include/baton_stream_abi.h`. CocoaPods puts it in the module of the pod, which is how the Swift sees the C types.

A pod can only use files from its own folder, so the files are in both `ios/Classes` and `macos/Classes`, and `test/viewer_files_test.dart` fails if the two differ, or if the header is not the crate's. Copy a changed file to the other folder, and the header from the crate, before you commit.

`swift_tests/run.sh` builds `BatonViewer.swift` and `BatonSource.swift` with their tests and runs them. Any computer with a Swift compiler can. The tests of the decoder, which decode a short video with VideoToolbox and compare every sample with what was coded (`PcmVideo.swift` writes the video), and those of the encoder, which check the pictures it makes and decode them with the decoder of the viewer, need a Mac, and so do those of the decoder of sound, which makes a tone with the encoder of the system and decodes it, and the one of the sharer of the Mac, which asks the system for the main screen and accepts any answer that is right (the permission to record the screen is the person's to give, so a computer may or may not have it). The tests of the sound give the pump a tap and an encoder that make what a test says, and the encoder of AAC is tested with the one of the system, which needs a Mac. Give the script `-sanitize=thread` or `-sanitize=address` to build with a sanitizer, as CI does.
