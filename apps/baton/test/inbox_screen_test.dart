// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/backend.dart';
import 'package:baton/src/inbox_controller.dart';
import 'package:baton/src/links.dart';
import 'package:baton/src/screens/inbox_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_clipboard.dart';

// Five minutes after the items in these tests were sent.
final _now = DateTime.fromMillisecondsSinceEpoch((1760529600 + 300) * 1000);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

/// Shows the screen for [session] and waits until the inbox has been read. The links it is
/// asked to open are written to [opened], and [opens] says whether that worked.
Future<InboxController> _show(
  WidgetTester tester,
  FakeSession session, {
  List<String>? opened,
  bool opens = true,
  Size size = const Size(900, 1600),
}) async {
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final controller = InboxController(session);
  addTearDown(controller.dispose);
  unawaited(controller.start());
  await tester.pumpWidget(
    MaterialApp(
      home: InboxScreen(
        controller: controller,
        now: () => _now,
        openLink: (address) async {
          opened?.add(address);
          return opens;
        },
      ),
    ),
  );
  await tester.pumpAndSettle();
  return controller;
}

const _link = ReceivedItem.link(
  url: 'https://example.com/plan',
  title: 'The plan',
);

void main() {
  testWidgets('says that nothing has come yet, and that Baton must be open', (
    tester,
  ) async {
    await _show(tester, _session());

    expect(find.text('Nothing here yet'), findsOneWidget);
    expect(
      find.textContaining('Baton has to be open on this device'),
      findsOne,
    );
    final clear = find.ancestor(
      of: find.byTooltip('Clear the inbox'),
      matching: find.byType(IconButton),
    );
    expect(
      tester.widget<IconButton>(clear).onPressed,
      isNull,
      reason: 'there is nothing to clear',
    );
  });

  testWidgets(
    'shows a link, who sent it and when, and opens only the checked address',
    (tester) async {
      final opened = <String>[];
      final session = _session()
        ..received = [
          sampleReceived(
            1,
            item: const ReceivedItem.link(
              url: 'HTTPS://Example.com/plan',
              title: 'The plan',
            ),
          ),
        ];
      await _show(tester, session, opened: opened);

      expect(find.text('Anna’s iPhone · 5 minutes ago'), findsOneWidget);
      expect(find.text('The plan'), findsOneWidget);
      expect(find.text('HTTPS://Example.com/plan'), findsOneWidget);

      await tester.tap(find.text('Open'));
      await tester.pump();

      expect(opened, ['https://example.com/plan']);
      expect(find.textContaining('could not open'), findsNothing);
    },
  );

  testWidgets(
    'says so when a link could not be opened, and it can still be copied',
    (tester) async {
      final clipboard = FakeClipboard(tester);
      final session = _session()..received = [sampleReceived(1, item: _link)];
      await _show(tester, session, opens: false);

      await tester.tap(find.text('Open'));
      await tester.pump();

      expect(
        find.text('Baton could not open that link. You can copy it instead.'),
        findsOneWidget,
      );

      await tester.tap(find.text('Copy'));
      await tester.pump();
      expect(clipboard.copied, 'https://example.com/plan');
    },
  );

  testWidgets('a link that is not a web address is not offered to open', (
    tester,
  ) async {
    final clipboard = FakeClipboard(tester);
    final opened = <String>[];
    final session = _session()
      ..received = [
        sampleReceived(3, item: const ReceivedItem.link(url: 'tel:+15551234')),
        sampleReceived(
          2,
          item: const ReceivedItem.link(url: 'smb://server/share/file'),
        ),
        sampleReceived(
          1,
          item: const ReceivedItem.link(url: 'javascript:alert(1)'),
        ),
      ];
    await _show(tester, session, opened: opened);

    expect(find.text('Open'), findsNothing);
    expect(
      find.text('This is not a web address, so Baton will not open it.'),
      findsNWidgets(3),
    );
    expect(find.text('tel:+15551234'), findsOneWidget);

    await tester.tap(find.text('Copy').first);
    await tester.pump();
    expect(clipboard.copied, 'tel:+15551234');
    expect(opened, isEmpty);
  });

  testWidgets('shows text, and copies it exactly as it was sent', (
    tester,
  ) async {
    final clipboard = FakeClipboard(tester);
    const sent = 'Door code:\r\n4711\u202E';
    final session = _session()
      ..received = [
        sampleReceived(1, item: const ReceivedItem.text(text: sent)),
      ];
    await _show(tester, session);

    // What is shown has the line end every system understands, and a mark where something
    // turned the text round.
    expect(find.text('Door code:\n4711�'), findsOneWidget);
    expect(find.text('Open'), findsNothing);

    await tester.tap(find.text('Copy'));
    await tester.pump();

    expect(clipboard.copied, sent);
    expect(find.text('Copied.'), findsOneWidget);
  });

  testWidgets('long text is cut short, until the person asks for all of it', (
    tester,
  ) async {
    final lines = [for (var i = 1; i <= 20; i++) 'line $i'];
    final session = _session()
      ..received = [
        sampleReceived(1, item: ReceivedItem.text(text: lines.join('\n'))),
        sampleReceived(2, item: const ReceivedItem.text(text: 'short')),
      ];
    await _show(tester, session);

    expect(find.text('Show more'), findsOneWidget, reason: 'only the long one');
    Text shown() => tester.widget<Text>(find.text(lines.join('\n')));
    expect(shown().maxLines, 6);

    await tester.tap(find.text('Show more'));
    await tester.pump();

    expect(find.text('Show less'), findsOneWidget);
    expect(find.byType(SelectableText), findsOneWidget);

    await tester.tap(find.text('Show less'));
    await tester.pump();
    expect(shown().maxLines, 6);
  });

  group('whether text is cut short is decided by how it wraps', () {
    // Each of these has fewer than 240 characters or fewer than six line ends, which is what
    // the screen once counted, and each still takes more than six lines on the screen.
    Future<void> showText(
      WidgetTester tester,
      String text, {
      Size size = const Size(900, 1600),
    }) async {
      final session = _session()
        ..received = [sampleReceived(1, item: ReceivedItem.text(text: text))];
      await _show(tester, session, size: size);
    }

    testWidgets('on a phone, four lines that each wrap twice or more', (
      tester,
    ) async {
      final line = 'word ' * 11;
      final text = List.filled(4, line.trim()).join('\n');
      expect(text.length, lessThan(240));

      await showText(tester, text, size: const Size(393, 852));

      expect(find.text('Show more'), findsOneWidget);
    });

    testWidgets('when the letters are made larger', (tester) async {
      tester.platformDispatcher.textScaleFactorTestValue = 1;
      addTearDown(tester.platformDispatcher.clearAllTestValues);
      final text = ('word ' * 30).trim();
      expect(text.length, lessThan(240));
      await showText(tester, text);
      expect(find.text('Show more'), findsNothing, reason: 'it fits as it is');

      tester.platformDispatcher.textScaleFactorTestValue = 2;
      await tester.pumpAndSettle();

      expect(find.text('Show more'), findsOneWidget);
      await tester.tap(find.text('Show more'));
      await tester.pump();
      expect(find.text('Show less'), findsOneWidget);
      expect(find.text(text), findsOneWidget, reason: 'all of it, once asked');
    });

    testWidgets('when the lines end in a carriage return and nothing else', (
      tester,
    ) async {
      await showText(tester, 'a\rb\rc\rd\re\rf\rg\rh');

      expect(find.text('Show more'), findsOneWidget);
    });

    testWidgets('and not when everything fits, however many characters', (
      tester,
    ) async {
      // Small letters, so that more than 240 characters take fewer than six lines.
      tester.platformDispatcher.textScaleFactorTestValue = 0.5;
      addTearDown(tester.platformDispatcher.clearAllTestValues);
      final text = ('word ' * 50).trim();
      expect(text.length, greaterThan(240));

      await showText(tester, text);

      expect(find.text(text), findsOneWidget);
      expect(find.text('Show more'), findsNothing);
    });

    testWidgets('in bold letters when Bold Text is on, which are wider', (
      tester,
    ) async {
      const text = 'A text that is the same either way, but for its weight';
      addTearDown(tester.platformDispatcher.clearAllTestValues);
      await showText(tester, text);
      Text shown() => tester.widget<Text>(find.text(text));
      expect(shown().style?.fontWeight, isNot(FontWeight.bold));

      tester.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(boldText: true);
      await tester.pumpAndSettle();

      // The test font is as wide in bold as it is otherwise, so the wrapping cannot show it. The
      // card gives the one style it worked out both to the text and to the measuring, so this is
      // the style that decides whether the cut is offered.
      expect(shown().style?.fontWeight, FontWeight.bold);
    });

    testWidgets('six lines are shown whole, seven are cut', (tester) async {
      final six = [for (var i = 1; i <= 6; i++) 'six $i'].join('\n');
      final seven = [for (var i = 1; i <= 7; i++) 'seven $i'].join('\n');
      final session = _session()
        ..received = [
          sampleReceived(2, item: ReceivedItem.text(text: seven)),
          sampleReceived(1, item: ReceivedItem.text(text: six)),
        ];

      await _show(tester, session);

      expect(find.text('Show more'), findsOneWidget);
      final cut = tester.widget<Text>(find.text(seven));
      expect(cut.maxLines, 6);
    });
  });

  testWidgets('removes one item', (tester) async {
    final session = _session()
      ..received = [
        sampleReceived(2, item: const ReceivedItem.text(text: 'second')),
        sampleReceived(1, item: const ReceivedItem.text(text: 'first')),
      ];
    await _show(tester, session);

    await tester.tap(find.byTooltip('Remove').last);
    await tester.pumpAndSettle();

    expect(session.removed, [1]);
    expect(find.text('first'), findsNothing);
    expect(find.text('second'), findsOneWidget);
  });

  testWidgets('says why an item could not be removed', (tester) async {
    final session = _session()..received = [sampleReceived(1)];
    await _show(tester, session);
    session.inboxFailure = const BatonError(
      kind: ErrorKind.storage,
      message: 'the inbox file cannot be written',
    );

    await tester.tap(find.byTooltip('Remove'));
    await tester.pumpAndSettle();

    expect(find.text('The inbox file cannot be written.'), findsOneWidget);
    expect(find.text('Hello from the phone'), findsOneWidget);
  });

  testWidgets('asks before it clears the inbox, and clears all of it', (
    tester,
  ) async {
    final session = _session()
      ..received = [sampleReceived(2), sampleReceived(1)];
    await _show(tester, session);

    await tester.tap(find.byTooltip('Clear the inbox'));
    await tester.pumpAndSettle();
    expect(
      find.text('All 2 items in it will be deleted from this device.'),
      findsOneWidget,
    );

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(session.clears, 0);
    expect(find.text('Hello from the phone'), findsNWidgets(2));

    await tester.tap(find.byTooltip('Clear the inbox'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Clear'));
    await tester.pumpAndSettle();

    expect(session.clears, 1);
    expect(find.text('Nothing here yet'), findsOneWidget);
  });

  testWidgets('one item is told as one', (tester) async {
    final session = _session()..received = [sampleReceived(1)];
    await _show(tester, session);

    await tester.tap(find.byTooltip('Clear the inbox'));
    await tester.pumpAndSettle();

    expect(
      find.text('The one item in it will be deleted from this device.'),
      findsOneWidget,
    );
  });

  testWidgets('a new item appears at the top as it comes', (tester) async {
    final session = _session()
      ..received = [
        sampleReceived(1, item: const ReceivedItem.text(text: 'older')),
      ];
    await _show(tester, session);

    session.arrive(
      sampleReceived(2, item: const ReceivedItem.text(text: 'newer')),
    );
    await tester.pumpAndSettle();

    final newer = tester.getTopLeft(find.text('newer')).dy;
    final older = tester.getTopLeft(find.text('older')).dy;
    expect(newer, lessThan(older));
  });

  testWidgets('says when the inbox cannot be read, and tries again', (
    tester,
  ) async {
    final session = _session()
      ..received = [sampleReceived(1)]
      ..inboxFailure = const BatonError(
        kind: ErrorKind.storage,
        message: 'the inbox file is damaged',
      );
    await _show(tester, session);

    expect(find.text('Could not read the inbox'), findsOneWidget);
    expect(find.text('The inbox file is damaged.'), findsOneWidget);

    session.inboxFailure = null;
    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();

    expect(find.text('Could not read the inbox'), findsNothing);
    expect(find.text('Hello from the phone'), findsOneWidget);
  });

  testWidgets('a link is opened with the address the library checked', (
    tester,
  ) async {
    // The screen has no say in what is opened: it asks the session, and passes on the answer.
    final opened = <String>[];
    final session = _session()..received = [sampleReceived(1, item: _link)];
    await _show(tester, session, opened: opened);

    await tester.tap(find.text('Open'));
    await tester.pump();

    expect(opened.single, session.webAddress('https://example.com/plan'));
  });

  test('the default way to open a link is the one that checks again', () {
    // The screens are given this unless a test gives them another.
    expect(openInBrowser, isA<OpenLink>());
  });
}
