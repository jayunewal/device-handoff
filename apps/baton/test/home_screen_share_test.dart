// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/screens/device_picker_sheet.dart';
import 'package:baton/src/screens/home_screen.dart';
import 'package:baton/src/screens/send_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_always_on.dart';
import 'fake_backend.dart';
import 'fake_control_access.dart';
import 'fake_notices.dart';
import 'fake_owner_check.dart';
import 'fake_screen.dart';
import 'fake_shares.dart';
import 'fake_shower.dart';

// Five minutes after the devices in these tests were last heard from.
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

const _me = Me(
  id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
  name: 'Pocket Phone',
  platform: Platform.ios,
  fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
);

FakeSession _session([List<Device> paired = const []]) =>
    FakeSession(me: _me, pairedDevices: paired);

/// A session whose list of devices cannot be read once a test says so.
class _CannotListDevices extends FakeSession {
  _CannotListDevices() : super(me: _me, pairedDevices: [_kitchenIpad]);

  bool failing = false;

  @override
  Future<List<Device>> devices() async {
    if (failing) throw StateError('Baton stopped');
    return super.devices();
  }
}

/// Shows the app's two places on a phone for [session], with [shares] as what was handed to Baton.
Future<void> _show(
  WidgetTester tester,
  FakeSession session,
  FakeShares shares,
) async {
  tester.view
    ..physicalSize = const Size(900, 1600)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: HomeScreen(
        session: session,
        now: () => _now,
        scanner: (context, onCode) => const Text('(camera)'),
        openLink: (address) async => true,
        shower: FakeShower(),
        screen: FakeScreen(),
        controlAccess: FakeControlAccess(ControlAccess.unavailable),
        screenAccess: FakeScreenAccess(ScreenAccess.unavailable),
        notices: FakeNotices(NoticeAccess.unavailable),
        ownerCheck: FakeOwnerCheck(),
        alwaysOn: FakeAlwaysOn(),
        shares: shares,
      ),
    ),
  );
  await tester.pumpAndSettle();
}

/// What is in the box of the screen for sending that is open.
String _typed(WidgetTester tester) => tester
    .widget<TextField>(
      find.descendant(
        of: find.byType(SendScreen),
        matching: find.byType(TextField),
      ),
    )
    .controller!
    .text;

/// The device that the screen for sending that is open sends to.
String _sendingTo(WidgetTester tester) =>
    tester.widget<SendScreen>(find.byType(SendScreen)).device.name;

/// The app goes to the background and comes back, through the states that the system goes
/// through.
Future<void> _leaveAndComeBack(WidgetTester tester) async {
  for (final state in [
    AppLifecycleState.inactive,
    AppLifecycleState.hidden,
    AppLifecycleState.paused,
    AppLifecycleState.hidden,
    AppLifecycleState.inactive,
    AppLifecycleState.resumed,
  ]) {
    tester.binding.handleAppLifecycleStateChanged(state);
  }
  await tester.pumpAndSettle();
}

void main() {
  group('what was handed to Baton to send', () {
    testWidgets('opens the box for the one device that is paired, with it in', (
      tester,
    ) async {
      final session = _session([_kitchenIpad]);
      await _show(tester, session, FakeShares(['some words to send']));

      expect(find.byType(SendScreen), findsOneWidget);
      expect(_sendingTo(tester), 'Kitchen iPad');
      expect(_typed(tester), 'some words to send');
      expect(find.text('Send text'), findsOneWidget);
      expect(
        session.sent,
        isEmpty,
        reason: 'nothing goes before the person taps Send',
      );
    });

    testWidgets('is sent when the person taps Send, as a link if it is one', (
      tester,
    ) async {
      final session = _session([_kitchenIpad]);
      await _show(tester, session, FakeShares(['https://example.com/a?b=c']));
      expect(find.text('Send link'), findsOneWidget);

      await tester.tap(find.text('Send link'));
      await tester.pumpAndSettle();

      expect(session.sent, [
        (
          id: _kitchenIpad.id,
          url: 'https://example.com/a?b=c',
          title: null,
          text: null,
        ),
      ]);
    });

    testWidgets('asks which device when there are several, and sends nothing', (
      tester,
    ) async {
      final session = _session([_kitchenIpad, _annasMac]);
      await _show(tester, session, FakeShares(['for the Mac']));

      expect(find.byType(DevicePickerSheet), findsOneWidget);
      expect(find.text('Send to which device?'), findsOneWidget);
      expect(find.byType(SendScreen), findsNothing);
      expect(session.sent, isEmpty);

      await tester.tap(
        find.descendant(
          of: find.byType(DevicePickerSheet),
          matching: find.text('Anna’s Mac'),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.byType(DevicePickerSheet), findsNothing);
      expect(_sendingTo(tester), 'Anna’s Mac');
      expect(_typed(tester), 'for the Mac');
      expect(session.sent, isEmpty);
    });

    testWidgets('is let go when the person closes the list of devices', (
      tester,
    ) async {
      final session = _session([_kitchenIpad, _annasMac]);
      await _show(tester, session, FakeShares(['not for anyone']));
      expect(find.byType(DevicePickerSheet), findsOneWidget);

      // A tap on the screen behind the sheet closes it.
      await tester.tapAt(const Offset(20, 20));
      await tester.pumpAndSettle();

      expect(find.byType(DevicePickerSheet), findsNothing);
      expect(find.byType(SendScreen), findsNothing);
      expect(session.sent, isEmpty);
    });

    testWidgets('says that there is no device to send to when none is paired', (
      tester,
    ) async {
      final session = _session();
      await _show(tester, session, FakeShares(['no one to give it to']));

      expect(find.byType(SendScreen), findsNothing);
      expect(
        find.descendant(
          of: find.byType(SnackBar),
          matching: find.textContaining('no paired device'),
        ),
        findsOneWidget,
      );
      expect(session.sent, isEmpty);
    });

    testWidgets('says that the devices could not be looked up, and goes on', (
      tester,
    ) async {
      final session = _CannotListDevices();
      // What was handed over is held back until the screens have read the list once.
      final shares = FakeShares(['text'])..hold();
      await _show(tester, session, shares);
      session.failing = true;

      shares.release();
      await tester.pumpAndSettle();

      expect(find.byType(SendScreen), findsNothing);
      expect(
        find.descendant(
          of: find.byType(SnackBar),
          matching: find.textContaining('could not look up your devices'),
        ),
        findsOneWidget,
      );
    });

    testWidgets('shows nothing when nothing was handed over', (tester) async {
      final shares = FakeShares();
      await _show(tester, _session([_kitchenIpad]), shares);

      expect(shares.asked, 1, reason: 'it is asked once the screen is up');
      expect(find.byType(SendScreen), findsNothing);
      expect(find.byType(DevicePickerSheet), findsNothing);
      expect(find.byType(SnackBar), findsNothing);
    });

    testWidgets('is taken again when the app comes back to the screen', (
      tester,
    ) async {
      final shares = FakeShares();
      await _show(tester, _session([_kitchenIpad]), shares);
      expect(find.byType(SendScreen), findsNothing);

      // The person shared something from another app, which brought Baton back.
      shares.hand('from the share sheet');
      await _leaveAndComeBack(tester);

      expect(shares.asked, 2);
      expect(_typed(tester), 'from the share sheet');
    });

    testWidgets(
      'is taken when the app is told of it, with no change in the app to show it',
      (tester) async {
        final shares = FakeShares();
        await _show(tester, _session([_kitchenIpad]), shares);
        expect(find.byType(SendScreen), findsNothing);

        // The action ran while Baton was in front, as on an iPad beside Safari: the app did not
        // leave the screen, so it did not come back to it either.
        shares.handAndTell('from the share sheet');
        await tester.pumpAndSettle();

        expect(shares.asked, 2);
        expect(_sendingTo(tester), 'Kitchen iPad');
        expect(_typed(tester), 'from the share sheet');
      },
    );

    testWidgets('that the app is told of while one is open waits its turn', (
      tester,
    ) async {
      final shares = FakeShares(['first']);
      await _show(tester, _session([_kitchenIpad]), shares);
      expect(_typed(tester), 'first');

      shares.handAndTell('later');
      await tester.pumpAndSettle();
      expect(
        _typed(tester),
        'first',
        reason: 'the person is not taken away from what they are doing',
      );

      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(_typed(tester), 'later');
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.byType(SendScreen), findsNothing);
    });

    testWidgets('that the app is told of twice at once comes once each', (
      tester,
    ) async {
      final shares = FakeShares();
      await _show(tester, _session([_kitchenIpad]), shares);

      shares
        ..handAndTell('one')
        ..handAndTell('two');
      await tester.pumpAndSettle();
      expect(_typed(tester), 'one');

      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(_typed(tester), 'two');

      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.byType(SendScreen), findsNothing);
    });

    testWidgets('is not lost when the app is told while a read is under way', (
      tester,
    ) async {
      final shares = FakeShares(['first'])..hold();
      await _show(tester, _session([_kitchenIpad]), shares);

      shares.handAndTell('second');
      shares.release();
      await tester.pumpAndSettle();
      expect(_typed(tester), 'first');

      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(_typed(tester), 'second');
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.byType(SendScreen), findsNothing);
    });

    testWidgets('is no longer listened for once the screen is closed', (
      tester,
    ) async {
      final shares = FakeShares();
      final session = _session([_kitchenIpad]);
      await _show(tester, session, shares);
      expect(shares.listening, isTrue);

      await tester.pumpWidget(const MaterialApp(home: SizedBox()));
      expect(shares.listening, isFalse);

      shares.handAndTell('too late');
      await tester.pumpAndSettle();
      expect(find.byType(SendScreen), findsNothing);
      expect(shares.asked, 1);
    });

    testWidgets('is not taken while the app is only passing through', (
      tester,
    ) async {
      final shares = FakeShares();
      await _show(tester, _session([_kitchenIpad]), shares);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      await tester.pump();

      expect(shares.asked, 1);
    });

    testWidgets('comes one after the other, each in a box of its own', (
      tester,
    ) async {
      final session = _session([_kitchenIpad]);
      await _show(tester, session, FakeShares(['first', 'second']));
      expect(_typed(tester), 'first');

      await tester.pageBack();
      await tester.pumpAndSettle();

      expect(find.byType(SendScreen), findsOneWidget);
      expect(_typed(tester), 'second');

      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.byType(SendScreen), findsNothing);
    });

    testWidgets('that comes while one is open waits until that one is closed', (
      tester,
    ) async {
      final shares = FakeShares(['first']);
      await _show(tester, _session([_kitchenIpad]), shares);
      expect(_typed(tester), 'first');

      shares.hand('later');
      await _leaveAndComeBack(tester);
      expect(
        _typed(tester),
        'first',
        reason: 'the person is not taken away from what they are doing',
      );

      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(_typed(tester), 'later');
    });

    testWidgets('is not lost to a read that takes a while', (tester) async {
      final shares = FakeShares(['slow'])..hold();
      await _show(tester, _session([_kitchenIpad]), shares);
      expect(find.byType(SendScreen), findsNothing);

      shares.release();
      await tester.pumpAndSettle();

      expect(_typed(tester), 'slow');
    });

    testWidgets('is let go of when the screen is closed before it comes', (
      tester,
    ) async {
      final shares = FakeShares(['too late'])..hold();
      final session = _session([_kitchenIpad]);
      await _show(tester, session, shares);

      await tester.pumpWidget(const MaterialApp(home: SizedBox()));
      shares.release();
      await tester.pumpAndSettle();

      expect(find.byType(SendScreen), findsNothing);
      expect(session.sent, isEmpty);
    });
  });
}
