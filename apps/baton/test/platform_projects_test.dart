// SPDX-License-Identifier: Apache-2.0

// The iOS and macOS projects cannot be built on the computers that most tests run on, so this
// checks the few settings in them that the app depends on and that a change could lose without
// anything failing until someone tried the app on a phone or a Mac.

import 'dart:io';

import 'package:baton/src/awake.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:flutter_test/flutter_test.dart';

String _read(String path) => File(path).readAsStringSync();

void main() {
  test('both apps are called Baton and have the same identifier', () {
    final ios = _read('ios/Runner.xcodeproj/project.pbxproj');
    expect(
      RegExp(r'PRODUCT_BUNDLE_IDENTIFIER = io\.github\.jayunewal\.baton;')
          .allMatches(ios),
      hasLength(3),
      reason: 'debug, profile and release',
    );
    final mac = _read('macos/Runner/Configs/AppInfo.xcconfig');
    expect(mac, contains('PRODUCT_NAME = Baton\n'));
    expect(
      mac,
      contains('PRODUCT_BUNDLE_IDENTIFIER = io.github.jayunewal.baton\n'),
    );
    expect(_read('ios/Runner/Info.plist'), contains('<string>Baton</string>'));
  });

  test('both apps say why they talk to devices on the local network', () {
    for (final plist in ['ios/Runner/Info.plist', 'macos/Runner/Info.plist']) {
      expect(_read(plist), contains('<key>NSLocalNetworkUsageDescription'));
    }
  });

  test('both apps say why they use the camera, which pairing needs', () {
    for (final plist in ['ios/Runner/Info.plist', 'macos/Runner/Info.plist']) {
      expect(_read(plist), contains('<key>NSCameraUsageDescription'));
    }
  });

  test('the sandboxed Mac app may use the camera', () {
    for (final file in [
      'macos/Runner/DebugProfile.entitlements',
      'macos/Runner/Release.entitlements',
    ]) {
      expect(
        _read(file),
        contains('<key>com.apple.security.device.camera</key>\n\t<true/>'),
        reason: file,
      );
    }
  });

  test('the Mac app is sandboxed and may use the network both ways', () {
    for (final file in [
      'macos/Runner/DebugProfile.entitlements',
      'macos/Runner/Release.entitlements',
    ]) {
      final entitlements = _read(file);
      for (final key in [
        'com.apple.security.app-sandbox',
        'com.apple.security.network.client',
        'com.apple.security.network.server',
      ]) {
        expect(
          entitlements,
          contains('<key>$key</key>\n\t<true/>'),
          reason: '$key in $file',
        );
      }
    }
  });

  test('the apps keep the folder that Baton uses out of backups', () {
    // The name is in three places: where Baton makes the folder, and where each app excludes
    // it from backups. They must agree, or the key that makes the device what it is is backed up.
    for (final file in [
      'ios/Runner/AppDelegate.swift',
      'macos/Runner/MainFlutterWindow.swift',
    ]) {
      final swift = _read(file);
      expect(
        swift,
        contains('let batonFolderName = "${RustBackend.folderName}"'),
        reason: file,
      );
      expect(swift, contains('isExcludedFromBackup = true'), reason: file);
    }
  });

  test('the iPhone app answers where Baton asks to keep the screen on', () {
    // The channel and its methods are named in lib/src/awake.dart and in the app. If they
    // differ, nothing fails but pairing: the screen goes off while the person is on the other
    // device, and the call is only logged.
    final swift = _read('ios/Runner/AppDelegate.swift');
    expect(swift, contains('name: "${PlatformScreenKeeper.channelName}"'));
    for (final method in ['keepOn', 'letTurnOff', 'isKeptOn']) {
      expect(swift, contains('case "$method":'), reason: method);
    }
  });

  test('the library is linked in a way that keeps its functions findable', () {
    for (final system in ['ios', 'macos']) {
      final podspec = _read('rust_builder/$system/baton_ffi.podspec');
      expect(podspec, contains('-force_load'));
      expect(podspec, contains("'STRIP_STYLE' => 'non-global'"));
    }
  });

  test('the pods name the system frameworks that the library needs', () {
    // The library is a static one, which does not bring them along. Without them the build
    // ends in "Undefined symbol" from the linker, after minutes of compiling Rust.
    expect(
      _read('rust_builder/ios/baton_ffi.podspec'),
      contains("s.frameworks = 'Network', 'SystemConfiguration'"),
    );
    expect(
      _read('rust_builder/macos/baton_ffi.podspec'),
      contains("s.frameworks = 'SystemConfiguration'"),
    );
  });
}
