// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/qr.dart';
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

Widget _screen(FakeSession session) => MaterialApp(
  home: DevicesScreen(
    session: session,
    now: () => _now,
    // No camera here: the pairing screens get a stand-in.
    scanner: (context, onCode) => const Text('(camera)'),
  ),
);

Future<void> _show(WidgetTester tester, FakeSession session) async {
  await tester.pumpWidget(_screen(session));
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
    expect(find.textContaining('Tap a device'), findsNothing);
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

  testWidgets('a tap on a device opens the screen to send to it', (
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
    expect(
      find.text('Tap a device to send it a link or some text.'),
      findsOneWidget,
    );

    await tester.tap(find.text('Phone'));
    await tester.pumpAndSettle();

    expect(find.text('Send to Phone'), findsOneWidget);
    expect(find.text('Send to Kitchen iPad'), findsNothing);

    await tester.pageBack();
    await tester.pumpAndSettle();

    expect(find.text('Send to Phone'), findsNothing);
    expect(find.text('Kitchen iPad'), findsOneWidget);
  });

  testWidgets('the send button on a device opens it too, and sends to it', (
    tester,
  ) async {
    final session = _session(
      devices: [
        _device('Kitchen iPad', Platform.ipadOs),
        _device('Phone', Platform.android),
      ],
    );
    await _show(tester, session);

    await tester.tap(find.byTooltip('Send to Kitchen iPad'));
    await tester.pumpAndSettle();
    expect(find.text('Send to Kitchen iPad'), findsOneWidget);

    await tester.enterText(find.byType(TextField), 'hello');
    await tester.pump();
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => (sent.id, sent.text)), [
      (_device('Kitchen iPad', Platform.ipadOs).id, 'hello'),
    ]);
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

  testWidgets('cancels the events that arrive after it has closed', (
    tester,
  ) async {
    final session = _session()..holdEvents();
    // The screen shows a spinner until the lists are read, so it never settles.
    await tester.pumpWidget(_screen(session));
    await tester.pump();
    expect(session.listens, 0, reason: 'the events have not arrived yet');

    await tester.pumpWidget(const SizedBox());
    session.releaseEvents();
    await tester.pump();
    expect(session.listens, 1, reason: 'a stream is told to stop by a cancel');
    expect(session.listening, isFalse);
  });

  testWidgets('offers the two ways to add a device', (tester) async {
    tester.view
      ..physicalSize = const Size(900, 1600)
      ..devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final session = _session();
    await _show(tester, session);

    await tester.tap(find.text('Add a device'));
    await tester.pumpAndSettle();
    expect(find.text('Show a code'), findsOneWidget);
    expect(find.text('Scan or enter a code'), findsOneWidget);

    await tester.tap(find.text('Show a code'));
    await tester.pumpAndSettle();
    expect(find.byType(QrView), findsOneWidget);
    expect(session.offersMade, 1);

    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(session.offersCancelled, 1);
    await tester.tap(find.text('Scan or enter a code'));
    await tester.pumpAndSettle();
    expect(find.text('(camera)'), findsOneWidget);
    expect(find.text('Connect'), findsOneWidget);
  });

  testWidgets('shows a device that was paired, once the pairing is done', (
    tester,
  ) async {
    final session = _session();
    await _show(tester, session);
    await tester.tap(find.text('Add a device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Show a code'));
    await tester.pumpAndSettle();

    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Yes, they match'));
    await tester.pump();
    final anna = _device('Anna’s iPhone', Platform.ios);
    session.pairedDevices = [anna];
    session.emit(Event.offerEnded(outcome: OfferOutcome.paired(device: anna)));
    session.emit(const Event.devicesChanged());
    await tester.pump();
    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();

    expect(find.text('Anna’s iPhone'), findsOneWidget);
    expect(find.text('No other devices yet'), findsNothing);
  });

  testWidgets('forgets a device when the person agrees', (tester) async {
    final session = _session(
      devices: [
        _device('Laptop', Platform.windows),
        _device('Phone', Platform.android),
      ],
    );
    await _show(tester, session);

    await tester.tap(find.byTooltip('More about Laptop'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Forget this device'));
    await tester.pumpAndSettle();
    expect(find.text('Forget Laptop?'), findsOneWidget);
    expect(session.unpaired, isEmpty, reason: 'not before the person agrees');
    await tester.tap(find.text('Forget'));
    await tester.pumpAndSettle();

    expect(session.unpaired, [_device('Laptop', Platform.windows).id]);
    expect(find.text('Laptop'), findsNothing);
    expect(find.text('Phone'), findsOneWidget);
  });

  testWidgets('keeps a device when the person says no', (tester) async {
    final session = _session(devices: [_device('Laptop', Platform.windows)]);
    await _show(tester, session);

    await tester.tap(find.byTooltip('More about Laptop'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Forget this device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(session.unpaired, isEmpty);
    expect(find.text('Laptop'), findsOneWidget);
  });

  testWidgets('does not mind a device that is gone already', (tester) async {
    final session = _session(devices: [_device('Laptop', Platform.windows)])
      ..unpairFailure = const BatonError(
        kind: ErrorKind.notPaired,
        message: 'That device is not paired.',
      );
    await _show(tester, session);
    // The other device forgot this one first; the list that is read next shows it.
    session.pairedDevices = [];

    await tester.tap(find.byTooltip('More about Laptop'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Forget this device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Forget'));
    await tester.pumpAndSettle();

    expect(find.text('That device is not paired.'), findsNothing);
    expect(find.text('Laptop'), findsNothing);
  });

  testWidgets('says when a device could not be forgotten', (tester) async {
    final session = _session(devices: [_device('Laptop', Platform.windows)])
      ..unpairFailure = const BatonError(
        kind: ErrorKind.storage,
        message: 'Baton could not save its list of devices.',
      );
    await _show(tester, session);

    await tester.tap(find.byTooltip('More about Laptop'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Forget this device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Forget'));
    await tester.pumpAndSettle();

    expect(
      find.text('Baton could not save its list of devices.'),
      findsOneWidget,
    );
    expect(find.text('Laptop'), findsOneWidget);
  });

  testWidgets('reads the devices again if they change while it reads', (
    tester,
  ) async {
    final session = _session()..holdDevices();
    await tester.pumpWidget(_screen(session));
    await tester.pump();

    // A device is paired after the first read began, and its event comes before that read
    // is over: what the read finds is out of date.
    session.pairedDevices = [_device('Laptop', Platform.windows)];
    session.emit(const Event.devicesChanged());
    await tester.pump();
    session.releaseDevices();
    await tester.pumpAndSettle();

    expect(find.text('Laptop'), findsOneWidget);
  });
}
