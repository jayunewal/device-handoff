// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/file_chooser.dart';
import 'package:baton/src/screens/home_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_shower.dart';

// Five minutes after the items in these tests were sent.
final _now = DateTime.fromMillisecondsSinceEpoch((1760529600 + 300) * 1000);

final _kitchenIpad = Device(
  id: 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
  lastSeenAt: 1760529600,
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: [_kitchenIpad],
);

/// Chooses one file without asking, so that a screen can be made to send it.
class _Chooses implements FileChooser {
  @override
  bool get canChooseFolders => false;

  @override
  Future<List<String>> files() async => ['/Users/me/holiday.jpg'];

  @override
  Future<String?> folder() async => null;

  @override
  Future<void> release(List<String> paths) async {}
}

/// Shows the app's two places for [session] and waits until it has read what it needs.
Future<FakeSession> _show(
  WidgetTester tester, [
  FakeSession? given,
  FileChooser chooser = const SystemFileChooser(),
  FakeShower? shower,
]) async {
  final session = given ?? _session();
  tester.view
    ..physicalSize = const Size(900, 1600)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: HomeScreen(
        session: session,
        now: () => _now,
        // No camera here: the pairing screens get a stand-in.
        scanner: (context, onCode) => const Text('(camera)'),
        openLink: (address) async => true,
        chooser: chooser,
        shower: shower ?? FakeShower(),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return session;
}

/// Opens the send screen of the Kitchen iPad and sends a file from it, which goes on until the
/// test ends it. The bar that does not fill never settles, so the tests that use this pump for
/// a time instead of settling.
Future<FakeSession> _sending(WidgetTester tester) async {
  final session = await _show(tester, null, _Chooses());
  await tester.tap(find.text('Kitchen iPad'));
  await tester.pumpAndSettle();
  await tester.tap(find.text('Choose files'));
  await tester.pump();
  expect(find.text('Stop sending'), findsOneWidget);
  return session;
}

const _sent = FilesSent(files: 1, bytes: 2000000, failed: 0, leftOut: 0);

const _plan = ReceivedItem.link(
  url: 'https://example.com/plan',
  title: 'The plan',
);

void main() {
  testWidgets('opens on the devices, with the inbox a tap away', (
    tester,
  ) async {
    await _show(tester);

    expect(find.text('Kitchen iPad'), findsOneWidget);
    expect(find.text('Nothing here yet'), findsNothing);

    await tester.tap(find.text('Inbox'));
    await tester.pumpAndSettle();

    expect(find.text('Nothing here yet'), findsOneWidget);
    expect(find.text('Kitchen iPad'), findsNothing);

    await tester.tap(find.text('Devices'));
    await tester.pumpAndSettle();

    expect(find.text('Kitchen iPad'), findsOneWidget);
    expect(find.text('Nothing here yet'), findsNothing);
  });

  testWidgets('says that something came, and counts it', (tester) async {
    final session = await _show(tester);

    session.arrive(sampleReceived(1, item: _plan));
    await tester.pumpAndSettle();

    expect(find.text('Anna’s iPhone sent you a link.'), findsOneWidget);
    expect(find.text('Inbox, 1 new'), findsOneWidget);

    session.arrive(sampleReceived(2));
    await tester.pumpAndSettle();

    expect(find.text('Anna’s iPhone sent you some text.'), findsOneWidget);
    expect(
      find.text('Anna’s iPhone sent you a link.'),
      findsNothing,
      reason: 'the newest notice replaces the one before it',
    );
    expect(find.text('Inbox, 2 new'), findsOneWidget);
  });

  testWidgets('a notice never says what came, which can be a password', (
    tester,
  ) async {
    final session = await _show(tester);

    session.arrive(
      sampleReceived(
        1,
        item: const ReceivedItem.text(text: 'The password is hunter2'),
      ),
    );
    await tester.pumpAndSettle();
    session.arrive(
      sampleReceived(
        2,
        item: const ReceivedItem.link(
          url: 'https://example.com/reset?token=abc123',
          title: 'Reset your password',
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Anna’s iPhone sent you a link.'), findsOneWidget);
    expect(find.textContaining('hunter2'), findsNothing);
    expect(find.textContaining('abc123'), findsNothing);
    expect(find.textContaining('Reset your password'), findsNothing);
  });

  testWidgets('View goes to the inbox, where what came is no longer new', (
    tester,
  ) async {
    final session = await _show(tester);
    session.arrive(sampleReceived(1, item: _plan));
    await tester.pumpAndSettle();

    await tester.tap(find.text('View'));
    await tester.pumpAndSettle();

    expect(find.text('The plan'), findsOneWidget);
    expect(find.text('Kitchen iPad'), findsNothing);
    expect(find.text('Inbox, 1 new'), findsNothing);
    expect(find.text('Anna’s iPhone sent you a link.'), findsNothing);
  });

  testWidgets('View leaves the screen that is in the way', (tester) async {
    final session = await _show(tester);
    await tester.tap(find.text('Kitchen iPad'));
    await tester.pumpAndSettle();
    expect(find.text('Send to Kitchen iPad'), findsOneWidget);

    session.arrive(sampleReceived(1, item: _plan));
    await tester.pumpAndSettle();
    expect(
      find.text('Anna’s iPhone sent you a link.'),
      findsOneWidget,
      reason: 'it is said over the screen the person is on',
    );
    await tester.tap(find.text('View'));
    await tester.pumpAndSettle();

    expect(find.text('Send to Kitchen iPad'), findsNothing);
    expect(find.text('The plan'), findsOneWidget);
  });

  testWidgets('what comes while the inbox is on show is seen as it comes', (
    tester,
  ) async {
    final session = await _show(tester);
    await tester.tap(find.text('Inbox'));
    await tester.pumpAndSettle();

    session.arrive(sampleReceived(1, item: _plan));
    await tester.pumpAndSettle();

    expect(find.text('The plan'), findsOneWidget);
    expect(find.textContaining('sent you'), findsNothing, reason: 'no notice');
    expect(find.textContaining('new'), findsNothing, reason: 'nothing new');

    // Once the person goes back to the devices, what comes is new again.
    await tester.tap(find.text('Devices'));
    await tester.pumpAndSettle();
    session.arrive(sampleReceived(2));
    await tester.pumpAndSettle();

    expect(find.text('Inbox, 1 new'), findsOneWidget);
    expect(find.text('Anna’s iPhone sent you some text.'), findsOneWidget);
  });

  testWidgets('what was in the inbox when the app opened is not new', (
    tester,
  ) async {
    final session = _session()..received = [sampleReceived(1, item: _plan)];
    await _show(tester, session);

    expect(find.text('Inbox'), findsOneWidget);
    expect(find.textContaining('new'), findsNothing);
    expect(find.textContaining('sent you'), findsNothing);
  });

  testWidgets('a request for files is asked at the bottom of both places', (
    tester,
  ) async {
    final session = await _show(tester);

    session.emit(Event.transferRequested(request: sampleTransfer()));
    await tester.pumpAndSettle();

    expect(
      find.text('Anna’s iPhone wants to send you 2 files'),
      findsOneWidget,
    );
    expect(find.text('holiday.jpg'), findsOneWidget);
    expect(
      find.byType(SnackBar),
      findsNothing,
      reason: 'the question is on show, so nothing needs to point to it',
    );

    await tester.tap(find.text('Inbox'));
    await tester.pumpAndSettle();

    expect(
      find.text('Anna’s iPhone wants to send you 2 files'),
      findsOneWidget,
      reason: 'it stays while the person looks at the inbox',
    );
    expect(session.transferAnswers, isEmpty);
  });

  testWidgets('a request is answered from the home screen', (tester) async {
    final session = await _show(tester);
    session.emit(Event.transferRequested(request: sampleTransfer(id: 4)));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Accept'));
    await tester.pump();

    expect(session.transferAnswers, [(request: 4, accept: true)]);
    expect(find.text('Accept'), findsNothing);
    expect(find.text('Receiving from Anna’s iPhone…'), findsOneWidget);

    // Let the transfer end, so that nothing is left going.
    session.emit(
      Event.transferEnded(end: sampleTransferEnd(sampleTransfer(id: 4))),
    );
    await tester.pump();
  });

  testWidgets('the files that arrived can be shown from the home screen', (
    tester,
  ) async {
    final shower = FakeShower();
    final session = await _show(
      tester,
      null,
      const SystemFileChooser(),
      shower,
    );
    session.emit(
      Event.transferEnded(end: sampleTransferEnd(sampleTransfer(id: 4))),
    );
    await tester.pumpAndSettle();
    expect(shower.asked, isEmpty);

    await tester.tap(find.byIcon(Icons.folder_open_outlined));
    await tester.pump();

    expect(shower.asked, [
      ['/Users/test/Downloads/holiday.jpg', '/Users/test/Downloads/notes.txt'],
    ]);
  });

  testWidgets('a request that comes over another screen is pointed out', (
    tester,
  ) async {
    final session = await _show(tester);
    await tester.tap(find.text('Kitchen iPad'));
    await tester.pumpAndSettle();
    expect(find.text('Send to Kitchen iPad'), findsOneWidget);

    session.emit(Event.transferRequested(request: sampleTransfer()));
    await tester.pumpAndSettle();

    expect(
      find.text('Anna’s iPhone wants to send you files.'),
      findsOneWidget,
      reason: 'it is said over the screen the person is on',
    );
    expect(
      find.textContaining('holiday.jpg'),
      findsNothing,
      reason: 'the names of the files can be private, and nothing says them',
    );

    await tester.tap(find.text('Review'));
    await tester.pumpAndSettle();

    expect(find.text('Send to Kitchen iPad'), findsNothing);
    expect(
      find.text('Anna’s iPhone wants to send you 2 files'),
      findsOneWidget,
    );
    expect(find.text('holiday.jpg'), findsOneWidget);
  });

  testWidgets('Review asks before it leaves a screen that is sending files', (
    tester,
  ) async {
    final session = await _sending(tester);
    session.emit(Event.transferRequested(request: sampleTransfer()));
    // The first frame starts the notice sliding in; the second is after it has arrived.
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 500));
    expect(find.text('Anna’s iPhone wants to send you files.'), findsOneWidget);

    await tester.tap(find.text('Review'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));

    expect(find.text('Stop sending to Kitchen iPad?'), findsOneWidget);
    expect(session.transfer.cancelled, isFalse);

    await tester.tap(find.text('Keep sending'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));

    expect(find.text('Send to Kitchen iPad'), findsOneWidget);
    expect(session.transfer.cancelled, isFalse);

    // Let the transfer end, so that nothing is left going.
    session.transfer.finish(_sent);
    await tester.pump();
  });

  testWidgets(
    'View asks the same, and goes to the inbox once the person says stop',
    (tester) async {
      final session = await _sending(tester);
      session.arrive(sampleReceived(1, item: _plan));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));

      await tester.tap(find.text('View'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));

      expect(find.text('Stop sending to Kitchen iPad?'), findsOneWidget);
      expect(session.transfer.cancelled, isFalse);

      await tester.tap(find.text('Stop'));
      await tester.pumpAndSettle();

      expect(session.transfer.cancelled, isTrue);
      expect(find.text('Send to Kitchen iPad'), findsNothing);
      expect(find.text('The plan'), findsOneWidget);
    },
  );
}
