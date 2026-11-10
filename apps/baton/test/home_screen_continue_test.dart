// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_access.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/screens/home_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_always_on.dart';
import 'fake_backend.dart';
import 'fake_continue_reader.dart';
import 'fake_control_access.dart';
import 'fake_notices.dart';
import 'fake_owner_check.dart';
import 'fake_screen.dart';
import 'fake_shower.dart';

const _report = ContinueItems(
  page: ContinuePage(url: 'https://example.com/report', title: 'The report'),
  clipboard: 'hello',
);

/// Shows the home screen of a Mac, which reads for [reader] when a device asks.
Future<FakeSession> _show(
  WidgetTester tester,
  FakeContinueReader reader,
) async {
  final session = FakeSession(
    me: const Me(
      id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
      name: 'Desk Mac',
      platform: Platform.macOs,
      fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
    ),
  );
  tester.view
    ..physicalSize = const Size(900, 1600)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: HomeScreen(
        session: session,
        // No camera here: the pairing screens get a stand-in.
        scanner: (context, onCode) => const Text('(camera)'),
        openLink: (address) async => true,
        shower: FakeShower(),
        screen: FakeScreen(),
        controlAccess: FakeControlAccess(ControlAccess.unavailable),
        screenAccess: FakeScreenAccess(ScreenAccess.unavailable),
        notices: FakeNotices(NoticeAccess.unavailable),
        ownerCheck: FakeOwnerCheck(),
        continueReader: reader,
        alwaysOn: FakeAlwaysOn(),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return session;
}

void main() {
  testWidgets('answers a device that asks what the person is working on', (
    tester,
  ) async {
    // Starting the server is what lets a phone reach a Mac, with the window open or not: without
    // it every question from a phone is never answered.
    final reader = FakeContinueReader(items: _report);
    final session = await _show(tester, reader);
    expect(reader.reads, 0, reason: 'nothing is read before a device asks');

    session.emit(Event.continueAsked(id: 9, from: 'Anna’s iPhone'));
    await tester.pumpAndSettle();

    expect(session.continueAnswers, [(request: 9, items: _report)]);
    expect(reader.reads, 1);
  });
}
