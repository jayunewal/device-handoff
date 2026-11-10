// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/always_on.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/mac_passwords.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/permissions.dart';
import 'package:baton/src/screens/permissions_screen.dart';
import 'package:baton/src/screens/restart_unlock_screen.dart';
import 'package:baton/src/screens/settings_screen.dart';
import 'package:baton/src/stay_awake.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_always_on.dart';
import 'fake_backend.dart';
import 'fake_control_access.dart';
import 'fake_notices.dart';
import 'fake_stay_awake.dart';
import 'fake_system_settings.dart';

final _now = DateTime.fromMillisecondsSinceEpoch(1760529600 * 1000);

FakeSession _session() => FakeSession(
  me: Me(
    id: 'f0' * 32,
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

/// Keeps nothing: the settings only need to know that there is one.
class _NoMacs extends MacPasswordKeeper {
  @override
  Future<List<SavedMac>> list() async => const [];

  @override
  Future<void> save(SavedMac mac, {String? password}) async {}

  @override
  Future<String?> open(String id, {required String reason}) async => null;

  @override
  Future<void> forget(String id) async {}
}

Future<void> _show(
  WidgetTester tester,
  FakeSession session, {
  FakeSystemSettings? system,
  FakeAlwaysOn? alwaysOn,
  FakeStayAwake? stayAwake,
  MacPasswordKeeper? macPasswords,
}) async {
  tester.view
    ..physicalSize = const Size(600, 1400)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: SettingsScreen(
        session: session,
        now: () => _now,
        platform: TargetPlatform.macOS,
        screenAccess: FakeScreenAccess(ScreenAccess.unavailable),
        notices: FakeNotices(NoticeAccess.unavailable),
        system:
            system ??
            FakeSystemSettings(const {
              Permission.accessibility: PermissionState.on,
            }),
        alwaysOn: alwaysOn ?? FakeAlwaysOn(SignInOpening.unavailable),
        stayAwake: stayAwake ?? FakeStayAwake(StayAwake.unavailable),
        macPasswords: macPasswords,
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'shows this device, whether others reach it, and its fingerprint',
    (tester) async {
      final session = _session();
      await _show(tester, session);
      expect(find.text('Desk Mac'), findsOneWidget);
      expect(find.text('Mac'), findsOneWidget);
      expect(find.text('wxyz-abcd-efgh-ijkl-mnop'), findsOneWidget);
      expect(find.text('Connecting…'), findsOneWidget);

      session.reach = const Network(
        relay: 'https://relay.example',
        addresses: ['192.168.1.20:4000'],
        lanDiscovery: true,
      );
      await tester.pump(const Duration(seconds: 3));
      expect(
        find.text('Online: your devices can reach this one'),
        findsOneWidget,
      );
    },
  );

  testWidgets('says how many permissions need the person, and leads there', (
    tester,
  ) async {
    await _show(
      tester,
      _session(),
      system: FakeSystemSettings(const {
        Permission.accessibility: PermissionState.off,
        Permission.inputMonitoring: PermissionState.off,
      }),
    );
    expect(find.text('2 need you'), findsOneWidget);
    await tester.tap(find.text('Permissions'));
    await tester.pumpAndSettle();
    expect(find.byType(PermissionsScreen), findsOneWidget);
  });

  testWidgets('says when every permission is on', (tester) async {
    await _show(tester, _session());
    expect(find.text('All on'), findsOneWidget);
  });

  testWidgets('opens the diagnostics', (tester) async {
    final session = _session()
      ..log.add('2026-10-15T08:40:00Z  INFO baton_app: Baton started');
    await _show(tester, session);
    await tester.tap(find.text('Diagnostics'));
    await tester.pumpAndSettle();
    expect(
      find.text('This device: Desk Mac (Mac), id f0f0f0f0f0'),
      findsOneWidget,
    );
    expect(
      find.textContaining('INFO baton_app: Baton started'),
      findsOneWidget,
    );
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.text('Diagnostics'), findsOneWidget);
  });

  testWidgets('leads to unlocking a Mac after restart only where there is a '
      'place for the passwords', (tester) async {
    await _show(tester, _session());
    expect(find.text('Unlock a Mac after restart'), findsNothing);

    await _show(tester, _session(), macPasswords: _NoMacs());
    await tester.tap(find.text('Unlock a Mac after restart'));
    await tester.pumpAndSettle();
    expect(find.byType(RestartUnlockScreen), findsOneWidget);
  });
  testWidgets('offers the Macs that are paired with this device when a Mac is '
      'added to unlock after a restart', (tester) async {
    Device paired(String name, Platform platform, String address) => Device(
      id: name.padRight(64, '0'),
      name: name,
      platform: platform,
      fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
      pairedAt: 1,
      localAddresses: [address],
    );
    final session = _session()
      ..pairedDevices = [
        paired('Studio Mac', Platform.macOs, '10.0.0.5'),
        paired('Jay iPhone', Platform.ios, '10.0.0.9'),
        paired('Kitchen Mac', Platform.macOs, '192.168.73.20'),
      ];
    await _show(tester, session, macPasswords: _NoMacs());
    await tester.tap(find.text('Unlock a Mac after restart'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add a Mac'));
    await tester.pumpAndSettle();

    // Both Macs, with where each was seen, and not the phone. There are two, so Baton waits to be
    // told which one before it looks.
    expect(find.text('Studio Mac'), findsOneWidget);
    expect(find.text('Kitchen Mac'), findsOneWidget);
    expect(find.text('Jay iPhone'), findsNothing);
    expect(find.text('Seen at 10.0.0.5'), findsOneWidget);
    expect(find.text('Seen at 192.168.73.20'), findsOneWidget);
  });

  group('opening at sign-in', () {
    const title = 'Open Baton when you sign in';

    testWidgets('has no switch where the system cannot be asked to', (
      tester,
    ) async {
      await _show(tester, _session());
      expect(find.text(title), findsNothing);
      expect(find.byType(Switch), findsNothing);
    });

    testWidgets('says where it stands, and turns it on', (tester) async {
      final alwaysOn = FakeAlwaysOn();
      await _show(tester, _session(), alwaysOn: alwaysOn);
      expect(find.text(title), findsOneWidget);
      expect(
        find.text(
          'Baton stays in the menu bar, so that your other devices can reach '
          'this Mac.',
        ),
        findsOneWidget,
      );
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);

      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      expect(alwaysOn.sets, [true]);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(alwaysOn.settingsOpened, 0);
    });

    testWidgets('turns it off from the row as well as the switch', (
      tester,
    ) async {
      final alwaysOn = FakeAlwaysOn(SignInOpening.on);
      await _show(tester, _session(), alwaysOn: alwaysOn);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);

      await tester.tap(find.text(title));
      await tester.pumpAndSettle();

      expect(alwaysOn.sets, [false]);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
    });

    testWidgets('leaves the switch off and opens Settings where the system '
        'waits for the person', (tester) async {
      final alwaysOn = FakeAlwaysOn()..onAnswer = SignInOpening.needsApproval;
      await _show(tester, _session(), alwaysOn: alwaysOn);

      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
      expect(alwaysOn.settingsOpened, 1);
      expect(
        find.text('Turned off in System Settings. Tap to turn it on there.'),
        findsOneWidget,
      );

      // The person said no there, and asks again: Settings opens again, and no state is made up.
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();
      expect(alwaysOn.settingsOpened, 2);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
    });

    testWidgets('says what the system said when it refuses, and leaves the '
        'switch as it was', (tester) async {
      final alwaysOn = FakeAlwaysOn()
        ..failure = const AlwaysOnFailure('Operation not permitted');
      await _show(tester, _session(), alwaysOn: alwaysOn);

      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      expect(
        find.text(
          'Baton could not change this. The system said: Operation not '
          'permitted',
        ),
        findsOneWidget,
      );
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
      expect(alwaysOn.settingsOpened, 0);

      // The switch is free again after a refusal.
      alwaysOn.failure = null;
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    });

    testWidgets('looks again when the person comes back from Settings', (
      tester,
    ) async {
      final alwaysOn = FakeAlwaysOn(SignInOpening.needsApproval);
      await _show(tester, _session(), alwaysOn: alwaysOn);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);

      // They said yes there.
      alwaysOn.opening = SignInOpening.on;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();

      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    });
  });

  group('staying awake while plugged in', () {
    const title = 'Stay awake while plugged in';
    const offText =
        'Keeps this Mac from sleeping while it is plugged in, so that your '
        'other devices can reach it. The screen can still turn off.';
    const holdingText =
        'This Mac stays awake while it is plugged in. The screen can still '
        'turn off, and a laptop with its lid shut can still sleep.';
    const waitingText =
        'This Mac is running on its battery, so it can sleep. It stays awake '
        'again once it is plugged in.';
    const refusedText =
        'Baton could not keep this Mac awake, because the system would not '
        'let it. Turn this off and on to try again.';

    testWidgets('has no switch where the Mac cannot be asked to', (
      tester,
    ) async {
      await _show(tester, _session());
      expect(find.text(title), findsNothing);
      expect(find.byType(Switch), findsNothing);
    });

    testWidgets('says what it does, and turns it on', (tester) async {
      final stay = FakeStayAwake();
      await _show(tester, _session(), stayAwake: stay);
      expect(find.text(title), findsOneWidget);
      expect(find.text(offText), findsOneWidget);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);

      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      expect(stay.sets, [true]);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(find.text(holdingText), findsOneWidget);
      expect(find.text(offText), findsNothing);
    });

    testWidgets('turns it off from the row as well as the switch', (
      tester,
    ) async {
      final stay = FakeStayAwake(StayAwake.holding);
      await _show(tester, _session(), stayAwake: stay);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(find.text(holdingText), findsOneWidget);

      await tester.tap(find.text(title));
      await tester.pumpAndSettle();

      expect(stay.sets, [false]);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
      expect(find.text(offText), findsOneWidget);
    });

    testWidgets('says that a Mac on its battery waits to be plugged in', (
      tester,
    ) async {
      final stay = FakeStayAwake(StayAwake.waiting);
      await _show(tester, _session(), stayAwake: stay);

      // It was asked for, so the switch is on, and the Mac is not held now.
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(find.text(waitingText), findsOneWidget);
      expect(find.text(holdingText), findsNothing);
    });

    testWidgets('says that the Mac was not held when it was asked on the '
        'battery', (tester) async {
      final stay = FakeStayAwake()..onAnswer = StayAwake.waiting;
      await _show(tester, _session(), stayAwake: stay);

      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(find.text(waitingText), findsOneWidget);
    });

    testWidgets('says when the system would not hold the Mac, and asks again '
        'on a new turn of the switch', (tester) async {
      final stay = FakeStayAwake()..onAnswer = StayAwake.refused;
      await _show(tester, _session(), stayAwake: stay);

      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      // Asked for, and not done: the switch is on and the sentence says so.
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(find.text(refusedText), findsOneWidget);
      expect(find.text(holdingText), findsNothing);

      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
      stay.onAnswer = StayAwake.holding;
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      expect(stay.sets, [true, false, true]);
      expect(find.text(holdingText), findsOneWidget);
    });

    testWidgets('holds the switch while the Mac is being asked', (
      tester,
    ) async {
      final stay = FakeStayAwake()..hold = Completer<void>();
      await _show(tester, _session(), stayAwake: stay);

      await tester.tap(find.byType(Switch));
      await tester.pump();
      expect(tester.widget<Switch>(find.byType(Switch)).onChanged, isNull);
      // A second tap on the row does nothing while the first is on its way.
      await tester.tap(find.text(title));
      await tester.pump();
      expect(stay.sets, [true]);

      stay.hold!.complete();
      await tester.pumpAndSettle();
      expect(tester.widget<Switch>(find.byType(Switch)).onChanged, isNotNull);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
    });

    testWidgets(
      'turns away a second tap that comes before the screen has changed',
      (tester) async {
        // Two taps in the same frame both reach the switch as it was before it was held, so the
        // change that goes to the Mac is the first, and the second is turned away by the screen.
        final stay = FakeStayAwake()..hold = Completer<void>();
        await _show(tester, _session(), stayAwake: stay);

        await tester.tap(find.byType(Switch));
        await tester.tap(find.byType(Switch));
        await tester.pump();
        expect(stay.sets, [true]);

        stay.hold!.complete();
        await tester.pumpAndSettle();
        expect(stay.sets, [true]);
        expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      },
    );

    testWidgets('does not touch a screen that closed while the Mac was asked', (
      tester,
    ) async {
      // Where it stands, when the screen opens.
      final looking = FakeStayAwake()..stateHold = Completer<void>();
      await _show(tester, _session(), stayAwake: looking);
      await tester.pumpWidget(const SizedBox());
      looking.stateHold!.complete();
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);

      // A change, from the switch.
      final changing = FakeStayAwake()..hold = Completer<void>();
      await _show(tester, _session(), stayAwake: changing);
      await tester.tap(find.byType(Switch));
      await tester.pump();
      await tester.pumpWidget(const SizedBox());
      changing.hold!.complete();
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
    });

    testWidgets('looks again when the person comes back to Baton', (
      tester,
    ) async {
      final stay = FakeStayAwake(StayAwake.waiting);
      await _show(tester, _session(), stayAwake: stay);
      expect(find.text(waitingText), findsOneWidget);
      final asked = stay.asks;

      // The Mac was plugged in meanwhile.
      stay.stay = StayAwake.holding;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();

      expect(stay.asks, asked + 1);
      expect(find.text(holdingText), findsOneWidget);
    });

    testWidgets('is a switch of its own beside opening at sign-in', (
      tester,
    ) async {
      final alwaysOn = FakeAlwaysOn();
      final stay = FakeStayAwake();
      await _show(tester, _session(), alwaysOn: alwaysOn, stayAwake: stay);
      expect(find.byType(Switch), findsNWidgets(2));

      await tester.tap(find.text(title));
      await tester.pumpAndSettle();

      expect(stay.sets, [true]);
      expect(alwaysOn.sets, isEmpty);
      await tester.tap(find.text('Open Baton when you sign in'));
      await tester.pumpAndSettle();
      expect(alwaysOn.sets, [true]);
      expect(stay.sets, [true]);
    });
  });
}
