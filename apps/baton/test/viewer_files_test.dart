// SPDX-License-Identifier: Apache-2.0

// The Swift that shows the windows of other devices, and the Swift that shares a window of this
// device, are the same for an iPhone and for a Mac (the iPhone has nothing in the part that
// captures), and a CocoaPods pod can only hold files from its own folder, so the files are in two
// places:
// rust_builder/ios/Classes and rust_builder/macos/Classes. Nothing builds one from the other, and
// the Mac jobs of CI only build the platform they are for, so a change that reached one folder
// and not the other would show as two apps that do different things. These tests fail at once.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// A file as text with the same line breaks everywhere: a checkout on Windows may have CRLF.
String _read(String path) =>
    File(path).readAsStringSync().replaceAll('\r\n', '\n');

const _files = [
  'BatonAacDecoder.swift',
  'BatonAacEncoder.swift',
  'BatonCapture.swift',
  'BatonControl.swift',
  'BatonEngineOutput.swift',
  'BatonFace.swift',
  'BatonFaceKeys.swift',
  'BatonGuard.swift',
  'BatonH264Decoder.swift',
  'BatonH264Encoder.swift',
  'BatonInput.swift',
  'BatonLock.swift',
  'BatonPrivacy.swift',
  'BatonPrivacyMac.swift',
  'BatonSound.swift',
  'BatonSoundMac.swift',
  'BatonSource.swift',
  'BatonSpeaker.swift',
  'BatonStreamPlugin.swift',
  'BatonUnlock.swift',
  'BatonUnlockMac.swift',
  'BatonViewer.swift',
  'baton_stream_abi.h',
];

List<String> _names(String system) =>
    Directory('rust_builder/$system/Classes')
        .listSync()
        .whereType<File>()
        .map((file) => file.uri.pathSegments.last)
        .toList()
      ..sort();

void main() {
  test('the iOS and the macOS plugin hold the same files, and no others', () {
    for (final system in ['ios', 'macos']) {
      expect(
        _names(system),
        _files,
        reason: 'a file left here is built into the app, a missing one is not',
      );
    }
    for (final file in _files) {
      expect(
        _read('rust_builder/ios/Classes/$file'),
        _read('rust_builder/macos/Classes/$file'),
        reason: '$file differs between ios/Classes and macos/Classes',
      );
    }
  });

  test('the header the Swift reads is the header of the crate', () {
    // The crate's header is what the Rust side is held to by its own tests. A copy that falls
    // behind it makes the Swift read the structs of another version.
    expect(
      _read('rust_builder/macos/Classes/baton_stream_abi.h'),
      _read('../../crates/baton-stream-abi/include/baton_stream_abi.h'),
    );
  });

  test('both pubspec entries name the class that Flutter registers', () {
    final pubspec = _read('rust_builder/pubspec.yaml');
    for (final system in ['ios', 'macos']) {
      expect(
        pubspec,
        contains('$system:\n        pluginClass: BatonStreamPlugin\n'),
        reason: 'Flutter registers the plugin of $system by this name',
      );
    }
    final plugin = _read('rust_builder/macos/Classes/BatonStreamPlugin.swift');
    expect(
      plugin,
      contains('@objc(BatonStreamPlugin)\npublic class BatonStreamPlugin'),
      reason: 'the registrant that Flutter writes looks for this name',
    );
  });

  test('only the plugin file knows Flutter', () {
    // The others build without Flutter on any computer with a Swift compiler (the ones that need
    // an Apple framework, on a Mac), and rust_builder/swift_tests/run.sh does just that.
    const others = [
      'BatonAacDecoder.swift',
      'BatonAacEncoder.swift',
      'BatonCapture.swift',
      'BatonControl.swift',
      'BatonEngineOutput.swift',
      'BatonFace.swift',
      'BatonFaceKeys.swift',
      'BatonGuard.swift',
      'BatonH264Decoder.swift',
      'BatonH264Encoder.swift',
      'BatonInput.swift',
      'BatonLock.swift',
      'BatonPrivacy.swift',
      'BatonPrivacyMac.swift',
      'BatonSound.swift',
      'BatonSoundMac.swift',
      'BatonSource.swift',
      'BatonSpeaker.swift',
      'BatonViewer.swift',
    ];
    for (final file in others) {
      final swift = _read('rust_builder/macos/Classes/$file');
      expect(swift, isNot(contains('import Flutter')), reason: file);
      expect(swift, isNot(contains('import FlutterMacOS')), reason: file);
    }
    // The files with the rules must build and be tested everywhere.
    for (final file in [
      'BatonViewer.swift',
      'BatonFace.swift',
      'BatonSource.swift',
      'BatonInput.swift',
      'BatonPrivacy.swift',
      'BatonGuard.swift',
      'BatonSound.swift',
      'BatonSpeaker.swift',
    ]) {
      final swift = _read('rust_builder/macos/Classes/$file');
      final imports = RegExp(
        r'^import (\w+)$',
        multiLine: true,
      ).allMatches(swift).map((match) => match.group(1)).toList();
      expect(imports, ['Foundation'], reason: file);
    }
  });

  test('the pods build what is in Classes, and the tests are not there', () {
    // The tests have a main of their own, and a second one in the framework would not link.
    for (final system in ['ios', 'macos']) {
      expect(
        _read('rust_builder/$system/baton_ffi.podspec'),
        contains("s.source_files     = 'Classes/**/*'"),
        reason: system,
      );
    }
    expect(File('rust_builder/swift_tests/run.sh').existsSync(), isTrue);
  });
}
