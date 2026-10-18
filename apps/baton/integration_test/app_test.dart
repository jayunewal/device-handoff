// SPDX-License-Identifier: Apache-2.0

// The app as it runs on a Mac or in an iPhone simulator: the real Rust library, found the way an
// installed app finds it, and the real screens. CI runs it on both. It is the first place where
// the build settings of the iOS and macOS projects, which nothing else here can try, meet the
// library.
//
//     flutter test integration_test -d macos
//
// Nothing here uses the internet: the settings it writes keep Baton to this computer.

import 'dart:async';
import 'dart:io' show Directory, File;

import 'package:baton/src/app.dart';
import 'package:baton/src/awake.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:baton/src/support_folder.dart';
import 'package:flutter/foundation.dart' show defaultTargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  // The system takes the screen away from the app for a moment now and then, and Baton lets go
  // of the screen of the device then. If a check of it fails, the log shows when that was.
  AppLifecycleListener(
    onStateChange: (state) => debugPrint(
      '${DateTime.now().toIso8601String()} the app is ${state.name}',
    ),
  );

  testWidgets('names the device, starts Baton and shows the device', (
    tester,
  ) async {
    final folder = await _folder();

    await tester.pumpWidget(
      BatonApp(backend: RustBackend(dataDir: folder.path)),
    );
    await _see(tester, find.text('Name this device'));

    await _type(tester, 'Test device');
    await tester.tap(find.text('Continue'));
    await _see(tester, find.text('No other devices yet'));
    await _expectScreenKeptOn(
      tester,
      kept: false,
      when: 'when no one is pairing',
    );

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

  testWidgets(
    'finds the folder for its files from the system, and the library opens it',
    (tester) async {
      // An iPhone with iOS 27 starts an app without HOME in its environment, and so does the
      // simulator that CI starts, and a Baton that looked for HOME did not start. Every other
      // test in this file is given a folder, so none of them would have noticed.
      final path = await const SupportFolder().path();

      expect(path, startsWith('/'));
      expect(path, endsWith('/Library/Application Support'));
      expect(
        Directory(path).existsSync(),
        isTrue,
        reason: 'the system made it',
      );
      // The iOS app makes Baton's folder there when it opens, and leaves it out of backups.
      expect(
        Directory('$path/${RustBackend.folderName}').existsSync(),
        isTrue,
        reason: 'the folder that the app made for Baton',
      );
      // The app looks for the folder the way it does when a person opens it, and the library
      // opens it. Asking for the name reads it and changes nothing.
      await RustBackend().deviceName();
    },
    skip: defaultTargetPlatform != TargetPlatform.iOS,
  );

  testWidgets('pairs with another Baton, by the code pasted in', (
    tester,
  ) async {
    await _pairThroughTheScreens(tester);

    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('shows what another Baton sends, and sends to it', (
    tester,
  ) async {
    final second = await _pairThroughTheScreens(tester);
    final firstId = (await second.devices()).single.id;

    // The second device sends some text. The person is told who sent what kind of thing, but
    // not what it says, and the inbox has it.
    await second.sendText(firstId, 'The door code is 4711');
    await _see(tester, find.text('Second device sent you some text.'));
    expect(find.textContaining('4711'), findsNothing);
    await tester.tap(find.text('View'));
    await _see(tester, find.text('The door code is 4711'));
    expect(find.text('Second device · just now'), findsOneWidget);

    // Back on the devices, this device sends the second one a link, through the send screen.
    await tester.tap(find.text('Devices'));
    await _see(tester, find.text('Second device'));
    await tester.tap(find.text('Second device'));
    await _see(tester, find.text('Send to Second device'));
    await _type(tester, ' HTTPS://Example.com/plan ');
    await tester.scrollUntilVisible(
      find.text('Send link'),
      200,
      scrollable: find.byType(Scrollable).first,
    );
    await tester.tap(find.text('Send link'));
    await _see(tester, find.text('Sent the link to Second device.'));

    await _until(
      tester,
      () async => (await second.inbox()).isNotEmpty,
      'the link to reach the second device',
    );
    final got = (await second.inbox()).single;
    expect(got.fromName, 'Test device');
    expect(got.item, isA<ReceivedItem_Link>());
    expect((got.item as ReceivedItem_Link).url, 'https://example.com/plan');

    await tester.pumpWidget(const SizedBox());
  });

  // Last, because the question that the system asks about the camera cannot be answered here. It
  // stays in front of the app, which stays inactive for whatever runs after.
  testWidgets('starts the camera view without taking the app down', (
    tester,
  ) async {
    final folder = await _folder();
    await RustBackend(dataDir: folder.path).setDeviceName('Test device');

    await tester.pumpWidget(
      BatonApp(backend: RustBackend(dataDir: folder.path)),
    );
    await _see(tester, find.text('No other devices yet'));
    await tester.tap(find.text('Add a device'));
    await _see(tester, find.text('Scan or enter a code'));
    await tester.tap(find.text('Scan or enter a code'));
    await _see(tester, find.text('Or paste the code', skipOffstage: false));

    // The camera starts, and on a phone the system asks about it. Nothing here depends on what
    // the camera sees, except that it must not take the app down, so give it time to do that.
    final until = DateTime.now().add(const Duration(seconds: 3));
    while (DateTime.now().isBefore(until)) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    expect(find.text('Or paste the code', skipOffstage: false), findsOneWidget);

    await tester.pumpWidget(const SizedBox());
  });
}

/// Starts the app with a new device and pairs it with a second Baton in this process, through
/// the screens: the second Baton shows a code, the app has it pasted in, both show six digits
/// and the second says yes. Returns the second Baton, which stops when the test ends. The app
/// is left on its devices, where the second one is listed.
///
/// The camera is the one thing left out: CI has none, so the code is pasted (see [_noCamera]).
Future<Session> _pairThroughTheScreens(WidgetTester tester) async {
  // This device, and a second Baton in this process that plays the device with the code.
  final mine = await _folder();
  await RustBackend(dataDir: mine.path).setDeviceName('Test device');
  final theirs = RustBackend(dataDir: (await _folder()).path);
  await theirs.setDeviceName('Second device');
  final second = await theirs.start();
  addTearDown(second.stop);
  final asked = Completer<PairingRequest>();
  final events = (await second.events()).listen((event) {
    if (event is Event_PairingRequested && !asked.isCompleted) {
      asked.complete(event.request);
    }
  });
  addTearDown(() => unawaited(events.cancel()));
  final offer = await second.offerPairing();

  await tester.pumpWidget(
    BatonApp(
      backend: RustBackend(dataDir: mine.path),
      scanner: _noCamera,
    ),
  );
  await _see(tester, find.text('No other devices yet'));
  await tester.tap(find.text('Add a device'));
  await _see(tester, find.text('Scan or enter a code'));
  await tester.tap(find.text('Scan or enter a code'));
  await _see(tester, find.text('Or paste the code', skipOffstage: false));
  await _expectScreenKeptOn(
    tester,
    kept: true,
    when: 'while the screen waits for a code',
  );

  // The screen can be longer than a small window, so scroll to what is wanted.
  final list = find.byType(Scrollable).first;
  await tester.scrollUntilVisible(
    find.byType(TextField),
    200,
    scrollable: list,
  );
  await tester.enterText(find.byType(TextField), offer.ticket);
  await tester.scrollUntilVisible(find.text('Connect'), 200, scrollable: list);
  await tester.tap(find.text('Connect'));

  // This device shows six digits, and the second one is asked about the same ones.
  await _see(tester, find.text('Check the other device'));
  final request = await asked.future.timeout(const Duration(seconds: 60));
  expect(request.name, 'Test device');
  expect(find.text(request.code), findsOneWidget);

  expect(second.answerPairing(request.id, accept: true), isTrue);
  await _see(tester, find.text('Paired with Second device'));
  await _expectScreenKeptOn(
    tester,
    kept: false,
    when: 'once the pairing is over',
  );
  await tester.tap(find.text('Done'));
  await _see(tester, find.text('Second device'));
  expect(find.text('No other devices yet'), findsNothing);
  await _until(
    tester,
    () async => (await second.devices()).isNotEmpty,
    'the second device to keep this one',
  );
  expect((await second.devices()).single.name, 'Test device');

  return second;
}

/// What the pairing tests have in place of the camera view. The real one asks the system for
/// permission the first time it starts, and on a phone the question stays in front of the app
/// until someone answers it, which no one does here. The app is inactive for as long as that
/// lasts, and Baton lets go of the screen of the device (see [_expectScreenKeptOn]), so a check
/// of the screen would pass or fail on whether it came before the question did. The test that
/// starts the real camera view is the last one.
Widget _noCamera(BuildContext context, ValueChanged<String> onCode) =>
    const SizedBox(height: 200, child: Center(child: Text('(camera)')));

/// Checks what an iPhone or iPad itself says about whether its screen is being kept on: that
/// Baton's request went through the code in the iOS app and reached the system. Elsewhere there
/// is no such screen, and this checks nothing.
///
/// Baton lets go of the screen whenever its app is not the active one, such as while the system
/// asks a question in front of it, and asks again once the app is back (see
/// ios/Runner/AppDelegate.swift). So a screen that should be [kept] on is given a while to be,
/// and a failure says what state the app was in. A screen that should not be kept on is checked
/// at once, because nothing but Baton turns it on.
Future<void> _expectScreenKeptOn(
  WidgetTester tester, {
  required bool kept,
  required String when,
}) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  const screen = PlatformScreenKeeper();
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  var isOn = await screen.isKeptOn();
  while (kept && !isOn && DateTime.now().isBefore(deadline)) {
    await tester.pump(const Duration(milliseconds: 200));
    isOn = await screen.isKeptOn();
  }
  final state = WidgetsBinding.instance.lifecycleState?.name ?? 'unknown';
  expect(
    isOn,
    kept,
    reason: 'the screen of the device $when (the state of the app: $state)',
  );
}

/// A folder for one device, with settings that keep it to this computer: no relay, no
/// discovery and no port mapping.
Future<Directory> _folder() async {
  final folder = await Directory.systemTemp.createTemp('baton-app-test-');
  addTearDown(() => folder.delete(recursive: true));
  await File('${folder.path}/config.toml').writeAsString(
    '[network]\nrelay = "disabled"\nlan_discovery = false\nport_mapping = false\n',
  );
  return folder;
}

/// Waits, in real time, until [done] says yes. Starting Baton takes a moment, and so does a
/// pairing. Then it lets the screen settle, because a page that has just appeared is still
/// sliding in.
Future<void> _until(
  WidgetTester tester,
  FutureOr<bool> Function() done,
  String what,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 60));
  while (!await done()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Never saw $what (${_describeScreen(tester)})');
    }
    await tester.pump(const Duration(milliseconds: 200));
  }
  await tester.pump(const Duration(milliseconds: 500));
}

/// What there is to say about the app and its screen when a wait runs out, for the log: a
/// failure that shows only what was missing does not show why.
String _describeScreen(WidgetTester tester) {
  final state = WidgetsBinding.instance.lifecycleState?.name ?? 'unknown';
  final view = tester.view;
  final fields = find.byType(TextField, skipOffstage: false).evaluate().length;
  return 'the state of the app: $state, the view: ${view.physicalSize} at '
      '${view.devicePixelRatio}x, text fields: $fields';
}

/// Waits until [finder] finds something.
Future<void> _see(WidgetTester tester, Finder finder) =>
    _until(tester, () => finder.evaluate().isNotEmpty, '$finder');

/// Types [text] into the text field on the screen, once the field is there. The title above it
/// can be seen before the field is, and typing into a field that is not there throws.
Future<void> _type(WidgetTester tester, String text) async {
  await _see(tester, find.byType(EditableText));
  await tester.enterText(find.byType(TextField), text);
}
