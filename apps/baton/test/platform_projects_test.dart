// SPDX-License-Identifier: Apache-2.0

// The iOS and macOS projects cannot be built on the computers that most tests run on, so this
// checks the few settings in them that the app depends on and that a change could lose without
// anything failing until someone tried the app on a phone or a Mac.

import 'dart:io';

import 'package:baton/src/awake.dart';
import 'package:baton/src/file_shower.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:baton/src/support_folder.dart';
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

  test('the iOS app says why it asks for a face, which an unlock needs', () {
    // The system may fail a request for Face ID from a program that has no such text.
    expect(
      _read('ios/Runner/Info.plist'),
      contains('<key>NSFaceIDUsageDescription'),
    );
  });

  test('the Mac app says why it records the sound of the Mac', () {
    // The system gives silence to a program that has no such text, and asks nothing.
    expect(
      _read('macos/Runner/Info.plist'),
      contains('<key>NSAudioCaptureUsageDescription'),
    );
  });

  test('the Mac app says why it puts files in Downloads', () {
    expect(
      _read('macos/Runner/Info.plist'),
      contains('<key>NSDownloadsFolderUsageDescription'),
    );
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

  test(
    'the sandboxed Mac app may put files in Downloads and read the ones chosen',
    () {
      // Without the first, received files go to a folder inside the sandbox that nobody looks in,
      // or are refused. Without the second, the open panel shows files that Baton cannot read.
      for (final file in [
        'macos/Runner/DebugProfile.entitlements',
        'macos/Runner/Release.entitlements',
      ]) {
        final entitlements = _read(file);
        for (final key in [
          'com.apple.security.files.downloads.read-write',
          'com.apple.security.files.user-selected.read-only',
        ]) {
          expect(
            entitlements,
            contains('<key>$key</key>\n\t<true/>'),
            reason: '$key in $file',
          );
        }
      }
    },
  );

  test('the iPhone app shows the files that arrive in the Files app', () {
    // The files are put in the Documents folder of the app, which the Files app lists only when
    // the app says so.
    final plist = _read('ios/Runner/Info.plist');
    for (final key in [
      'UIFileSharingEnabled',
      'LSSupportsOpeningDocumentsInPlace',
    ]) {
      expect(plist, contains('<key>$key</key>\n\t<true/>'), reason: key);
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

  test(
    'the iPhone app answers where Baton asks for the folder of its files',
    () {
      // The channel and its question are named in lib/src/support_folder.dart and in the app. If
      // they differ, Baton does not start on a phone, and the error only says that the system
      // would not say.
      final swift = _read('ios/Runner/AppDelegate.swift');
      expect(swift, contains('name: "${SupportFolder.channelName}"'));
      expect(swift, contains('case "${SupportFolder.question}":'));
    },
  );

  test(
    'both apps answer where Baton asks for the folder of received files',
    () {
      // The channel and its question are named in lib/src/support_folder.dart and in each app. If
      // they differ, files are not taken, and Baton says only that the system would not say where.
      for (final file in [
        'ios/Runner/AppDelegate.swift',
        'macos/Runner/MainFlutterWindow.swift',
      ]) {
        final swift = _read(file);
        expect(
          swift,
          contains('name: "${SupportFolder.channelName}"'),
          reason: file,
        );
        expect(
          swift,
          contains('case "${SupportFolder.receiveQuestion}":'),
          reason: file,
        );
      }
    },
  );

  test('both apps answer where Baton asks to show the files that arrived', () {
    // The channel and its question are named in lib/src/file_shower.dart and in each app. If they
    // differ, the button that takes the person to the files can only say that it could not.
    for (final file in [
      'ios/Runner/AppDelegate.swift',
      'macos/Runner/MainFlutterWindow.swift',
    ]) {
      final swift = _read(file);
      expect(
        swift,
        contains('name: "${SystemFileShower.channelName}"'),
        reason: file,
      );
      expect(
        swift,
        contains('case "${SystemFileShower.question}":'),
        reason: file,
      );
    }
  });

  test('both apps register what answers the question about showing files', () {
    // An answer that nothing registered is the same to Baton as an app that does not know the
    // question: the button can only say that it could not, and nothing else here fails.
    expect(
      _read('ios/Runner/AppDelegate.swift'),
      contains(
        '    showReceivedFilesInTheFilesApp(messenger: engineBridge.applicationRegistrar.messenger())',
      ),
    );
    expect(
      _read('macos/Runner/MainFlutterWindow.swift'),
      contains(
        '    showReceivedFilesInFinder(messenger: flutterViewController.engine.binaryMessenger)',
      ),
    );
  });

  test('the apps show the files that arrived and never open them', () {
    // What a file does when it is opened is for the person to choose, and Baton opens nothing
    // that another device sent. Finder is asked to select the items, and the Files app is asked
    // to open a folder. Neither is asked to open a file.
    final mac = _read('macos/Runner/MainFlutterWindow.swift');
    expect(mac, contains('NSWorkspace.shared.activateFileViewerSelecting('));
    for (final opening in [
      'NSWorkspace.shared.open',
      'NSWorkspace.shared.openFile',
      'NSWorkspace.shared.openApplication',
      'NSWorkspace.shared.launchApplication',
    ]) {
      expect(mac, isNot(contains(opening)), reason: opening);
    }
    final ios = _read('ios/Runner/AppDelegate.swift');
    expect(ios, contains('address.scheme = "shareddocuments"'));
    expect(
      ios,
      contains('item.deletingLastPathComponent()'),
      reason: 'anything but a folder that arrived alone is shown by its folder',
    );
    // A folder that the system takes for a document, such as Report.pages, is not opened either:
    // it is shown by the folder that holds it, like a file.
    expect(ios, contains('forKeys: [.isPackageKey]'));
    expect(ios, contains('isFolder.boolValue && !isPackage'));
  });

  test('the Mac app builds its guard and signs it to inherit the sandbox', () {
    // The guard locks the Mac when the app goes away during a private session
    // (docs/adr/0011-privacy-mode.md). Only the build puts it into the app, and the app looks
    // for it by name, so a lost phase or a renamed program would show as a Mac that cannot start
    // a private session, on the first try with a phone.
    final project = _read('macos/Runner.xcodeproj/project.pbxproj');
    expect(project, contains('/* Build Guard */,'), reason: 'Runner runs it');
    expect(project, contains(r'sh \"$SRCROOT/Guard/build-guard.sh\"'));
    final script = _read('macos/Guard/build-guard.sh');
    expect(script, contains(r'"$programs/BatonGuard"'));
    expect(script, contains('BatonGuard.swift'));
    expect(script, contains('BatonLock.swift'));
    expect(script, contains('--identifier io.github.jayunewal.baton.guard'));
    expect(
      _read('rust_builder/macos/Classes/BatonPrivacyMac.swift'),
      contains('forAuxiliaryExecutable: "BatonGuard"'),
    );
    // A program that inherits the sandbox of its parent may hold no other entitlement.
    final keys = RegExp(r'<key>([^<]+)</key>')
        .allMatches(_read('macos/Guard/Guard.entitlements'))
        .map((match) => match.group(1))
        .toList();
    expect(keys, [
      'com.apple.security.app-sandbox',
      'com.apple.security.inherit',
    ]);
  });

  test('the guard is built from a checkout whose path has a space in it', () {
    // Xcode hands the script paths such as "/Volumes/Backup/Personal Projects/...", and a list of
    // programs kept in a string and left unquoted falls apart at the space: lipo is then given
    // "/Volumes/Backup/Personal" and "Projects/..." and the build of the whole app stops. The
    // tools are replaced by scripts that write down what they were given, and lipo, like the
    // real one, refuses a file that is not there.
    final root = Directory.systemTemp.createTempSync('baton guard ');
    addTearDown(() => root.deleteSync(recursive: true));
    final tools = Directory('${root.path}/tools')..createSync();
    final notes = File('${root.path}/notes');
    void tool(String name, String body) {
      final file = File('${tools.path}/$name')..writeAsStringSync(body);
      expect(Process.runSync('chmod', ['+x', file.path]).exitCode, 0);
    }

    // `xcrun swiftc ... -o program` makes the program.
    tool('xcrun', r'''#!/bin/sh
while [ "$#" -gt 0 ]; do
  [ "$1" = "-o" ] && : > "$2"
  shift
done
''');
    tool('lipo', '''#!/bin/sh
echo "lipo \$*" >> "${notes.path}"
output=""
while [ "\$#" -gt 0 ]; do
  case "\$1" in
    -create) ;;
    -output) output="\$2"; shift ;;
    *) [ -e "\$1" ] || { echo "lipo: file not found: \$1" >&2; exit 1; } ;;
  esac
  shift
done
: > "\$output"
''');
    tool('codesign', '''#!/bin/sh
echo "codesign \$*" >> "${notes.path}"
''');
    final checkout = Directory('${root.path}/Personal Projects/app/macos')
      ..createSync(recursive: true);
    Directory('${checkout.path}/../rust_builder/macos/Classes')
        .createSync(recursive: true);
    final build = Directory('${root.path}/Build Products/Baton.app')
      ..createSync(recursive: true);
    final derived = Directory('${root.path}/Derived Files')..createSync();
    final script = File('macos/Guard/build-guard.sh').absolute.path;
    final ran = Process.runSync(
      'sh',
      [script],
      environment: {
        'PATH': '${tools.path}:${Platform.environment['PATH']}',
        'SRCROOT': checkout.path,
        'SDKROOT': '${root.path}/SDK',
        'TARGET_BUILD_DIR': build.path,
        'CONTENTS_FOLDER_PATH': 'Contents',
        'DERIVED_FILE_DIR': derived.path,
        'ARCHS': 'arm64 x86_64',
      },
    );
    expect(ran.exitCode, 0, reason: '${ran.stdout}${ran.stderr}');
    final work = '${derived.path}/baton-guard';
    final program = '${build.path}/Contents/MacOS/BatonGuard';
    expect(
      notes.readAsLinesSync().first,
      'lipo -create $work/BatonGuard-arm64 $work/BatonGuard-x86_64 '
      '-output $program',
    );
    expect(File(program).existsSync(), isTrue);
    expect(notes.readAsStringSync(), contains('codesign '));
  }, skip: Platform.isWindows ? 'it runs sh' : false);

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
