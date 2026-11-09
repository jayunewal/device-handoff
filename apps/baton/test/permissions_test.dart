// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/permissions.dart';
import 'package:baton/src/screens/devices_screen.dart';
import 'package:baton/src/screens/permissions_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_control_access.dart';
import 'fake_notices.dart';
import 'fake_system_settings.dart';

Future<List<PermissionCheck>> _check(
  TargetPlatform platform, {
  ControlAccess control = ControlAccess.allowed,
  ScreenAccess screen = ScreenAccess.allowed,
  NoticeAccess notices = NoticeAccess.allowed,
  FakeSystemSettings? system,
}) => checkPermissions(
  platform: platform,
  controlAccess: FakeControlAccess(control),
  screenAccess: FakeScreenAccess(screen),
  notices: FakeNotices(notices),
  system:
      system ??
      FakeSystemSettings(
        cameraState: PermissionState.on,
        faceIdState: PermissionState.on,
      ),
);

Future<void> _showPermissions(
  WidgetTester tester, {
  TargetPlatform platform = TargetPlatform.macOS,
  FakeControlAccess? control,
  FakeScreenAccess? screen,
  FakeNotices? notices,
  required FakeSystemSettings system,
}) async {
  tester.view
    ..physicalSize = const Size(600, 1600)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: PermissionsScreen(
        platform: platform,
        controlAccess: control ?? FakeControlAccess(),
        screenAccess: screen ?? FakeScreenAccess(),
        notices: notices ?? FakeNotices(),
        system: system,
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  group('what is checked', () {
    test(
      'a Mac checks control, recording, notices, camera and network',
      () async {
        expect(await _check(TargetPlatform.macOS), const [
          PermissionCheck(Permission.control, PermissionState.on),
          PermissionCheck(Permission.screenRecording, PermissionState.on),
          PermissionCheck(Permission.notifications, PermissionState.on),
          PermissionCheck(Permission.camera, PermissionState.on),
          PermissionCheck(Permission.localNetwork, PermissionState.cannotCheck),
        ]);
      },
    );

    test('a Mac says what is off, and what was never asked', () async {
      final checks = await _check(
        TargetPlatform.macOS,
        control: ControlAccess.needed,
        screen: ScreenAccess.needed,
        notices: NoticeAccess.needed,
        system: FakeSystemSettings(cameraState: PermissionState.off),
      );
      expect(checks.take(4), const [
        PermissionCheck(Permission.control, PermissionState.off),
        PermissionCheck(Permission.screenRecording, PermissionState.off),
        PermissionCheck(Permission.notifications, PermissionState.notAsked),
        PermissionCheck(Permission.camera, PermissionState.off),
      ]);
      expect(checks.where((check) => check.needsAttention), hasLength(4));
    });

    test('what a Mac cannot do has no row', () async {
      final checks = await _check(
        TargetPlatform.macOS,
        control: ControlAccess.unavailable,
        screen: ScreenAccess.unavailable,
        notices: NoticeAccess.unavailable,
        system: FakeSystemSettings(),
      );
      expect(checks, const [
        PermissionCheck(Permission.localNetwork, PermissionState.cannotCheck),
      ]);
    });

    test('an iPhone checks Face ID, camera and network', () async {
      expect(await _check(TargetPlatform.iOS), const [
        PermissionCheck(Permission.faceId, PermissionState.on),
        PermissionCheck(Permission.camera, PermissionState.on),
        PermissionCheck(Permission.localNetwork, PermissionState.cannotCheck),
      ]);
    });

    test('the network, which cannot be checked, never needs attention', () {
      expect(
        const PermissionCheck(
          Permission.localNetwork,
          PermissionState.cannotCheck,
        ).needsAttention,
        isFalse,
      );
    });

    test('other systems check nothing', () async {
      expect(await _check(TargetPlatform.android), isEmpty);
    });
  });

  group('the permissions screen', () {
    testWidgets('says all is on when it is', (tester) async {
      await _showPermissions(
        tester,
        system: FakeSystemSettings(cameraState: PermissionState.on),
      );
      expect(find.text('Everything Baton can check is on'), findsOneWidget);
      expect(find.text('On'), findsNWidgets(4));
      // Only the network, which Baton cannot check, has a button.
      expect(find.text('Open Settings'), findsOneWidget);
    });

    testWidgets('asks for what was never asked, in one tap', (tester) async {
      final system = FakeSystemSettings(cameraState: PermissionState.notAsked);
      await _showPermissions(tester, system: system);
      expect(find.text('1 permission needs you'), findsOneWidget);
      await tester.tap(find.text('Turn On'));
      await tester.pumpAndSettle();
      expect(system.cameraRequests, 1);
      expect(system.opened, isEmpty);
      expect(find.text('Everything Baton can check is on'), findsOneWidget);
    });

    testWidgets('opens Settings for what was turned off', (tester) async {
      final system = FakeSystemSettings(cameraState: PermissionState.off);
      final notices = FakeNotices(NoticeAccess.denied);
      await _showPermissions(tester, notices: notices, system: system);
      expect(find.text('2 permissions need you'), findsOneWidget);
      await tester.tap(find.bySemanticsLabel('Open Settings, Camera'));
      await tester.pumpAndSettle();
      expect(system.opened, [Permission.camera]);
      await tester.tap(find.bySemanticsLabel('Open Settings, Notifications'));
      await tester.pumpAndSettle();
      expect(system.opened, [Permission.camera, Permission.notifications]);
      expect(notices.requests, 0);
    });

    testWidgets('control and recording ask the Mac, which opens Settings', (
      tester,
    ) async {
      final control = FakeControlAccess(ControlAccess.needed);
      final screen = FakeScreenAccess(ScreenAccess.needed);
      final system = FakeSystemSettings(cameraState: PermissionState.on);
      await _showPermissions(
        tester,
        control: control,
        screen: screen,
        system: system,
      );
      await tester.tap(find.bySemanticsLabel('Open Settings, Accessibility'));
      await tester.tap(
        find.bySemanticsLabel('Open Settings, Screen & System Audio Recording'),
      );
      await tester.pumpAndSettle();
      expect(control.requests, 1);
      expect(screen.requests, 1);
      expect(system.opened, isEmpty);
    });

    testWidgets('says where to go when Settings does not open', (tester) async {
      final system = FakeSystemSettings(cameraState: PermissionState.off)
        ..opens = false;
      await _showPermissions(tester, system: system);
      await tester.tap(find.bySemanticsLabel('Open Settings, Camera'));
      await tester.pump();
      expect(
        find.text('Open System Settings, Privacy & Security, Camera.'),
        findsOneWidget,
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('looks again when the person comes back', (tester) async {
      final system = FakeSystemSettings(cameraState: PermissionState.off);
      await _showPermissions(tester, system: system);
      expect(find.text('1 permission needs you'), findsOneWidget);
      system.cameraState = PermissionState.on;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();
      expect(find.text('Everything Baton can check is on'), findsOneWidget);
    });

    testWidgets('shows Face ID on an iPhone', (tester) async {
      final system = FakeSystemSettings(
        cameraState: PermissionState.on,
        faceIdState: PermissionState.notSetUp,
      );
      await _showPermissions(
        tester,
        platform: TargetPlatform.iOS,
        system: system,
      );
      expect(find.text('Face ID'), findsOneWidget);
      expect(find.text('Not set up'), findsOneWidget);
      await tester.tap(find.bySemanticsLabel('Open Settings, Face ID'));
      await tester.pumpAndSettle();
      expect(system.opened, [Permission.faceId]);
    });
  });

  group('the devices screen', () {
    Future<void> showDevices(
      WidgetTester tester,
      FakeSystemSettings system,
    ) async {
      tester.view
        ..physicalSize = const Size(600, 1600)
        ..devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        MaterialApp(
          home: DevicesScreen(
            session: FakeSession(
              me: Me(
                id: 'f' * 64,
                name: 'Desk Mac',
                platform: Platform.macOs,
                fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
              ),
            ),
            scanner: (context, onCode) => const Text('(camera)'),
            platform: TargetPlatform.macOS,
            controlAccess: FakeControlAccess(ControlAccess.unavailable),
            screenAccess: FakeScreenAccess(ScreenAccess.unavailable),
            notices: FakeNotices(NoticeAccess.unavailable),
            system: system,
          ),
        ),
      );
      await tester.pumpAndSettle();
    }

    testWidgets('says when a permission needs the person, and leads there', (
      tester,
    ) async {
      await showDevices(
        tester,
        FakeSystemSettings(cameraState: PermissionState.off),
      );
      expect(find.text('1 permission needs you'), findsOneWidget);
      await tester.tap(find.text('Review'));
      await tester.pumpAndSettle();
      expect(find.byType(PermissionsScreen), findsOneWidget);
    });

    testWidgets('says nothing when all is on', (tester) async {
      await showDevices(
        tester,
        FakeSystemSettings(cameraState: PermissionState.on),
      );
      expect(find.text('Review'), findsNothing);
    });

    testWidgets('the menu leads to the permissions', (tester) async {
      await showDevices(
        tester,
        FakeSystemSettings(cameraState: PermissionState.on),
      );
      await tester.tap(find.byTooltip('More'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Permissions'));
      await tester.pumpAndSettle();
      expect(find.byType(PermissionsScreen), findsOneWidget);
    });
  });
}
