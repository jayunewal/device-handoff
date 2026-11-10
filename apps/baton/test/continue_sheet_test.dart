// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/continue_controller.dart';
import 'package:baton/src/screens/continue_sheet.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_clipboard.dart';

const _mac = Device(
  id: 'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1',
  name: 'Anna’s Mac',
  platform: Platform.macOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1790000000,
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Phone',
    platform: Platform.ios,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: const [_mac],
);

const _report = ContinueOutcome.items(
  items: ContinueItems(
    page: ContinuePage(url: 'https://example.com/report', title: 'The report'),
    clipboard: 'hello there',
  ),
);

/// Shows the sheet for [_mac] on [session], which opens addresses with [opened].
Future<void> _show(
  WidgetTester tester,
  FakeSession session, {
  List<String>? opened,
  bool opens = true,
}) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: ContinueSheet(
          // A sheet of its own for each session: one that is already there would keep its first.
          key: ObjectKey(session),
          session: session,
          device: _mac,
          openLink: (address) async {
            opened?.add(address);
            return opens;
          },
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('asks the device when it opens, and says so while it waits', (
    tester,
  ) async {
    final session = _session()..holdContinue();
    session.continueOutcome = _report;

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: ContinueSheet(session: session, device: _mac),
        ),
      ),
    );
    await tester.pump();

    expect(find.text('Continue from Anna’s Mac'), findsOneWidget);
    expect(find.text('Asking Anna’s Mac…'), findsOneWidget);
    expect(session.continueAsked, [_mac.id]);

    session.releaseContinue();
    await tester.pumpAndSettle();

    expect(find.text('Asking Anna’s Mac…'), findsNothing);
    expect(find.text('The report'), findsOneWidget);
  });

  group('what the device has open', () {
    testWidgets('is the page, with its title, and the text', (tester) async {
      final session = _session()..continueOutcome = _report;

      await _show(tester, session);

      expect(find.text('Page in front'), findsOneWidget);
      expect(find.text('The report'), findsOneWidget);
      expect(find.text('https://example.com/report'), findsOneWidget);
      expect(find.text('On the clipboard'), findsOneWidget);
      expect(find.text('hello there'), findsOneWidget);
      expect(find.widgetWithText(FilledButton, 'Open'), findsOneWidget);
      expect(find.text('Copy address'), findsOneWidget);
      expect(find.widgetWithText(FilledButton, 'Copy'), findsOneWidget);
    });

    testWidgets('may be a page with no title, or text alone', (tester) async {
      final session = _session()
        ..continueOutcome = const ContinueOutcome.items(
          items: ContinueItems(page: ContinuePage(url: 'https://example.com/')),
        );
      await _show(tester, session);

      expect(find.text('https://example.com/'), findsOneWidget);
      expect(find.text('On the clipboard'), findsNothing);

      final text = _session()
        ..continueOutcome = const ContinueOutcome.items(
          items: ContinueItems(clipboard: 'just this'),
        );
      await _show(tester, text);

      expect(find.text('Page in front'), findsNothing);
      expect(find.text('just this'), findsOneWidget);
    });

    testWidgets('says that there is nothing to continue, and that it is not '
        'a failure', (tester) async {
      final session = _session();

      await _show(tester, session);

      expect(
        find.text(
          'Nothing to continue. Anna’s Mac is locked or asleep, or has no '
          'page open and nothing copied.',
        ),
        findsOneWidget,
      );
      expect(find.text('Try again'), findsNothing);
    });

    testWidgets('is shown with what cannot be seen or reads backwards made '
        'visible', (tester) async {
      final session = _session()
        ..continueOutcome = const ContinueOutcome.items(
          items: ContinueItems(
            page: ContinuePage(
              url: 'https://example.com/',
              title: 'Pay\u202Egpj.exe',
            ),
          ),
        );

      await _show(tester, session);

      expect(find.text('Pay�gpj.exe'), findsOneWidget);
    });
  });

  group('the page', () {
    testWidgets('is opened with the address that Baton checked, in one tap', (
      tester,
    ) async {
      final opened = <String>[];
      final session = _session()..continueOutcome = _report;
      await _show(tester, session, opened: opened);

      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();

      expect(opened, ['https://example.com/report']);
    });

    testWidgets('says so in the sheet when it could not be opened', (
      tester,
    ) async {
      final session = _session()..continueOutcome = _report;
      await _show(tester, session, opens: false);

      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();

      expect(
        find.text(
          'Baton could not open that page. You can copy its address instead.',
        ),
        findsOneWidget,
      );
    });

    testWidgets('is not opened when its address is not a web address, which '
        'is said', (tester) async {
      for (final address in [
        'file:///etc/passwd',
        'javascript:alert(1)',
        'ftp://example.com/x',
        'not an address',
      ]) {
        final opened = <String>[];
        final session = _session()
          ..continueOutcome = ContinueOutcome.items(
            items: ContinueItems(page: ContinuePage(url: address)),
          );
        await _show(tester, session, opened: opened);

        expect(find.text('Open'), findsNothing, reason: address);
        expect(
          find.text('This is not a web address, so Baton will not open it.'),
          findsOneWidget,
          reason: address,
        );
        expect(find.text('Copy address'), findsOneWidget, reason: address);
        expect(opened, isEmpty);
      }
    });

    testWidgets('has its address copied, as the device gave it', (
      tester,
    ) async {
      final clipboard = FakeClipboard(tester);
      final session = _session()..continueOutcome = _report;
      await _show(tester, session);

      await tester.tap(find.text('Copy address'));
      await tester.pumpAndSettle();

      expect(clipboard.copied, 'https://example.com/report');
      expect(find.text('Copied the address.'), findsOneWidget);
    });
  });

  group('the text', () {
    testWidgets('is copied in one tap, which the button says', (tester) async {
      final clipboard = FakeClipboard(tester);
      final session = _session()..continueOutcome = _report;
      await _show(tester, session);

      await tester.tap(find.widgetWithText(FilledButton, 'Copy'));
      await tester.pumpAndSettle();

      expect(clipboard.copied, 'hello there');
      expect(find.widgetWithText(FilledButton, 'Copied'), findsOneWidget);
    });

    testWidgets('is shown cut short where it is long, and copied whole', (
      tester,
    ) async {
      final clipboard = FakeClipboard(tester);
      final long = List.generate(200, (i) => 'line $i').join('\n');
      final session = _session()
        ..continueOutcome = ContinueOutcome.items(
          items: ContinueItems(clipboard: long),
        );
      await _show(tester, session);

      // The text is held whole, so that it can be copied, and set in six lines.
      final shown = tester.widget<Text>(find.textContaining('line 0'));
      expect(shown.maxLines, 6);
      expect(shown.overflow, TextOverflow.ellipsis);

      await tester.tap(find.text('Copy'));
      await tester.pumpAndSettle();

      expect(clipboard.copied, long);
    });

    testWidgets('is copied as it is, not as it is shown', (tester) async {
      final clipboard = FakeClipboard(tester);
      final session = _session()
        ..continueOutcome = const ContinueOutcome.items(
          items: ContinueItems(clipboard: 'a\r\nb\u202Ec'),
        );
      await _show(tester, session);

      await tester.tap(find.text('Copy'));
      await tester.pumpAndSettle();

      expect(clipboard.copied, 'a\r\nb\u202Ec');
    });
  });

  group('when the device gives nothing', () {
    testWidgets('says what the person at it does, if it was not allowed', (
      tester,
    ) async {
      final session = _session()
        ..continueOutcome = const ContinueOutcome.notAllowed();

      await _show(tester, session);

      expect(
        find.text(
          'Anna’s Mac has not let this device continue from it. At Anna’s '
          'Mac, open Baton, open the menu of this device in the list, choose '
          '“Let this device continue from this Mac” and turn it on.',
        ),
        findsOneWidget,
      );
      // Asking again changes nothing: only the person at the Mac can.
      expect(find.text('Try again'), findsNothing);
    });

    testWidgets('says that it is busy, and asks again when the person says '
        'so', (tester) async {
      final session = _session()
        ..continueOutcome = const ContinueOutcome.busy();
      await _show(tester, session);

      expect(
        find.text('Anna’s Mac is busy right now. Try again in a moment.'),
        findsOneWidget,
      );

      session.continueOutcome = _report;
      await tester.tap(find.text('Try again'));
      await tester.pumpAndSettle();

      expect(session.continueAsked, [_mac.id, _mac.id]);
      expect(find.text('The report'), findsOneWidget);
      expect(find.text('Try again'), findsNothing);
    });

    testWidgets('says what to check when it could not read what is open', (
      tester,
    ) async {
      final session = _session()
        ..continueOutcome = const ContinueOutcome.unavailable();

      await _show(tester, session);

      expect(
        find.textContaining(
          'Anna’s Mac could not read what it has open just now.',
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining(
          'System Settings, under Privacy & Security, Automation',
        ),
        findsOneWidget,
      );
      expect(find.text('Try again'), findsOneWidget);
    });

    testWidgets('says that its version of Baton cannot do this yet', (
      tester,
    ) async {
      final session = _session()
        ..continueOutcome = const ContinueOutcome.unsupported();

      await _show(tester, session);

      expect(
        find.text('Anna’s Mac has a version of Baton that cannot do this yet.'),
        findsOneWidget,
      );
      expect(find.text('Try again'), findsNothing);
    });

    testWidgets('says when it cannot be reached, and can be asked again', (
      tester,
    ) async {
      final session = _session()
        ..continueFailure = const BatonError(
          kind: ErrorKind.unreachable,
          message: 'could not reach it',
        );

      await _show(tester, session);

      expect(
        find.text(
          'Baton could not reach Anna’s Mac. Check that it is awake and '
          'online.',
        ),
        findsOneWidget,
      );
      expect(find.text('Try again'), findsOneWidget);
    });

    testWidgets('says when the devices are not paired any more', (
      tester,
    ) async {
      final session = _session()
        ..continueFailure = const BatonError(
          kind: ErrorKind.notPaired,
          message: 'not paired',
        );

      await _show(tester, session);

      expect(
        find.text('This device is not paired with Anna’s Mac any more.'),
        findsOneWidget,
      );
      expect(find.text('Try again'), findsNothing);
    });

    testWidgets('gives the sentence of the library for any other failure', (
      tester,
    ) async {
      final session = _session()
        ..continueFailure = const BatonError(
          kind: ErrorKind.other,
          message: 'something went wrong',
        );

      await _show(tester, session);

      expect(find.text('Something went wrong.'), findsOneWidget);
      expect(find.text('Try again'), findsOneWidget);
    });
  });

  group('the sheet', () {
    testWidgets('opens over the screen, from a button', (tester) async {
      final session = _session()..continueOutcome = _report;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) => Scaffold(
              body: Center(
                child: FilledButton(
                  onPressed: () => showContinueSheet(
                    context,
                    session: session,
                    device: _mac,
                  ),
                  child: const Text('Continue'),
                ),
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.text('Continue'));
      await tester.pumpAndSettle();

      expect(find.text('Continue from Anna’s Mac'), findsOneWidget);
      expect(find.text('The report'), findsOneWidget);
      expect(session.continueAsked, [_mac.id]);
    });

    testWidgets('keeps nothing when it is closed: asking again asks the '
        'device again', (tester) async {
      final session = _session()..continueOutcome = _report;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) => Scaffold(
              body: Center(
                child: FilledButton(
                  onPressed: () => showContinueSheet(
                    context,
                    session: session,
                    device: _mac,
                  ),
                  child: const Text('Continue'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Continue'));
      await tester.pumpAndSettle();
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();
      expect(find.text('The report'), findsNothing);

      session.continueOutcome = const ContinueOutcome.items(
        items: ContinueItems(clipboard: 'newer'),
      );
      await tester.tap(find.text('Continue'));
      await tester.pumpAndSettle();

      expect(session.continueAsked, [_mac.id, _mac.id]);
      expect(find.text('newer'), findsOneWidget);
      expect(find.text('The report'), findsNothing);
    });

    testWidgets('is closed while the device is being asked, and the answer '
        'that comes is dropped', (tester) async {
      final session = _session()..holdContinue();
      session.continueOutcome = _report;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: ContinueSheet(session: session, device: _mac),
          ),
        ),
      );
      await tester.pump();

      await tester.pumpWidget(const MaterialApp(home: Scaffold()));
      session.releaseContinue();
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
    });
  });

  group('the controller', () {
    test('shows only the latest answer when it is asked again', () async {
      final session = _session();
      final controller = ContinueController(session, _mac);
      addTearDown(controller.dispose);

      final first = session.holdContinue();
      session.continueOutcome = const ContinueOutcome.busy();
      final asking = controller.ask();
      final second = session.holdContinue();
      final askingAgain = controller.ask();

      // The first answer comes after the second question was put: it is old, and not shown.
      first.complete();
      await asking;
      expect(controller.view, isA<ContinueAsking>());

      session.continueOutcome = _report;
      second.complete();
      await askingAgain;
      expect(controller.view, isA<ContinueFound>());
    });

    test('hands the system the address that Baton checked, never the one '
        'that came', () async {
      final session = _session()
        ..continueOutcome = const ContinueOutcome.items(
          items: ContinueItems(
            page: ContinuePage(url: '  https://example.com/a b '),
          ),
        );
      final controller = ContinueController(session, _mac);
      addTearDown(controller.dispose);

      await controller.ask();

      final found = controller.view as ContinueFound;
      expect(found.page!.url, '  https://example.com/a b ');
      expect(found.address, session.webAddress('  https://example.com/a b '));
      expect(found.address, isNot(found.page!.url));
    });
  });
}
