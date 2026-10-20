// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_gestures.dart';
import 'package:flutter_test/flutter_test.dart';

const _none = WindowModifiers(
  shift: false,
  control: false,
  option: false,
  command: false,
  capsLock: false,
);

const _shift = WindowModifiers(
  shift: true,
  control: false,
  option: false,
  command: false,
  capsLock: false,
);

/// A picture of 200 by 100 points: a place on it is a place in the unit square.
Offset _unit(Offset local) => Offset(local.dx / 200, local.dy / 100);

WindowInput _pointer(double x, double y, WindowPointer action) =>
    WindowInput.pointer(x: x, y: y, modifiers: _none, action: action);

const _left = WindowButton.left;
const _right = WindowButton.right;

class _Rig {
  _Rig({
    double slop = 10,
    double scrollGain = 1,
    WindowModifiers Function()? modifiers,
  }) {
    gestures = TouchGestures(
      send: sent.add,
      unit: _unit,
      modifiers: modifiers ?? () => _none,
      onLongPress: () => longPresses++,
      slop: slop,
      scrollGain: scrollGain,
    );
  }

  /// Fingers with the slop and the gain that a screen starts with.
  _Rig.standard() {
    gestures = TouchGestures(
      send: sent.add,
      unit: _unit,
      modifiers: () => _none,
      onLongPress: () => longPresses++,
    );
  }

  late final TouchGestures gestures;
  final List<WindowInput> sent = [];
  int longPresses = 0;

  /// Puts a finger down at [at], where the screen is the picture shifted by (1000, 2000).
  void down(int finger, Offset at) =>
      gestures.down(finger, at, at + const Offset(1000, 2000));

  void move(int finger, Offset to) =>
      gestures.move(finger, to, to + const Offset(1000, 2000));

  void up(int finger, Offset at) => gestures.up(finger, at);

  List<WindowInput> take() {
    final taken = List.of(sent);
    sent.clear();
    return taken;
  }
}

void main() {
  group('one finger', () {
    testWidgets(
      'that touches and lifts clicks the left button where it touched',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(100, 50));
        expect(rig.sent, isEmpty, reason: 'it is not known to be a click yet');
        rig.up(1, const Offset(104, 53));

        expect(rig.take(), [
          _pointer(0.5, 0.5, const WindowPointer.move()),
          _pointer(0.5, 0.5, const WindowPointer.down(button: _left)),
          _pointer(0.5, 0.5, const WindowPointer.up(button: _left)),
        ]);
        expect(rig.gestures.dragging, isFalse);
      },
    );

    testWidgets(
      'that stays clicks the right button, once, when it has stayed',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(50, 25));
        await tester.pump(const Duration(milliseconds: 499));
        expect(rig.sent, isEmpty);
        expect(rig.longPresses, 0);
        await tester.pump(const Duration(milliseconds: 1));

        expect(rig.take(), [
          _pointer(0.25, 0.25, const WindowPointer.move()),
          _pointer(0.25, 0.25, const WindowPointer.down(button: _right)),
          _pointer(0.25, 0.25, const WindowPointer.up(button: _right)),
        ]);
        expect(rig.longPresses, 1);

        // Nothing follows: not while it moves, not when it lifts.
        rig.move(1, const Offset(150, 75));
        rig.up(1, const Offset(150, 75));
        await tester.pump(const Duration(seconds: 2));
        expect(rig.sent, isEmpty);
        expect(rig.longPresses, 1);
      },
    );

    testWidgets(
      'that moves a little and stays is still a click with the right button',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(50, 25));
        rig.move(1, const Offset(55, 25));
        await tester.pump(const Duration(milliseconds: 500));

        expect(rig.take(), [
          _pointer(0.25, 0.25, const WindowPointer.move()),
          _pointer(0.25, 0.25, const WindowPointer.down(button: _right)),
          _pointer(0.25, 0.25, const WindowPointer.up(button: _right)),
        ]);
        expect(rig.longPresses, 1);
      },
    );

    testWidgets(
      'that lifts before it has stayed does not click the right button later',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(50, 25));
        await tester.pump(const Duration(milliseconds: 300));
        rig.up(1, const Offset(50, 25));
        rig.take();
        await tester.pump(const Duration(seconds: 1));

        expect(rig.sent, isEmpty);
        expect(rig.longPresses, 0);
      },
    );

    testWidgets(
      'that moves further than the slop drags with the left button from where it began',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(100, 50));
        rig.move(1, const Offset(105, 50));
        expect(rig.sent, isEmpty, reason: 'within the slop, nothing yet');
        rig.move(1, const Offset(100, 61));

        expect(rig.take(), [
          _pointer(0.5, 0.5, const WindowPointer.move()),
          _pointer(0.5, 0.5, const WindowPointer.down(button: _left)),
          _pointer(0.5, 0.61, const WindowPointer.move()),
        ]);
        expect(rig.gestures.dragging, isTrue);

        rig.move(1, const Offset(120, 70));
        rig.up(1, const Offset(140, 80));

        expect(rig.take(), [
          _pointer(0.6, 0.7, const WindowPointer.move()),
          _pointer(0.7, 0.8, const WindowPointer.up(button: _left)),
        ]);
        expect(rig.gestures.dragging, isFalse);

        // The long press that was waiting for it does not come.
        await tester.pump(const Duration(seconds: 2));
        expect(rig.sent, isEmpty);
        expect(rig.longPresses, 0);
      },
    );

    testWidgets('that moves exactly the slop is still a click', (tester) async {
      final rig = _Rig(slop: 10);

      rig.down(1, const Offset(100, 50));
      rig.move(1, const Offset(110, 50));
      rig.up(1, const Offset(110, 50));

      expect(rig.take(), [
        _pointer(0.5, 0.5, const WindowPointer.move()),
        _pointer(0.5, 0.5, const WindowPointer.down(button: _left)),
        _pointer(0.5, 0.5, const WindowPointer.up(button: _left)),
      ]);
    });

    testWidgets('has a slop of 10 points until the screen sets another', (
      tester,
    ) async {
      final rig = _Rig.standard();

      rig.down(1, const Offset(100, 50));
      rig.move(1, const Offset(110, 50));
      expect(rig.sent, isEmpty, reason: '10 points is still a click');
      rig.move(1, const Offset(110.5, 50));

      expect(rig.take(), [
        _pointer(0.5, 0.5, const WindowPointer.move()),
        _pointer(0.5, 0.5, const WindowPointer.down(button: _left)),
        _pointer(0.5525, 0.5, const WindowPointer.move()),
      ]);
    });

    testWidgets('keeps a drag on the picture when the finger leaves it', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(190, 90));
      rig.move(1, const Offset(170, 90));
      rig.take();
      rig.move(1, const Offset(260, 140));
      rig.up(1, const Offset(-30, 160));

      expect(rig.take(), [
        _pointer(1, 1, const WindowPointer.move()),
        _pointer(0, 1, const WindowPointer.up(button: _left)),
      ]);
    });

    testWidgets('sends the modifiers that are down at each event', (
      tester,
    ) async {
      var current = _none;
      final rig = _Rig(modifiers: () => current);

      current = _shift;
      rig.down(1, const Offset(100, 50));
      current = _none;
      rig.up(1, const Offset(100, 50));

      expect(
        rig.take().map((e) => (e as WindowInput_Pointer).modifiers),
        [_none, _none, _none],
        reason: 'they are asked for when the click is made, not when it began',
      );

      rig.down(1, const Offset(100, 50));
      current = _shift;
      rig.up(1, const Offset(100, 50));

      expect(rig.take().map((e) => (e as WindowInput_Pointer).modifiers), [
        _shift,
        _shift,
        _shift,
      ]);
    });
  });

  group('two fingers', () {
    testWidgets(
      'scroll by how far their middle moves on the screen, at the middle',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(80, 50));
        rig.down(2, const Offset(120, 50));
        expect(rig.sent, isEmpty);

        rig.move(1, const Offset(80, 60));
        expect(rig.take(), [
          _pointer(0.5, 0.55, const WindowPointer.scroll(dx: 0, dy: 5)),
        ], reason: 'one finger moved 10, so the middle moved 5');

        rig.move(2, const Offset(130, 60));
        expect(rig.take(), [
          _pointer(0.525, 0.6, const WindowPointer.scroll(dx: 5, dy: 5)),
        ]);

        rig.move(1, const Offset(70, 40));
        rig.move(2, const Offset(120, 40));
        expect(rig.take().length, 2);
      },
    );

    testWidgets('scroll by a gain', (tester) async {
      final rig = _Rig(scrollGain: 3);

      rig.down(1, const Offset(80, 50));
      rig.down(2, const Offset(120, 50));
      rig.move(1, const Offset(80, 52));
      rig.move(2, const Offset(120, 52));

      expect(rig.take(), [
        _pointer(0.5, 0.51, const WindowPointer.scroll(dx: 0, dy: 3)),
        _pointer(0.5, 0.52, const WindowPointer.scroll(dx: 0, dy: 3)),
      ]);
    });

    testWidgets('scroll by as far as they move, until the screen says more', (
      tester,
    ) async {
      final rig = _Rig.standard();

      rig.down(1, const Offset(80, 50));
      rig.down(2, const Offset(120, 50));
      rig.move(1, const Offset(80, 52));
      rig.move(2, const Offset(120, 52));

      expect(rig.take(), [
        _pointer(0.5, 0.51, const WindowPointer.scroll(dx: 0, dy: 1)),
        _pointer(0.5, 0.52, const WindowPointer.scroll(dx: 0, dy: 1)),
      ]);
    });

    testWidgets('say nothing for a move that leaves the middle where it was', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(80, 50));
      rig.down(2, const Offset(120, 50));
      rig.move(1, const Offset(70, 50));
      rig.take();
      rig.move(2, const Offset(120, 50));

      expect(rig.sent, isEmpty);
    });

    testWidgets(
      'are not a click when they lift, and the first finger is not a click either',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(80, 50));
        await tester.pump(const Duration(milliseconds: 200));
        rig.down(2, const Offset(120, 50));
        rig.up(2, const Offset(120, 50));
        rig.up(1, const Offset(80, 50));
        await tester.pump(const Duration(seconds: 2));

        expect(rig.sent, isEmpty);
        expect(rig.longPresses, 0);
      },
    );

    testWidgets(
      'that come while one finger drags let go of the left button where it is',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(100, 50));
        rig.move(1, const Offset(100, 70));
        rig.take();
        rig.down(2, const Offset(150, 50));

        expect(rig.take(), [
          _pointer(0.5, 0.7, const WindowPointer.up(button: _left)),
        ]);
        expect(rig.gestures.dragging, isFalse);

        rig.move(2, const Offset(150, 56));
        expect(rig.take(), [
          _pointer(0.625, 0.63, const WindowPointer.scroll(dx: 0, dy: 3)),
        ]);
      },
    );

    testWidgets('that come after the right button was clicked do nothing', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      await tester.pump(const Duration(milliseconds: 500));
      rig.take();
      rig.down(2, const Offset(150, 50));
      rig.move(2, const Offset(150, 80));
      rig.move(1, const Offset(100, 80));

      expect(rig.sent, isEmpty);
    });

    testWidgets(
      'are followed by nothing until every finger is up, and then by anything',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(80, 50));
        rig.down(2, const Offset(120, 50));
        rig.up(1, const Offset(80, 50));
        rig.move(2, const Offset(120, 90));
        expect(rig.sent, isEmpty, reason: 'one finger is left of a scroll');
        rig.up(2, const Offset(120, 90));
        expect(rig.sent, isEmpty);

        rig.down(3, const Offset(100, 50));
        rig.up(3, const Offset(100, 50));
        expect(
          rig.take().length,
          3,
          reason: 'a click, as if none had come before',
        );
      },
    );

    testWidgets(
      'leave a finger that comes while one is still down to do nothing',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(80, 50));
        rig.down(2, const Offset(120, 50));
        rig.up(1, const Offset(80, 50));
        rig.down(3, const Offset(100, 50));
        rig.up(3, const Offset(100, 50));
        await tester.pump(const Duration(seconds: 2));

        expect(rig.sent, isEmpty);
        expect(rig.longPresses, 0);
      },
    );

    testWidgets('leave a third finger out of the middle', (tester) async {
      final rig = _Rig();

      rig.down(1, const Offset(80, 50));
      rig.down(2, const Offset(120, 50));
      rig.down(3, const Offset(10, 10));
      rig.move(3, const Offset(10, 90));
      expect(rig.sent, isEmpty);

      rig.move(1, const Offset(80, 60));
      expect(rig.take(), [
        _pointer(0.5, 0.55, const WindowPointer.scroll(dx: 0, dy: 5)),
      ]);
    });
  });

  group('when a finger is taken away', () {
    testWidgets('a drag lets go of the left button where the finger was', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      rig.move(1, const Offset(100, 80));
      rig.take();
      rig.gestures.cancel(1);

      expect(rig.take(), [
        _pointer(0.5, 0.8, const WindowPointer.up(button: _left)),
      ]);
      expect(rig.gestures.dragging, isFalse);
    });

    testWidgets('a finger that waited is no click, and no long press follows', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      rig.gestures.cancel(1);
      await tester.pump(const Duration(seconds: 2));

      expect(rig.sent, isEmpty);
      expect(rig.longPresses, 0);
    });

    testWidgets('one of two leaves the other to do nothing until all are up', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(80, 50));
      rig.down(2, const Offset(120, 50));
      rig.gestures.cancel(1);
      rig.move(2, const Offset(120, 90));
      rig.up(2, const Offset(120, 90));

      expect(rig.sent, isEmpty);
    });

    testWidgets(
      'one of two leaves a finger that comes next to do nothing too',
      (tester) async {
        final rig = _Rig();

        rig.down(1, const Offset(80, 50));
        rig.down(2, const Offset(120, 50));
        rig.gestures.cancel(1);
        rig.down(3, const Offset(100, 50));
        rig.up(3, const Offset(100, 50));
        await tester.pump(const Duration(seconds: 2));

        expect(rig.sent, isEmpty);
        expect(rig.longPresses, 0);
      },
    );

    testWidgets('the only finger leaves the next one free to click', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      rig.gestures.cancel(1);
      rig.down(1, const Offset(100, 50));
      rig.up(1, const Offset(100, 50));

      expect(rig.take(), [
        _pointer(0.5, 0.5, const WindowPointer.move()),
        _pointer(0.5, 0.5, const WindowPointer.down(button: _left)),
        _pointer(0.5, 0.5, const WindowPointer.up(button: _left)),
      ]);
    });

    testWidgets('a finger that was never down is no matter', (tester) async {
      final rig = _Rig();

      rig.gestures.cancel(7);
      rig.move(7, const Offset(10, 10));
      rig.up(7, const Offset(10, 10));

      expect(rig.sent, isEmpty);
    });
  });

  group('reset', () {
    testWidgets('lets go of the left button of a drag, once', (tester) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      rig.move(1, const Offset(100, 80));
      rig.take();
      rig.gestures.reset();
      rig.gestures.reset();

      expect(rig.take(), [
        _pointer(0.5, 0.8, const WindowPointer.up(button: _left)),
      ]);
    });

    testWidgets('forgets the fingers and the click that was waiting', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      rig.gestures.reset();
      await tester.pump(const Duration(seconds: 2));
      rig.up(1, const Offset(100, 50));

      expect(rig.sent, isEmpty);
      expect(rig.longPresses, 0);

      rig.down(1, const Offset(100, 50));
      rig.up(1, const Offset(100, 50));
      expect(rig.take().length, 3, reason: 'a new gesture starts afresh');
    });

    testWidgets('does not leave its wait to the finger that comes next', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      await tester.pump(const Duration(milliseconds: 300));
      rig.gestures.reset();
      rig.down(2, const Offset(100, 50));
      await tester.pump(const Duration(milliseconds: 300));

      expect(rig.sent, isEmpty, reason: 'the new finger has stayed 300 ms');
      expect(rig.longPresses, 0);

      await tester.pump(const Duration(milliseconds: 200));
      expect(rig.longPresses, 1);
    });
  });

  testWidgets('a finger that is told to be down twice is down once', (
    tester,
  ) async {
    final rig = _Rig();

    rig.down(1, const Offset(100, 50));
    rig.down(1, const Offset(10, 10));
    rig.up(1, const Offset(100, 50));

    expect(rig.take(), [
      _pointer(0.5, 0.5, const WindowPointer.move()),
      _pointer(0.5, 0.5, const WindowPointer.down(button: _left)),
      _pointer(0.5, 0.5, const WindowPointer.up(button: _left)),
    ]);
  });

  group('whether a finger is down', () {
    testWidgets('is told for as long as a finger is', (tester) async {
      final rig = _Rig();
      expect(rig.gestures.touching, isFalse);

      rig.down(1, const Offset(100, 50));
      expect(rig.gestures.touching, isTrue);
      rig.down(2, const Offset(120, 50));
      expect(rig.gestures.touching, isTrue);
      rig.up(1, const Offset(100, 50));
      expect(rig.gestures.touching, isTrue);
      rig.up(2, const Offset(120, 50));
      expect(rig.gestures.touching, isFalse);
    });

    testWidgets('is no when a finger is taken away, or all are forgotten', (
      tester,
    ) async {
      final rig = _Rig();

      rig.down(1, const Offset(100, 50));
      rig.gestures.cancel(1);
      expect(rig.gestures.touching, isFalse);

      rig.down(1, const Offset(100, 50));
      rig.down(2, const Offset(120, 50));
      rig.gestures.reset();
      expect(rig.gestures.touching, isFalse);
    });
  });
}
