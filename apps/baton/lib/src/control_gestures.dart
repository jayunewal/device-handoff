// SPDX-License-Identifier: Apache-2.0

import 'dart:async';
import 'dart:ui' show Offset, VoidCallback;

import 'backend.dart';

/// What fingers on the picture of a window do there, told as what a mouse would do.
///
/// A finger that touches and lifts is a click. A finger that stays is a click with the right
/// button, made when it has stayed long enough. A finger that moves drags with the left button,
/// from the place where it first touched. Two fingers that move scroll the window, as if they
/// dragged its content. A third finger, and anything that follows a gesture that ended some other
/// way, is left alone until every finger is up.
///
/// The class knows nothing of widgets. The screen tells it what its fingers do, with their places
/// on the picture and on the screen of the device, and it gives each [WindowInput] to [send]. What
/// may be sent is decided in Rust, once: this only has to say what the person did.
class TouchGestures {
  TouchGestures({
    required this.send,
    required this.unit,
    required this.modifiers,
    this.onLongPress,
    this.slop = 10,
    this.longPressAfter = const Duration(milliseconds: 500),
    this.scrollGain = 1,
  });

  /// Gives an event to the window.
  final void Function(WindowInput input) send;

  /// Where a place on the picture is, across and down it, from 0 to 1 when it is on the picture.
  /// A place outside the picture gives a number outside that range.
  final Offset Function(Offset local) unit;

  /// The modifier keys that are down now, asked for each event.
  final WindowModifiers Function() modifiers;

  /// A finger stayed long enough to be a click with the right button, which has been made.
  final VoidCallback? onLongPress;

  /// How far a finger may move, on the screen, before it is dragging and no longer a click.
  final double slop;

  /// How long a finger stays before it is a click with the right button.
  final Duration longPressAfter;

  /// How many points the window scrolls for a point that two fingers move on the screen. A
  /// picture is smaller than the window it shows, so the content under the fingers keeps up with
  /// them only if this is more than 1. The screen sets it as the picture changes size.
  double scrollGain;

  final Map<int, _Finger> _fingers = {};
  _Phase _phase = _Phase.idle;
  Timer? _hold;

  /// The middle of the first two fingers, on the screen, while they scroll.
  Offset _middle = Offset.zero;

  /// Whether the left button is down, because of a drag.
  bool get dragging => _phase == _Phase.dragging;

  /// Whether a finger is down.
  bool get touching => _fingers.isNotEmpty;

  /// A finger touched the picture. [local] is where, on the picture, and [global] on the screen.
  void down(int pointer, Offset local, Offset global) {
    if (_fingers.containsKey(pointer)) return;
    _fingers[pointer] = _Finger(local, global);
    switch (_phase) {
      case _Phase.idle:
        _phase = _Phase.waiting;
        _hold = Timer(longPressAfter, _stayed);
      case _Phase.waiting:
        _stopHold();
        _startScroll();
      case _Phase.dragging:
        _letGoOfLeft(_fingers.values.first.local);
        _startScroll();
      case _Phase.held || _Phase.scrolling || _Phase.ignoring:
        break;
    }
  }

  /// A finger moved to [local] on the picture and [global] on the screen.
  void move(int pointer, Offset local, Offset global) {
    final finger = _fingers[pointer];
    if (finger == null) return;
    finger
      ..local = local
      ..global = global;
    switch (_phase) {
      case _Phase.waiting:
        if ((local - finger.start).distance > slop) {
          _stopHold();
          _phase = _Phase.dragging;
          _pointer(finger.start, const WindowPointer.move());
          _pointer(
            finger.start,
            const WindowPointer.down(button: WindowButton.left),
          );
          _pointer(local, const WindowPointer.move());
        }
      case _Phase.dragging:
        _pointer(local, const WindowPointer.move());
      case _Phase.scrolling:
        _scroll();
      case _Phase.idle || _Phase.held || _Phase.ignoring:
        break;
    }
  }

  /// A finger lifted from [local] on the picture.
  void up(int pointer, Offset local) {
    final finger = _fingers.remove(pointer);
    if (finger == null) return;
    switch (_phase) {
      case _Phase.waiting:
        _stopHold();
        _click(finger.start, WindowButton.left);
      case _Phase.dragging:
        _pointer(local, WindowPointer.up(button: WindowButton.left));
      case _Phase.scrolling:
        _phase = _Phase.ignoring;
      case _Phase.idle || _Phase.held || _Phase.ignoring:
        break;
    }
    if (_fingers.isEmpty) _phase = _Phase.idle;
  }

  /// The system took a finger away, because something else needs it: a button that is down is
  /// let go of where the finger last was.
  void cancel(int pointer) {
    final finger = _fingers.remove(pointer);
    if (finger == null) return;
    if (_phase == _Phase.dragging) _letGoOfLeft(finger.local);
    _stopHold();
    _phase = _fingers.isEmpty ? _Phase.idle : _Phase.ignoring;
  }

  /// Lets go of what is held and forgets every finger, for a screen that is going away or a
  /// window that is not controlled any more.
  void reset() {
    if (_phase == _Phase.dragging) {
      _letGoOfLeft(_fingers.values.first.local);
    }
    _stopHold();
    _fingers.clear();
    _phase = _Phase.idle;
  }

  /// A finger has stayed where it touched: the right button is clicked there.
  void _stayed() {
    _hold = null;
    if (_phase != _Phase.waiting) return;
    _phase = _Phase.held;
    _click(_fingers.values.first.start, WindowButton.right);
    onLongPress?.call();
  }

  void _stopHold() {
    _hold?.cancel();
    _hold = null;
  }

  void _startScroll() {
    _phase = _Phase.scrolling;
    _middle = _middleOf((finger) => finger.global);
  }

  void _scroll() {
    final middle = _middleOf((finger) => finger.global);
    final moved = (middle - _middle) * scrollGain;
    _middle = middle;
    if (moved == Offset.zero) return;
    _pointer(
      _middleOf((finger) => finger.local),
      WindowPointer.scroll(dx: moved.dx, dy: moved.dy),
    );
  }

  /// The middle of the first two fingers.
  Offset _middleOf(Offset Function(_Finger finger) place) {
    final two = _fingers.values.take(2).map(place).toList();
    return (two[0] + two[1]) / 2;
  }

  void _click(Offset local, WindowButton button) {
    _pointer(local, const WindowPointer.move());
    _pointer(local, WindowPointer.down(button: button));
    _pointer(local, WindowPointer.up(button: button));
  }

  void _letGoOfLeft(Offset local) {
    _pointer(local, WindowPointer.up(button: WindowButton.left));
  }

  /// Gives a pointer event at [local]. A place off the picture is moved to its edge: this is only
  /// asked for while a button is down, or for a place that is on the picture.
  void _pointer(Offset local, WindowPointer action) {
    final place = unit(local);
    send(
      WindowInput.pointer(
        x: place.dx.clamp(0.0, 1.0).toDouble(),
        y: place.dy.clamp(0.0, 1.0).toDouble(),
        modifiers: modifiers(),
        action: action,
      ),
    );
  }
}

/// What the fingers are doing as a whole.
enum _Phase {
  /// No finger is down.
  idle,

  /// One finger is down and has not moved much or stayed long: a click, or a click with the right
  /// button, or the start of a drag.
  waiting,

  /// One finger moves with the left button down.
  dragging,

  /// One finger stayed, and the right button was clicked. Nothing follows until it lifts.
  held,

  /// Two fingers scroll.
  scrolling,

  /// A gesture that ended some way other than by all the fingers lifting. Nothing follows until
  /// they all have.
  ignoring,
}

/// A finger that is down.
class _Finger {
  _Finger(this.start, this.global) : local = start;

  /// Where it first touched, on the picture.
  final Offset start;

  /// Where it is now, on the picture.
  Offset local;

  /// Where it is now, on the screen.
  Offset global;
}
