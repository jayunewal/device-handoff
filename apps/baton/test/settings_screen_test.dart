// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/always_on.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/mac_passwords.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/permissions.dart';
import 'package:baton/src/screens/permissions_screen.dart';
import 'package:baton/src/screens/restart_unlock_screen.dart';
import 'package:baton/src/screens/settings_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_always_on.dart';
import 'fake_backend.dart';
import 'fake_control_access.dart';
import 'fake_notices.dart';
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
}
