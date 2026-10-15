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
import 'package:baton/src/backend.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('names the device, starts Baton and shows the device', (
    tester,
  ) async {
    final folder = await _folder();

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
    await tester.enterText(
      find.byType(TextField),
      ' HTTPS://Example.com/plan ',
    );
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
}

/// Starts the app with a new device and pairs it with a second Baton in this process, through
/// the screens: the second Baton shows a code, the app has it pasted in, both show six digits
/// and the second says yes. Returns the second Baton, which stops when the test ends. The app
/// is left on its devices, where the second one is listed.
///
/// The camera is the one thing left out: CI has none, so the code is pasted.
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

  await tester.pumpWidget(BatonApp(backend: RustBackend(dataDir: mine.path)));
  await _see(tester, find.text('No other devices yet'));
  await tester.tap(find.text('Add a device'));
  await _see(tester, find.text('Scan or enter a code'));
  await tester.tap(find.text('Scan or enter a code'));
  await _see(tester, find.text('Or paste the code', skipOffstage: false));

  // The camera is on the screen and sees what it sees. Nothing here depends on it, except
  // that it must not take the app down. The screen can be longer than a small window, so
  // scroll to what is wanted.
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
      fail('Never saw $what');
    }
    await tester.pump(const Duration(milliseconds: 200));
  }
  await tester.pump(const Duration(milliseconds: 500));
}

/// Waits until [finder] finds something.
Future<void> _see(WidgetTester tester, Finder finder) =>
    _until(tester, () => finder.evaluate().isNotEmpty, '$finder');
