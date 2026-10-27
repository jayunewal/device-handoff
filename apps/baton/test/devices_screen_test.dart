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
  void Function(Device device, {required bool control})? onStartScreen,
  bool canGiveScreen = false,
  ControlAccessKeeper? controlAccess,
  ScreenAccessKeeper? screenAccess,
}) => MaterialApp(
  home: DevicesScreen(
    session: session,
    now: () => _now,
    // No camera here: the pairing screens get a stand-in.
    scanner: (context, onCode) => const Text('(camera)'),
    onShareWindow: onShareWindow,
    onStartScreen: onStartScreen,
    canGiveScreen: canGiveScreen,
    controlAccess:
        controlAccess ?? FakeControlAccess(ControlAccess.unavailable),
    screenAccess: screenAccess ?? FakeScreenAccess(ScreenAccess.unavailable),
  ),
);

Future<void> _show(
  WidgetTester tester,
  FakeSession session, {
  void Function(Device device, {required bool control})? onShareWindow,
  void Function(Device device, {required bool control})? onStartScreen,
  bool canGiveScreen = false,
  ControlAccessKeeper? controlAccess,
  ScreenAccessKeeper? screenAccess,
}) async {
  await tester.pumpWidget(
    _screen(
      session,
      onShareWindow: onShareWindow,
      onStartScreen: onStartScreen,
      canGiveScreen: canGiveScreen,
      controlAccess: controlAccess,
      screenAccess: screenAccess,
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

  group('a screen that a device is asked for', () {
    final mac = _device('Anna’s Mac', Platform.macOs);
    final asked = <(String, bool)>[];

    Future<void> show(
      WidgetTester tester, {
      List<Device>? devices,
      bool canAsk = true,
    }) async {
      asked.clear();
      await _show(
        tester,
        _session(devices: devices ?? [mac]),
        onStartScreen: canAsk
            ? (device, {required control}) => asked.add((device.name, control))
            : null,
      );
    }

    testWidgets('is a button on a Mac, which only asks', (tester) async {
      await show(tester);

      await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
      await tester.pumpAndSettle();

      expect(find.text('Look at its screen'), findsOneWidget);
      expect(find.text('Look at its screen and control it'), findsOneWidget);
      expect(asked, isEmpty);
    });

    testWidgets('is asked for to look at when that is chosen', (tester) async {
      await show(tester);

      await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Look at its screen'));
      await tester.pumpAndSettle();

      expect(asked, [('Anna’s Mac', false)]);
    });

    testWidgets('is asked for to look at and control when that is chosen', (
      tester,
    ) async {
      await show(tester);

      await tester.tap(find.byTooltip('Look at the screen of Anna’s Mac'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Look at its screen and control it'));
      await tester.pumpAndSettle();

      expect(asked, [('Anna’s Mac', true)]);
    });

    testWidgets('is not offered for a device that gives no screen', (
      tester,
    ) async {
      await show(
        tester,
        devices: [
          _device('Kitchen iPad', Platform.ipadOs),
          _device('Phone', Platform.android),
          _device('Laptop', Platform.windows),
        ],
      );

      expect(find.byIcon(Icons.visibility_outlined), findsNothing);
      expect(
        find.byTooltip('Look at the screen of Kitchen iPad'),
        findsNothing,
      );
      expect(find.byTooltip('Look at the screen of Phone'), findsNothing);
      expect(find.byTooltip('Look at the screen of Laptop'), findsNothing);
    });

    testWidgets('is not offered where screens are not shown', (tester) async {
      await show(tester, canAsk: false);

      expect(find.byTooltip('Look at the screen of Anna’s Mac'), findsNothing);
      expect(find.byTooltip('Send to Anna’s Mac'), findsOneWidget);
    });
  });

  group('what a device may do with the screen of this one', () {
    final phone = _device('Phone', Platform.ios);
    late FakeSession session;
    late FakeControlAccess keeper;
    late FakeScreenAccess recorder;

    const recordingSentence =
        'To let Phone ask for the screen, allow Baton to record it in System '
        'Settings, under Privacy & Security, Screen & System Audio Recording. '
        'Then quit Baton and open it again.';
    const controlSentence =
        'To let Phone control a window or a screen, allow Baton to control '
        'this computer in System Settings, under Privacy & Security, '
        'Accessibility. If Baton is in the list already, take it out and add '
        'it again.';

    Future<void> show(
      WidgetTester tester, {
      ControlAccess access = ControlAccess.allowed,
      ScreenAccess recording = ScreenAccess.allowed,
      bool canGiveScreen = true,
    }) async {
      session = _session(devices: [phone]);
      keeper = FakeControlAccess(access);
      recorder = FakeScreenAccess(recording);
      await _show(
        tester,
        session,
        onShareWindow: (device, {required control}) {},
        canGiveScreen: canGiveScreen,
        controlAccess: keeper,
        screenAccess: recorder,
      );
    }

    Future<void> open(WidgetTester tester) async {
      await tester.tap(find.byTooltip('More about Phone'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Let this device ask for the screen…'));
      await tester.pumpAndSettle();
    }

    Future<void> choose(WidgetTester tester, String choice) async {
      await tester.tap(find.text(choice));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
    }

    StartAccess? selected(WidgetTester tester) => tester
        .widget<RadioGroup<StartAccess>>(find.byType(RadioGroup<StartAccess>))
        .groupValue;

    testWidgets(
      'is a choice in the menu of a device, where the screen can be given',
      (tester) async {
        await show(tester);

        await tester.tap(find.byTooltip('More about Phone'));
        await tester.pumpAndSettle();

        expect(
          find.text('Let this device ask for the screen…'),
          findsOneWidget,
        );
        expect(find.text('Forget this device'), findsOneWidget);
      },
    );

    testWidgets('is not a choice where no screen can be given', (tester) async {
      await show(tester, canGiveScreen: false);

      await tester.tap(find.byTooltip('More about Phone'));
      await tester.pumpAndSettle();

      expect(find.text('Let this device ask for the screen…'), findsNothing);
      expect(find.text('Forget this device'), findsOneWidget);
    });

    testWidgets('starts at off for a device that was just paired', (
      tester,
    ) async {
      await show(tester);

      await open(tester);

      expect(
        find.text('When Phone asks for the screen of this Mac'),
        findsOneWidget,
      );
      expect(selected(tester), StartAccess.off);
      expect(find.text('Do not show it'), findsOneWidget);
      expect(find.text('Show it to look at'), findsOneWidget);
      expect(find.text('Show it to look at and control'), findsOneWidget);
    });

    testWidgets('starts at what was kept', (tester) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.look;

      await open(tester);

      expect(selected(tester), StartAccess.look);
    });

    testWidgets('is kept as the person chose it', (tester) async {
      await show(tester);
      await open(tester);

      await choose(tester, 'Show it to look at');

      expect(session.startAccessSet, [
        (id: phone.id, access: StartAccess.look),
      ]);
      expect(find.text('Save'), findsNothing, reason: 'the dialog is closed');
      expect(keeper.requests, 0);
    });

    testWidgets('is kept as off when that is chosen', (tester) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.control;
      await open(tester);

      await choose(tester, 'Do not show it');

      expect(session.startAccessSet, [(id: phone.id, access: StartAccess.off)]);
    });

    testWidgets('is left alone when the person cancels', (tester) async {
      await show(tester);
      await open(tester);

      await tester.tap(find.text('Show it to look at'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      expect(session.startAccessSet, isEmpty);
      expect(find.text('Save'), findsNothing);
    });

    testWidgets('is not kept again when nothing was changed', (tester) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.look;
      await open(tester);

      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();

      expect(session.startAccessSet, isEmpty);
    });

    final hearing = find.byType(SwitchListTile);

    bool hears(WidgetTester tester) =>
        tester.widget<SwitchListTile>(hearing).value;

    Future<void> flip(WidgetTester tester) async {
      await tester.tap(hearing);
      await tester.pumpAndSettle();
    }

    Future<void> save(WidgetTester tester) async {
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
    }

    testWidgets('shows the sound as on only where the device may control', (
      tester,
    ) async {
      await show(tester);
      for (final (access, heard) in [
        (StartAccess.off, false),
        (StartAccess.look, false),
        (StartAccess.control, true),
      ]) {
        session.startAccesses[phone.id] = access;
        await open(tester);
        expect(find.text('Let Phone hear the sound'), findsOneWidget);
        expect(hears(tester), heard, reason: '$access');
        await tester.tap(find.text('Cancel'));
        await tester.pumpAndSettle();
      }
    });

    testWidgets(
      'shows what was said of the sound, whatever the device may do',
      (tester) async {
        await show(tester);
        session.startAccesses[phone.id] = StartAccess.look;
        session.hearSettings[phone.id] = true;
        await open(tester);
        expect(hears(tester), isTrue);
        await tester.tap(find.text('Cancel'));
        await tester.pumpAndSettle();

        session.startAccesses[phone.id] = StartAccess.control;
        session.hearSettings[phone.id] = false;
        await open(tester);
        expect(hears(tester), isFalse);
      },
    );

    testWidgets('says that the sound goes by the session until something is '
        'said of it', (tester) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.look;
      await open(tester);

      // Nothing was said: the device hears where it controls, not where it only looks.
      final byTheSession = find.textContaining(
        'Left as it is, Phone hears when it controls this',
      );
      expect(byTheSession, findsOneWidget);
      expect(find.textContaining('every session'), findsNothing);

      // A yes is for every session, and moving the switch back leaves it to the session.
      await flip(tester);
      expect(
        find.textContaining('Phone hears in every session, also when it only '),
        findsOneWidget,
      );
      expect(byTheSession, findsNothing);
      await flip(tester);
      expect(byTheSession, findsOneWidget);
    });

    testWidgets('says what was said of the sound, a yes and a no', (
      tester,
    ) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.control;
      session.hearSettings[phone.id] = true;
      await open(tester);
      expect(
        find.textContaining('Phone hears in every session, also when it only '),
        findsOneWidget,
      );
      expect(find.textContaining('Left as it is'), findsNothing);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      session.hearSettings[phone.id] = false;
      await open(tester);
      expect(
        find.textContaining('Phone gets the screen with no sound, and this'),
        findsOneWidget,
      );
      expect(find.textContaining('every session'), findsNothing);
      expect(find.textContaining('Left as it is'), findsNothing);
    });

    testWidgets('follows the choice of the screen until the person moves the '
        'switch', (tester) async {
      await show(tester);
      await open(tester);

      await tester.tap(find.text('Show it to look at and control'));
      await tester.pumpAndSettle();
      expect(hears(tester), isTrue);
      await tester.tap(find.text('Show it to look at'));
      await tester.pumpAndSettle();
      expect(hears(tester), isFalse);

      await flip(tester);
      expect(hears(tester), isTrue);
      await tester.tap(find.text('Show it to look at and control'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Do not show it'));
      await tester.pumpAndSettle();
      expect(hears(tester), isTrue, reason: 'the person said so');
    });

    testWidgets('keeps that a device that can only look may hear', (
      tester,
    ) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.look;
      await open(tester);

      await flip(tester);
      await save(tester);

      expect(session.hearSettingSet, [(id: phone.id, setting: true)]);
      expect(
        session.startAccessSet,
        isEmpty,
        reason: 'the choice is as it was',
      );
      expect(find.text('Save'), findsNothing, reason: 'the dialog is closed');
    });

    testWidgets('keeps that a device that may control may not hear', (
      tester,
    ) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.control;
      await open(tester);

      await flip(tester);
      await save(tester);

      expect(session.hearSettingSet, [(id: phone.id, setting: false)]);
      expect(session.startAccessSet, isEmpty);
    });

    testWidgets('keeps nothing of the sound when it is left to the choice', (
      tester,
    ) async {
      await show(tester);
      await open(tester);

      await choose(tester, 'Show it to look at and control');

      expect(session.startAccessSet, [
        (id: phone.id, access: StartAccess.control),
      ]);
      expect(session.hearSettingSet, isEmpty);
    });

    testWidgets(
      'keeps nothing when the switch is moved back to what the device '
      'gets anyway',
      (tester) async {
        await show(tester);
        session.startAccesses[phone.id] = StartAccess.look;
        await open(tester);

        await flip(tester);
        await flip(tester);
        await save(tester);

        expect(session.hearSettingSet, isEmpty);
        expect(session.startAccessSet, isEmpty);
      },
    );

    testWidgets('takes back what was said when it is moved to what the device '
        'gets anyway', (tester) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.look;
      session.hearSettings[phone.id] = true;
      await open(tester);

      await flip(tester);
      await save(tester);

      expect(session.hearSettingSet, [(id: phone.id, setting: null)]);
      expect(session.hearSettings, isEmpty);
    });

    testWidgets('keeps nothing again when what was said is left as it was', (
      tester,
    ) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.control;
      session.hearSettings[phone.id] = true;
      await open(tester);

      await save(tester);

      expect(session.hearSettingSet, isEmpty);
      expect(session.startAccessSet, isEmpty);
    });

    testWidgets('keeps the choice of the screen and the sound together', (
      tester,
    ) async {
      await show(tester);
      await open(tester);

      await tester.tap(find.text('Show it to look at'));
      await tester.pumpAndSettle();
      await flip(tester);
      await save(tester);

      expect(session.startAccessSet, [
        (id: phone.id, access: StartAccess.look),
      ]);
      expect(session.hearSettingSet, [(id: phone.id, setting: true)]);
    });

    testWidgets('says why the sound could not be kept', (tester) async {
      await show(tester);
      session.startAccesses[phone.id] = StartAccess.look;
      session.hearSettingFailure = const BatonError(
        kind: ErrorKind.storage,
        message: 'Baton could not keep the setting.',
      );
      await open(tester);

      await flip(tester);
      await save(tester);

      expect(find.text('Baton could not keep the setting.'), findsOneWidget);
    });

    testWidgets(
      'lets the device control the screen when the system allows it',
      (tester) async {
        await show(tester);
        await open(tester);

        await choose(tester, 'Show it to look at and control');

        expect(session.startAccessSet, [
          (id: phone.id, access: StartAccess.control),
        ]);
        expect(keeper.requests, 0);
        expect(find.textContaining('To let Phone control'), findsNothing);
      },
    );

    testWidgets(
      'asks the system, once it is kept, when control has not been allowed',
      (tester) async {
        await show(tester, access: ControlAccess.needed);
        await open(tester);

        await choose(tester, 'Show it to look at and control');

        expect(session.startAccessSet, [
          (id: phone.id, access: StartAccess.control),
        ]);
        expect(keeper.requests, 1);
        expect(find.text(controlSentence), findsOneWidget);
        expect(recorder.requests, 0, reason: 'the screen can be recorded');
      },
    );

    testWidgets('does not ask the system for the device that may only look', (
      tester,
    ) async {
      await show(tester, access: ControlAccess.needed);
      await open(tester);

      await choose(tester, 'Show it to look at');

      expect(keeper.requests, 0);
    });

    testWidgets(
      'asks the system to record the screen, once it is kept, when that has '
      'not been allowed',
      (tester) async {
        await show(tester, recording: ScreenAccess.needed);
        await open(tester);

        await choose(tester, 'Show it to look at');

        expect(session.startAccessSet, [
          (id: phone.id, access: StartAccess.look),
        ]);
        expect(recorder.requests, 1);
        expect(keeper.requests, 0);
        expect(find.text(recordingSentence), findsOneWidget);
      },
    );

    testWidgets('asks the system to record the screen for a device that may '
        'control it as well', (tester) async {
      await show(tester, recording: ScreenAccess.needed);
      await open(tester);

      await choose(tester, 'Show it to look at and control');

      expect(recorder.requests, 1);
      expect(keeper.requests, 0, reason: 'control was allowed');
      expect(find.text(recordingSentence), findsOneWidget);
      expect(find.text(controlSentence), findsNothing);
    });

    testWidgets(
      'asks for both consents, and says both, when neither is given',
      (tester) async {
        await show(
          tester,
          access: ControlAccess.needed,
          recording: ScreenAccess.needed,
        );
        await open(tester);

        await choose(tester, 'Show it to look at and control');

        expect(recorder.requests, 1);
        expect(keeper.requests, 1);
        // One notice says both, since a second would take the first off the screen.
        expect(
          find.text('$recordingSentence $controlSentence'),
          findsOneWidget,
        );
      },
    );

    testWidgets('asks nothing of the system when the screen can be recorded', (
      tester,
    ) async {
      await show(tester);
      await open(tester);

      await choose(tester, 'Show it to look at');

      expect(session.startAccessSet, isNotEmpty);
      expect(recorder.requests, 0);
      expect(
        find.textContaining('Screen & System Audio Recording'),
        findsNothing,
      );
    });

    testWidgets('asks the system when the setting is saved, not before', (
      tester,
    ) async {
      await show(tester);
      await open(tester);
      // The person took the consent back after the screen was shown.
      recorder.access = ScreenAccess.needed;

      await choose(tester, 'Show it to look at');

      expect(recorder.requests, 1);
    });

    testWidgets(
      'does not ask to record the screen when nothing is to be shown',
      (tester) async {
        await show(tester, recording: ScreenAccess.needed);
        session.startAccesses[phone.id] = StartAccess.look;
        await open(tester);

        await choose(tester, 'Do not show it');

        expect(session.startAccessSet, [
          (id: phone.id, access: StartAccess.off),
        ]);
        expect(recorder.asked, 0);
        expect(recorder.requests, 0);
      },
    );

    testWidgets(
      'does not ask to record a screen that this device cannot give',
      (tester) async {
        await show(tester, recording: ScreenAccess.unavailable);
        await open(tester);

        await choose(tester, 'Show it to look at');

        expect(session.startAccessSet, isNotEmpty);
        expect(recorder.requests, 0);
        expect(
          find.textContaining('Screen & System Audio Recording'),
          findsNothing,
        );
      },
    );

    testWidgets('says in the dialog that saving asks to record the screen', (
      tester,
    ) async {
      await show(tester);

      await open(tester);

      expect(
        find.textContaining(
          'when you save, this Mac asks for that if it has not been given',
        ),
        findsOneWidget,
      );
    });

    testWidgets('leaves control out where the system cannot do it', (
      tester,
    ) async {
      await show(tester, access: ControlAccess.unavailable);

      await open(tester);

      expect(find.text('Show it to look at'), findsOneWidget);
      expect(find.text('Show it to look at and control'), findsNothing);
    });

    testWidgets('keeps control in the dialog where it was allowed before', (
      tester,
    ) async {
      await show(tester, access: ControlAccess.unavailable);
      session.startAccesses[phone.id] = StartAccess.control;

      await open(tester);

      expect(find.text('Show it to look at and control'), findsOneWidget);
      expect(selected(tester), StartAccess.control);
    });

    testWidgets('says why it could not be kept', (tester) async {
      await show(tester);
      session.startAccessFailure = const BatonError(
        kind: ErrorKind.storage,
        message: 'Baton could not keep the setting.',
      );
      await open(tester);

      await choose(tester, 'Show it to look at');

      expect(find.text('Baton could not keep the setting.'), findsOneWidget);
      expect(keeper.requests, 0);
    });

    testWidgets(
      'asks for no consent to control when the setting could not be kept',
      (tester) async {
        await show(
          tester,
          access: ControlAccess.needed,
          recording: ScreenAccess.needed,
        );
        session.startAccessFailure = const BatonError(
          kind: ErrorKind.storage,
          message: 'Baton could not keep the setting.',
        );
        await open(tester);

        await choose(tester, 'Show it to look at and control');

        expect(find.text('Baton could not keep the setting.'), findsOneWidget);
        expect(keeper.requests, 0);
        expect(recorder.asked, 0);
        expect(recorder.requests, 0);
      },
    );

    testWidgets('shows the choice that the person made until it is kept', (
      tester,
    ) async {
      await show(tester);
      await open(tester);
      expect(selected(tester), StartAccess.off);

      await tester.tap(find.text('Show it to look at'));
      await tester.pumpAndSettle();
      expect(selected(tester), StartAccess.look);

      await tester.tap(find.text('Show it to look at and control'));
      await tester.pumpAndSettle();
      expect(selected(tester), StartAccess.control);
      expect(session.startAccessSet, isEmpty);
    });

    testWidgets('asks the system about control where no window is shared', (
      tester,
    ) async {
      session = _session(devices: [phone]);
      keeper = FakeControlAccess(ControlAccess.allowed);
      await _show(tester, session, canGiveScreen: true, controlAccess: keeper);

      await open(tester);

      // The screen did not start from "unavailable", which is what it is before the system has
      // answered: it asked.
      expect(find.text('Show it to look at'), findsOneWidget);
      expect(find.text('Show it to look at and control'), findsOneWidget);
    });

    testWidgets('opens nothing for a device that was forgotten meanwhile', (
      tester,
    ) async {
      await show(tester);
      session.startAccessUnknown = const BatonError(
        kind: ErrorKind.notPaired,
        message: 'Phone is not paired with this device.',
      );

      await tester.tap(find.byTooltip('More about Phone'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Let this device ask for the screen…'));
      await tester.pumpAndSettle();

      expect(find.text('Save'), findsNothing);
      expect(session.startAccessSet, isEmpty);
    });

    testWidgets('names the device as fit to show', (tester) async {
      const name = 'Pad\u202Eexe';
      session = _session(devices: [_device(name, Platform.ios)]);
      await _show(
        tester,
        session,
        onShareWindow: (device, {required control}) {},
        canGiveScreen: true,
        controlAccess: FakeControlAccess(),
      );

      await tester.tap(find.byTooltip('More about $name'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Let this device ask for the screen…'));
      await tester.pumpAndSettle();

      expect(
        find.text('When ${visibleText(name)} asks for the screen of this Mac'),
        findsOneWidget,
      );
      expect(visibleText(name), isNot(name));
    });

    testWidgets('names the device as fit to show in the notice', (
      tester,
    ) async {
      const name = 'Pad\u202Eexe';
      session = _session(devices: [_device(name, Platform.ios)]);
      await _show(
        tester,
        session,
        onShareWindow: (device, {required control}) {},
        canGiveScreen: true,
        controlAccess: FakeControlAccess(),
        screenAccess: FakeScreenAccess(ScreenAccess.needed),
      );
      await tester.tap(find.byTooltip('More about $name'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Let this device ask for the screen…'));
      await tester.pumpAndSettle();

      await choose(tester, 'Show it to look at');

      expect(
        find.textContaining('To let ${visibleText(name)} ask for the screen'),
        findsOneWidget,
      );
      expect(visibleText(name), isNot(name));
    });
  });
}
