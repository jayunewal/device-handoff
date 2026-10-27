// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/window_sound.dart';
import 'package:baton/src/windows_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

const _ipad = Device(
  id: 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
  lastSeenAt: 1760529600,
);

const _mac = Device(
  id: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  name: 'Anna’s Mac',
  platform: Platform.macOs,
  fingerprint: 'mnop-qrst-uvwx-yzab-cdef',
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
  pairedDevices: [_ipad],
);

/// Starts a controller on [session]. Without an [openingLimit] it has the one that it has in the
/// app, which the tests of the time that a window may take to begin rely on.
Future<WindowsController> _controller(
  FakeSession session, {
  FakeScreen? screen,
  Duration? openingLimit,
}) async {
  final keeper = screen ?? FakeScreen();
  final controller = openingLimit == null
      ? WindowsController(session, screen: keeper)
      : WindowsController(session, screen: keeper, openingLimit: openingLimit);
  addTearDown(controller.dispose);
  await controller.start();
  return controller;
}

/// Hands [request] to the controller, as Baton does when a device offers a window.
Future<void> _offer(FakeSession session, WindowRequest request) async {
  session.emit(Event.windowRequested(request: request));
  await pumpEventQueue();
}

Future<void> _begin(FakeSession session, WindowSession window) async {
  session.emit(Event.windowStarted(session: window));
  await pumpEventQueue();
}

Future<void> _end(FakeSession session, int id, WindowEnd end) async {
  session.emit(Event.windowEnded(id: id, end: end));
  await pumpEventQueue();
}

void main() {
  group('an offer to show a window', () {
    test(
      'waits for the person, and the screens are told that it came',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final asked = <WindowRequest>[];
        final heard = controller.asked.listen(asked.add);
        addTearDown(heard.cancel);

        await _offer(session, sampleWindowRequest);

        expect(controller.requests, [sampleWindowRequest]);
        expect(asked, [sampleWindowRequest]);
        expect(controller.opening, isNull);
        expect(controller.sessions, isEmpty);
        expect(controller.notes, isEmpty);
      },
    );

    test('that comes twice is one offer', () async {
      final session = _session();
      final controller = await _controller(session);
      final asked = <WindowRequest>[];
      final heard = controller.asked.listen(asked.add);
      addTearDown(heard.cancel);

      await _offer(session, sampleWindowRequest);
      await _offer(session, sampleWindowRequest);

      expect(controller.requests, [sampleWindowRequest]);
      expect(asked, hasLength(1));
    });

    test(
      'is said yes to, and the window is being opened until it begins',
      () async {
        final session = _session();
        final controller = await _controller(session);
        await _offer(session, sampleWindowRequest);

        expect(controller.answer(sampleWindowRequest, accept: true), isTrue);

        expect(session.windowAnswers, [(request: 3, accept: true)]);
        expect(controller.requests, isEmpty);
        expect(controller.opening, sampleWindowRequest);
        expect(controller.sessions, isEmpty);

        final window = sampleWindowSession();
        final begun = <WindowSession>[];
        final heard = controller.started.listen(begun.add);
        addTearDown(heard.cancel);
        await _begin(session, window);

        expect(controller.opening, isNull);
        expect(controller.sessions, [window]);
        expect(begun, [window]);
      },
    );

    test('is said no to, and nothing is opened', () async {
      final session = _session();
      final controller = await _controller(session);
      await _offer(session, sampleWindowRequest);

      expect(controller.answer(sampleWindowRequest, accept: false), isTrue);

      expect(session.windowAnswers, [(request: 3, accept: false)]);
      expect(controller.requests, isEmpty);
      expect(controller.opening, isNull);
      expect(controller.notes, isEmpty);
    });

    test('is answered once', () async {
      final session = _session();
      final controller = await _controller(session);
      await _offer(session, sampleWindowRequest);

      expect(controller.answer(sampleWindowRequest, accept: true), isTrue);
      expect(controller.answer(sampleWindowRequest, accept: false), isFalse);

      expect(session.windowAnswers, [(request: 3, accept: true)]);
      expect(controller.opening, sampleWindowRequest);
    });

    test(
      'that the other device took back is not opened, and the person is told',
      () async {
        final session = _session()..windowRequestWaiting = false;
        final controller = await _controller(session);
        await _offer(session, sampleWindowRequest);

        expect(controller.answer(sampleWindowRequest, accept: true), isFalse);

        expect(controller.requests, isEmpty);
        expect(controller.opening, isNull);
        expect(controller.notes, hasLength(1));
        expect(controller.notes.single, isA<WindowWithdrawn>());
        expect(
          (controller.notes.single as WindowWithdrawn).request,
          sampleWindowRequest,
        );
      },
    );

    test(
      'that the other device took back, and was said no to, says nothing',
      () async {
        final session = _session()..windowRequestWaiting = false;
        final controller = await _controller(session);
        await _offer(session, sampleWindowRequest);

        expect(controller.answer(sampleWindowRequest, accept: false), isFalse);

        expect(controller.requests, isEmpty);
        expect(controller.notes, isEmpty);
      },
    );

    test(
      'goes without an answer when the other device takes it back',
      () async {
        final session = _session();
        final controller = await _controller(session);
        const other = WindowRequest(
          id: 4,
          deviceId: 'd1',
          deviceName: 'Desk PC',
          display: false,
          width: 800,
          height: 600,
          control: false,
        );
        await _offer(session, sampleWindowRequest);
        await _offer(session, other);

        session.emit(const Event.windowRequestEnded(id: 3));
        await pumpEventQueue();

        expect(controller.requests, [other]);
        expect(controller.notes, isEmpty);
      },
    );

    test('that ends without being on the list changes nothing', () async {
      final session = _session();
      final controller = await _controller(session);
      var told = 0;
      controller.addListener(() => told++);

      session.emit(const Event.windowRequestEnded(id: 9));
      await pumpEventQueue();

      expect(told, 0);
      expect(controller.notes, isEmpty);
    });

    test('that was said yes to and could not be shown says why', () async {
      final session = _session();
      final controller = await _controller(session);
      await _offer(session, sampleWindowRequest);
      controller.answer(sampleWindowRequest, accept: true);

      session.emit(
        const Event.windowRequestEnded(
          id: 3,
          failure: 'This device cannot show windows.',
        ),
      );
      await pumpEventQueue();

      expect(controller.opening, isNull);
      expect(controller.notes, hasLength(1));
      expect(controller.notes.single, isA<WindowFailed>());
      expect(
        (controller.notes.single as WindowFailed).message,
        'This device cannot show windows.',
      );
    });

    test('that was said yes to and ends without a failure is no longer being opened', () async {
      final session = _session();
      final controller = await _controller(session);
      await _offer(session, sampleWindowRequest);
      controller.answer(sampleWindowRequest, accept: true);

      session.emit(const Event.windowRequestEnded(id: 3));
      await pumpEventQueue();

      expect(controller.opening, isNull);
      expect(controller.notes, isEmpty);
    });

    test(
      'that another offer ends does not stop the one that is being opened',
      () async {
        final session = _session();
        final controller = await _controller(session);
        await _offer(session, sampleWindowRequest);
        controller.answer(sampleWindowRequest, accept: true);

        session.emit(const Event.windowRequestEnded(id: 8));
        await pumpEventQueue();

        expect(controller.opening, sampleWindowRequest);
      },
    );

    testWidgets('is given up on when the window does not begin in time', (
      tester,
    ) async {
      final session = _session();
      final controller = await _controller(session);
      session.emit(Event.windowRequested(request: sampleWindowRequest));
      await tester.pump();
      controller.answer(sampleWindowRequest, accept: true);

      await tester.pump(const Duration(seconds: 19));
      expect(controller.opening, sampleWindowRequest);
      expect(controller.notes, isEmpty);

      await tester.pump(const Duration(seconds: 2));
      expect(controller.opening, isNull);
      expect(controller.notes, hasLength(1));
      expect(controller.notes.single, isA<WindowNotOpened>());
      expect(
        (controller.notes.single as WindowNotOpened).request,
        sampleWindowRequest,
      );
    });

    testWidgets(
      'that begins after it was given up on is shown, and not told as a failure',
      (tester) async {
        final session = _session();
        final controller = await _controller(
          session,
          openingLimit: const Duration(seconds: 5),
        );
        session.emit(Event.windowRequested(request: sampleWindowRequest));
        await tester.pump();
        controller.answer(sampleWindowRequest, accept: true);
        await tester.pump(const Duration(seconds: 6));
        expect(controller.notes.single, isA<WindowNotOpened>());
        final begun = <WindowSession>[];
        final heard = controller.started.listen(begun.add);
        addTearDown(heard.cancel);

        final window = sampleWindowSession();
        session.emit(Event.windowStarted(session: window));
        await tester.pump();

        expect(controller.sessions, [window]);
        expect(begun, [window]);
        expect(controller.notes, isEmpty);
      },
    );

    testWidgets(
      'that begins after it was given up on takes away its own note, and not the note of another device',
      (tester) async {
        final session = _session();
        final controller = await _controller(
          session,
          openingLimit: const Duration(seconds: 5),
        );
        const other = WindowRequest(
          id: 4,
          deviceId: 'd1',
          deviceName: 'Desk PC',
          display: false,
          width: 800,
          height: 600,
          control: false,
        );
        session.emit(Event.windowRequested(request: sampleWindowRequest));
        session.emit(const Event.windowRequested(request: other));
        await tester.pump();
        controller.answer(sampleWindowRequest, accept: true);
        await tester.pump(const Duration(seconds: 6));
        controller.answer(other, accept: true);
        await tester.pump(const Duration(seconds: 6));
        expect(
          controller.notes.map((note) => (note as WindowNotOpened).request),
          [other, sampleWindowRequest],
        );

        // The window of the first device begins; the second one never did.
        session.emit(Event.windowStarted(session: sampleWindowSession()));
        await tester.pump();

        expect(
          controller.notes.map((note) => (note as WindowNotOpened).request),
          [other],
        );
      },
    );

    testWidgets('that is opened in time is not given up on later', (
      tester,
    ) async {
      final session = _session();
      final controller = await _controller(
        session,
        openingLimit: const Duration(seconds: 5),
      );
      session.emit(Event.windowRequested(request: sampleWindowRequest));
      await tester.pump();
      controller.answer(sampleWindowRequest, accept: true);
      session.emit(Event.windowStarted(session: sampleWindowSession()));
      await tester.pump(const Duration(seconds: 1));

      await tester.pump(const Duration(seconds: 30));

      expect(controller.notes, isEmpty);
    });

    testWidgets('is not given up on after another window was said yes to', (
      tester,
    ) async {
      final session = _session();
      final controller = await _controller(
        session,
        openingLimit: const Duration(seconds: 10),
      );
      session.emit(Event.windowRequested(request: sampleWindowRequest));
      await tester.pump();
      controller.answer(sampleWindowRequest, accept: true);
      await tester.pump(const Duration(seconds: 8));
      const next = WindowRequest(
        id: 4,
        deviceId: 'd1',
        deviceName: 'Desk PC',
        display: false,
        width: 800,
        height: 600,
        control: false,
      );
      session.emit(const Event.windowRequested(request: next));
      await tester.pump();
      controller.answer(next, accept: true);

      // The first answer's time has passed, and the second's has not.
      await tester.pump(const Duration(seconds: 5));

      expect(controller.opening, next);
      expect(controller.notes, isEmpty);

      // Nothing is left pending when the window is let go of.
      controller.dispose();
    });

    testWidgets(
      'is not given up on at the time that was up for a window that began before it',
      (tester) async {
        final session = _session();
        final controller = await _controller(
          session,
          openingLimit: const Duration(seconds: 10),
        );
        session.emit(Event.windowRequested(request: sampleWindowRequest));
        await tester.pump();
        controller.answer(sampleWindowRequest, accept: true);
        // The first window begins, and the person says yes to another one 8 seconds after the
        // first yes.
        session.emit(Event.windowStarted(session: sampleWindowSession()));
        await tester.pump(const Duration(seconds: 8));
        const next = WindowRequest(
          id: 4,
          deviceId: 'd1',
          deviceName: 'Desk PC',
          display: false,
          width: 800,
          height: 600,
          control: false,
        );
        session.emit(const Event.windowRequested(request: next));
        await tester.pump();
        controller.answer(next, accept: true);

        // The time of the first answer is over, and that of the second is not.
        await tester.pump(const Duration(seconds: 5));
        expect(controller.opening, next);
        expect(controller.notes, isEmpty);

        // The second is given up on when its own time is over.
        await tester.pump(const Duration(seconds: 6));
        expect(controller.opening, isNull);
        expect(controller.notes.single, isA<WindowNotOpened>());
        controller.dispose();
      },
    );
  });

  group('a window that runs', () {
    test('that is shown here is told to the screens', () async {
      final session = _session();
      final controller = await _controller(session);
      final begun = <WindowSession>[];
      final heard = controller.started.listen(begun.add);
      addTearDown(heard.cancel);
      final window = sampleWindowSession(id: 4, screen: 9);

      await _begin(session, window);

      expect(controller.sessions, [window]);
      expect(begun, [window]);
    });

    test(
      'that is shared from here is on the list, and not told as one to draw',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final begun = <WindowSession>[];
        final heard = controller.started.listen(begun.add);
        addTearDown(heard.cancel);
        final window = sampleWindowSession(role: WindowRole.sharing);

        await _begin(session, window);

        expect(controller.sessions, [window]);
        expect(begun, isEmpty);
      },
    );

    test('that begins twice is one window', () async {
      final session = _session();
      final controller = await _controller(session);
      final begun = <WindowSession>[];
      final heard = controller.started.listen(begun.add);
      addTearDown(heard.cancel);
      final window = sampleWindowSession();

      await _begin(session, window);
      await _begin(session, window);

      expect(controller.sessions, [window]);
      expect(begun, hasLength(1));
    });

    test('that begins while another offer is being opened leaves the offer alone if it is shared', () async {
      final session = _session();
      final controller = await _controller(session);
      await _offer(session, sampleWindowRequest);
      controller.answer(sampleWindowRequest, accept: true);

      await _begin(session, sampleWindowSession(role: WindowRole.sharing));

      expect(controller.opening, sampleWindowRequest);
    });

    test(
      'that the other device ends is taken off, and the person is told',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final window = sampleWindowSession();
        await _begin(session, window);

        await _end(session, 1, const WindowEnd.otherDevice());

        expect(controller.sessions, isEmpty);
        expect(controller.notes, hasLength(1));
        final note = controller.notes.single as WindowOver;
        expect(note.session, window);
        expect(note.end, const WindowEnd.otherDevice());
      },
    );

    test('that is lost or fails is told the same way', () async {
      final session = _session();
      final controller = await _controller(session);
      await _begin(session, sampleWindowSession(id: 1));
      await _begin(
        session,
        sampleWindowSession(id: 2, role: WindowRole.sharing),
      );

      await _end(session, 1, const WindowEnd.lost());
      await _end(
        session,
        2,
        const WindowEnd.failed(message: 'Something broke.'),
      );

      expect(controller.sessions, isEmpty);
      expect(controller.notes.map((note) => (note as WindowOver).end), [
        const WindowEnd.failed(message: 'Something broke.'),
        const WindowEnd.lost(),
      ]);
    });

    test('that this device ended says nothing', () async {
      final session = _session();
      final controller = await _controller(session);
      await _begin(session, sampleWindowSession());

      await _end(session, 1, const WindowEnd.thisDevice());

      expect(controller.sessions, isEmpty);
      expect(controller.notes, isEmpty);
    });

    test('that was never on the list ends without a word', () async {
      final session = _session();
      final controller = await _controller(session);
      var told = 0;
      controller.addListener(() => told++);

      await _end(session, 5, const WindowEnd.otherDevice());

      expect(told, 0);
      expect(controller.notes, isEmpty);
    });

    test('ends the one that was asked for, and no other', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1, role: WindowRole.sharing);
      final second = sampleWindowSession(id: 2);
      await _begin(session, first);
      await _begin(session, second);

      expect(controller.end(second), isTrue);

      expect(session.windowsEnded, [2]);
      expect(controller.sessions, [first]);
    });

    test('that is ended is off the list at once, and its end is not told as one to mind', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      controller.end(window);

      expect(controller.sessions, isEmpty);
      expect(told, 1);

      await _end(session, 1, const WindowEnd.thisDevice());

      expect(controller.notes, isEmpty);
      expect(told, 1);
    });

    test('that is ended twice is told to the screens once', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      await _begin(session, window);
      controller.end(window);
      var told = 0;
      controller.addListener(() => told++);

      controller.end(window);

      expect(told, 0);
      expect(controller.sessions, isEmpty);
    });

    test('that was not running is said so by Baton', () async {
      final session = _session()..windowRuns = false;
      final controller = await _controller(session);

      expect(controller.end(sampleWindowSession()), isFalse);
      expect(session.windowsEnded, [1]);
    });

    test('has its end found by the window, until it is put away', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1);
      final second = sampleWindowSession(id: 2);
      await _begin(session, first);
      await _begin(session, second);
      await _end(session, 2, const WindowEnd.lost());

      expect(controller.over(first), isNull);
      final over = controller.over(second);
      expect(over, isNotNull);
      expect(over!.end, const WindowEnd.lost());

      controller.dismiss(over);

      expect(controller.over(second), isNull);
      expect(controller.notes, isEmpty);
    });
  });

  group('the picture of a window that runs', () {
    Future<void> say(FakeSession session, int id, int width, int height) async {
      session.emit(Event.windowPicture(id: id, width: width, height: height));
      await pumpEventQueue();
    }

    test('is not known until its source has said how large it is', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();

      expect(controller.pictureOf(window), isNull);
      await _begin(session, window);

      expect(controller.pictureOf(window), isNull);
    });

    test('is as large as the source says, and the screens are told', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      await say(session, 1, 1920, 1200);

      expect(controller.pictureOf(window), (width: 1920, height: 1200));
      expect(told, 1);
    });

    test('changes when the source says another size', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      await _begin(session, window);
      await say(session, 1, 1920, 1200);
      var told = 0;
      controller.addListener(() => told++);

      await say(session, 1, 800, 800);

      expect(controller.pictureOf(window), (width: 800, height: 800));
      expect(told, 1);

      // Only one of the two edges changes.
      await say(session, 1, 800, 600);
      expect(controller.pictureOf(window), (width: 800, height: 600));
      await say(session, 1, 640, 600);
      expect(controller.pictureOf(window), (width: 640, height: 600));
      expect(told, 3);
    });

    test(
      'is not told to the screens again when the size is the same',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final window = sampleWindowSession();
        await _begin(session, window);
        await say(session, 1, 1920, 1200);
        var told = 0;
        controller.addListener(() => told++);

        await say(session, 1, 1920, 1200);

        expect(told, 0);
        expect(controller.pictureOf(window), (width: 1920, height: 1200));
      },
    );

    test('is kept for each window by itself', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1);
      final second = sampleWindowSession(id: 2);
      await _begin(session, first);
      await _begin(session, second);

      await say(session, 2, 640, 480);
      expect(controller.pictureOf(first), isNull);
      expect(controller.pictureOf(second), (width: 640, height: 480));

      await say(session, 1, 1280, 720);
      expect(controller.pictureOf(first), (width: 1280, height: 720));
      expect(controller.pictureOf(second), (width: 640, height: 480));
    });

    test('is not kept for a window that is not running', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      var told = 0;
      controller.addListener(() => told++);

      // Before it began, which Baton does not do, and after it ended.
      await say(session, 1, 1920, 1200);
      expect(controller.pictureOf(window), isNull);
      expect(told, 0);
      await _begin(session, window);
      await _end(session, 1, const WindowEnd.otherDevice());
      final toldBefore = told;
      await say(session, 1, 1920, 1200);

      expect(controller.pictureOf(window), isNull);
      expect(told, toldBefore);
    });

    test('is not kept for a window that is not the one that runs', () async {
      final session = _session();
      final controller = await _controller(session);
      final running = sampleWindowSession(id: 1);
      final other = sampleWindowSession(id: 2);
      await _begin(session, running);
      var told = 0;
      controller.addListener(() => told++);

      // A window that is not here says how large its picture is, while another runs.
      await say(session, 2, 1920, 1200);

      expect(controller.pictureOf(other), isNull);
      expect(controller.pictureOf(running), isNull);
      expect(told, 0);
    });

    test('is not kept for a window that is shared from here', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(role: WindowRole.sharing);
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      await say(session, 1, 1920, 1200);

      expect(controller.pictureOf(window), isNull);
      expect(told, 0);
    });

    test('is not changed to a size that is nothing', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      await _begin(session, window);
      await say(session, 1, 1920, 1200);
      var told = 0;
      controller.addListener(() => told++);

      for (final size in [
        (0, 1200),
        (1920, 0),
        (0, 0),
        (-1, 1200),
        (1920, -5),
      ]) {
        await say(session, 1, size.$1, size.$2);
      }

      expect(controller.pictureOf(window), (width: 1920, height: 1200));
      expect(told, 0);
    });

    test('is forgotten when the window ends', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      await _begin(session, window);
      await say(session, 1, 1920, 1200);

      await _end(session, 1, const WindowEnd.lost());

      expect(controller.pictureOf(window), isNull);
      // A window that begins with the same number again, which Baton does not do, starts over.
      await _begin(session, window);
      expect(controller.pictureOf(window), isNull);
    });

    test('is forgotten when the person ends the window', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession();
      await _begin(session, window);
      await say(session, 1, 1920, 1200);

      controller.end(window);

      expect(controller.pictureOf(window), isNull);
      await _begin(session, window);
      expect(controller.pictureOf(window), isNull);
    });
  });

  group('what the person does on a window that runs', () {
    const noKeys = WindowModifiers(
      shift: false,
      control: false,
      option: false,
      command: false,
      capsLock: false,
    );
    const touch = WindowInput.pointer(
      x: 0.25,
      y: 0.5,
      modifiers: noKeys,
      action: WindowPointer.down(button: WindowButton.left),
    );
    const press = WindowInput.key(
      code: 4,
      down: true,
      modifiers: noKeys,
      repeat: false,
      text: 'a',
    );

    test('is given to Baton with the number of the window', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(id: 5, control: true);
      await _begin(session, window);

      expect(controller.input(window, touch), isTrue);
      expect(controller.input(window, press), isTrue);

      expect(session.windowInputs, [
        (session: 5, input: touch),
        (session: 5, input: press),
      ]);
    });

    test('is taken or not as Baton says', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      await _begin(session, window);

      session.windowInputTaken = false;
      expect(controller.input(window, touch), isFalse);
      session.windowInputTaken = true;
      expect(controller.input(window, touch), isTrue);

      // Not being taken is not the end of the window.
      expect(controller.sessions, [window]);
      expect(controller.notes, isEmpty);
    });

    test('goes to the window that it is for, and to no other', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1, control: true);
      final second = sampleWindowSession(id: 2, control: true);
      await _begin(session, first);
      await _begin(session, second);

      controller.input(second, touch);
      controller.input(first, press);

      expect(session.windowInputs, [
        (session: 2, input: touch),
        (session: 1, input: press),
      ]);
    });

    test('is let go of for the window that it is for, and no other', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1, control: true);
      final second = sampleWindowSession(id: 2, control: true);
      await _begin(session, first);
      await _begin(session, second);

      controller.releaseInput(second);

      expect(session.windowInputsReleased, [2]);
      expect(session.windowInputs, isEmpty);
    });

    test('is not a change to what the screens show', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      controller.input(window, touch);
      controller.releaseInput(window);

      expect(told, 0);
    });
  });

  group('what the person lifts of the privacy of a screen', () {
    const dark = (screen: false, input: false);

    test('is off when the session starts', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true, private: true);
      await _begin(session, window);

      expect(controller.liftedOf(window), dark);
      expect(session.windowLifts, isEmpty);
    });

    test('is switched one side at a time, and each switch tells the other device both', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(id: 5, control: true, private: true);
      await _begin(session, window);

      expect(controller.lift(window, screen: true), isTrue);
      expect(controller.liftedOf(window), (screen: true, input: false));
      expect(controller.lift(window, input: true), isTrue);
      expect(controller.liftedOf(window), (screen: true, input: true));
      expect(controller.lift(window, screen: false), isTrue);
      expect(controller.liftedOf(window), (screen: false, input: true));
      expect(controller.lift(window, input: false), isTrue);
      expect(controller.liftedOf(window), dark);

      expect(session.windowLifts, [
        (session: 5, screen: true, input: false),
        (session: 5, screen: true, input: true),
        (session: 5, screen: false, input: true),
        (session: 5, screen: false, input: false),
      ]);
    });

    test(
      'can be switched on and off any number of times in one session',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final window = sampleWindowSession(control: true, private: true);
        await _begin(session, window);

        for (var round = 0; round < 6; round++) {
          expect(controller.lift(window, screen: true, input: true), isTrue);
          expect(controller.liftedOf(window), (screen: true, input: true));
          expect(controller.lift(window, screen: false, input: false), isTrue);
          expect(controller.liftedOf(window), dark);
        }

        expect(session.windowLifts.length, 12);
        expect(controller.sessions, [window]);
      },
    );

    test(
      'tells the other device nothing when it says what already is',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final window = sampleWindowSession(control: true, private: true);
        await _begin(session, window);
        var told = 0;
        controller.addListener(() => told++);

        expect(controller.lift(window), isTrue);
        expect(controller.lift(window, screen: false, input: false), isTrue);
        expect(controller.lift(window, screen: true), isTrue);
        expect(controller.lift(window, screen: true), isTrue);
        expect(controller.lift(window, screen: true, input: false), isTrue);

        expect(session.windowLifts, [(session: 1, screen: true, input: false)]);
        expect(told, 1, reason: 'the screens are told of the one change');
      },
    );

    test('stays as it was when the other device could not be told', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true, private: true);
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      session.windowLiftTaken = false;
      expect(controller.lift(window, screen: true), isFalse);
      expect(controller.liftedOf(window), dark);
      expect(told, 0);
      // Not being told is not the end of the window.
      expect(controller.sessions, [window]);
      expect(controller.notes, isEmpty);

      // The same switch again tells it once more, and works when it can.
      session.windowLiftTaken = true;
      expect(controller.lift(window, screen: true), isTrue);
      expect(controller.liftedOf(window), (screen: true, input: false));
      expect(session.windowLifts, [
        (session: 1, screen: true, input: false),
        (session: 1, screen: true, input: false),
      ]);
    });

    test('goes to the window that it is for, and to no other', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1, control: true, private: true);
      final second = sampleWindowSession(id: 2, control: true, private: true);
      await _begin(session, first);
      await _begin(session, second);

      controller.lift(second, input: true);

      expect(controller.liftedOf(second), (screen: false, input: true));
      expect(controller.liftedOf(first), dark);
      expect(session.windowLifts, [(session: 2, screen: false, input: true)]);
    });

    test(
      'is off again for the next session, whichever way the last one ended',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final first = sampleWindowSession(id: 1, control: true, private: true);
        final second = sampleWindowSession(id: 2, control: true, private: true);
        await _begin(session, first);
        await _begin(session, second);
        controller.lift(first, screen: true, input: true);
        controller.lift(second, screen: true, input: true);

        // The other device ended one, and the person ended the other.
        await _end(session, 1, const WindowEnd.otherDevice());
        controller.end(second);
        expect(controller.liftedOf(first), dark);
        expect(controller.liftedOf(second), dark);

        // A session that begins with the same number, as a new one does for a device that was
        // asked for its screen again, starts with both off: nothing is remembered.
        final again = sampleWindowSession(id: 1, control: true, private: true);
        await _begin(session, again);
        expect(controller.liftedOf(again), dark);
        expect(session.windowLifts.length, 2);
      },
    );

    test('is not for a window that was not asked for as private', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      await _begin(session, window);

      // Baton is the one that knows which windows may be lifted, and it says no.
      session.windowLiftTaken = false;
      expect(controller.lift(window, screen: true), isFalse);
      expect(controller.liftedOf(window), dark);
    });
  });

  group('the sound of a screen', () {
    Future<void> hearOf(FakeSession session, int id, WindowSound sound) async {
      session.emit(Event.windowSound(id: id, sound: sound));
      await pumpEventQueue();
    }

    const asking = (stage: SoundStage.asking, why: null);
    const playing = (stage: SoundStage.playing, why: null);

    test('is off when the session starts, and nothing is asked', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);

      expect(controller.soundOf(window), soundOff);
      expect(session.windowHears, isEmpty);
    });

    test('is asked for, plays, and is turned off by the person', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(id: 5, display: true, sound: true);
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      expect(controller.hear(window, listen: true), isTrue);
      expect(controller.soundOf(window), asking);
      await hearOf(session, 5, WindowSound.playing);
      expect(controller.soundOf(window), playing);
      expect(controller.hear(window, listen: false), isTrue);
      expect(controller.soundOf(window), soundOff);
      // Baton says so too, once the speaker has stopped: nothing new to show.
      await hearOf(session, 5, WindowSound.off);

      expect(session.windowHears, [
        (session: 5, listen: true),
        (session: 5, listen: false),
      ]);
      expect(told, 3, reason: 'asking, playing and off, each once');
    });

    test('asks nothing when it says what already is', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);

      expect(controller.hear(window, listen: false), isTrue);
      expect(controller.hear(window, listen: true), isTrue);
      expect(controller.hear(window, listen: true), isTrue);
      await hearOf(session, 1, WindowSound.playing);
      expect(controller.hear(window, listen: true), isTrue);

      expect(session.windowHears, [(session: 1, listen: true)]);
    });

    test('is told why the other device sends none, and asks again when the person does', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);
      controller.hear(window, listen: true);

      await hearOf(session, 1, WindowSound.noPermission);
      expect(controller.soundOf(window), (
        stage: SoundStage.silent,
        why: WindowSound.noPermission,
      ));

      // Asking again is asking again, though the sound is wanted already.
      expect(controller.hear(window, listen: true), isTrue);
      expect(controller.soundOf(window), asking);
      await hearOf(session, 1, WindowSound.failed);
      expect(controller.soundOf(window), (
        stage: SoundStage.silent,
        why: WindowSound.failed,
      ));
      await hearOf(session, 1, WindowSound.playing);
      expect(controller.soundOf(window), playing);

      expect(session.windowHears, [
        (session: 1, listen: true),
        (session: 1, listen: true),
      ]);
    });

    test(
      'can be turned off while the other device says it sends none',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final window = sampleWindowSession(display: true, sound: true);
        await _begin(session, window);
        controller.hear(window, listen: true);
        await hearOf(session, 1, WindowSound.other);

        expect(controller.hear(window, listen: false), isTrue);

        expect(controller.soundOf(window), soundOff);
        expect(session.windowHears.last, (session: 1, listen: false));
      },
    );

    test('stays as it was when the other device could not be asked', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      session.windowHearTaken = false;
      expect(controller.hear(window, listen: true), isFalse);
      expect(controller.soundOf(window), soundOff);
      expect(told, 0);
      // Not being asked is not the end of the window.
      expect(controller.sessions, [window]);
      expect(controller.notes, isEmpty);

      session.windowHearTaken = true;
      expect(controller.hear(window, listen: true), isTrue);
      expect(controller.soundOf(window), asking);
    });

    test('goes to the window that it is for, and to no other', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1, display: true, sound: true);
      final second = sampleWindowSession(id: 2, display: true, sound: true);
      await _begin(session, first);
      await _begin(session, second);

      controller.hear(second, listen: true);
      await hearOf(session, 2, WindowSound.playing);

      expect(controller.soundOf(second), playing);
      expect(controller.soundOf(first), soundOff);
      expect(session.windowHears, [(session: 2, listen: true)]);
    });

    test(
      'is asked for at once in a private session, and in no other',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final private = sampleWindowSession(
          id: 1,
          display: true,
          control: true,
          private: true,
          sound: true,
        );
        final plain = sampleWindowSession(id: 2, display: true, sound: true);
        final mute = sampleWindowSession(
          id: 3,
          display: true,
          control: true,
          private: true,
        );
        final shared = sampleWindowSession(
          id: 4,
          role: WindowRole.sharing,
          display: true,
          private: true,
          sound: true,
        );
        await _begin(session, private);
        await _begin(session, plain);
        await _begin(session, mute);
        await _begin(session, shared);

        // The first has sound to ask for and is private. The second is not private. The third has
        // no sound to ask for, and the fourth is a screen that this device shares.
        expect(controller.soundOf(private), asking);
        expect(controller.soundOf(plain), soundOff);
        expect(controller.soundOf(mute), soundOff);
        expect(controller.soundOf(shared), soundOff);
        expect(session.windowHears, [(session: 1, listen: true)]);
      },
    );

    test(
      'is not shown for a screen that is not here, or that is over',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final shared = sampleWindowSession(
          id: 1,
          role: WindowRole.sharing,
          display: true,
        );
        await _begin(session, shared);
        var told = 0;
        controller.addListener(() => told++);

        await hearOf(session, 1, WindowSound.playing);
        await hearOf(session, 9, WindowSound.playing);

        expect(controller.soundOf(shared), soundOff);
        expect(told, 0);
      },
    );

    test(
      'is off again for the next session, whichever way the last one ended',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final first = sampleWindowSession(id: 1, display: true, sound: true);
        final second = sampleWindowSession(id: 2, display: true, sound: true);
        await _begin(session, first);
        await _begin(session, second);
        controller.hear(first, listen: true);
        controller.hear(second, listen: true);
        await hearOf(session, 1, WindowSound.playing);
        await hearOf(session, 2, WindowSound.playing);

        // The other device ended one, and the person ended the other.
        await _end(session, 1, const WindowEnd.otherDevice());
        controller.end(second);
        expect(controller.soundOf(first), soundOff);
        expect(controller.soundOf(second), soundOff);

        // A sound event that is late for a window that is gone changes nothing.
        await hearOf(session, 1, WindowSound.playing);
        expect(controller.soundOf(first), soundOff);

        // A session that begins with the same number starts with sound off.
        final again = sampleWindowSession(id: 1, display: true, sound: true);
        await _begin(session, again);
        expect(controller.soundOf(again), soundOff);
      },
    );

    test('is told when the other device does not let this one hear, and asks again when the person does', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);
      controller.hear(window, listen: true);

      await hearOf(session, 1, WindowSound.notAllowed);
      expect(controller.soundOf(window), (
        stage: SoundStage.silent,
        why: WindowSound.notAllowed,
      ));

      // The person there said yes since: the next ask is taken, in the same session.
      expect(controller.hear(window, listen: true), isTrue);
      expect(controller.soundOf(window), asking);
      await hearOf(session, 1, WindowSound.playing);
      expect(controller.soundOf(window), playing);
      expect(session.windowHears, [
        (session: 1, listen: true),
        (session: 1, listen: true),
      ]);
    });

    test('is told when this device could not play it, and asks again when the person does', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);
      controller.hear(window, listen: true);
      await hearOf(session, 1, WindowSound.playing);

      // Baton has told the other device to stop by itself, so nothing is asked here.
      await hearOf(session, 1, WindowSound.cannotPlay);
      expect(controller.soundOf(window), (
        stage: SoundStage.silent,
        why: WindowSound.cannotPlay,
      ));
      expect(session.windowHears, [(session: 1, listen: true)]);

      expect(controller.hear(window, listen: true), isTrue);
      expect(controller.soundOf(window), asking);
      await hearOf(session, 1, WindowSound.playing);
      expect(controller.soundOf(window), playing);
      expect(session.windowHears, [
        (session: 1, listen: true),
        (session: 1, listen: true),
      ]);
    });

    test('keeps the line about the silence away once the person put it away, until the sound stands differently', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);
      controller.hear(window, listen: true);
      await hearOf(session, 1, WindowSound.noPermission);
      var told = 0;
      controller.addListener(() => told++);
      expect(controller.silenceClosedOf(window), isFalse);

      controller.closeSilence(window);
      controller.closeSilence(window);

      expect(controller.silenceClosedOf(window), isTrue);
      expect(told, 1, reason: 'the second close changes nothing');
      // Only the line goes: the sound stays as it was, and the other device is asked nothing.
      expect(controller.soundOf(window), (
        stage: SoundStage.silent,
        why: WindowSound.noPermission,
      ));
      expect(session.windowHears, [(session: 1, listen: true)]);

      // The same news again is not news, but another reason is.
      await hearOf(session, 1, WindowSound.noPermission);
      expect(controller.silenceClosedOf(window), isTrue);
      await hearOf(session, 1, WindowSound.cannotPlay);
      expect(controller.silenceClosedOf(window), isFalse);

      // Asking again, and turning the sound off, start the line afresh.
      controller.closeSilence(window);
      expect(controller.silenceClosedOf(window), isTrue);
      controller.hear(window, listen: true);
      expect(controller.silenceClosedOf(window), isFalse);
      await hearOf(session, 1, WindowSound.failed);
      controller.closeSilence(window);
      controller.hear(window, listen: false);
      expect(controller.silenceClosedOf(window), isFalse);
    });

    test('has no line to put away unless the sound is silent', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);
      var told = 0;
      controller.addListener(() => told++);

      controller.closeSilence(window);
      controller.hear(window, listen: true);
      controller.closeSilence(window);
      await hearOf(session, 1, WindowSound.playing);
      controller.closeSilence(window);

      expect(controller.silenceClosedOf(window), isFalse);
      expect(told, 2, reason: 'asking and playing, each once');
    });

    test(
      'forgets the line that was put away when the screen is over',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final window = sampleWindowSession(display: true, sound: true);
        await _begin(session, window);
        controller.hear(window, listen: true);
        await hearOf(session, 1, WindowSound.noPermission);
        controller.closeSilence(window);

        session.emit(
          Event.windowEnded(id: 1, end: const WindowEnd.otherDevice()),
        );
        await pumpEventQueue();

        expect(controller.silenceClosedOf(window), isFalse);
      },
    );

    test('is told to the screens once for each change, not for the same state again', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(display: true, sound: true);
      await _begin(session, window);
      controller.hear(window, listen: true);
      var told = 0;
      controller.addListener(() => told++);

      await hearOf(session, 1, WindowSound.playing);
      await hearOf(session, 1, WindowSound.playing);
      await hearOf(session, 1, WindowSound.noPermission);
      await hearOf(session, 1, WindowSound.noPermission);

      expect(told, 2);
    });
  });

  group('what the other device did not do', () {
    Future<void> say(
      FakeSession session,
      int id,
      WindowNoticeReason reason,
      int count,
    ) async {
      session.emit(Event.windowNotice(id: id, reason: reason, count: count));
      await pumpEventQueue();
    }

    test('is told for the window that it was done on', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      await _begin(session, window);
      final heard = <WindowDropped>[];
      final listening = controller.dropped.listen(heard.add);
      addTearDown(listening.cancel);
      var told = 0;
      controller.addListener(() => told++);

      await say(session, 1, WindowNoticeReason.paused, 3);
      await say(session, 1, WindowNoticeReason.locked, 1);

      expect(heard, [
        (session: window, reason: WindowNoticeReason.paused, count: 3),
        (session: window, reason: WindowNoticeReason.locked, count: 1),
      ]);
      // It is for a sentence on the screen, not for the list of windows.
      expect(controller.notes, isEmpty);
      expect(controller.sessions, [window]);
      expect(told, 0);
    });

    test('is heard by every screen that listens', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      await _begin(session, window);
      final first = <WindowDropped>[];
      final second = <WindowDropped>[];
      final listeningFirst = controller.dropped.listen(first.add);
      final listeningSecond = controller.dropped.listen(second.add);
      addTearDown(listeningFirst.cancel);
      addTearDown(listeningSecond.cancel);

      await say(session, 1, WindowNoticeReason.covered, 2);

      final expected = (
        session: window,
        reason: WindowNoticeReason.covered,
        count: 2,
      );
      expect(first, [expected]);
      expect(second, [expected]);
    });

    test('is told for each window by itself', () async {
      final session = _session();
      final controller = await _controller(session);
      final first = sampleWindowSession(id: 1, control: true);
      final second = sampleWindowSession(id: 2, control: true);
      await _begin(session, first);
      await _begin(session, second);
      final heard = <WindowDropped>[];
      final listening = controller.dropped.listen(heard.add);
      addTearDown(listening.cancel);

      await say(session, 2, WindowNoticeReason.covered, 2);

      expect(heard, [
        (session: second, reason: WindowNoticeReason.covered, count: 2),
      ]);
    });

    test('is not told for a window that is not running', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      final heard = <WindowDropped>[];
      final listening = controller.dropped.listen(heard.add);
      addTearDown(listening.cancel);

      // Before it began, which Baton does not do, and after it ended.
      await say(session, 1, WindowNoticeReason.paused, 1);
      await _begin(session, window);
      await _end(session, 1, const WindowEnd.otherDevice());
      await say(session, 1, WindowNoticeReason.paused, 1);

      expect(heard, isEmpty);
    });

    test('is not told for a window that is shared from here', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(role: WindowRole.sharing);
      await _begin(session, window);
      final heard = <WindowDropped>[];
      final listening = controller.dropped.listen(heard.add);
      addTearDown(listening.cancel);

      await say(session, 1, WindowNoticeReason.paused, 1);

      expect(heard, isEmpty);
    });

    test('is not told when nothing was dropped', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      await _begin(session, window);
      final heard = <WindowDropped>[];
      final listening = controller.dropped.listen(heard.add);
      addTearDown(listening.cancel);

      await say(session, 1, WindowNoticeReason.paused, 0);
      await say(session, 1, WindowNoticeReason.paused, -4);

      expect(heard, isEmpty);
    });

    test('is not heard after the controller was let go of', () async {
      final session = _session();
      final controller = await _controller(session);
      final window = sampleWindowSession(control: true);
      await _begin(session, window);
      final heard = <WindowDropped>[];
      var closed = false;
      controller.dropped.listen(heard.add, onDone: () => closed = true);

      controller.dispose();
      await say(session, 1, WindowNoticeReason.paused, 1);

      expect(heard, isEmpty);
      expect(closed, isTrue);
    });
  });

  group('sharing a window', () {
    test('asks Baton to share it with the device, and is offering until the device has answered', () async {
      final session = _session();
      final controller = await _controller(session);
      session.holdShare();

      final done = controller.share(_ipad);
      await pumpEventQueue();

      expect(session.windowsShared, [(id: _ipad.id, control: false)]);
      expect(controller.offering, _ipad);

      session.releaseShare();
      await done;

      expect(controller.offering, isNull);
      expect(controller.notes, isEmpty);
    });

    test('passes on that the other device may control the window', () async {
      final session = _session();
      final controller = await _controller(session);

      await controller.share(_ipad, control: true);

      expect(session.windowsShared, [(id: _ipad.id, control: true)]);
    });

    test('is one offer at a time', () async {
      final session = _session();
      final controller = await _controller(session);
      session.holdShare();

      final first = controller.share(_ipad);
      await pumpEventQueue();
      await controller.share(_ipad);

      expect(session.windowsShared, hasLength(1));

      session.releaseShare();
      await first;
    });

    test('can be done again after an offer', () async {
      final session = _session();
      final controller = await _controller(session);

      await controller.share(_ipad);
      await controller.share(_ipad);

      expect(session.windowsShared, hasLength(2));
    });

    test('says nothing when the person chose no window', () async {
      final session = _session()..sharing = const WindowShared.nothingChosen();
      final controller = await _controller(session);

      await controller.share(_ipad);

      expect(controller.notes, isEmpty);
      expect(controller.sessions, isEmpty);
    });

    test('leaves the window to its event when it began', () async {
      final session = _session();
      final window = sampleWindowSession(role: WindowRole.sharing);
      session.sharing = WindowShared.started(session: window);
      final controller = await _controller(session);

      await controller.share(_ipad);

      // The session comes as an event, and a result that arrives after the window ended would
      // bring back a window that is over.
      expect(controller.sessions, isEmpty);
      expect(controller.notes, isEmpty);
    });

    test('tells the person why a device did not take the window', () async {
      final session = _session()
        ..sharing = const WindowShared.declined(
          message: 'Kitchen iPad did not accept the window.',
        );
      final controller = await _controller(session);

      await controller.share(_ipad);

      expect(controller.notes, hasLength(1));
      expect(
        (controller.notes.single as WindowFailed).message,
        'Kitchen iPad did not accept the window.',
      );
      expect(controller.offering, isNull);
    });

    test('tells the person why it failed', () async {
      final session = _session()
        ..shareFailure = const BatonError(
          kind: ErrorKind.unreachable,
          message: 'Kitchen iPad did not answer.',
        );
      final controller = await _controller(session);

      await controller.share(_ipad);

      expect(
        (controller.notes.single as WindowFailed).message,
        'Kitchen iPad did not answer.',
      );
      expect(controller.offering, isNull);
    });

    test(
      'is let go of when the controller is, and what comes after is not told',
      () async {
        final session = _session();
        final controller = WindowsController(session, screen: FakeScreen());
        await controller.start();
        session.holdShare();
        final done = controller.share(_ipad);
        await pumpEventQueue();
        var told = 0;
        controller.addListener(() => told++);

        controller.dispose();
        session.releaseShare();
        await done;

        expect(told, 0);
      },
    );
  });

  group('asking a device for its screen', () {
    test(
      'asks Baton to start it, and is asking until the device has answered',
      () async {
        final session = _session();
        final controller = await _controller(session);
        session.holdStart();

        final done = controller.startScreen(_mac);
        await pumpEventQueue();

        expect(session.screensAsked, [(id: _mac.id, control: false)]);
        expect(controller.asking, _mac);
        expect(controller.askingControl, isFalse);

        session.releaseStart();
        await done;

        expect(controller.asking, isNull);
        expect(controller.askingControl, isFalse);
        expect(controller.notes, isEmpty);
      },
    );

    test('passes on that the person wishes to control the screen', () async {
      final session = _session();
      final controller = await _controller(session);
      session.holdStart();

      final done = controller.startScreen(_mac, control: true);
      await pumpEventQueue();

      expect(controller.askingControl, isTrue);

      session.releaseStart();
      await done;

      expect(session.screensAsked, [(id: _mac.id, control: true)]);
      expect(controller.askingControl, isFalse);
    });

    test('is one at a time, and not while a window is being offered', () async {
      final session = _session();
      final controller = await _controller(session);
      session.holdStart();

      final first = controller.startScreen(_mac);
      await pumpEventQueue();
      await controller.startScreen(_mac);
      await controller.share(_ipad);

      expect(session.screensAsked, hasLength(1));
      expect(session.windowsShared, isEmpty);

      session.releaseStart();
      await first;
      session.holdShare();
      final offer = controller.share(_ipad);
      await pumpEventQueue();
      await controller.startScreen(_mac);

      expect(session.screensAsked, hasLength(1));
      expect(controller.asking, isNull);

      session.releaseShare();
      await offer;
    });

    test('can be done again after an answer', () async {
      final session = _session();
      final controller = await _controller(session);

      await controller.startScreen(_mac);
      await controller.startScreen(_mac);

      expect(session.screensAsked, hasLength(2));
    });

    test(
      'keeps the screen of the phone on from the moment the person asks',
      () async {
        final session = _session();
        final screen = FakeScreen();
        final controller = await _controller(session, screen: screen);
        session.holdStart();

        final done = controller.startScreen(_mac);
        await pumpEventQueue();

        expect(screen.asked, ['keepOn']);

        session.releaseStart();
        await done;

        expect(screen.asked, ['keepOn', 'letTurnOff']);
      },
    );

    test('keeps the screen of the phone on while it waits, whatever else is answered', () async {
      final session = _session();
      final screen = FakeScreen();
      final controller = await _controller(session, screen: screen);
      session.holdStart();

      final done = controller.startScreen(_mac);
      // Another device offers a window meanwhile, and the person says no to it. The Mac has not
      // answered yet, and the phone has to stay awake for it.
      await _offer(session, sampleWindowRequest);
      expect(controller.answer(sampleWindowRequest, accept: false), isTrue);
      await pumpEventQueue();

      expect(controller.asking, _mac);
      expect(screen.on, isTrue);

      session.releaseStart();
      await done;

      expect(screen.on, isFalse);
    });

    test('leaves the screen to its event when it began', () async {
      final session = _session();
      final controller = await _controller(session);

      await controller.startScreen(_mac);

      // The session comes as an event, and a result that arrives after the screen ended would
      // bring back a screen that is over.
      expect(controller.sessions, isEmpty);
      expect(controller.notes, isEmpty);
    });

    test(
      'shows the screen that began, with no size yet, as any other window',
      () async {
        final session = _session();
        final controller = await _controller(session);
        final shown = <WindowSession>[];
        final heard = controller.started.listen(shown.add);
        addTearDown(heard.cancel);
        session.holdStart();

        final done = controller.startScreen(_mac);
        await pumpEventQueue();
        final screen = sampleWindowSession(
          display: true,
          title: null,
          app: null,
          width: 0,
          height: 0,
        );
        await _begin(session, screen);

        expect(shown, [screen]);
        expect(controller.sessions, [screen]);
        expect(controller.pictureOf(screen), isNull);
        expect(
          controller.asking,
          _mac,
          reason: 'the device has not answered yet',
        );

        session.releaseStart();
        await done;
        session.emit(
          Event.windowPicture(id: screen.id, width: 2560, height: 1600),
        );
        await pumpEventQueue();

        expect(controller.pictureOf(screen), (width: 2560, height: 1600));
        expect(controller.asking, isNull);
      },
    );

    test('tells the person why a device did not start it', () async {
      final session = _session()
        ..screenStart = const ScreenStarted.notStarted(
          message:
              'Anna’s Mac has not allowed this device to ask for its screen.',
        );
      final controller = await _controller(session);

      await controller.startScreen(_mac);

      expect(controller.notes, hasLength(1));
      expect(
        (controller.notes.single as WindowFailed).message,
        'Anna’s Mac has not allowed this device to ask for its screen.',
      );
      expect(controller.asking, isNull);
      expect(controller.sessions, isEmpty);
    });

    test('tells the person why it failed', () async {
      final session = _session()
        ..startFailure = const BatonError(
          kind: ErrorKind.unreachable,
          message: 'Anna’s Mac did not answer.',
        );
      final screen = FakeScreen();
      final controller = await _controller(session, screen: screen);

      await controller.startScreen(_mac);

      expect(
        (controller.notes.single as WindowFailed).message,
        'Anna’s Mac did not answer.',
      );
      expect(controller.asking, isNull);
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    test(
      'is let go of when the controller is, and what comes after is not told',
      () async {
        final session = _session();
        final controller = WindowsController(session, screen: FakeScreen());
        await controller.start();
        session.holdStart();
        final done = controller.startScreen(_mac);
        await pumpEventQueue();
        var told = 0;
        controller.addListener(() => told++);

        controller.dispose();
        session.releaseStart();
        await done;

        expect(told, 0);
      },
    );
  });

  group('the notes', () {
    test('are the last few, newest first', () async {
      final session = _session();
      final controller = await _controller(session);
      for (var i = 1; i <= WindowsController.keepNotes + 2; i++) {
        await _begin(session, sampleWindowSession(id: i, device: 'Mac $i'));
        await _end(session, i, const WindowEnd.otherDevice());
      }

      final names = controller.notes.map(
        (note) => (note as WindowOver).session.deviceName,
      );
      expect(names, ['Mac 7', 'Mac 6', 'Mac 5', 'Mac 4', 'Mac 3']);
    });

    test('are put away one at a time', () async {
      final session = _session();
      final controller = await _controller(session);
      await _begin(session, sampleWindowSession(id: 1));
      await _begin(session, sampleWindowSession(id: 2));
      await _end(session, 1, const WindowEnd.lost());
      await _end(session, 2, const WindowEnd.lost());
      final newest = controller.notes.first;
      var told = 0;
      controller.addListener(() => told++);

      controller.dismiss(newest);

      expect(controller.notes, hasLength(1));
      expect(controller.notes.single, isNot(newest));
      expect(told, 1);

      controller.dismiss(newest);

      expect(told, 1, reason: 'a note that is gone is not told again');
    });
  });

  group('the screen of the device', () {
    test('stays on from an offer until the window begins', () async {
      final screen = FakeScreen();
      final session = _session();
      final controller = await _controller(session, screen: screen);
      expect(screen.asked, isEmpty);

      await _offer(session, sampleWindowRequest);
      expect(screen.asked, ['keepOn']);

      controller.answer(sampleWindowRequest, accept: true);
      expect(screen.asked, ['keepOn'], reason: 'it is still being opened');

      await _begin(session, sampleWindowSession());
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('stays on for as long after the answer as after the offer', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      final controller = await _controller(
        session,
        screen: screen,
        openingLimit: const Duration(minutes: 1),
      );
      session.emit(Event.windowRequested(request: sampleWindowRequest));
      await tester.pump();
      expect(screen.on, isTrue);
      await tester.pump(const Duration(minutes: 4, seconds: 50));
      expect(screen.on, isTrue);

      controller.answer(sampleWindowRequest, accept: true);

      // The hold that the offer began runs out at five minutes, unless the answer renewed it.
      await tester.pump(const Duration(seconds: 15));
      expect(screen.on, isTrue);
      expect(screen.asked, ['keepOn']);

      // Nothing is left pending when the controller is let go of.
      controller.dispose();
    });

    test('goes back to locking when the offer is said no to', () async {
      final screen = FakeScreen();
      final session = _session();
      final controller = await _controller(session, screen: screen);
      await _offer(session, sampleWindowRequest);

      controller.answer(sampleWindowRequest, accept: false);

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    test('goes back to locking when the offer is taken back', () async {
      final screen = FakeScreen();
      final session = _session();
      await _controller(session, screen: screen);
      await _offer(session, sampleWindowRequest);

      session.emit(const Event.windowRequestEnded(id: 3));
      await pumpEventQueue();

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    test('stays on while one offer waits after another is answered', () async {
      final screen = FakeScreen();
      final session = _session();
      final controller = await _controller(session, screen: screen);
      const other = WindowRequest(
        id: 4,
        deviceId: 'd1',
        deviceName: 'Desk PC',
        display: false,
        width: 800,
        height: 600,
        control: false,
      );
      await _offer(session, sampleWindowRequest);
      await _offer(session, other);

      controller.answer(sampleWindowRequest, accept: false);

      expect(screen.on, isTrue);
      expect(screen.asked, ['keepOn']);
    });

    test('is let go of with the controller', () async {
      final screen = FakeScreen();
      final session = _session();
      final controller = WindowsController(session, screen: screen);
      await controller.start();
      await _offer(session, sampleWindowRequest);

      controller.dispose();

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    test('is not touched by a window that is shared', () async {
      final screen = FakeScreen();
      final session = _session();
      await _controller(session, screen: screen);

      await _begin(session, sampleWindowSession(role: WindowRole.sharing));

      expect(screen.asked, isEmpty);
    });
  });

  group('the events', () {
    test('are listened to until the controller is let go of', () async {
      final session = _session();
      final controller = WindowsController(session, screen: FakeScreen());
      await controller.start();
      expect(session.listening, isTrue);

      controller.dispose();

      expect(session.listening, isFalse);
    });

    test('that arrive after it was let go of are not heard', () async {
      final session = _session();
      final controller = WindowsController(session, screen: FakeScreen());
      await controller.start();
      controller.dispose();

      session.emit(Event.windowRequested(request: sampleWindowRequest));
      await pumpEventQueue();

      expect(controller.requests, isEmpty);
    });

    test('are not listened to at all if the controller was let go of before they came', () async {
      final session = _session()..holdEvents();
      final controller = WindowsController(session, screen: FakeScreen());
      final started = controller.start();
      controller.dispose();

      session.releaseEvents();
      await started;
      await pumpEventQueue();

      expect(session.listening, isFalse);
    });

    test('that are not about windows are left to other screens', () async {
      final session = _session();
      final controller = await _controller(session);
      var told = 0;
      controller.addListener(() => told++);

      session.emit(const Event.devicesChanged());
      session.emit(const Event.resync());
      await pumpEventQueue();

      expect(told, 0);
    });

    test('are let go of once, however often the controller is', () async {
      final session = _session();
      final controller = WindowsController(session, screen: FakeScreen());
      await controller.start();

      controller
        ..dispose()
        ..dispose();

      expect(session.listening, isFalse);
    });
  });

  test('says what the device does with windows, as Baton says it', () async {
    final session = _session()
      ..abilities = const WindowAbilities(
        canShow: true,
        canShare: false,
        canControl: false,
        canGiveScreen: true,
      );
    final controller = await _controller(session);

    expect(
      controller.abilities,
      const WindowAbilities(
        canShow: true,
        canShare: false,
        canControl: false,
        canGiveScreen: true,
      ),
    );
    expect(controller.abilities, session.windowAbilities());
  });

  test(
    'keeps what it shows out of reach of the screens that read it',
    () async {
      final session = _session();
      final controller = await _controller(session);
      await _offer(session, sampleWindowRequest);
      await _begin(session, sampleWindowSession());
      await _end(session, 1, const WindowEnd.lost());

      expect(
        () => controller.requests.add(sampleWindowRequest),
        throwsUnsupportedError,
      );
      expect(
        () => controller.sessions.add(sampleWindowSession()),
        throwsUnsupportedError,
      );
      expect(() => controller.notes.clear(), throwsUnsupportedError);
    },
  );
}
