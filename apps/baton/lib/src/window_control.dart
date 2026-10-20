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
/// fingers on its picture, the keys of a keyboard that is attached to this device, the keys that
/// the screen has of its own, and the text typed on the keyboard of the screen.
///
/// All of it is given to the window as [WindowInput]. What may be sent is decided in Rust, once,
/// and what the other device does not do comes back as a [notice]. Nothing here is on until the
/// person turns it on, and turning it off, or leaving, lets go of every button and key that is
/// down.
class WindowControl extends ChangeNotifier {
  WindowControl({
    required this.windows,
    required this.session,
    required this.pictureSize,
    this.noticeFor = const Duration(seconds: 4),
  }) {
    gestures = TouchGestures(
      send: _send,
      unit: _unit,
      modifiers: _modifiers,
      onLongPress: _longPress,
    );
  }

  final WindowsController windows;

  /// The window that is controlled, which this device shows.
  final WindowSession session;

  /// How large the picture is on the screen of this device, or `null` before it is laid out.
  final Size? Function() pictureSize;

  /// How long a [notice] stays.
  final Duration noticeFor;

  /// The modifier keys that the person chose on the screen for the next thing they do.
  final ChosenModifiers chosen = ChosenModifiers();

  /// What the fingers on the picture do.
  late final TouchGestures gestures;

  /// The keys of a keyboard that were sent as down and not yet as up.
  final Set<int> _down = {};
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
    HardwareKeyboard.instance.addHandler(key);
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
    HardwareKeyboard.instance.removeHandler(key);
    unawaited(_dropped?.cancel());
    _dropped = null;
    _noticeTimer?.cancel();
    _noticeTimer = null;
    _notice = null;
    gestures.reset();
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

  // MARK: Fingers

  /// A finger touched the picture. `event.localPosition` has to be a place on the picture. Only
  /// a new finger is asked whether control is on: the gestures know no other finger while it is
  /// off, and leave the ones they do not know alone.
  void pointerDown(PointerDownEvent event) {
    final size = pictureSize();
    if (!_on || size == null) return;
    gestures
      ..scrollGain = _scrollGain(size)
      ..down(event.pointer, event.localPosition, event.position);
  }

  void pointerMove(PointerMoveEvent event) {
    gestures.move(event.pointer, event.localPosition, event.position);
  }

  void pointerUp(PointerUpEvent event) {
    gestures.up(event.pointer, event.localPosition);
    _lifted();
  }

  void pointerCancel(PointerCancelEvent event) {
    gestures.cancel(event.pointer);
    _lifted();
  }

  /// What was chosen on the screen is for the gesture that has ended.
  void _lifted() {
    if (!gestures.touching) chosen.spend();
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
  /// does not have, like the Globe key, is left to the system.
  bool key(KeyEvent event) {
    if (!_on) return false;
    final input = windowKey(event, _modifiers());
    if (input == null) return false;
    if (event is KeyUpEvent) {
      // A key that went down before control was turned on has nothing to be let go of.
      if (!_down.remove(input.code)) return false;
      _send(input);
      return true;
    }
    _down.add(input.code);
    _send(input);
    if (event is KeyDownEvent) chosen.spend();
    return true;
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
    _notice = describeDropped(dropped.reason, session.deviceName);
    _noticeTimer?.cancel();
    _noticeTimer = Timer(noticeFor, () {
      _notice = null;
      notifyListeners();
    });
    notifyListeners();
  }
}

/// What the other device did not do of what the person did on a window, in a sentence for the
/// person. [device] is the name of the other device, which came from it.
String describeDropped(WindowNoticeReason reason, String device) {
  final name = visibleText(device);
  return switch (reason) {
    WindowNoticeReason.controlOff =>
      '$name has not let this device control the window.',
    WindowNoticeReason.paused => 'The window is paused on $name.',
    WindowNoticeReason.locked => '$name is locked.',
    WindowNoticeReason.notOnScreen =>
      'The window is not on the screen of $name.',
    WindowNoticeReason.tooFast => 'That was faster than $name takes.',
    WindowNoticeReason.outOfDate =>
      'The picture was out of date, so that was not done. Try again.',
    WindowNoticeReason.resized =>
      'The window changed its size, so that was not done.',
    WindowNoticeReason.outsideWindow => 'That was outside the window.',
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
