// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/awake.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/qr.dart';
import 'package:baton/src/screens/show_code_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

// The offer in the fake lapses five minutes after this.
final _start = DateTime.fromMillisecondsSinceEpoch(1760529600 * 1000);

const _anna = Device(
  id: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  name: 'Anna’s iPhone',
  platform: Platform.ios,
  fingerprint: 'iphn-abcd-efgh-ijkl-mnop',
  pairedAt: 1760529700,
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

/// Opens the screen from a first screen with an Open button, so that leaving it can be seen.
Future<void> _open(
  WidgetTester tester,
  FakeSession session, {
  DateTime Function()? now,
  bool settle = true,
  ScreenKeeper? screen,
}) async {
  _tall(tester);
  await tester.pumpWidget(
    MaterialApp(
      home: Builder(
        builder: (context) => Scaffold(
          body: TextButton(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => ShowCodeScreen(
                  session: session,
                  now: now ?? () => _start,
                  screen: screen ?? FakeScreen(),
                ),
              ),
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open'));
  if (settle) {
    await tester.pumpAndSettle();
  } else {
    // A spinner never settles: the route is built by one frame, and its page by the next.
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
  }
}

/// A screen tall enough to show all of a page, as a tablet or a Mac window is.
void _tall(WidgetTester tester) {
  tester.view
    ..physicalSize = const Size(900, 1600)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
}

Future<void> _close(WidgetTester tester) async {
  await tester.pageBack();
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('makes a code, shows it as a QR code and counts the time down', (
    tester,
  ) async {
    final session = _session()..holdOffer();
    var clock = _start;
    _tall(tester);
    await tester.pumpWidget(
      MaterialApp(
        home: ShowCodeScreen(session: session, now: () => clock),
      ),
    );
    await tester.pump();
    expect(find.text('Getting a code ready…'), findsOneWidget);
    expect(find.byType(QrView), findsNothing);

    session.releaseOffer();
    await tester.pumpAndSettle();
    expect(find.byType(QrView), findsOneWidget);
    expect(find.text('Scan this on your other device'), findsOneWidget);
    expect(find.text('Works for 5:00 more'), findsOneWidget);
    expect(session.offersMade, 1);

    clock = _start.add(const Duration(seconds: 90));
    await tester.pump(const Duration(seconds: 1));
    expect(find.text('Works for 3:30 more'), findsOneWidget);
  });

  testWidgets('copies the code as text', (tester) async {
    String? copied;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied = (call.arguments as Map)['text'] as String?;
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    await _open(tester, _session());

    await tester.tap(find.text('Copy the code as text'));
    await tester.pumpAndSettle();

    expect(copied, sampleTicket);
    expect(
      find.text('Copied. Paste it in Baton on your other device.'),
      findsOneWidget,
    );
  });

  testWidgets('puts the six digits to the person, and pairs on a yes', (
    tester,
  ) async {
    final session = _session();
    await _open(tester, session);

    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();
    expect(find.text('Anna’s iPhone wants to pair'), findsOneWidget);
    expect(find.text('iPhone or iPad'), findsOneWidget);
    expect(find.text('514 643'), findsOneWidget);
    expect(find.text('iphn-abcd-efgh-ijkl-mnop'), findsOneWidget);
    expect(find.byType(QrView), findsNothing, reason: 'the code is put away');

    await tester.tap(find.text('Yes, they match'));
    await tester.pump();
    expect(session.answers, [(request: 7, accept: true)]);
    expect(find.text('Pairing…'), findsOneWidget);

    session.emit(
      const Event.offerEnded(outcome: OfferOutcome.paired(device: _anna)),
    );
    await tester.pump();
    expect(find.text('Paired with Anna’s iPhone'), findsOneWidget);

    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    expect(
      find.text('Open'),
      findsOneWidget,
      reason: 'back on the first screen',
    );
    expect(
      session.offersCancelled,
      0,
      reason: 'a code that was said yes to ends by itself',
    );
  });

  testWidgets('turns a device down on a no, and offers another code', (
    tester,
  ) async {
    final session = _session();
    await _open(tester, session);
    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();

    await tester.tap(find.text('No, cancel'));
    await tester.pump();
    expect(session.answers, [(request: 7, accept: false)]);
    session.emit(const Event.offerEnded(outcome: OfferOutcome.declined()));
    await tester.pumpAndSettle();
    expect(find.text('You said no'), findsOneWidget);
    expect(find.byType(QrView), findsNothing);

    await tester.tap(find.text('Show a new code'));
    await tester.pumpAndSettle();
    expect(session.offersMade, 2);
    expect(find.byType(QrView), findsOneWidget);
    expect(session.offersCancelled, 0);
  });

  testWidgets('takes the question away when the other device stops asking', (
    tester,
  ) async {
    final session = _session();
    await _open(tester, session);
    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();
    expect(find.text('514 643'), findsOneWidget);

    session.emit(const Event.pairingRequestEnded(id: 7));
    await tester.pumpAndSettle();

    expect(find.text('514 643'), findsNothing);
    expect(
      find.byType(QrView),
      findsOneWidget,
      reason: 'the code is still good',
    );
  });

  testWidgets('asks about one device at a time, and says that others wait', (
    tester,
  ) async {
    final session = _session();
    await _open(tester, session);
    session.emit(const Event.pairingRequested(request: sampleRequest));
    session.emit(
      const Event.pairingRequested(
        request: PairingRequest(
          id: 8,
          deviceId: 'c',
          name: 'Unknown tablet',
          platform: Platform.android,
          fingerprint: 'tabl-abcd-efgh-ijkl-mnop',
          code: '902 117',
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Anna’s iPhone wants to pair'), findsOneWidget);
    expect(find.text('Unknown tablet wants to pair'), findsNothing);
    expect(find.text('Another device is also asking.'), findsOneWidget);

    // A third arrives while the first is on the screen, and is counted.
    session.emit(
      const Event.pairingRequested(
        request: PairingRequest(
          id: 9,
          deviceId: 'd',
          name: 'Old laptop',
          platform: Platform.windows,
          fingerprint: 'lapt-abcd-efgh-ijkl-mnop',
          code: '307 884',
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('2 other devices are also asking.'), findsOneWidget);

    // The first gives up, and the second is asked about.
    session.emit(const Event.pairingRequestEnded(id: 7));
    await tester.pumpAndSettle();
    expect(find.text('Unknown tablet wants to pair'), findsOneWidget);
    expect(find.text('902 117'), findsOneWidget);
    expect(find.text('Another device is also asking.'), findsOneWidget);
  });

  testWidgets('does not wait for a request that is not waiting any more', (
    tester,
  ) async {
    final session = _session()..requestWaiting = false;
    await _open(tester, session);
    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Yes, they match'));
    await tester.pumpAndSettle();

    expect(session.answers, [(request: 7, accept: true)]);
    expect(find.text('Pairing…'), findsNothing);
    expect(find.byType(QrView), findsOneWidget);
  });

  testWidgets('says when the code ran out, and makes another', (tester) async {
    final session = _session();
    await _open(tester, session);

    session.emit(const Event.offerEnded(outcome: OfferOutcome.expired()));
    await tester.pumpAndSettle();
    expect(find.text('The code ran out'), findsOneWidget);

    await tester.tap(find.text('Show a new code'));
    await tester.pumpAndSettle();
    expect(find.byType(QrView), findsOneWidget);
    expect(session.offersMade, 2);
  });

  testWidgets('says what went wrong when pairing failed', (tester) async {
    final session = _session();
    await _open(tester, session);
    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();

    session.emit(
      const Event.offerEnded(
        outcome: OfferOutcome.failed(message: 'The connection was lost.'),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Pairing did not work'), findsOneWidget);
    expect(find.text('The connection was lost.'), findsOneWidget);
    expect(find.text('514 643'), findsNothing, reason: 'the question is gone');
  });

  testWidgets('says when a code cannot be made, and tries again', (
    tester,
  ) async {
    final session = _session()
      ..offerFailure = const BatonError(
        kind: ErrorKind.network,
        message: 'Baton could not reach the network.',
      );
    await _open(tester, session);
    expect(find.text('Could not make a code'), findsOneWidget);
    expect(find.text('Baton could not reach the network.'), findsOneWidget);

    session.offerFailure = null;
    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    expect(find.byType(QrView), findsOneWidget);
    expect(session.offersMade, 2);
  });

  testWidgets('withdraws the code when the screen is closed', (tester) async {
    final session = _session();
    await _open(tester, session);
    expect(session.offersCancelled, 0);

    await _close(tester);

    expect(session.offersCancelled, 1);
    expect(session.listening, isFalse, reason: 'the events are let go of');
  });

  testWidgets('withdraws a question that is open when the screen is closed', (
    tester,
  ) async {
    final session = _session();
    await _open(tester, session);
    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();

    await _close(tester);

    expect(session.offersCancelled, 1);
    expect(session.answers, isEmpty);
  });

  testWidgets('leaves a code alone that has just been said yes to', (
    tester,
  ) async {
    final session = _session();
    await _open(tester, session);
    session.emit(const Event.pairingRequested(request: sampleRequest));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Yes, they match'));
    await tester.pump();

    // A withdrawal that got there before the yes would win.
    await _close(tester);

    expect(session.answers, [(request: 7, accept: true)]);
    expect(session.offersCancelled, 0);
  });

  testWidgets('withdraws a code that was made after the screen had gone', (
    tester,
  ) async {
    final session = _session()..holdOffer();
    await _open(tester, session, settle: false);
    await _close(tester);
    expect(
      session.offersCancelled,
      0,
      reason: 'there is nothing to withdraw yet',
    );

    session.releaseOffer();
    await tester.pumpAndSettle();

    expect(session.offersCancelled, 1);
  });

  testWidgets('does not take the end of an earlier code for its own', (
    tester,
  ) async {
    final session = _session()..holdOffer();
    await _open(tester, session, settle: false);

    // The code that this screen's last visit withdrew ends as this one is being made.
    session.emit(const Event.offerEnded(outcome: OfferOutcome.cancelled()));
    await tester.pump();
    session.releaseOffer();
    await tester.pumpAndSettle();

    expect(find.byType(QrView), findsOneWidget);
    expect(find.text('The code was withdrawn'), findsNothing);
  });

  group('the screen of the device', () {
    testWidgets('stays on while the code is shown and answered', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      await _open(tester, session, screen: screen);
      expect(screen.asked, ['keepOn']);

      session.emit(const Event.pairingRequested(request: sampleRequest));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Yes, they match'));
      await tester.pump();
      expect(screen.asked, ['keepOn'], reason: 'asked once, and still on');

      session.emit(
        const Event.offerEnded(outcome: OfferOutcome.paired(device: _anna)),
      );
      await tester.pump();
      expect(find.text('Paired with Anna’s iPhone'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('is on from the moment a code is asked for', (tester) async {
      final screen = FakeScreen();
      final session = _session()..holdOffer();

      await _open(tester, session, screen: screen, settle: false);

      expect(find.text('Getting a code ready…'), findsOneWidget);
      expect(screen.on, isTrue);
    });

    testWidgets('is let go when the code runs out, and kept on for the next', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      await _open(tester, session, screen: screen);

      session.emit(const Event.offerEnded(outcome: OfferOutcome.expired()));
      await tester.pumpAndSettle();
      expect(find.text('The code ran out'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      await tester.tap(find.text('Show a new code'));
      await tester.pumpAndSettle();
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
    });

    testWidgets('is let go when a device is turned down or pairing fails', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      await _open(tester, session, screen: screen);
      session.emit(const Event.pairingRequested(request: sampleRequest));
      await tester.pumpAndSettle();

      session.emit(
        const Event.offerEnded(
          outcome: OfferOutcome.failed(message: 'The connection was lost.'),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('Pairing did not work'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('is let go when no code can be made, and kept on to retry', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session()
        ..offerFailure = const BatonError(
          kind: ErrorKind.network,
          message: 'Baton could not reach the network.',
        );
      await _open(tester, session, screen: screen);
      expect(find.text('Could not make a code'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      session.offerFailure = null;
      await tester.tap(find.text('Try again'));
      await tester.pumpAndSettle();
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
    });

    testWidgets('is let go when the screen is closed', (tester) async {
      final screen = FakeScreen();
      await _open(tester, _session(), screen: screen);
      expect(screen.on, isTrue);

      await _close(tester);

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('is let go when the screen is closed with a question open', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      await _open(tester, session, screen: screen);
      session.emit(const Event.pairingRequested(request: sampleRequest));
      await tester.pumpAndSettle();

      await _close(tester);

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets(
      'is not kept on again by a code made after the screen had gone',
      (tester) async {
        final screen = FakeScreen();
        final session = _session()..holdOffer();
        await _open(tester, session, screen: screen, settle: false);
        await _close(tester);
        expect(screen.asked, ['keepOn', 'letTurnOff']);

        session.releaseOffer();
        await tester.pumpAndSettle();

        expect(screen.asked, ['keepOn', 'letTurnOff']);
      },
    );

    testWidgets('is let go after the limit, if the screen is left open', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      await _open(tester, session, screen: screen);

      await tester.pump(awakeLimit + const Duration(seconds: 1));
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      // Somebody is there after all: a device asks, and the person has the screen again.
      session.emit(const Event.pairingRequested(request: sampleRequest));
      await tester.pumpAndSettle();
      expect(find.text('514 643'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
    });

    testWidgets('counts the limit again from each step', (tester) async {
      final screen = FakeScreen();
      final session = _session();
      await _open(tester, session, screen: screen);

      // A device asks just before the limit, and the person takes most of a limit again to
      // answer: the pairing is still going on after more than the limit has passed.
      await tester.pump(awakeLimit - const Duration(minutes: 1));
      session.emit(const Event.pairingRequested(request: sampleRequest));
      await tester.pumpAndSettle();
      await tester.pump(awakeLimit - const Duration(minutes: 1));
      expect(screen.asked, ['keepOn'], reason: 'asked once, and still on');

      // Answering is a step too.
      await tester.tap(find.text('Yes, they match'));
      await tester.pump();
      await tester.pump(awakeLimit - const Duration(minutes: 1));
      expect(screen.asked, ['keepOn'], reason: 'asked once, and still on');

      // And with no step after it the screen is let go.
      await tester.pump(const Duration(minutes: 2));
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });
  });
}
