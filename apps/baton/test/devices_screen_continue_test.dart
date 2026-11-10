// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/continue_reader.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/screens/devices_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_continue_reader.dart';
import 'fake_control_access.dart';
import 'fake_notices.dart';

final _now = DateTime(2026, 10, 15, 12);

Device _device(String name, Platform platform) => Device(
  id: name.padRight(64, '0'),
  name: name,
  platform: platform,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: _now.millisecondsSinceEpoch ~/ 1000 - 3 * 24 * 3600,
  lastSeenAt: null,
);

FakeSession _session(
  List<Device> devices, {
  Platform thisDevice = Platform.macOs,
}) => FakeSession(
  me: Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: thisDevice,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: devices,
);

Future<void> _show(
  WidgetTester tester,
  FakeSession session, {
  FakeContinueReader? reader,
  List<String>? opened,
}) async {
  await tester.pumpWidget(
    MaterialApp(
      home: DevicesScreen(
        session: session,
        now: () => _now,
        scanner: (context, onCode) => const Text('(camera)'),
        controlAccess: FakeControlAccess(ControlAccess.unavailable),
        screenAccess: FakeScreenAccess(ScreenAccess.unavailable),
        notices: FakeNotices(NoticeAccess.unavailable),
        continueReader: reader ?? FakeContinueReader(),
        openLink: (address) async {
          opened?.add(address);
          return true;
        },
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  group('continuing from a device', () {
    final mac = _device('Anna’s Mac', Platform.macOs);

    testWidgets('is a button on a Mac, and on no other device', (tester) async {
      final session = _session([
        mac,
        _device('Phone', Platform.ios),
        _device('Kitchen iPad', Platform.ipadOs),
        _device('Laptop', Platform.windows),
        _device('Handset', Platform.android),
      ], thisDevice: Platform.ios);

      await _show(tester, session);

      expect(find.byTooltip('Continue from Anna’s Mac'), findsOneWidget);
      for (final name in ['Phone', 'Kitchen iPad', 'Laptop', 'Handset']) {
        expect(
          find.byTooltip('Continue from $name'),
          findsNothing,
          reason: name,
        );
      }
    });

    testWidgets('asks the Mac what is open, and shows it in a sheet', (
      tester,
    ) async {
      final session = _session([mac], thisDevice: Platform.ios)
        ..continueOutcome = const ContinueOutcome.items(
          items: ContinueItems(
            page: ContinuePage(
              url: 'https://example.com/report',
              title: 'The report',
            ),
            clipboard: 'hello',
          ),
        );
      await _show(tester, session);

      await tester.tap(find.byTooltip('Continue from Anna’s Mac'));
      await tester.pumpAndSettle();

      expect(session.continueAsked, [mac.id]);
      expect(find.text('Continue from Anna’s Mac'), findsOneWidget);
      expect(find.text('The report'), findsOneWidget);
      expect(find.text('hello'), findsOneWidget);
    });

    testWidgets('opens the page with the system that the screen was given', (
      tester,
    ) async {
      final opened = <String>[];
      final session = _session([mac], thisDevice: Platform.ios)
        ..continueOutcome = const ContinueOutcome.items(
          items: ContinueItems(
            page: ContinuePage(url: 'https://example.com/report'),
          ),
        );
      await _show(tester, session, opened: opened);

      await tester.tap(find.byTooltip('Continue from Anna’s Mac'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();

      expect(opened, ['https://example.com/report']);
    });

    testWidgets('asks nothing of a device until the person taps', (
      tester,
    ) async {
      final session = _session([mac], thisDevice: Platform.ios);

      await _show(tester, session);

      expect(session.continueAsked, isEmpty);
    });

    testWidgets('is a button on a Mac for another Mac too', (tester) async {
      final session = _session([mac]);

      await _show(tester, session);

      expect(find.byTooltip('Continue from Anna’s Mac'), findsOneWidget);
    });
  });

  group('what a device may do with what this Mac is working on', () {
    final phone = _device('Anna’s iPhone', Platform.ios);
    const entry = 'Let this device continue from this Mac…';
    late FakeSession session;
    late FakeContinueReader reader;

    Future<void> show(
      WidgetTester tester, {
      Platform thisDevice = Platform.macOs,
    }) async {
      session = _session([phone], thisDevice: thisDevice);
      reader = FakeContinueReader();
      await _show(tester, session, reader: reader);
    }

    Future<void> open(WidgetTester tester) async {
      await tester.tap(find.byTooltip('More about Anna’s iPhone'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(entry));
      await tester.pumpAndSettle();
    }

    Future<void> save(WidgetTester tester, {bool toggle = false}) async {
      if (toggle) {
        await tester.tap(find.byType(Switch));
        await tester.pumpAndSettle();
      }
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
    }

    bool isOn(WidgetTester tester) =>
        tester.widget<SwitchListTile>(find.byType(SwitchListTile)).value;

    testWidgets('is a choice in the menu of a device on a Mac', (tester) async {
      await show(tester);

      await tester.tap(find.byTooltip('More about Anna’s iPhone'));
      await tester.pumpAndSettle();

      expect(find.text(entry), findsOneWidget);
    });

    testWidgets('is not a choice on a device that cannot read it', (
      tester,
    ) async {
      await show(tester, thisDevice: Platform.ios);

      await tester.tap(find.byTooltip('More about Anna’s iPhone'));
      await tester.pumpAndSettle();

      expect(find.textContaining('continue from this'), findsNothing);
      expect(find.text('Forget this device'), findsOneWidget);
    });

    testWidgets('starts at off for a device that was just paired', (
      tester,
    ) async {
      await show(tester);
      await open(tester);

      expect(
        find.text('When Anna’s iPhone asks what you are working on'),
        findsOneWidget,
      );
      expect(isOn(tester), isFalse);
    });

    testWidgets('starts at what was kept', (tester) async {
      await show(tester);
      session.continueAccesses[phone.id] = true;
      await open(tester);

      expect(isOn(tester), isTrue);
    });

    testWidgets('is kept as on when the person turns it on and saves', (
      tester,
    ) async {
      await show(tester);
      await open(tester);

      await save(tester, toggle: true);

      expect(session.continueAccessSet, [(id: phone.id, allowed: true)]);
      expect(session.continueAccesses[phone.id], isTrue);
    });

    testWidgets('is kept as off when the person turns it off and saves', (
      tester,
    ) async {
      await show(tester);
      session.continueAccesses[phone.id] = true;
      await open(tester);

      await save(tester, toggle: true);

      expect(session.continueAccessSet, [(id: phone.id, allowed: false)]);
      expect(session.continueAccesses[phone.id], isFalse);
    });

    testWidgets('is left alone when the person cancels', (tester) async {
      await show(tester);
      await open(tester);
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      expect(session.continueAccessSet, isEmpty);
      expect(reader.accessAsked, isEmpty);
    });

    testWidgets('is not kept again when nothing was changed', (tester) async {
      await show(tester);
      session.continueAccesses[phone.id] = true;
      await open(tester);

      await save(tester);

      expect(session.continueAccessSet, isEmpty);
    });

    testWidgets('asks the system about each browser once it is on, to be '
        'answered while the person is here', (tester) async {
      await show(tester);
      await open(tester);

      await save(tester, toggle: true);

      expect(reader.accessAsked, [
        (browser: Browser.safari, ask: true),
        (browser: Browser.chrome, ask: true),
      ]);
    });

    testWidgets('asks the system again when it is saved as on again, which '
        'is how the person asks after a no', (tester) async {
      await show(tester);
      session.continueAccesses[phone.id] = true;
      await open(tester);

      await save(tester);

      expect(session.continueAccessSet, isEmpty);
      expect(reader.accessAsked, hasLength(2));
    });

    testWidgets('asks the system about nothing when it is turned off', (
      tester,
    ) async {
      await show(tester);
      session.continueAccesses[phone.id] = true;
      await open(tester);

      await save(tester, toggle: true);

      expect(reader.accessAsked, isEmpty);
    });

    testWidgets('says that the device can ask, and no more when a browser '
        'can be read', (tester) async {
      await show(tester);
      reader.access[Browser.safari] = BrowserAccess.needed;
      await open(tester);

      await save(tester, toggle: true);

      // The system asked, and the person said yes.
      expect(reader.access[Browser.safari], BrowserAccess.allowed);
      expect(
        find.text(
          'Anna’s iPhone can now ask this Mac what you are working on.',
        ),
        findsOneWidget,
      );
      expect(find.text('OK'), findsNothing);
    });

    testWidgets('says what to do where the person said no to a browser, and '
        'stays until they have read it', (tester) async {
      await show(tester);
      reader.access[Browser.safari] = BrowserAccess.needed;
      reader.answers[Browser.safari] = BrowserAccess.denied;
      await open(tester);

      await save(tester, toggle: true);

      expect(
        find.textContaining(
          'Anna’s iPhone can now ask this Mac what you are working on. macOS '
          'says that Baton may not control Safari',
        ),
        findsOneWidget,
      );
      expect(find.text('OK'), findsOneWidget);
    });

    testWidgets('says that the system has not asked yet when no browser is '
        'open', (tester) async {
      await show(tester);
      reader.access[Browser.safari] = BrowserAccess.notOpen;
      reader.access[Browser.chrome] = BrowserAccess.notOpen;
      await open(tester);

      await save(tester, toggle: true);

      expect(
        find.textContaining(
          'Safari and Chrome are not open, so macOS has not '
          'asked about them yet.',
        ),
        findsOneWidget,
      );
    });

    testWidgets('says why it could not be kept, and asks the system about '
        'nothing', (tester) async {
      await show(tester);
      session.continueAccessFailure = const BatonError(
        kind: ErrorKind.storage,
        message: 'Baton could not keep the setting.',
      );
      await open(tester);

      await save(tester, toggle: true);

      expect(find.text('Baton could not keep the setting.'), findsOneWidget);
      expect(reader.accessAsked, isEmpty);
    });

    testWidgets('is not asked of a device that is not paired any more', (
      tester,
    ) async {
      await show(tester);
      session.startAccessUnknown = const BatonError(
        kind: ErrorKind.notPaired,
        message: 'not paired',
      );

      await tester.tap(find.byTooltip('More about Anna’s iPhone'));
      await tester.pumpAndSettle();
      await tester.tap(find.text(entry));
      await tester.pumpAndSettle();

      // Nothing to choose: the dialog does not open.
      expect(find.byType(SwitchListTile), findsNothing);
    });

    testWidgets('says what the device is given, and what is left out', (
      tester,
    ) async {
      await show(tester);
      await open(tester);

      expect(
        find.textContaining(
          'the address and title of the page that is in '
          'front in Safari or Chrome, and the text on the clipboard',
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining('Nothing is kept, and no one here is asked.'),
        findsOneWidget,
      );
      expect(
        find.textContaining('locked or its screen is off'),
        findsOneWidget,
      );
      // Each paragraph starts as a sentence does, also the one that names this Mac.
      final limits = tester.widget<Text>(
        find.textContaining('locked or its screen is off'),
      );
      for (final paragraph in limits.data!.split('\n\n')) {
        expect(paragraph, matches(RegExp(r'^[A-Z]')), reason: paragraph);
      }
      expect(find.textContaining('private windows of Chrome'), findsOneWidget);
      expect(
        find.textContaining('It cannot promise that for Safari'),
        findsOneWidget,
      );
      expect(
        find.textContaining(
          'macOS asks whether Baton may control Safari and Chrome',
        ),
        findsOneWidget,
      );
    });
  });
}
