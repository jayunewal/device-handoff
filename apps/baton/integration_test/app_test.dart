// SPDX-License-Identifier: Apache-2.0

// The app as it runs on a Mac or in an iPhone simulator: the real Rust library, found the way an
// installed app finds it, and the real screens. CI runs it on both. It is the first place where
// the build settings of the iOS and macOS projects, which nothing else here can try, meet the
// library.
//
//     flutter test integration_test -d macos
//
// Nothing here uses the internet: the settings it writes keep Baton to this computer.

import 'dart:io' show Directory, File;

import 'package:baton/src/app.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('names the device, starts Baton and shows the device', (
    tester,
  ) async {
    final folder = await Directory.systemTemp.createTemp('baton-app-test-');
    addTearDown(() => folder.delete(recursive: true));
    await File('${folder.path}/config.toml').writeAsString(
      '[network]\nrelay = "disabled"\nlan_discovery = false\nport_mapping = false\n',
    );

    await tester.pumpWidget(
      BatonApp(backend: RustBackend(dataDir: folder.path)),
    );
    await _see(tester, find.text('Name this device'));

    await tester.enterText(find.byType(TextField), 'Test device');
    await tester.tap(find.text('Continue'));
    await _see(tester, find.text('No other devices yet'));

    // The name went through the library and came back, and so did the fingerprint.
    expect(find.text('Test device'), findsOneWidget);
    expect(
      find.textContaining(RegExp(r'^(\w{4}-){4}\w{4}$')),
      findsOneWidget,
      reason: 'the fingerprint of this device',
    );

    // Closing the app stops Baton, which lets go of the folder.
    await tester.pumpWidget(const SizedBox());
  });
}

/// Waits, in real time, until [finder] finds something. Starting Baton takes a moment.
Future<void> _see(WidgetTester tester, Finder finder) async {
  final deadline = DateTime.now().add(const Duration(seconds: 60));
  while (finder.evaluate().isEmpty) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Never saw $finder');
    }
    await tester.pump(const Duration(milliseconds: 200));
  }
}
