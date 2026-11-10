// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/file_chooser.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/owner_check.dart';
import 'package:baton/src/screens/devices_screen.dart';
import 'package:baton/src/screens/home_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_always_on.dart';
import 'fake_backend.dart';
import 'fake_control_access.dart';
import 'fake_notices.dart';
import 'fake_owner_check.dart';
import 'fake_screen.dart';
import 'fake_shower.dart';
import 'session_controls.dart';

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

final _annasMac = Device(
  id: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  name: 'Anna’s Mac',
  platform: Platform.macOs,
  fingerprint: 'mnop-qrst-uvwx-yzab-cdef',
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
  FakeScreen? screen,
  ControlAccessKeeper? controlAccess,
  ScreenAccessKeeper? screenAccess,
  NoticeKeeper? notices,
  OwnerCheck? ownerCheck,
  FakeAlwaysOn? alwaysOn,
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
        screen: screen ?? FakeScreen(),
        controlAccess:
            controlAccess ?? FakeControlAccess(ControlAccess.unavailable),
        screenAccess:
            screenAccess ?? FakeScreenAccess(ScreenAccess.unavailable),
        notices: notices ?? FakeNotices(NoticeAccess.unavailable),
        ownerCheck: ownerCheck ?? FakeOwnerCheck(),
        alwaysOn: alwaysOn ?? FakeAlwaysOn(),
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

/// A device that shows the windows of others and shares its own.
const _windowAbilities = WindowAbilities(
  canShow: true,
  canShare: true,
  canControl: false,
  canGiveScreen: false,
  canBeUnlocked: false,
  canUnlock: false,
);

/// A phone, which shows the screens of others and shares nothing.
const _phoneAbilities = WindowAbilities(
  canShow: true,
  canShare: false,
  canControl: false,
  canGiveScreen: false,
  canBeUnlocked: false,
  canUnlock: false,
);

/// A Mac that gives its screen to a device that was allowed to ask for it.
const _macAbilities = WindowAbilities(
  canShow: false,
  canShare: true,
  canControl: true,
  canGiveScreen: true,
  canBeUnlocked: false,
  canUnlock: false,
);

/// A Mac that a paired device can unlock while it is locked.
const _unlockableMacAbilities = WindowAbilities(
  canShow: false,
  canShare: true,
  canControl: true,
  canGiveScreen: true,
  canBeUnlocked: true,
  canUnlock: false,
);

const _plan = ReceivedItem.link(
  url: 'https://example.com/plan',
  title: 'The plan',
);

/// The name of a place, as the bar at the bottom shows it, or as the sidebar of a wide window
/// gives it to a screen reader.
Finder _named(String name) => find.byWidgetPredicate(
  (widget) =>
      widget is Text && (widget.data == name || widget.semanticsLabel == name),
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
    expect(_named('Inbox, 1 new'), findsOneWidget);

    session.arrive(sampleReceived(2));
    await tester.pumpAndSettle();

    expect(find.text('Anna’s iPhone sent you some text.'), findsOneWidget);
    expect(
      find.text('Anna’s iPhone sent you a link.'),
      findsNothing,
      reason: 'the newest notice replaces the one before it',
    );
    expect(_named('Inbox, 2 new'), findsOneWidget);
  });

  testWidgets('says how a link that waited to be sent ended, on any screen', (
    tester,
  ) async {
    final session = await _show(tester);
    final first = session.queue(
      deviceId: _kitchenIpad.id,
      deviceName: 'Kitchen iPad',
    );
    await tester.pumpAndSettle();

    session.settle(
      first,
      OutboxEnd.delivered,
      'Sent a link (example.com) to Kitchen iPad.',
    );
    await tester.pumpAndSettle();
    expect(
      find.text('Sent a link (example.com) to Kitchen iPad.'),
      findsOneWidget,
    );

    // The person has gone on to the screen for sending, and the next ending shows there.
    final second = session.queue(
      deviceId: _kitchenIpad.id,
      deviceName: 'Kitchen iPad',
      summary: 'some text (4 characters)',
    );
    await tester.tap(find.text('Kitchen iPad'));
    await tester.pumpAndSettle();
    expect(find.text('Send to Kitchen iPad'), findsOneWidget);

    session.settle(
      second,
      OutboxEnd.expired,
      'Gave up on sending some text (4 characters) to Kitchen iPad: it could not '
      'be reached for a day. Send it again when Kitchen iPad is open.',
    );
    await tester.pumpAndSettle();

    expect(
      find.text(
        'Gave up on sending some text (4 characters) to Kitchen iPad: it could '
        'not be reached for a day. Send it again when Kitchen iPad is open.',
      ),
      findsOneWidget,
    );
    expect(
      find.text('Sent a link (example.com) to Kitchen iPad.'),
      findsNothing,
      reason: 'the newest sentence replaces the one before it',
    );
  });

  testWidgets('says nothing of an item that the person gave up on', (
    tester,
  ) async {
    final session = await _show(tester);
    final item = session.queue(
      deviceId: _kitchenIpad.id,
      deviceName: 'Kitchen iPad',
    );
    await tester.pumpAndSettle();

    await session.cancelOutbox(item.id);
    await tester.pumpAndSettle();

    expect(find.byType(SnackBar), findsNothing);
  });

  testWidgets('shows what the library said about an item that waited as plain '
      'text', (tester) async {
    final session = await _show(tester);
    final item = session.queue(
      deviceId: _kitchenIpad.id,
      deviceName: 'Kitchen\u202Eiriad',
    );
    await tester.pumpAndSettle();

    session.settle(
      item,
      OutboxEnd.refused,
      'Kitchen\u202Eiriad did not take the link.',
    );
    await tester.pumpAndSettle();

    expect(
      find.text('Kitchen\uFFFDiriad did not take the link.'),
      findsOneWidget,
    );
  });

  testWidgets(
    'shows the sentence about an item that did not go for longer than '
    'the one about an item that went',
    (tester) async {
      final session = await _show(tester);
      const went = 'Sent a link (example.com) to Kitchen iPad.';
      const notTaken = 'Kitchen iPad did not take the link.';

      final first = session.queue(
        deviceId: _kitchenIpad.id,
        deviceName: 'Kitchen iPad',
      );
      session.settle(first, OutboxEnd.delivered, went);
      await tester.pumpAndSettle();
      expect(find.text(went), findsOneWidget);
      await tester.pump(const Duration(seconds: 7));
      await tester.pumpAndSettle();
      expect(find.text(went), findsNothing, reason: 'a short time is enough');

      for (final end in [OutboxEnd.refused, OutboxEnd.expired]) {
        final item = session.queue(
          deviceId: _kitchenIpad.id,
          deviceName: 'Kitchen iPad',
        );
        session.settle(item, end, notTaken);
        await tester.pumpAndSettle();
        expect(find.text(notTaken), findsOneWidget);
        await tester.pump(const Duration(seconds: 7));
        await tester.pumpAndSettle();
        expect(
          find.text(notTaken),
          findsOneWidget,
          reason: 'still there after the time that was enough for the other',
        );
        await tester.pump(const Duration(seconds: 6));
        await tester.pumpAndSettle();
        expect(find.text(notTaken), findsNothing, reason: 'and not for ever');
      }
    },
  );

  testWidgets('lets go of the events of the session when it is closed', (
    tester,
  ) async {
    final session = await _show(tester);
    expect(session.listening, isTrue);

    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();

    expect(session.listening, isFalse);
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
    expect(_named('Inbox, 1 new'), findsNothing);
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

    expect(_named('Inbox, 1 new'), findsOneWidget);
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

  testWidgets('a device that cannot share a window offers no button for it', (
    tester,
  ) async {
    await _show(tester);

    expect(
      find.byTooltip('Share a window or screen with Kitchen iPad'),
      findsNothing,
    );
  });

  testWidgets('a device that can share a window does so from the row', (
    tester,
  ) async {
    final session = _session()..abilities = _windowAbilities;
    await _show(tester, session);

    await tester.tap(
      find.byTooltip('Share a window or screen with Kitchen iPad'),
    );
    await tester.pumpAndSettle();

    expect(session.windowsShared, [(id: _kitchenIpad.id, control: false)]);
  });

  testWidgets('a window can be shared to be controlled, from the row', (
    tester,
  ) async {
    final session = _session()..abilities = _windowAbilities;
    await _show(
      tester,
      session,
      const SystemFileChooser(),
      null,
      null,
      FakeControlAccess(),
    );

    await tester.tap(
      find.byTooltip('Share a window or screen with Kitchen iPad'),
    );
    await tester.pumpAndSettle();
    await tester.tap(
      find.text('Share a window or screen to look at and control'),
    );
    await tester.pumpAndSettle();

    expect(session.windowsShared, [(id: _kitchenIpad.id, control: true)]);
  });

  testWidgets('a device that shows no screen offers no button to ask for one', (
    tester,
  ) async {
    final session = _session()..pairedDevices = [_annasMac];
    await _show(tester, session);

    expect(find.byTooltip('Look at the screen of Anna’s Mac'), findsNothing);
  });

  testWidgets(
    'a Mac that gives its screen lets the person say which device may ask',
    (tester) async {
      final session = _session()..abilities = _macAbilities;
      await _show(tester, session);

      await tester.tap(find.byTooltip('More about Kitchen iPad'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Let this device ask for the screen…'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Show it to look at'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();

      expect(session.startAccessSet, [
        (id: _kitchenIpad.id, access: StartAccess.look),
      ]);
    },
  );

  testWidgets(
    'a Mac asks the system to record its screen when a device may ask',
    (tester) async {
      final session = _session()..abilities = _macAbilities;
      final recorder = FakeScreenAccess(ScreenAccess.needed);
      await _show(
        tester,
        session,
        const SystemFileChooser(),
        null,
        null,
        null,
        recorder,
      );

      await tester.tap(find.byTooltip('More about Kitchen iPad'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Let this device ask for the screen…'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Show it to look at'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();

      expect(recorder.requests, 1);
    },
  );

  testWidgets(
    'a Mac that can be unlocked lets the person set it up for a tablet',
    (tester) async {
      final session = _session()..abilities = _unlockableMacAbilities;
      await _show(
        tester,
        session,
        const SystemFileChooser(),
        null,
        null,
        null,
        null,
        FakeNotices(),
      );

      await tester.tap(find.byTooltip('More about Kitchen iPad'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Let this device unlock this Mac…'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'correct horse');
      await tester.pump();
      await tester.tap(find.widgetWithText(FilledButton, 'Set up'));
      await tester.pumpAndSettle();

      expect(session.unlocksSetUp, [
        (id: _kitchenIpad.id, password: 'correct horse'),
      ]);
      expect(
        find.text('Kitchen iPad can now unlock this Mac while it is locked.'),
        findsOneWidget,
      );
    },
  );

  testWidgets('a device that cannot be unlocked offers no such choice', (
    tester,
  ) async {
    final session = _session()..abilities = _macAbilities;
    await _show(tester, session);

    await tester.tap(find.byTooltip('More about Kitchen iPad'));
    await tester.pumpAndSettle();

    expect(find.text('Let this device unlock this Mac…'), findsNothing);
  });

  testWidgets('a device that unlocks others offers no choice to be unlocked', (
    tester,
  ) async {
    final session = _session()..abilities = _phoneAbilities;
    await _show(tester, session);

    await tester.tap(find.byTooltip('More about Kitchen iPad'));
    await tester.pumpAndSettle();

    expect(find.text('Let this device unlock this Mac…'), findsNothing);
  });

  testWidgets(
    'a notification tells the person that a device unlocked this one',
    (tester) async {
      final notices = FakeNotices();
      final session = _session()..abilities = _unlockableMacAbilities;
      await _show(
        tester,
        session,
        const SystemFileChooser(),
        null,
        null,
        null,
        null,
        notices,
      );

      session.emit(
        Event.unlockAttempted(entry: sampleUnlockEntry(device: 'Kitchen iPad')),
      );
      await tester.pumpAndSettle();

      expect(notices.posted.map((n) => n.title), [
        'This device was unlocked from Kitchen iPad',
      ]);
    },
  );

  testWidgets('no notification is shown for an attempt that came to nothing', (
    tester,
  ) async {
    final notices = FakeNotices();
    final session = _session()..abilities = _unlockableMacAbilities;
    await _show(
      tester,
      session,
      const SystemFileChooser(),
      null,
      null,
      null,
      null,
      notices,
    );

    session.emit(
      Event.unlockAttempted(
        entry: sampleUnlockEntry(result: UnlockResult.declined),
      ),
    );
    await tester.pumpAndSettle();

    expect(notices.posted, isEmpty);
  });

  testWidgets(
    'a notification tells the person what came while they cannot see Baton',
    (tester) async {
      final notices = FakeNotices();
      final alwaysOn = FakeAlwaysOn()..seen = false;
      final session = await _show(
        tester,
        null,
        const SystemFileChooser(),
        null,
        null,
        null,
        null,
        notices,
        null,
        alwaysOn,
      );

      session.arrive(sampleReceived(1));
      session.emit(Event.transferRequested(request: sampleTransfer()));
      session.emit(Event.windowRequested(request: sampleWindowRequest));
      session.emit(
        const Event.windowRequested(
          request: WindowRequest(
            id: 4,
            deviceId: 'd3',
            deviceName: 'Anna’s Mac',
            display: true,
            width: 3024,
            height: 1964,
            control: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(notices.posted.map((n) => n.title), [
        'Anna’s iPhone sent you some text',
        'Anna’s iPhone wants to send you files',
        'Anna’s Mac wants to show you a window',
        'Anna’s Mac wants to show you its screen',
      ]);
      // Not one of them says what it is about: the title of a window can be the name of a file.
      expect('${notices.posted}', isNot(contains('Budget.numbers')));

      // The person came back to the window: it tells them, and a notification would say it twice.
      alwaysOn.seen = true;
      session.arrive(sampleReceived(2));
      await tester.pumpAndSettle();

      expect(notices.posted, hasLength(4));
      expect(find.text('Anna’s iPhone sent you some text.'), findsOneWidget);
    },
  );

  testWidgets('a device that gives no screen offers no such choice', (
    tester,
  ) async {
    final session = _session()..abilities = _phoneAbilities;
    await _show(tester, session);

    await tester.tap(find.byTooltip('More about Kitchen iPad'));
    await tester.pumpAndSettle();

    expect(find.text('Let this device ask for the screen…'), findsNothing);
  });

  testWidgets(
    'a device that shares windows but cannot give its screen offers no such '
    'choice',
    (tester) async {
      final session = _session()
        ..abilities = const WindowAbilities(
          canShow: true,
          canShare: true,
          canControl: true,
          canGiveScreen: false,
          canBeUnlocked: false,
          canUnlock: false,
        );
      await _show(tester, session);

      await tester.tap(find.byTooltip('More about Kitchen iPad'));
      await tester.pumpAndSettle();

      expect(find.text('Let this device ask for the screen…'), findsNothing);
    },
  );

  testWidgets(
    'a screen that is asked for opens on a screen of its own, once it comes',
    (tester) async {
      final screen = FakeScreen();
      final session = _session()
        ..pairedDevices = [_annasMac]
        ..abilities = _phoneAbilities
        ..holdStart();
      await _show(tester, session, const SystemFileChooser(), null, screen);

      await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Look at its screen'));
      await tester.pump();

      expect(session.screensAsked, [(id: _annasMac.id, control: false)]);
      expect(find.text('Asking Anna’s Mac for its screen'), findsOneWidget);
      expect(
        screen.on,
        isTrue,
        reason: 'the screen stays on while it is asked',
      );

      // The Mac starts it, and the screen that shows it opens before the Mac has answered.
      session.emit(
        Event.windowStarted(
          session: sampleWindowSession(
            display: true,
            title: null,
            app: null,
            width: 0,
            height: 0,
          ),
        ),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));

      expect(
        find.text('Waiting for the screen of Anna’s Mac'),
        findsOneWidget,
        reason: 'it has no size to draw before its first picture',
      );
      expect(find.byType(Texture), findsNothing);
      expect(find.text('The screen cannot be shown here'), findsNothing);

      session.releaseStart();
      await tester.pump();
      session.emit(const Event.windowPicture(id: 1, width: 2560, height: 1600));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));

      expect(find.byType(Texture), findsOneWidget);
      expect(find.text('Waiting for the screen of Anna’s Mac'), findsNothing);
      expect(screen.on, isTrue, reason: 'and while the screen shows');

      // The person stops it, after being asked.
      await openSessionControls(tester);
      await openSessionControls(tester);
      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Stop'));
      await tester.pumpAndSettle();

      expect(session.windowsEnded, [1]);
      expect(find.byType(Texture), findsNothing);
      expect(screen.on, isFalse);
    },
  );

  testWidgets('a screen is asked for only after the person is confirmed', (
    tester,
  ) async {
    final owner = FakeOwnerCheck()..hold();
    final session = _session()
      ..pairedDevices = [_annasMac]
      ..abilities = _phoneAbilities;
    await _show(
      tester,
      session,
      const SystemFileChooser(),
      null,
      null,
      null,
      null,
      null,
      owner,
    );

    await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Look at its screen and control it'));
    await tester.pump();

    expect(owner.asked, ['Look at the screen of Anna’s Mac and control it']);
    expect(
      session.screensAsked,
      isEmpty,
      reason: 'Anna’s Mac is sent nothing before the person is confirmed',
    );

    owner.release();
    await tester.pump();

    expect(session.screensAsked, [(id: _annasMac.id, control: true)]);
  });

  testWidgets(
    'a person who cannot be confirmed asks nothing of the Mac, and is told why',
    (tester) async {
      final screen = FakeScreen();
      final session = _session()
        ..pairedDevices = [_annasMac]
        ..abilities = _phoneAbilities;
      await _show(
        tester,
        session,
        const SystemFileChooser(),
        null,
        screen,
        null,
        null,
        null,
        FakeOwnerCheck(OwnerCheckResult.noLock),
      );

      await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Look at its screen'));
      await tester.pumpAndSettle();

      expect(session.screensAsked, isEmpty);
      expect(
        find.text(describeOwnerRefusal(OwnerCheckResult.noLock, 'Anna’s Mac')!),
        findsOneWidget,
      );
      expect(find.textContaining('Asking'), findsNothing);
      expect(screen.on, isFalse);
    },
  );

  testWidgets('a person who stops the question is left as they were', (
    tester,
  ) async {
    final screen = FakeScreen();
    final session = _session()
      ..pairedDevices = [_annasMac]
      ..abilities = _phoneAbilities;
    await _show(
      tester,
      session,
      const SystemFileChooser(),
      null,
      screen,
      null,
      null,
      null,
      FakeOwnerCheck(OwnerCheckResult.cancelled),
    );

    await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Look at its screen'));
    await tester.pumpAndSettle();

    expect(session.screensAsked, isEmpty);
    expect(find.textContaining('not asked for its screen'), findsNothing);
    expect(find.textContaining('Asking'), findsNothing);
    expect(screen.on, isFalse);
  });

  testWidgets('a screen that the Mac would not start is told with the reason', (
    tester,
  ) async {
    final screen = FakeScreen();
    final session = _session()
      ..pairedDevices = [_annasMac]
      ..abilities = _phoneAbilities
      ..screenStart = const ScreenStarted.notStarted(
        message:
            'Anna’s Mac has not allowed this device to ask for its screen. '
            'Someone there can allow it in the settings for this device.',
      );
    await _show(tester, session, const SystemFileChooser(), null, screen);

    await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Look at its screen and control it'));
    await tester.pumpAndSettle();

    expect(session.screensAsked, [(id: _annasMac.id, control: true)]);
    expect(
      find.text(
        'Anna’s Mac has not allowed this device to ask for its screen. '
        'Someone there can allow it in the settings for this device.',
      ),
      findsOneWidget,
    );
    expect(find.textContaining('Asking'), findsNothing);
    expect(find.byType(Texture), findsNothing);
    expect(screen.on, isFalse);
  });

  testWidgets(
    'an offer to show a window is asked at the bottom of both places',
    (tester) async {
      final session = await _show(tester);

      session.emit(Event.windowRequested(request: sampleWindowRequest));
      await tester.pumpAndSettle();

      expect(
        find.text('Anna’s Mac wants to show you a window'),
        findsOneWidget,
      );
      expect(find.text('Budget.numbers · Numbers'), findsOneWidget);
      expect(
        find.byType(SnackBar),
        findsNothing,
        reason: 'the question is on show, so nothing needs to point to it',
      );

      await tester.tap(find.text('Inbox'));
      await tester.pumpAndSettle();

      expect(
        find.text('Anna’s Mac wants to show you a window'),
        findsOneWidget,
        reason: 'it stays while the person looks at the inbox',
      );
      expect(session.windowAnswers, isEmpty);
    },
  );

  testWidgets('a window that is accepted opens on a screen of its own', (
    tester,
  ) async {
    final screen = FakeScreen();
    final session = await _show(
      tester,
      null,
      const SystemFileChooser(),
      null,
      screen,
    );
    session.emit(Event.windowRequested(request: sampleWindowRequest));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Show it'));
    await tester.pump();

    expect(session.windowAnswers, [(request: 3, accept: true)]);
    expect(find.text('Opening the window from Anna’s Mac…'), findsOneWidget);
    expect(screen.on, isTrue, reason: 'the screen stays on while it opens');

    session.emit(Event.windowStarted(session: sampleWindowSession()));
    await tester.pumpAndSettle();

    expect(find.byType(Texture), findsOneWidget);
    expect(find.text('Opening the window from Anna’s Mac…'), findsNothing);
    expect(screen.on, isTrue, reason: 'and while the window shows');

    // The person stops it, after being asked.
    await openSessionControls(tester);
    await tester.tap(find.byTooltip('Stop showing'));
    await tester.pumpAndSettle();
    expect(find.text('Stop showing this window?'), findsOneWidget);
    await tester.tap(find.text('Stop'));
    await tester.pumpAndSettle();

    expect(session.windowsEnded, [1]);
    expect(find.byType(Texture), findsNothing);
    expect(find.text('Kitchen iPad'), findsOneWidget);
    expect(screen.on, isFalse);
  });

  testWidgets('an offer that comes over another screen is pointed out', (
    tester,
  ) async {
    final session = await _show(tester);
    await tester.tap(find.text('Kitchen iPad'));
    await tester.pumpAndSettle();
    expect(find.text('Send to Kitchen iPad'), findsOneWidget);

    session.emit(Event.windowRequested(request: sampleWindowRequest));
    await tester.pumpAndSettle();

    expect(
      find.text('Anna’s Mac wants to show you a window.'),
      findsOneWidget,
      reason: 'it is said over the screen the person is on',
    );
    expect(
      find.textContaining('Budget'),
      findsNothing,
      reason: 'the title of a window can be private, and nothing says it',
    );

    await tester.tap(find.text('Review'));
    await tester.pumpAndSettle();

    expect(find.text('Send to Kitchen iPad'), findsNothing);
    expect(find.text('Anna’s Mac wants to show you a window'), findsOneWidget);
    expect(find.text('Budget.numbers · Numbers'), findsOneWidget);
  });

  testWidgets('an offer of a whole screen is pointed out as one', (
    tester,
  ) async {
    final session = await _show(tester);
    await tester.tap(find.text('Kitchen iPad'));
    await tester.pumpAndSettle();

    session.emit(
      Event.windowRequested(
        request: const WindowRequest(
          id: 8,
          deviceId: 'd3',
          deviceName: 'Anna’s Mac',
          display: true,
          width: 3024,
          height: 1964,
          control: false,
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(
      find.text('Anna’s Mac wants to show you its screen.'),
      findsOneWidget,
    );
    expect(find.textContaining('a window'), findsNothing);
  });

  testWidgets(
    'the notice of an offer has the name of the device made fit to show',
    (tester) async {
      final session = await _show(tester);
      await tester.tap(find.text('Kitchen iPad'));
      await tester.pumpAndSettle();

      session.emit(
        const Event.windowRequested(
          request: WindowRequest(
            id: 5,
            deviceId: 'd2',
            deviceName: 'Mac\u202Eexe',
            display: false,
            width: 800,
            height: 600,
            control: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.text('Mac\uFFFDexe wants to show you a window.'),
        findsOneWidget,
      );
    },
  );

  testWidgets('Review asks before it leaves a window that is shown', (
    tester,
  ) async {
    final session = await _show(tester);
    session.emit(Event.windowStarted(session: sampleWindowSession()));
    await tester.pumpAndSettle();
    expect(find.byType(Texture), findsOneWidget);

    session.emit(Event.windowRequested(request: sampleWindowRequest));
    await tester.pumpAndSettle();
    expect(find.text('Anna’s Mac wants to show you a window.'), findsOneWidget);

    await tester.tap(find.text('Review'));
    await tester.pumpAndSettle();

    expect(find.text('Stop showing this window?'), findsOneWidget);
    expect(session.windowsEnded, isEmpty);

    await tester.tap(find.text('Keep showing'));
    await tester.pumpAndSettle();

    expect(find.byType(Texture), findsOneWidget);
    expect(session.windowsEnded, isEmpty);

    // Let the window end, so that nothing is left going.
    session.emit(const Event.windowEnded(id: 1, end: WindowEnd.otherDevice()));
    await tester.pumpAndSettle();
  });

  testWidgets(
    'a window of this device that another shows is stopped from here',
    (tester) async {
      final session = await _show(tester);

      session.emit(
        Event.windowStarted(
          session: sampleWindowSession(role: WindowRole.sharing),
        ),
      );
      await tester.pumpAndSettle();

      expect(
        find.text('Sharing Budget.numbers with Anna’s Mac'),
        findsOneWidget,
      );
      expect(
        find.byType(Texture),
        findsNothing,
        reason: 'it is not shown here',
      );

      await tester.tap(find.text('Stop sharing'));
      await tester.pumpAndSettle();

      expect(session.windowsEnded, [1]);
      expect(find.text('Sharing Budget.numbers with Anna’s Mac'), findsNothing);
    },
  );

  testWidgets('a narrow screen has the places in a bar at the bottom', (
    tester,
  ) async {
    await _show(tester);
    tester.view.physicalSize = const Size(430, 932);
    await tester.pumpAndSettle();
    expect(find.byType(NavigationBar), findsOneWidget);
    expect(find.text('Baton'), findsNothing, reason: 'no sidebar');
    await tester.tap(find.text('Settings'));
    await tester.pumpAndSettle();
    expect(find.text('wxyz-abcd-efgh-ijkl-mnop'), findsOneWidget);
  });

  testWidgets('a wide screen lists the places in a sidebar', (tester) async {
    await _show(tester);
    expect(find.byType(NavigationBar), findsNothing);
    expect(find.text('Baton'), findsOneWidget);
    await tester.tap(find.text('Settings'));
    await tester.pumpAndSettle();
    expect(find.text('wxyz-abcd-efgh-ijkl-mnop'), findsOneWidget);
    await tester.tap(find.text('Devices').first);
    await tester.pumpAndSettle();
    expect(find.text('Kitchen iPad'), findsOneWidget);
  });

  testWidgets('the places are kept when the window crosses into the sidebar', (
    tester,
  ) async {
    await _show(tester);
    tester.view.physicalSize = const Size(430, 932);
    await tester.pumpAndSettle();
    final narrow = tester.state(find.byType(DevicesScreen));
    tester.view.physicalSize = const Size(1200, 900);
    await tester.pumpAndSettle();
    expect(find.text('Baton'), findsOneWidget, reason: 'the sidebar');
    expect(
      tester.state(find.byType(DevicesScreen)),
      same(narrow),
      reason: 'the same Devices screen, not one built anew',
    );
  });
}
