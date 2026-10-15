// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/send_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_clipboard.dart';

// Five minutes after the iPad was last heard from.
final _now = DateTime.fromMillisecondsSinceEpoch((1760529600 + 300) * 1000);

final _kitchenIpad = Device(
  id: 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
  lastSeenAt: 1760529600,
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: [_kitchenIpad],
);

/// Shows the screen for sending to the iPad, on a surface of [size], and returns the session
/// behind it.
Future<FakeSession> _show(
  WidgetTester tester, {
  FakeSession? session,
  Size size = const Size(900, 1600),
}) async {
  final shown = session ?? _session();
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: SendScreen(session: shown, device: _kitchenIpad, now: () => _now),
    ),
  );
  return shown;
}

final _box = find.byType(TextField);

String _typed(WidgetTester tester) =>
    tester.widget<TextField>(_box).controller!.text;

/// Whether the button with [label] can be pressed.
bool _enabled(WidgetTester tester, String label) =>
    tester
        .widget<ButtonStyleButton>(
          find.ancestor(
            of: find.text(label),
            matching: find.bySubtype<ButtonStyleButton>(),
          ),
        )
        .onPressed !=
    null;

Future<void> _type(WidgetTester tester, String text) async {
  await tester.enterText(_box, text);
  await tester.pump();
}

void main() {
  testWidgets('names the device, and has nothing to send until there is text', (
    tester,
  ) async {
    await _show(tester);

    expect(find.text('Send to Kitchen iPad'), findsOneWidget);
    expect(
      find.text('iPad · seen 5 minutes ago\nabcd-efgh-ijkl-mnop-qrst'),
      findsOneWidget,
    );
    expect(
      find.text('Baton has to be open on Kitchen iPad for it to arrive.'),
      findsOneWidget,
    );
    expect(find.textContaining('It will be sent as'), findsNothing);
    expect(_enabled(tester, 'Send text'), isFalse);
    expect(_enabled(tester, 'Paste'), isTrue);
  });

  testWidgets('says whether what is typed goes as a link or as text', (
    tester,
  ) async {
    await _show(tester);

    await _type(tester, 'https://example.com/plan');
    expect(
      find.text('It will be sent as a link, which Kitchen iPad can open.'),
      findsOneWidget,
    );
    expect(_enabled(tester, 'Send link'), isTrue);

    await _type(tester, 'Have a look at https://example.com/plan');
    expect(
      find.text('It will be sent as text, which Kitchen iPad can copy.'),
      findsOneWidget,
    );
    expect(_enabled(tester, 'Send text'), isTrue);

    await _type(tester, 'tel:+15551234');
    expect(find.text('Send text'), findsOneWidget);

    await _type(tester, '  \n ');
    expect(find.textContaining('It will be sent as'), findsNothing);
    expect(_enabled(tester, 'Send text'), isFalse);
  });

  testWidgets('sends a link, says so, and empties the box', (tester) async {
    final session = await _show(tester);

    await _type(tester, ' HTTPS://Example.com/plan ');
    await tester.tap(find.text('Send link'));
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => (sent.id, sent.url, sent.text)), [
      (_kitchenIpad.id, 'https://example.com/plan', null),
    ]);
    expect(find.text('Sent the link to Kitchen iPad.'), findsOneWidget);
    expect(_typed(tester), isEmpty);
    expect(_enabled(tester, 'Send text'), isFalse, reason: 'nothing to send');
  });

  testWidgets('sends text exactly as it was typed', (tester) async {
    final session = await _show(tester);
    const typed = '  The door code is\n4711  ';

    await _type(tester, typed);
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();

    expect(session.sent.single.id, _kitchenIpad.id);
    expect(session.sent.single.text, typed);
    expect(session.sent.single.url, isNull);
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);
    expect(_typed(tester), isEmpty);
  });

  testWidgets('says why a send failed, and what was typed stays', (
    tester,
  ) async {
    final session = _session()
      ..sendFailure = const BatonError(
        kind: ErrorKind.unreachable,
        message: 'could not reach Kitchen iPad. Baton has to be open on it',
      );
    await _show(tester, session: session);

    await _type(tester, 'hello');
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();

    expect(
      find.text('Could not reach Kitchen iPad. Baton has to be open on it.'),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.error_outline), findsOneWidget);
    expect(_typed(tester), 'hello');
    expect(_enabled(tester, 'Send text'), isTrue, reason: 'try again');

    // Once it can be reached, the same words go.
    session.sendFailure = null;
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['hello']);
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);
    expect(find.byIcon(Icons.error_outline), findsNothing);
  });

  testWidgets('shows that it is sending, and cannot send twice', (
    tester,
  ) async {
    final session = _session()..holdSend();
    await _show(tester, session: session);
    await _type(tester, 'hello');

    await tester.tap(find.text('Send text'));
    await tester.pump();

    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(
      find.text(
        'Sending to Kitchen iPad. If Baton is not open there, Baton gives up '
        'after a while.',
      ),
      findsOneWidget,
    );
    expect(_enabled(tester, 'Send text'), isFalse);
    expect(_enabled(tester, 'Paste'), isFalse);

    session.releaseSend();
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['hello']);
    expect(find.byType(CircularProgressIndicator), findsNothing);
    expect(find.textContaining('Sending to Kitchen iPad'), findsNothing);
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);
  });

  testWidgets('does not empty the box if something else was typed meanwhile', (
    tester,
  ) async {
    final session = _session()..holdSend();
    await _show(tester, session: session);
    await _type(tester, 'first');
    await tester.tap(find.text('Send text'));
    await tester.pump();

    await _type(tester, 'second');
    session.releaseSend();
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['first']);
    expect(_typed(tester), 'second');
  });

  testWidgets('forgets what happened once something else is typed', (
    tester,
  ) async {
    await _show(tester);
    await _type(tester, 'hello');
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);

    await _type(tester, 'and another thing');

    expect(find.text('Sent the text to Kitchen iPad.'), findsNothing);
  });

  testWidgets('pastes what is on the clipboard into the box', (tester) async {
    FakeClipboard(tester, holds: 'https://example.com/plan');
    await _show(tester);

    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(_typed(tester), 'https://example.com/plan');
    expect(_enabled(tester, 'Send link'), isTrue);
    expect(find.textContaining('no text on the clipboard'), findsNothing);
  });

  testWidgets('says when there is nothing on the clipboard to paste', (
    tester,
  ) async {
    final clipboard = FakeClipboard(tester);
    await _show(tester);
    await _type(tester, 'keep this');

    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(find.text('There is no text on the clipboard.'), findsOneWidget);
    expect(_typed(tester), 'keep this', reason: 'the box is left as it was');

    clipboard.holds = '  \n ';
    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(find.text('There is no text on the clipboard.'), findsOneWidget);
    expect(_typed(tester), 'keep this', reason: 'blank is nothing');
  });

  testWidgets('checks that the iPad answers, and how', (tester) async {
    final session = await _show(tester);

    await tester.tap(find.text('Check the connection'));
    await tester.pumpAndSettle();

    expect(session.pinged, [_kitchenIpad.id]);
    expect(
      find.text('Kitchen iPad answered in 24 ms, over your local network.'),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.check_circle_outline), findsOneWidget);
  });

  testWidgets('shows that it is waiting for the iPad to answer', (
    tester,
  ) async {
    final session = _session()..holdPing();
    await _show(tester, session: session);

    await tester.tap(find.text('Check the connection'));
    await tester.pump();

    expect(find.text('Waiting for Kitchen iPad to answer.'), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(_enabled(tester, 'Check the connection'), isFalse);

    session.releasePing();
    await tester.pumpAndSettle();

    expect(find.textContaining('Waiting for'), findsNothing);
    expect(
      find.text('Kitchen iPad answered in 24 ms, over your local network.'),
      findsOneWidget,
    );
  });

  testWidgets('says when the iPad does not answer', (tester) async {
    final session = _session()
      ..pingFailure = const BatonError(
        kind: ErrorKind.unreachable,
        message: 'could not reach Kitchen iPad. Baton has to be open on it',
      );
    await _show(tester, session: session);

    await tester.tap(find.text('Check the connection'));
    await tester.pumpAndSettle();

    expect(
      find.text('Could not reach Kitchen iPad. Baton has to be open on it.'),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.error_outline), findsOneWidget);
  });

  testWidgets('a send that ends after the screen is closed does no harm', (
    tester,
  ) async {
    final session = _session()..holdSend();
    await _show(tester, session: session);
    await _type(tester, 'hello');
    await tester.tap(find.text('Send text'));
    await tester.pump();

    await tester.pumpWidget(const SizedBox());
    session.releaseSend();
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['hello']);
    expect(tester.takeException(), isNull);
  });

  testWidgets('fits a small phone with the largest text', (tester) async {
    tester.platformDispatcher.textScaleFactorTestValue = 2.5;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    final session = await _show(tester, size: const Size(320, 568));
    // The text this large fills the first screen with the device, so the box is a scroll away.
    final list = find.byType(Scrollable).first;
    await tester.scrollUntilVisible(_box, 100, scrollable: list);
    await _type(tester, 'https://example.com/plan');
    await tester.scrollUntilVisible(
      find.text('Send link'),
      100,
      scrollable: list,
    );
    await tester.pump();
    await tester.tap(find.text('Send link'));
    await tester.pumpAndSettle();

    // An overflow would have been thrown while the tree was laid out.
    expect(tester.takeException(), isNull);
    expect(session.sent, hasLength(1));
  });
}
