// SPDX-License-Identifier: Apache-2.0

import 'dart:io';

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

const _allOn = {
  Permission.accessibility: PermissionState.on,
  Permission.inputMonitoring: PermissionState.on,
  Permission.screenRecording: PermissionState.on,
  Permission.camera: PermissionState.on,
  Permission.faceId: PermissionState.on,
};

Map<Permission, PermissionState> _with(
  Map<Permission, PermissionState> changes,
) => {..._allOn, ...changes};

Future<List<PermissionCheck>> _check(
  TargetPlatform platform, {
  NoticeAccess notices = NoticeAccess.allowed,
  Map<Permission, PermissionState> states = _allOn,
}) => checkPermissions(
  platform: platform,
  notices: FakeNotices(notices),
  system: FakeSystemSettings(states),
);

Future<void> _showPermissions(
  WidgetTester tester, {
  TargetPlatform platform = TargetPlatform.macOS,
  FakeScreenAccess? screen,
  FakeNotices? notices,
  required FakeSystemSettings system,
}) async {
  tester.view
    ..physicalSize = const Size(600, 2000)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: PermissionsScreen(
        platform: platform,
        screenAccess: screen ?? FakeScreenAccess(),
        notices: notices ?? FakeNotices(),
        system: system,
      ),
    ),
  );
  await tester.pumpAndSettle();
}

/// The body of the Swift function that starts with [start], up to the brace that closes it.
String _body(String swift, String start) {
  final from = swift.indexOf(start);
  expect(from, isNonNegative, reason: start);
  return swift.substring(from, swift.indexOf('\n}\n', from));
}

void main() {
  group('what is checked', () {
    test(
      'a Mac checks what a private screen and control need, and more',
      () async {
        expect(await _check(TargetPlatform.macOS), const [
          PermissionCheck(Permission.accessibility, PermissionState.on),
          PermissionCheck(Permission.inputMonitoring, PermissionState.on),
          PermissionCheck(Permission.screenRecording, PermissionState.on),
          PermissionCheck(Permission.systemAudio, PermissionState.cannotCheck),
          PermissionCheck(Permission.notifications, PermissionState.on),
          PermissionCheck(Permission.camera, PermissionState.on),
          PermissionCheck(Permission.localNetwork, PermissionState.cannotCheck),
        ]);
      },
    );

    test('a Mac says what is off, and what was never asked', () async {
      final checks = await _check(
        TargetPlatform.macOS,
        notices: NoticeAccess.needed,
        states: _with({
          Permission.accessibility: PermissionState.off,
          Permission.inputMonitoring: PermissionState.off,
          Permission.screenRecording: PermissionState.off,
          Permission.camera: PermissionState.off,
        }),
      );
      expect(checks.take(6), const [
        PermissionCheck(Permission.accessibility, PermissionState.off),
        PermissionCheck(Permission.inputMonitoring, PermissionState.off),
        PermissionCheck(Permission.screenRecording, PermissionState.off),
        PermissionCheck(Permission.systemAudio, PermissionState.cannotCheck),
        PermissionCheck(Permission.notifications, PermissionState.notAsked),
        PermissionCheck(Permission.camera, PermissionState.off),
      ]);
      expect(checks.where((check) => check.needsAttention), hasLength(5));
    });

    test('what a Mac cannot have has no row', () async {
      final checks = await _check(
        TargetPlatform.macOS,
        notices: NoticeAccess.unavailable,
        states: const {},
      );
      expect(checks, const [
        PermissionCheck(Permission.systemAudio, PermissionState.cannotCheck),
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
      await _showPermissions(tester, system: FakeSystemSettings(_allOn));
      expect(find.text('Everything Baton can check is on'), findsOneWidget);
      expect(find.text('On'), findsNWidgets(5));
      // Only the sound and the network, which Baton cannot check, have a button.
      expect(find.text('Open Settings'), findsNWidgets(2));
    });

    testWidgets('asks for what was never asked, in one tap', (tester) async {
      final system = FakeSystemSettings(
        _with({Permission.camera: PermissionState.notAsked}),
      );
      await _showPermissions(tester, system: system);
      expect(find.text('1 permission needs you'), findsOneWidget);
      await tester.tap(find.text('Turn On'));
      await tester.pumpAndSettle();
      expect(system.requested, [Permission.camera]);
      expect(system.opened, isEmpty);
      expect(find.text('Everything Baton can check is on'), findsOneWidget);
    });

    testWidgets('opens Settings for what was turned off', (tester) async {
      final system = FakeSystemSettings(
        _with({Permission.camera: PermissionState.off}),
      );
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
      expect(system.requested, isEmpty);
    });

    testWidgets('what a private screen needs is asked of the Mac, which opens '
        'Settings', (tester) async {
      final screen = FakeScreenAccess();
      final system = FakeSystemSettings(
        _with({
          Permission.accessibility: PermissionState.off,
          Permission.inputMonitoring: PermissionState.off,
          Permission.screenRecording: PermissionState.off,
        }),
      );
      await _showPermissions(tester, screen: screen, system: system);
      expect(find.text('3 permissions need you'), findsOneWidget);
      expect(find.text('Input Monitoring'), findsOneWidget);
      await tester.tap(find.bySemanticsLabel('Open Settings, Accessibility'));
      await tester.pumpAndSettle();
      await tester.tap(
        find.bySemanticsLabel('Open Settings, Input Monitoring'),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.bySemanticsLabel('Open Settings, Screen Recording'),
      );
      await tester.pumpAndSettle();
      // Screen Recording is asked of the app itself, and not of the plugin that shares the screen,
      // which may not be there.
      expect(system.requested, [
        Permission.accessibility,
        Permission.inputMonitoring,
        Permission.screenRecording,
      ]);
      expect(screen.requests, 0);
      expect(system.opened, isEmpty);
    });

    testWidgets('the sound is asked for while the person can see the '
        'question, then Settings opens', (tester) async {
      final screen = FakeScreenAccess();
      final system = FakeSystemSettings(_allOn);
      await _showPermissions(tester, screen: screen, system: system);
      expect(find.text('System Audio Recording'), findsOneWidget);
      // It cannot be checked, so it never counts as needing the person.
      expect(find.text('Everything Baton can check is on'), findsOneWidget);
      await tester.tap(
        find.bySemanticsLabel('Open Settings, System Audio Recording'),
      );
      await tester.pumpAndSettle();
      expect(screen.soundRequests, 1);
      expect(system.opened, [Permission.systemAudio]);
    });

    testWidgets('says where to go when Settings does not open', (tester) async {
      final system = FakeSystemSettings(
        _with({Permission.camera: PermissionState.off}),
      )..opens = false;
      await _showPermissions(tester, system: system);
      await tester.tap(find.bySemanticsLabel('Open Settings, Camera'));
      await tester.pump();
      expect(
        find.text('Open System Settings, Privacy & Security, Camera.'),
        findsOneWidget,
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('looks again when the person comes back', (tester) async {
      final system = FakeSystemSettings(
        _with({Permission.camera: PermissionState.off}),
      );
      await _showPermissions(tester, system: system);
      expect(find.text('1 permission needs you'), findsOneWidget);
      system.states[Permission.camera] = PermissionState.on;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();
      expect(find.text('Everything Baton can check is on'), findsOneWidget);
    });

    testWidgets('shows Face ID on an iPhone', (tester) async {
      final system = FakeSystemSettings(
        _with({Permission.faceId: PermissionState.notSetUp}),
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
        FakeSystemSettings(
          _with({Permission.inputMonitoring: PermissionState.off}),
        ),
      );
      expect(find.text('1 permission needs you'), findsOneWidget);
      await tester.tap(find.text('Review'));
      await tester.pumpAndSettle();
      expect(find.byType(PermissionsScreen), findsOneWidget);
    });

    testWidgets('says nothing when all is on', (tester) async {
      await showDevices(tester, FakeSystemSettings(_allOn));
      expect(find.text('Review'), findsNothing);
    });

    testWidgets('looks again when the person comes back', (tester) async {
      final system = FakeSystemSettings(
        _with({Permission.camera: PermissionState.off}),
      );
      await showDevices(tester, system);
      expect(find.text('Review'), findsOneWidget);
      system.states[Permission.camera] = PermissionState.on;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();
      expect(find.text('Review'), findsNothing);
    });
  });

  group('the channel', () {
    final mac = File('macos/Runner/MainFlutterWindow.swift').readAsStringSync();
    final ios = File('ios/Runner/AppDelegate.swift').readAsStringSync();

    test('both apps answer on the channel, with the same questions', () {
      for (final swift in [mac, ios]) {
        expect(swift, contains('"${PlatformSystemSettings.channelName}"'));
        for (final question in [
          PlatformSystemSettings.stateQuestion,
          PlatformSystemSettings.requestQuestion,
          PlatformSystemSettings.openQuestion,
        ]) {
          expect(swift, contains('case "$question":'), reason: question);
        }
        for (final answer in PlatformSystemSettings.answers.keys) {
          if (answer == 'notSetUp' && swift == mac) continue;
          expect(swift, contains('"$answer"'), reason: answer);
        }
      }
    });

    test('the Mac knows each permission that it keeps by name', () {
      final states = _body(mac, 'func permissionState(');
      for (final permission in [
        Permission.accessibility,
        Permission.inputMonitoring,
        Permission.screenRecording,
        Permission.camera,
      ]) {
        expect(
          states,
          contains('case "${permission.name}":'),
          reason: permission.name,
        );
      }
      final requests = _body(mac, 'func requestPermission(');
      expect(
        requests,
        contains('case "accessibility", "inputMonitoring", "screenRecording":'),
      );
      expect(requests, contains('CGRequestScreenCaptureAccess()'));
      expect(states, contains('CGPreflightScreenCaptureAccess()'));
      expect(requests, contains('case "camera":'));
      // And has a pane of System Settings for each permission that has a button there.
      final panes = _body(mac, 'func settingsAddress(');
      for (final permission in Permission.values.where(
        (permission) => permission != Permission.faceId,
      )) {
        expect(
          panes,
          contains(RegExp('case [^\n]*"${permission.name}"[,:]')),
          reason: 'the pane for ${permission.name}',
        );
      }
      expect(panes, contains('Privacy_ListenEvent'));
    });

    test('the iPhone knows the camera and Face ID by name', () {
      final states = _body(ios, 'func permissionState(');
      expect(states, contains('case "${Permission.camera.name}":'));
      expect(states, contains('case "${Permission.faceId.name}":'));
    });
  });
}
