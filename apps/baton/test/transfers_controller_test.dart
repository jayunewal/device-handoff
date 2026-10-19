// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/awake.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/transfers_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Kitchen iPad',
    platform: Platform.ipadOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

Future<TransfersController> _controller(
  FakeSession session, {
  FakeScreen? screen,
}) async {
  final controller = TransfersController(
    session,
    screen: screen ?? FakeScreen(),
  );
  addTearDown(controller.dispose);
  await controller.start();
  return controller;
}

/// Hands [request] to the controller, as Baton does when a device asks to send files.
Future<void> _ask(FakeSession session, TransferRequest request) async {
  session.emit(Event.transferRequested(request: request));
  await pumpEventQueue();
}

Future<void> _end(FakeSession session, TransferEnd end) async {
  session.emit(Event.transferEnded(end: end));
  await pumpEventQueue();
}

void main() {
  test(
    'a request waits for the person, and the screens are told that it came',
    () async {
      final session = _session();
      final controller = await _controller(session);
      final asked = <TransferRequest>[];
      final heard = controller.asked.listen(asked.add);
      addTearDown(heard.cancel);
      final request = sampleTransfer();

      await _ask(session, request);

      expect(controller.requests, [request]);
      expect(asked, [request]);
      expect(controller.coming, isEmpty);
      expect(controller.ended, isEmpty);
    },
  );

  test(
    'yes is told to Baton, and the transfer is on its way until it ends',
    () async {
      final session = _session();
      final controller = await _controller(session);
      final request = sampleTransfer(id: 7);
      await _ask(session, request);

      expect(controller.answer(request, accept: true), isTrue);

      expect(session.transferAnswers, [(request: 7, accept: true)]);
      expect(controller.requests, isEmpty);
      expect(controller.coming, [request]);

      final end = sampleTransferEnd(request);
      await _end(session, end);

      expect(controller.coming, isEmpty);
      expect(controller.ended, [end]);
    },
  );

  test('no is told to Baton, and nothing is on its way', () async {
    final session = _session();
    final controller = await _controller(session);
    final request = sampleTransfer();
    await _ask(session, request);

    expect(controller.answer(request, accept: false), isTrue);

    expect(session.transferAnswers, [(request: 3, accept: false)]);
    expect(controller.requests, isEmpty);
    expect(controller.coming, isEmpty);
    expect(controller.ended, isEmpty);
  });

  test('a request that was not waiting any more is not on its way, whatever the answer', () async {
    final session = _session()..transferWaiting = false;
    final controller = await _controller(session);
    final request = sampleTransfer();
    await _ask(session, request);

    expect(controller.answer(request, accept: true), isFalse);

    expect(controller.requests, isEmpty);
    expect(controller.coming, isEmpty);
  });

  test('a request is answered once', () async {
    final session = _session();
    final controller = await _controller(session);
    final request = sampleTransfer();
    await _ask(session, request);

    expect(controller.answer(request, accept: true), isTrue);
    expect(controller.answer(request, accept: false), isFalse);

    expect(session.transferAnswers, [(request: 3, accept: true)]);
    expect(controller.coming, [request]);
  });

  test(
    'a request that the other device gave up goes without an answer',
    () async {
      final session = _session();
      final controller = await _controller(session);
      await _ask(session, sampleTransfer(id: 3));
      await _ask(session, sampleTransfer(id: 4, from: 'Desk Mac'));

      session.emit(const Event.transferRequestEnded(id: 3));
      await pumpEventQueue();

      expect(controller.requests.map((request) => request.id), [4]);
      expect(session.transferAnswers, isEmpty);
    },
  );

  test('the end of a request that is not waiting tells no one', () async {
    final session = _session();
    final controller = await _controller(session);
    final request = sampleTransfer(id: 3);
    await _ask(session, request);
    var told = 0;
    controller.addListener(() => told++);

    // A request that was never heard of.
    session.emit(const Event.transferRequestEnded(id: 9));
    await pumpEventQueue();

    expect(told, 0);
    expect(controller.requests, [request]);
  });

  test('two requests wait side by side, oldest first', () async {
    final session = _session();
    final controller = await _controller(session);
    final first = sampleTransfer(id: 3);
    final second = sampleTransfer(id: 4, from: 'Desk Mac');
    await _ask(session, first);
    await _ask(session, second);

    expect(controller.requests, [first, second]);
    expect(controller.answer(second, accept: true), isTrue);
    expect(controller.requests, [first]);
    expect(controller.coming, [second]);
    expect(controller.answer(first, accept: false), isTrue);
    expect(session.transferAnswers, [
      (request: 4, accept: true),
      (request: 3, accept: false),
    ]);
  });

  test('a request that is heard twice waits once', () async {
    final session = _session();
    final controller = await _controller(session);
    final asked = <TransferRequest>[];
    final heard = controller.asked.listen(asked.add);
    addTearDown(heard.cancel);

    await _ask(session, sampleTransfer());
    await _ask(session, sampleTransfer());

    expect(controller.requests, hasLength(1));
    expect(asked, hasLength(1));
  });

  test('how transfers ended is kept for a while, newest first, until it is put away', () async {
    final session = _session();
    final controller = await _controller(session);
    final ended = [
      for (var id = 1; id <= TransfersController.keepEnded + 2; id++)
        sampleTransferEnd(sampleTransfer(id: id)),
    ];

    for (final end in ended) {
      await _end(session, end);
    }

    expect(
      controller.ended,
      ended.reversed.take(TransfersController.keepEnded).toList(),
    );
    controller.dismiss(ended.last);
    expect(controller.ended.map((end) => end.request), [
      for (var id = ended.length - 1; id > 2; id--) id,
    ]);
    // Putting away what is gone already changes nothing.
    controller.dismiss(ended.last);
    expect(controller.ended, hasLength(TransfersController.keepEnded - 1));
  });

  test('the last five transfers that ended are kept', () {
    // The architecture notes say five, and the test above counts with the constant.
    expect(TransfersController.keepEnded, 5);
  });

  test('a transfer that was given up ends as that, and one that lost files says so', () async {
    final session = _session();
    final controller = await _controller(session);
    final request = sampleTransfer();
    final dropped = sampleTransferEnd(
      request,
      result: const TransferResult.dropped(),
    );
    final lost = sampleTransferEnd(
      request,
      result: const TransferResult.received(failed: 1),
    );

    await _end(session, dropped);
    await _end(session, lost);

    expect(controller.ended, [lost, dropped]);
  });

  test('other events are not for this list', () async {
    final session = _session();
    final controller = await _controller(session);
    var changed = 0;
    controller.addListener(() => changed++);

    session.emit(const Event.resync());
    session.emit(const Event.devicesChanged());
    await pumpEventQueue();

    expect(changed, 0);
  });

  test(
    'whether this device takes files, and where they go, come from Baton',
    () async {
      final session = _session();
      final controller = await _controller(session);

      expect(controller.taking, isA<Receiving_On>());
      expect(controller.folder, '/Users/test/Downloads');

      session
        ..taking = const Receiving.unusable(message: 'The folder is not there.')
        ..receiveFolder = null;
      expect(
        controller.taking,
        isA<Receiving_Unusable>().having(
          (taking) => taking.message,
          'message',
          'The folder is not there.',
        ),
      );
      expect(controller.folder, isNull);
    },
  );

  test('it stops listening when it is let go of', () async {
    final session = _session();
    final controller = TransfersController(session, screen: FakeScreen());
    await controller.start();
    expect(session.listening, isTrue);

    controller.dispose();
    await pumpEventQueue();

    expect(session.listening, isFalse);
    // Events that come after that are nobody's business, and do no harm.
    session.emit(Event.transferRequested(request: sampleTransfer()));
    await pumpEventQueue();
  });

  group('the screen of a phone', () {
    test('stays on from the request until the transfer ends', () async {
      final session = _session();
      final screen = FakeScreen();
      final controller = await _controller(session, screen: screen);
      final request = sampleTransfer();
      expect(screen.asked, isEmpty);

      await _ask(session, request);
      expect(screen.asked, ['keepOn']);

      controller.answer(request, accept: true);
      expect(screen.asked, ['keepOn'], reason: 'the transfer is on its way');

      await _end(session, sampleTransferEnd(request));
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    test(
      'goes back to locking when the answer is no, or the request goes',
      () async {
        final session = _session();
        final screen = FakeScreen();
        final controller = await _controller(session, screen: screen);
        final first = sampleTransfer(id: 3);
        final second = sampleTransfer(id: 4);
        await _ask(session, first);
        await _ask(session, second);

        controller.answer(first, accept: false);
        expect(screen.asked, ['keepOn'], reason: 'another request waits');

        session.emit(const Event.transferRequestEnded(id: 4));
        await pumpEventQueue();
        expect(screen.asked, ['keepOn', 'letTurnOff']);
      },
    );

    testWidgets(
      'stays on for the limit and no longer when nothing says how far a transfer has got',
      (tester) async {
        final session = _session();
        final screen = FakeScreen();
        final controller = TransfersController(session, screen: screen);
        addTearDown(controller.dispose);
        await controller.start();
        final request = sampleTransfer();
        session.emit(Event.transferRequested(request: request));
        await tester.pump();
        controller.answer(request, accept: true);

        await tester.pump(awakeLimit - const Duration(seconds: 1));
        expect(screen.on, isTrue);

        await tester.pump(const Duration(seconds: 2));
        expect(screen.asked, ['keepOn', 'letTurnOff']);

        // The end of the transfer finds the screen let go already, and does not let go twice.
        session.emit(Event.transferEnded(end: sampleTransferEnd(request)));
        await tester.pump();
        expect(screen.asked, ['keepOn', 'letTurnOff']);
      },
    );

    testWidgets(
      'is held for the limit again from the yes, and not only from the request',
      (tester) async {
        final session = _session();
        final screen = FakeScreen();
        final controller = TransfersController(session, screen: screen);
        addTearDown(controller.dispose);
        await controller.start();
        final request = sampleTransfer();
        session.emit(Event.transferRequested(request: request));
        await tester.pump();

        // The person takes their time and says yes just before the hold from the request runs out.
        await tester.pump(awakeLimit - const Duration(seconds: 1));
        controller.answer(request, accept: true);

        await tester.pump(const Duration(seconds: 30));
        expect(
          screen.on,
          isTrue,
          reason: 'the limit from the request has passed',
        );

        await tester.pump(awakeLimit);
        expect(screen.asked, ['keepOn', 'letTurnOff']);
      },
    );
  });
}
