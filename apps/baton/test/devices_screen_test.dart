// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/format.dart';
import 'package:baton/src/qr.dart';
import 'package:baton/src/screens/devices_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_control_access.dart';

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

Widget _screen(
  FakeSession session, {
  void Function(Device device, {required bool control})? onShareWindow,
  ControlAccessKeeper? controlAccess,
}) => MaterialApp(
  home: DevicesScreen(
    session: session,
    now: () => _now,
    // No camera here: the pairing screens get a stand-in.
    scanner: (context, onCode) => const Text('(camera)'),
    onShareWindow: onShareWindow,
    controlAccess:
        controlAccess ?? FakeControlAccess(ControlAccess.unavailable),
  ),
);

Future<void> _show(
  WidgetTester tester,
  FakeSession session, {
  void Function(Device device, {required bool control})? onShareWindow,
  ControlAccessKeeper? controlAccess,
}) async {
  await tester.pumpWidget(
    _screen(
      session,
      onShareWindow: onShareWindow,
      controlAccess: controlAccess,
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

  testWidgets('opens the diagnostics from the menu at the top', (tester) async {
    tester.view
      ..physicalSize = const Size(900, 1600)
      ..devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final session = _session()
      ..log.add('2026-10-15T08:40:00Z  INFO baton_app: Baton started');
    await _show(tester, session);

    await tester.tap(find.byTooltip('More'));
    await tester.pumpAndSettle();
    expect(find.text('Diagnostics'), findsOneWidget, reason: 'the menu entry');
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
    expect(find.text('No other devices yet'), findsOneWidget);
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

  testWidgets('offers no button to share a window where none can be shared', (
    tester,
  ) async {
    await _show(
      tester,
      _session(devices: [_device('Phone', Platform.android)]),
    );

    expect(find.byTooltip('Share a window or screen with Phone'), findsNothing);
    expect(find.byTooltip('Send to Phone'), findsOneWidget);
  });

  testWidgets('a button on a device shares a window with it, and only that', (
    tester,
  ) async {
    final shared = <(Device, bool)>[];
    await _show(
      tester,
      _session(
        devices: [
          _device('Kitchen iPad', Platform.ipadOs),
          _device('Phone', Platform.android),
        ],
      ),
      onShareWindow: (device, {required control}) =>
          shared.add((device, control)),
    );

    await tester.tap(find.byTooltip('Share a window or screen with Phone'));
    await tester.pumpAndSettle();

    expect(shared.map((share) => (share.$1.name, share.$2)), [
      ('Phone', false),
    ]);
    expect(
      find.text('Send to Phone'),
      findsNothing,
      reason: 'the button is not a tap on the device',
    );
  });

  group('a window that can be controlled', () {
    final phone = _device('Phone', Platform.android);
    final shared = <(String, bool)>[];

    Future<FakeControlAccess> show(
      WidgetTester tester,
      ControlAccess access, {
      List<Device>? devices,
    }) async {
      shared.clear();
      final keeper = FakeControlAccess(access);
      await _show(
        tester,
        _session(devices: devices ?? [phone]),
        onShareWindow: (device, {required control}) =>
            shared.add((device.name, control)),
        controlAccess: keeper,
      );
      return keeper;
    }

    Future<void> choose(
      WidgetTester tester,
      String choice, {
      String device = 'Phone',
    }) async {
      await tester.tap(find.byTooltip('Share a window or screen with $device'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(choice));
      await tester.pumpAndSettle();
    }

    testWidgets('is offered as a choice when the system lets it be', (
      tester,
    ) async {
      await show(tester, ControlAccess.allowed);

      await tester.tap(find.byTooltip('Share a window or screen with Phone'));
      await tester.pumpAndSettle();

      expect(find.text('Share to look at'), findsOneWidget);
      expect(
        find.text('Share a window or screen to look at and control'),
        findsOneWidget,
      );
      expect(shared, isEmpty, reason: 'the button only asks');
    });

    testWidgets('is shared with control when that is chosen', (tester) async {
      await show(tester, ControlAccess.allowed);

      await choose(tester, 'Share a window or screen to look at and control');

      expect(shared, [('Phone', true)]);
    });

    testWidgets('is shared to be looked at when that is chosen', (
      tester,
    ) async {
      await show(tester, ControlAccess.allowed);

      await choose(tester, 'Share to look at');

      expect(shared, [('Phone', false)]);
    });

    testWidgets(
      'asks the system, and does not share, when it has not said yes',
      (tester) async {
        final keeper = await show(tester, ControlAccess.needed);

        await choose(tester, 'Share a window or screen to look at and control');

        expect(shared, isEmpty);
        expect(keeper.requests, 1);
        expect(
          find.textContaining(
            'To let Phone control a window or a screen, allow Baton to control '
            'this computer in System Settings, under Privacy & Security, '
            'Accessibility. If Baton is in the list already, take it out and '
            'add it again. Then share it again.',
          ),
          findsOneWidget,
        );
      },
    );

    testWidgets('can still be shared to be looked at when it has not', (
      tester,
    ) async {
      final keeper = await show(tester, ControlAccess.needed);

      await choose(tester, 'Share to look at');

      expect(shared, [('Phone', false)]);
      expect(keeper.requests, 0);
    });

    testWidgets(
      'is asked about again when it is chosen, not when it was shown',
      (tester) async {
        final keeper = await show(tester, ControlAccess.allowed);
        keeper.access = ControlAccess.needed;

        await choose(tester, 'Share a window or screen to look at and control');

        expect(shared, isEmpty);
        expect(keeper.requests, 1);
      },
    );

    testWidgets('says that the system is too old when it is', (tester) async {
      final keeper = await show(tester, ControlAccess.allowed);
      keeper.access = ControlAccess.unavailable;

      await choose(tester, 'Share a window or screen to look at and control');

      expect(shared, isEmpty);
      expect(keeper.requests, 0);
      expect(
        find.text(
          'Controlling a window or a screen needs macOS 15.2 or later.',
        ),
        findsOneWidget,
      );
    });

    testWidgets('is not offered when the system cannot do it', (tester) async {
      await show(tester, ControlAccess.unavailable);

      await tester.tap(find.byTooltip('Share a window or screen with Phone'));
      await tester.pumpAndSettle();

      expect(find.text('Share to look at'), findsNothing);
      expect(shared, [('Phone', false)]);
    });

    testWidgets('is offered once the person has said yes and comes back', (
      tester,
    ) async {
      final keeper = await show(tester, ControlAccess.unavailable);
      keeper.access = ControlAccess.allowed;

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Share a window or screen with Phone'));
      await tester.pumpAndSettle();

      expect(find.text('Share to look at'), findsOneWidget);
    });

    testWidgets('is not asked about again once the screen is gone', (
      tester,
    ) async {
      final keeper = await show(tester, ControlAccess.allowed);
      final asked = keeper.asked;
      await tester.pumpWidget(const SizedBox());

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();

      expect(asked, greaterThan(0));
      expect(keeper.asked, asked);
    });

    testWidgets(
      'names the device as fit to show when the system must be asked',
      (tester) async {
        const name = 'Pad\u202Eexe';
        await show(
          tester,
          ControlAccess.needed,
          devices: [_device(name, Platform.android)],
        );

        await choose(
          tester,
          'Share a window or screen to look at and control',
          device: name,
        );

        expect(
          find.textContaining(
            'To let ${visibleText(name)} control a window or a screen',
          ),
          findsOneWidget,
        );
        expect(visibleText(name), isNot(name));
      },
    );

    testWidgets('says it for the device that was chosen last, and only that', (
      tester,
    ) async {
      await show(
        tester,
        ControlAccess.needed,
        devices: [phone, _device('Tablet', Platform.android)],
      );

      // The second device lies under the note about the first, so the order is the other way.
      await choose(
        tester,
        'Share a window or screen to look at and control',
        device: 'Tablet',
      );
      await choose(tester, 'Share a window or screen to look at and control');

      expect(
        find.textContaining('To let Phone control a window or a screen'),
        findsOneWidget,
      );
      expect(
        find.textContaining('To let Tablet control a window or a screen'),
        findsNothing,
      );
    });

    testWidgets('is not asked about on a device that shares no window', (
      tester,
    ) async {
      final keeper = FakeControlAccess();
      await _show(tester, _session(devices: [phone]), controlAccess: keeper);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pumpAndSettle();

      expect(keeper.asked, 0);
      expect(
        find.byTooltip('Share a window or screen with Phone'),
        findsNothing,
      );
    });
  });
}
