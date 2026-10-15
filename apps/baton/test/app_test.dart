// SPDX-License-Identifier: Apache-2.0

// The app from its first screen to the devices, with a fake Baton behind it.

import 'package:baton/src/app.dart';
import 'package:baton/src/backend.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

void main() {
  testWidgets('asks for a name the first time, then starts Baton with it', (
    tester,
  ) async {
    final backend = FakeBackend();
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();

    expect(find.text('Name this device'), findsOneWidget);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller?.text,
      'Test Mac',
      reason: 'the field starts with the name that was suggested',
    );
    expect(backend.starts, 0, reason: 'nothing starts before the person chose');

    await tester.enterText(find.byType(TextField), 'Kitchen iPad');
    await tester.tap(find.text('Continue'));
    await tester.pumpAndSettle();

    expect(backend.namesSaved, ['Kitchen iPad']);
    expect(backend.starts, 1);
    expect(find.text('Name this device'), findsNothing);
    expect(find.text('Kitchen iPad'), findsOneWidget);
    expect(find.text('No other devices yet'), findsOneWidget);
  });

  testWidgets('says when a name cannot be used, and asks again', (
    tester,
  ) async {
    final backend = FakeBackend();
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();

    await tester.enterText(find.byType(TextField), '   ');
    await tester.tap(find.text('Continue'));
    await tester.pumpAndSettle();
    expect(find.text(noVisibleName.message), findsOneWidget);
    expect(find.text('Name this device'), findsOneWidget);
    expect(backend.starts, 0);

    await tester.enterText(find.byType(TextField), 'Desk');
    await tester.tap(find.text('Continue'));
    await tester.pumpAndSettle();
    expect(find.text(noVisibleName.message), findsNothing);
    expect(find.text('Desk'), findsOneWidget);
    expect(backend.starts, 1);
  });

  testWidgets('does not ask for a name that is saved', (tester) async {
    final backend = FakeBackend(savedName: 'Office Mac');
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();

    expect(find.text('Name this device'), findsNothing);
    expect(find.text('Office Mac'), findsOneWidget);
    expect(backend.starts, 1);
    expect(backend.namesSaved, isEmpty);
  });

  testWidgets('says why Baton could not start, and tries again', (
    tester,
  ) async {
    final backend = FakeBackend(
      savedName: 'Desk',
      startFailures: [
        const BatonError(
          kind: ErrorKind.alreadyRunning,
          message: 'Another Baton already has this device’s folder open.',
        ),
      ],
    );
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();

    expect(find.text('Baton could not start'), findsOneWidget);
    expect(
      find.text('Another Baton already has this device’s folder open.'),
      findsOneWidget,
    );

    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    expect(find.text('Baton could not start'), findsNothing);
    expect(find.text('Desk'), findsOneWidget);
    expect(backend.starts, 2);
  });

  testWidgets('shows the details of a failed start, with what was logged', (
    tester,
  ) async {
    tester.view
      ..physicalSize = const Size(900, 1600)
      ..devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final backend = FakeBackend(
      savedName: 'Desk',
      startFailures: [
        const BatonError(
          kind: ErrorKind.alreadyRunning,
          message: 'Another Baton already has this device’s folder open.',
        ),
      ],
    )..log.add('2026-10-15T08:40:00Z ERROR baton_app: could not start');
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Show details'));
    await tester.pumpAndSettle();

    expect(find.text('Diagnostics'), findsOneWidget);
    expect(
      find.text(
        'Problem: Another Baton already has this device’s folder open.',
      ),
      findsOneWidget,
    );
    expect(find.text('Baton is not running.'), findsOneWidget);
    expect(
      find.textContaining('ERROR baton_app: could not start'),
      findsOneWidget,
    );

    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.text('Baton could not start'), findsOneWidget);
    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    expect(find.text('Desk'), findsOneWidget);
  });

  testWidgets('shows a failure that did not come from Baton as well', (
    tester,
  ) async {
    final backend = FakeBackend(
      savedName: 'Desk',
      startFailures: [ArgumentError('library not found')],
    );
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();

    expect(find.text('Baton could not start'), findsOneWidget);
    expect(
      find.textContaining('Something unexpected went wrong'),
      findsOneWidget,
    );
    expect(find.textContaining('library not found'), findsOneWidget);
  });

  testWidgets('a name that cannot be saved is a failure, not a bad name', (
    tester,
  ) async {
    final backend = FakeBackend()
      ..setNameFailure = const BatonError(
        kind: ErrorKind.storage,
        message: 'The settings file cannot be used.',
      );
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();

    await tester.enterText(find.byType(TextField), 'Desk');
    await tester.tap(find.text('Continue'));
    await tester.pumpAndSettle();
    expect(find.text('Baton could not start'), findsOneWidget);
    expect(find.text('The settings file cannot be used.'), findsOneWidget);
    expect(backend.starts, 0);
  });

  testWidgets('stops Baton when the app closes', (tester) async {
    final backend = FakeBackend(savedName: 'Desk');
    await tester.pumpWidget(BatonApp(backend: backend));
    await tester.pumpAndSettle();
    expect(backend.session.stopped, isFalse);
    expect(backend.session.listening, isTrue);

    await tester.pumpWidget(const SizedBox());
    expect(backend.session.stopped, isTrue);
    expect(backend.session.listening, isFalse);
  });
}
