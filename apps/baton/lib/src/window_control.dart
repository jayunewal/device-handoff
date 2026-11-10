// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';

import 'backend.dart';
import 'control_gestures.dart';
import 'control_keys.dart';
import 'format.dart';
import 'windows_controller.dart';

/// What the person does to a window of another device that they control from this one: their
/// fingers on its picture, or the mouse or the trackpad that is attached to this device, the keys
/// of a keyboard that is attached to it, the keys that the screen has of its own, and the text
/// typed on the keyboard of the screen.
///
/// All of it is given to the window as [WindowInput]. What may be sent is decided in Rust, once,
/// and what the other device does not do comes back as a [notice]. Nothing here is on until the
/// person turns it on, and turning it off, or leaving, lets go of every button and key that is
/// down.
///
/// The keys of an attached keyboard are not heard here. The screen gives each one to [key] from
/// its focus scope, where a key that the window takes stops, so that the shortcuts of the app and
/// the field of the keyboard do not act on it as well.
class WindowControl extends ChangeNotifier {
  WindowControl({
    required this.windows,
    required this.session,
    required this.pictureSize,
    this.noticeFor = const Duration(seconds: 4),
    this.repeatAfter = const Duration(milliseconds: 400),
    this.repeatEvery = const Duration(milliseconds: 80),
    this.repeatFor = const Duration(seconds: 30),
  }) {
    gestures = TouchGestures(
      send: _send,
      unit: _unit,
      modifiers: _modifiers,
      onLongPress: _longPress,
    );
    mouse = MouseGestures(send: _send, unit: _unit, modifiers: _modifiers);
  }

  final WindowsController windows;

  /// The window that is controlled, which this device shows.
  final WindowSession session;

  /// How large the picture is on the screen of this device, or `null` before it is laid out.
  final Size? Function() pictureSize;

  /// How long a [notice] stays.
  final Duration noticeFor;

  /// How long a key of a keyboard is held before it repeats, how often it repeats then, and for
  /// how long at the most. The repeats are made here for a device whose keyboard sends none (see
  /// [key]), so a key whose release is never heard must not repeat for ever: it stops after
  /// [repeatFor]. The numbers are starting values, like those of the fingers, until a keyboard
  /// has been tried with them.
  final Duration repeatAfter;
  final Duration repeatEvery;
  final Duration repeatFor;

  /// The modifier keys that the person chose on the screen for the next thing they do.
  final ChosenModifiers chosen = ChosenModifiers();

  /// What the fingers on the picture do.
  late final TouchGestures gestures;

  /// What the mouse or the trackpad on the picture does.
  late final MouseGestures mouse;

  /// The keys of a keyboard that were sent as down and not yet as up.
  final Set<int> _down = {};

  /// The key that repeats while it is held, if one does. It is the last one that went down, as it
  /// is on a Mac: another key that goes down takes the repeat, and the first does not get it back.
  _Held? _holding;
  StreamSubscription<WindowDropped>? _dropped;
  Timer? _noticeTimer;
  String? _notice;
  bool _on = false;

  /// The last size the picture had, for the fingers that are down when it goes.
  Size _size = Size.zero;

  /// Whether the person controls the window.
  bool get on => _on;

  /// A sentence about what the other device did not do of what the person did, for a few
  /// seconds after it said so, or `null`.
  String? get notice => _notice;

  /// Starts to give the window what the person does.
  void turnOn() {
    if (_on) return;
    _on = true;
    _dropped = windows.dropped.listen(_told);
    notifyListeners();
  }

  /// Stops, and lets go of what is held.
  void turnOff() {
    if (!_on) return;
    _leave();
    notifyListeners();
  }

  void _leave() {
    _on = false;
    unawaited(_dropped?.cancel());
    _dropped = null;
    _noticeTimer?.cancel();
    _noticeTimer = null;
    _notice = null;
    gestures.reset();
    mouse.reset();
    _stopHolding();
    _down.clear();
    windows.releaseInput(session);
    chosen.spend();
  }

  @override
  void dispose() {
    if (_on) _leave();
    chosen.dispose();
    super.dispose();
  }

  // MARK: Fingers, a mouse and a trackpad

  /// A finger touched the picture, or a button of a mouse or a trackpad went down on it.
  /// `event.localPosition` has to be a place on the picture. Only a new finger or button is asked
  /// whether control is on: the gestures know no other while it is off, and leave the ones they
  /// do not know alone.
  ///
  /// A mouse is a pointer and not a finger: it has no slop and no long press, and its buttons are
  /// the buttons of the window (see [MouseGestures]). Which of the two an event is, is told by its
  /// kind, and a pen is a finger.
  void pointerDown(PointerDownEvent event) {
    final size = pictureSize();
    if (!_on || size == null) return;
    if (_isPointer(event)) {
      // A press that says no button is the primary one.
      mouse.down(
        event.localPosition,
        event.buttons == 0 ? const {WindowButton.left} : _buttonsOf(event),
      );
      return;
    }
    gestures
      ..scrollGain = _scrollGain(size)
      ..down(event.pointer, event.localPosition, event.position);
  }

  void pointerMove(PointerMoveEvent event) {
    if (_isPointer(event)) {
      // A move that says no button is down does not say that they came up: the release of the
      // last one does.
      mouse.move(
        event.localPosition,
        event.buttons == 0 ? null : _buttonsOf(event),
      );
      return;
    }
    gestures.move(event.pointer, event.localPosition, event.position);
  }

  void pointerUp(PointerUpEvent event) {
    if (_isPointer(event)) {
      mouse.up(event.localPosition);
    } else {
      gestures.up(event.pointer, event.localPosition);
    }
    _lifted();
  }

  void pointerCancel(PointerCancelEvent event) {
    if (_isPointer(event)) {
      mouse.reset();
    } else {
      gestures.cancel(event.pointer);
    }
    _lifted();
  }

  /// A mouse or a trackpad moved over the picture with no button down: the pointer of the window
  /// goes where it is, so that what shows when the pointer is over it shows, as it does on a Mac.
  void pointerHover(PointerHoverEvent event) {
    if (!_on || !_isPointer(event) || pictureSize() == null) return;
    mouse.hover(event.localPosition);
  }

  /// A wheel turned over the picture. `PointerScrollEvent.scrollDelta` is how far a scrollable of
  /// this device moves its offset, which moves its content the other way: so the content of the
  /// window moves by the opposite, as the content of anything else on this device does. The
  /// amount is in points of the screen, and of the picture before it was zoomed, the way a finger
  /// that moves on the picture is measured.
  void pointerSignal(PointerSignalEvent event) {
    if (event is! PointerScrollEvent) return;
    final size = pictureSize();
    if (!_on || size == null) return;
    final moved = PointerEvent.transformDeltaViaPositions(
      untransformedEndPosition: event.position + event.scrollDelta,
      untransformedDelta: event.scrollDelta,
      transform: event.transform,
    );
    // No wheel ends a gesture, so nothing that was chosen on the screen is used up by one: it is
    // used up by the next click or key, and it shows until then.
    mouse
      ..scrollGain = _scrollGain(size)
      ..scroll(event.localPosition, -moved);
  }

  /// The two fingers of a trackpad moved over the picture. They drag the content of the window,
  /// as fingers on the picture would.
  void pointerPanZoomUpdate(PointerPanZoomUpdateEvent event) {
    final size = pictureSize();
    if (!_on || size == null) return;
    mouse
      ..scrollGain = _scrollGain(size)
      ..scroll(event.localPosition, event.localPanDelta);
  }

  void pointerPanZoomEnd(PointerPanZoomEndEvent event) => _lifted();

  /// Whether [event] is of a mouse or a trackpad, which a person moves and clicks, and not of a
  /// finger or a pen, which touch.
  bool _isPointer(PointerEvent event) =>
      event.kind == PointerDeviceKind.mouse ||
      event.kind == PointerDeviceKind.trackpad;

  /// The buttons of the window that [event] says are down. A button that the window has no button
  /// for is left out.
  Set<WindowButton> _buttonsOf(PointerEvent event) => {
    if (event.buttons & kPrimaryMouseButton != 0) WindowButton.left,
    if (event.buttons & kSecondaryMouseButton != 0) WindowButton.right,
    if (event.buttons & kMiddleMouseButton != 0) WindowButton.middle,
  };

  /// What was chosen on the screen is for the gesture that has ended.
  void _lifted() {
    if (!gestures.touching && !mouse.pressing) chosen.spend();
  }

  Offset _unit(Offset local) {
    _size = pictureSize() ?? _size;
    if (_size.isEmpty) return Offset.zero;
    return Offset(local.dx / _size.width, local.dy / _size.height);
  }

  /// How far the window scrolls for a point that the fingers move. The picture has the pixels of
  /// the window, which is twice the points of it on the display of a Mac, and it is drawn smaller
  /// than the window: so it takes a gain for the content to keep up with the fingers. This is a
  /// first guess, until a Mac and a phone have been tried together.
  double _scrollGain(Size picture) {
    final pixels = windows.pictureOf(session)?.width ?? session.width;
    return (pixels / picture.width / 2).clamp(1.0, 8.0).toDouble();
  }

  void _longPress() {
    // A right click shows nothing on this device, and the touch would be lost.
    unawaited(HapticFeedback.mediumImpact().catchError((Object _) {}));
  }

  // MARK: Keys

  /// A key of a hardware keyboard. Returns whether the window took it: a key that the window
  /// does not have, like the Globe key, is left to the system, and so is any key while control is
  /// off.
  ///
  /// A key that is held repeats on a Mac. The keyboard of an iPhone or an iPad says nothing of it,
  /// a held key is one event of going down, and the window would get a single press; so on those
  /// the repeats are made here, after [repeatAfter] and then every [repeatEvery]. A platform that
  /// sends repeats of its own has them given to the window as they come.
  bool key(KeyEvent event) {
    if (!_on) return false;
    final input = windowKey(event, _modifiers());
    if (input == null) return false;
    if (event is KeyUpEvent) {
      // A key that went down before control was turned on has nothing to be let go of.
      if (!_down.remove(input.code)) return false;
      if (_holding?.key == event.physicalKey) _stopHolding();
      _send(input);
      return true;
    }
    _down.add(input.code);
    _send(input);
    if (event is KeyDownEvent) {
      chosen.spend();
      _hold(event, input);
    }
    return true;
  }

  /// Starts to repeat the key that went down, in place of the one that repeated, if this device
  /// has to make the repeats.
  void _hold(KeyDownEvent event, WindowInput_Key first) {
    _stopHolding();
    if (defaultTargetPlatform != TargetPlatform.iOS) return;
    final held = _Held(event.physicalKey, first);
    held.timer = Timer(repeatAfter, () {
      held.timer = Timer.periodic(repeatEvery, (_) => _repeat(held));
      _repeat(held);
    });
    _holding = held;
  }

  /// Repeats a key that is held, once, if it still is.
  ///
  /// A key that repeats by itself, with nobody to say that it came up, would type for ever, so
  /// this goes on only while the keyboard says that the key is down, which it does also when the
  /// release was not heard here (the focus was elsewhere), and for no longer than [repeatFor].
  void _repeat(_Held held) {
    final code = held.first.code;
    if (!_on || !_down.contains(code)) {
      _stopHolding();
      return;
    }
    if (!HardwareKeyboard.instance.physicalKeysPressed.contains(held.key)) {
      // It came up, and nothing here heard it: it is let go of now.
      _stopHolding();
      _down.remove(code);
      _send(
        WindowInput.key(
          code: code,
          down: false,
          modifiers: _modifiers(),
          repeat: false,
        ),
      );
      return;
    }
    if (repeatEvery * held.repeats >= repeatFor) {
      _stopHolding();
      return;
    }
    held.repeats++;
    // The text of the key is what it typed with the modifiers it went down with. Others change
    // it, and the other device makes the text of the key by its own layout then.
    final modifiers = _modifiers();
    _send(
      WindowInput.key(
        code: code,
        down: true,
        modifiers: modifiers,
        repeat: true,
        text: modifiers == held.first.modifiers ? held.first.text : null,
      ),
    );
  }

  void _stopHolding() {
    _holding?.timer?.cancel();
    _holding = null;
  }

  /// A key of the screen was pressed: it goes down and comes up.
  void tapKey(int code) {
    if (!_on) return;
    keyTap(code, _modifiers()).forEach(_send);
    chosen.spend();
  }

  /// The person typed on the keyboard of the screen: they deleted [deleted] characters and then
  /// typed [text].
  void type(int deleted, String text) {
    if (!_on) return;
    final modifiers = _modifiers();
    for (var i = 0; i < deleted; i++) {
      keyTap(HidKey.backspace, modifiers).forEach(_send);
    }
    typed(text, modifiers).forEach(_send);
    chosen.spend();
  }

  // MARK: Both

  WindowModifiers _modifiers() =>
      chosen.now(modifiersOf(HardwareKeyboard.instance));

  /// Gives an event to the window. Baton may not take it, and no reason for that ends the window,
  /// so the person goes on.
  void _send(WindowInput input) => windows.input(session, input);

  void _told(WindowDropped dropped) {
    if (dropped.session.id != session.id) return;
    _notice = describeDropped(
      dropped.reason,
      session.deviceName,
      display: session.display,
    );
    _noticeTimer?.cancel();
    _noticeTimer = Timer(noticeFor, () {
      _notice = null;
      notifyListeners();
    });
    notifyListeners();
  }
}

/// The key of a keyboard that is held and repeats.
class _Held {
  _Held(this.key, this.first);

  /// The key, to ask the keyboard whether it is still down.
  final PhysicalKeyboardKey key;

  /// What the key did when it went down: the code of its repeats, and the modifiers and the text
  /// that they have as long as the modifiers are the same.
  final WindowInput_Key first;

  /// The wait before the first repeat, and then the repeats.
  Timer? timer;

  /// How many times it has repeated.
  int repeats = 0;
}

/// What the other device did not do of what the person did on a window, or on a whole screen when
/// [display], in a sentence for the person. [device] is the name of the other device, which came
/// from it.
String describeDropped(
  WindowNoticeReason reason,
  String device, {
  bool display = false,
}) {
  final name = visibleText(device);
  final noun = display ? 'screen' : 'window';
  return switch (reason) {
    WindowNoticeReason.controlOff =>
      '$name has not let this device control the $noun.',
    WindowNoticeReason.paused => 'The $noun is paused on $name.',
    WindowNoticeReason.locked => '$name is locked.',
    WindowNoticeReason.notOnScreen =>
      display
          ? 'The screen is gone from $name.'
          : 'The window is not on the screen of $name.',
    WindowNoticeReason.tooFast => 'That was faster than $name takes.',
    WindowNoticeReason.outOfDate =>
      'The picture was out of date, so that was not done. Try again.',
    WindowNoticeReason.pictureUnknown =>
      '$name does not know the picture that you touched, so that was not '
          'done. Try again.',
    WindowNoticeReason.resized =>
      'The $noun changed its size, so that was not done.',
    WindowNoticeReason.outsideWindow => 'That was outside the $noun.',
    WindowNoticeReason.covered =>
      'Another window is in front of that place on $name.',
    WindowNoticeReason.noKeyboard =>
      'The window does not have the keyboard on $name.',
    WindowNoticeReason.appWindowInFront =>
      'Another window of the same app is in front on $name, so a key might go '
          'to it.',
    WindowNoticeReason.tooManyKeys => 'Too many keys are held down at once.',
    WindowNoticeReason.notPressed =>
      'A key or a button was let go that was not pressed.',
    WindowNoticeReason.other => '$name did not do that.',
  };
}
