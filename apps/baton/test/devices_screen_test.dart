// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/devices_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

final _now = DateTime(2026, 10, 15, 12);

int _secondsAgo(Duration ago) =>
    _now.subtract(ago).millisecondsSinceEpoch ~/ 1000;

Device _device(String name, Platform platform, {Duration? seen}) => Device(
  id: name.padRight(64, '0'),
  name: name,
  platform: platform,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: _secondsAgo(const Duration(days: 3)),
  lastSeenAt: seen == null ? null : _secondsAgo(seen),
);

FakeSession _session({List<Device> devices = const []}) => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: devices,
);

Future<void> _show(WidgetTester tester, FakeSession session) async {
  await tester.pumpWidget(
    MaterialApp(
      home: DevicesScreen(session: session, now: () => _now),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('shows this device, and that nothing is paired with it yet', (
    tester,
  ) async {
    await _show(tester, _session());

    expect(find.text('Desk Mac'), findsOneWidget);
    expect(find.text('Mac'), findsOneWidget);
    expect(find.text('wxyz-abcd-efgh-ijkl-mnop'), findsOneWidget);
    expect(find.text('No other devices yet'), findsOneWidget);
  });

  testWidgets('lists the paired devices, with when each was last heard from', (
    tester,
  ) async {
    final session = _session(
      devices: [
        _device(
          'Kitchen iPad',
          Platform.ipadOs,
          seen: const Duration(minutes: 5),
        ),
        _device('Phone', Platform.android),
      ],
    );
    await _show(tester, session);

    expect(find.text('No other devices yet'), findsNothing);
    expect(find.text('Kitchen iPad'), findsOneWidget);
    expect(find.text('iPad · seen 5 minutes ago'), findsOneWidget);
    expect(find.text('Phone'), findsOneWidget);
    expect(find.text('Android · not seen yet'), findsOneWidget);
  });

  testWidgets('follows devices being paired, renamed and forgotten', (
    tester,
  ) async {
    final session = _session();
    await _show(tester, session);
    expect(find.text('No other devices yet'), findsOneWidget);

    session.pairedDevices = [_device('Laptop', Platform.windows)];
    session.emit(const Event.devicesChanged());
    await tester.pumpAndSettle();
    expect(find.text('Laptop'), findsOneWidget);
    expect(find.text('No other devices yet'), findsNothing);

    // Events that were missed because they came too fast: read everything again.
    session.pairedDevices = [_device('Laptop at work', Platform.windows)];
    session.emit(const Event.resync());
    await tester.pumpAndSettle();
    expect(find.text('Laptop'), findsNothing);
    expect(find.text('Laptop at work'), findsOneWidget);

    session.pairedDevices = [];
    session.emit(const Event.devicesChanged());
    await tester.pumpAndSettle();
    expect(find.text('Laptop at work'), findsNothing);
    expect(find.text('No other devices yet'), findsOneWidget);
  });

  testWidgets('says whether other devices can reach this one', (tester) async {
    final session = _session();
    await _show(tester, session);
    expect(find.text('Connecting…'), findsOneWidget);
    expect(find.text('Online'), findsNothing);

    // There is no event for this, so the screen asks every few seconds.
    session.reach = const Network(
      relay: 'https://relay.example',
      addresses: ['192.168.1.20:4000'],
      lanDiscovery: true,
    );
    await tester.pump(const Duration(seconds: 3));
    expect(find.text('Online'), findsOneWidget);
    expect(find.text('Connecting…'), findsNothing);
  });

  testWidgets('lets go of the events and the timer when it closes', (
    tester,
  ) async {
    final session = _session();
    await _show(tester, session);
    expect(session.listening, isTrue);

    // A timer that is still running fails the test, so this also shows it stopped.
    await tester.pumpWidget(const SizedBox());
    expect(session.listening, isFalse);
  });
}
