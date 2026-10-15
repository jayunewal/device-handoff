// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/diagnostics_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_clipboard.dart';

final _now = DateTime.utc(2026, 10, 15, 8, 44, 54);

const _started =
    '2026-10-15T08:40:00.000001Z  INFO baton_app: Baton started device=f0f0f0f0f0';
const _listening =
    '2026-10-15T08:40:01.000002Z  INFO baton_core: Baton is listening';

FakeSession _session({List<String> log = const []}) => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  reach: const Network(
    relay: 'https://relay.example./',
    addresses: ['192.168.1.5:4000'],
    lanDiscovery: true,
  ),
  log: [...log],
);

/// Shows the screen for [session], or for a Baton that could not start, on a surface of [size].
Future<void> _show(
  WidgetTester tester, {
  FakeSession? session,
  List<String> Function()? recentLog,
  String? problem,
  Size size = const Size(900, 1600),
}) async {
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: DiagnosticsScreen(
        recentLog: recentLog ?? session!.recentLog,
        session: session,
        problem: problem,
        system: 'macos 14.5',
        now: () => _now,
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('shows this device, how it is connected, and the log', (
    tester,
  ) async {
    await _show(tester, session: _session(log: [_started, _listening]));

    expect(find.text('Diagnostics'), findsOneWidget);
    for (final text in [
      'Taken: 2026-10-15T08:44:54Z',
      'System: macos 14.5',
      'This device: Desk Mac (Mac), id f0f0f0f0f0',
      'Paired devices: 0',
      'Relay: https://relay.example./',
      'Addresses other devices can try: 192.168.1.5:4000',
      'Finding devices on the local network: on',
      '2 lines, oldest first.',
      _started,
      _listening,
    ]) {
      expect(find.text(text), findsOneWidget, reason: text);
    }
    // The text can be selected, to copy a part of it.
    expect(find.byType(SelectionArea), findsOneWidget);
  });

  testWidgets('says what is hidden and what is not before it is copied', (
    tester,
  ) async {
    await _show(tester, session: _session());

    expect(
      find.textContaining('Addresses on the internet are hidden.'),
      findsOneWidget,
    );
    expect(
      find.textContaining('the addresses of your own network'),
      findsOneWidget,
    );
  });

  testWidgets('says so when nothing has been logged', (tester) async {
    await _show(tester, session: _session());

    expect(find.text('Nothing has been logged.'), findsOneWidget);
  });

  testWidgets('copies all of it, to paste into a bug report', (tester) async {
    final clipboard = FakeClipboard(tester);
    await _show(tester, session: _session(log: [_started, _listening]));

    await tester.tap(find.text('Copy'));
    await tester.pump();

    expect(clipboard.copied, '''
Baton diagnostics

Taken: 2026-10-15T08:44:54Z
System: macos 14.5
This device: Desk Mac (Mac), id f0f0f0f0f0
Paired devices: 0
Relay: https://relay.example./
Addresses other devices can try: 192.168.1.5:4000
Finding devices on the local network: on

Log
2 lines, oldest first.
$_started
$_listening
''');
    expect(find.text('Copied.'), findsOneWidget);
  });

  testWidgets('reads again when asked', (tester) async {
    final session = _session(log: [_started]);
    await _show(tester, session: session);
    expect(find.text('1 line, oldest first.'), findsOneWidget);
    expect(find.text(_listening), findsNothing);

    session.log.add(_listening);
    await tester.tap(find.text('Refresh'));
    await tester.pumpAndSettle();

    expect(find.text('2 lines, oldest first.'), findsOneWidget);
    expect(find.text(_started), findsOneWidget);
    expect(find.text(_listening), findsOneWidget);
  });

  testWidgets('can be shown when Baton could not start', (tester) async {
    await _show(
      tester,
      recentLog: () => [
        '2026-10-15T08:40:00Z ERROR baton_app: could not start',
      ],
      problem: 'Another Baton already has this device’s folder open.',
    );

    expect(
      find.text(
        'Problem: Another Baton already has this device’s folder open.',
      ),
      findsOneWidget,
    );
    expect(find.text('Baton is not running.'), findsOneWidget);
    expect(find.textContaining('This device:'), findsNothing);
    expect(find.textContaining('could not start'), findsOneWidget);
  });

  testWidgets('wraps a log line that is longer than the screen', (
    tester,
  ) async {
    final long = 'x' * 600;
    await _show(
      tester,
      session: _session(log: ['short', long]),
      size: const Size(360, 1400),
    );

    final one = tester.getSize(find.text('short')).height;
    expect(tester.getSize(find.text(long)).height, greaterThan(3 * one));
    expect(tester.takeException(), isNull);
  });
}
