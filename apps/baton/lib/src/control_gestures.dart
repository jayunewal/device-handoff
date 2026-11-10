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

  /// How many points the window scrolls for a point that two fingers move on the picture. A
  /// picture is smaller than the window it shows, so the content under the fingers keeps up with
  /// them only if this is more than 1. The screen sets it as the picture changes size. A picture
  /// that is zoomed is moved by the fingers as far as they move on the screen, which is further
  /// than they move on the picture, and it is the picture that the window has to keep up with.
  double scrollGain;

  final Map<int, _Finger> _fingers = {};
  _Phase _phase = _Phase.idle;
  Timer? _hold;

  /// The middle of the first two fingers, on the picture, while they scroll.
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
    finger.local = local;
    switch (_phase) {
      case _Phase.waiting:
        if ((global - finger.startGlobal).distance > slop) {
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
    _middle = _middleOf();
  }

  void _scroll() {
    final middle = _middleOf();
    final moved = (middle - _middle) * scrollGain;
    _middle = middle;
    if (moved == Offset.zero) return;
    _pointer(middle, WindowPointer.scroll(dx: moved.dx, dy: moved.dy));
  }

  /// The middle of the first two fingers, on the picture.
  Offset _middleOf() {
    final two = _fingers.values.take(2).map((finger) => finger.local).toList();
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
  void _pointer(Offset local, WindowPointer action) =>
      send(_pointerAt(unit, local, modifiers(), action));
}

/// What a mouse or a trackpad does on the picture of a window, told as what it does on a Mac.
///
/// A mouse is not a finger, so nothing is waited for and nothing is guessed. The pointer of the
/// window follows the pointer of the mouse, with a button down or not. A button that goes down is
/// down at that place, and one that comes up is up, whichever button it is, so that holding a
/// button, a double click, a drag and a click with the right button are what the person does with
/// the mouse, with no slop and no long press. The wheel of a mouse and the two fingers of a
/// trackpad scroll the window.
///
/// The class knows nothing of widgets, like [TouchGestures]. The screen tells it what the mouse
/// does, with its places on the picture, and it gives each [WindowInput] to [send]. What may be
/// sent is decided in Rust, once: this only has to say what the person did.
class MouseGestures {
  MouseGestures({
    required this.send,
    required this.unit,
    required this.modifiers,
    this.scrollGain = 1,
  });

  /// Gives an event to the window.
  final void Function(WindowInput input) send;

  /// Where a place on the picture is, across and down it, from 0 to 1 when it is on the picture.
  /// A place outside the picture gives a number outside that range.
  final Offset Function(Offset local) unit;

  /// The modifier keys that are down now, asked for each event.
  final WindowModifiers Function() modifiers;

  /// How many points the window scrolls for a point that its content moves on the picture, as for
  /// [TouchGestures.scrollGain]: the picture is smaller than the window, and the content under the
  /// pointer keeps up with the wheel or the fingers only if this is more than 1. The screen sets
  /// it as the picture changes size.
  double scrollGain;

  /// The buttons that this has pressed and that are not up yet.
  final Set<WindowButton> _down = {};

  /// Where the pointer was last, on the picture, while a button was down.
  Offset _at = Offset.zero;

  /// Whether a button is down.
  bool get pressing => _down.isNotEmpty;

  /// The pointer moved over the picture with no button down. The pointer of the window goes
  /// there.
  void hover(Offset local) => _pointer(local, const WindowPointer.move());

  /// A button went down at [local]. [buttons] are all the buttons that are down then.
  void down(Offset local, Set<WindowButton> buttons) {
    // The pointer is at the place before the button goes down there, as it is for a finger: the
    // Mac takes a click from where its own pointer is.
    if (_down.isEmpty) _pointer(local, const WindowPointer.move());
    _at = local;
    _change(local, buttons);
  }

  /// The pointer moved with a button down, which drags with it. [buttons] are the buttons that
  /// are down now, if the event says: a button can go down or come up while another is held, with
  /// no event of its own. A move that follows no press of this is left alone, as it is for a
  /// pointer that was pressed while the window was not controlled.
  void move(Offset local, [Set<WindowButton>? buttons]) {
    if (_down.isEmpty) return;
    _at = local;
    if (buttons != null) _change(local, buttons);
    if (_down.isNotEmpty) _pointer(local, const WindowPointer.move());
  }

  /// The last button came up at [local].
  void up(Offset local) {
    if (_down.isEmpty) return;
    _at = local;
    _change(local, const {});
  }

  /// The system took the pointer away, because something else needs it: what is down is let go
  /// of where the pointer last was. It is also what a screen that is going away, or a window that
  /// is not controlled any more, asks for.
  void reset() => _change(_at, const {});

  /// The window is scrolled at [local]: its content moves by [delta] on the picture, positive to
  /// the right and down, as a finger that drags it would move it.
  void scroll(Offset local, Offset delta) {
    final moved = delta * scrollGain;
    if (moved == Offset.zero) return;
    _pointer(local, WindowPointer.scroll(dx: moved.dx, dy: moved.dy));
  }

  /// Lets go of the buttons that are not in [wanted] and presses those that are not down yet, at
  /// [local], the first before the second.
  void _change(Offset local, Set<WindowButton> wanted) {
    final released = _down.difference(wanted).toList();
    final pressed = wanted.difference(_down).toList();
    _down.removeAll(released);
    for (final button in released) {
      _pointer(local, WindowPointer.up(button: button));
    }
    _down.addAll(pressed);
    for (final button in pressed) {
      _pointer(local, WindowPointer.down(button: button));
    }
  }

  void _pointer(Offset local, WindowPointer action) =>
      send(_pointerAt(unit, local, modifiers(), action));
}

/// A pointer event at [local] on the picture, which [unit] puts from 0 to 1 across and down it. A
/// place off the picture is moved to its edge: a viewer sends nothing outside the picture unless a
/// button is down, and a drag that leaves the picture goes on at its edge.
WindowInput _pointerAt(
  Offset Function(Offset local) unit,
  Offset local,
  WindowModifiers modifiers,
  WindowPointer action,
) {
  final place = unit(local);
  return WindowInput.pointer(
    x: place.dx.clamp(0.0, 1.0).toDouble(),
    y: place.dy.clamp(0.0, 1.0).toDouble(),
    modifiers: modifiers,
    action: action,
  );
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
  _Finger(this.start, this.startGlobal) : local = start;

  /// Where it first touched, on the picture.
  final Offset start;

  /// Where it first touched, on the screen.
  final Offset startGlobal;

  /// Where it is now, on the picture.
  Offset local;
}
