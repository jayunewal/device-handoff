// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_keys.dart';
import 'package:baton/src/format.dart';
import 'package:baton/src/screens/window_screen.dart';
import 'package:baton/src/window_control.dart';
import 'package:baton/src/windows_controller.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

/// Everything a test of control holds on to.
class _Shown {
  _Shown(this.session, this.controller);

  final FakeSession session;
  final WindowsController controller;

  /// What the window was given, without the number of the window, which is 1.
  List<WindowInput> get given => [
    for (final (:session, :input) in session.windowInputs)
      if (session == 1) input,
  ];

  /// Forgets what the window was given.
  void forget() => session.windowInputs.clear();
}

/// The window of the tests, a wide one that is shown across the whole view: the picture is 900 by
/// 562.5 points, and its corner is where [_corner] says.
WindowSession _window({
  bool control = true,
  int width = 1600,
  int height = 1000,
}) => sampleWindowSession(control: control, width: width, height: height);

const _me = Me(
  id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
);

/// Opens the screen of the window over a page that says "Home", once Baton has said that the
/// window began.
Future<_Shown> _open(
  WidgetTester tester, {
  bool control = true,
  int width = 1600,
  int height = 1000,
}) async {
  final shown = _window(control: control, width: width, height: height);
  final session = FakeSession(me: _me);
  final controller = WindowsController(session, screen: FakeScreen());
  await controller.start();
  addTearDown(controller.dispose);
  session.emit(Event.windowStarted(session: shown));
  await tester.pump();
  tester.view
    ..physicalSize = const Size(900, 1600)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: Builder(
        builder: (context) => Scaffold(
          body: Center(
            child: TextButton(
              onPressed: () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => WindowScreen(
                    controller: controller,
                    session: shown,
                    screen: FakeScreen(),
                  ),
                ),
              ),
              child: const Text('Home'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Home'));
  await tester.pumpAndSettle();
  return _Shown(session, controller);
}

/// Turns control on, with the switch in the bar at the top.
Future<void> _turnOn(WidgetTester tester) async {
  await tester.tap(find.text('Control'));
  await tester.pumpAndSettle();
}

/// The top left corner of the picture.
Offset _corner(WidgetTester tester) => tester.getTopLeft(find.byType(Texture));

/// The place on the screen where the picture is at [x] across and [y] down, from 0 to 1.
Offset _at(WidgetTester tester, double x, double y) {
  final size = tester.getSize(find.byType(Texture));
  return _corner(tester) + Offset(size.width * x, size.height * y);
}

WindowInput _pointer(
  double x,
  double y,
  WindowPointer action, {
  WindowModifiers modifiers = noModifiers,
}) => WindowInput.pointer(x: x, y: y, modifiers: modifiers, action: action);

WindowInput _key(
  int code, {
  bool down = true,
  WindowModifiers modifiers = noModifiers,
  bool repeat = false,
  String? text,
}) => WindowInput.key(
  code: code,
  down: down,
  modifiers: modifiers,
  repeat: repeat,
  text: text,
);

List<WindowInput> _tap(int code, [WindowModifiers modifiers = noModifiers]) => [
  _key(code, modifiers: modifiers),
  _key(code, down: false, modifiers: modifiers),
];

List<WindowInput> _click(
  double x,
  double y, {
  WindowButton button = WindowButton.left,
  WindowModifiers modifiers = noModifiers,
}) => [
  _pointer(x, y, const WindowPointer.move(), modifiers: modifiers),
  _pointer(x, y, WindowPointer.down(button: button), modifiers: modifiers),
  _pointer(x, y, WindowPointer.up(button: button), modifiers: modifiers),
];

const _command = WindowModifiers(
  shift: false,
  control: false,
  option: false,
  command: true,
  capsLock: false,
);

const _shift = WindowModifiers(
  shift: true,
  control: false,
  option: false,
  command: false,
  capsLock: false,
);

// The events of a keyboard that are given to the control straight, for what only its answer
// shows.
const _aDown = KeyDownEvent(
  physicalKey: PhysicalKeyboardKey.keyA,
  logicalKey: LogicalKeyboardKey.keyA,
  character: 'a',
  timeStamp: Duration.zero,
);
const _aRepeat = KeyRepeatEvent(
  physicalKey: PhysicalKeyboardKey.keyA,
  logicalKey: LogicalKeyboardKey.keyA,
  character: 'a',
  timeStamp: Duration.zero,
);
const _aUp = KeyUpEvent(
  physicalKey: PhysicalKeyboardKey.keyA,
  logicalKey: LogicalKeyboardKey.keyA,
  timeStamp: Duration.zero,
);
const _fnDown = KeyDownEvent(
  physicalKey: PhysicalKeyboardKey.fn,
  logicalKey: LogicalKeyboardKey.fn,
  timeStamp: Duration.zero,
);
const _fnUp = KeyUpEvent(
  physicalKey: PhysicalKeyboardKey.fn,
  logicalKey: LogicalKeyboardKey.fn,
  timeStamp: Duration.zero,
);

// The events of a finger, with the same place on the picture and on the screen.
PointerDownEvent _fingerDown(int pointer, Offset at) =>
    PointerDownEvent(pointer: pointer, position: at);
PointerMoveEvent _fingerMoved(int pointer, Offset at) =>
    PointerMoveEvent(pointer: pointer, position: at);
PointerUpEvent _fingerUp(int pointer, Offset at) =>
    PointerUpEvent(pointer: pointer, position: at);
PointerCancelEvent _fingerCancelled(int pointer, Offset at) =>
    PointerCancelEvent(pointer: pointer, position: at);

/// A control with no screen around it, for what only the control does. The picture that it is
/// told of has the size [picture], which a test can change or take away.
class _Control {
  _Control._(this.backend, this.windows, this.window) {
    control = WindowControl(
      windows: windows,
      session: window,
      pictureSize: () => picture,
    );
  }

  /// Opens the window as a window that runs here, and a control for it.
  static Future<_Control> start(
    WidgetTester tester, {
    int width = 1600,
    int height = 1000,
  }) async {
    final backend = FakeSession(me: _me);
    final windows = WindowsController(backend, screen: FakeScreen());
    await windows.start();
    addTearDown(windows.dispose);
    final window = _window(width: width, height: height);
    backend.emit(Event.windowStarted(session: window));
    await tester.pump();
    final started = _Control._(backend, windows, window);
    addTearDown(started.control.dispose);
    return started;
  }

  final FakeSession backend;
  final WindowsController windows;
  final WindowSession window;
  late final WindowControl control;

  /// How large the picture is on the screen, or `null` before it is laid out.
  Size? picture = const Size(400, 300);

  /// What the window was given.
  List<WindowInput> get given => [
    for (final (:session, :input) in backend.windowInputs)
      if (session == 1) input,
  ];

  /// Forgets what the window was given.
  void forget() => backend.windowInputs.clear();
}

void main() {
  group('the switch for control', () {
    testWidgets('is not there for a window that cannot be controlled', (
      tester,
    ) async {
      await _open(tester, control: false);

      expect(find.text('Control'), findsNothing);
      expect(find.text('Esc'), findsNothing);
    });

    testWidgets('is there for a window that can, and control is off', (
      tester,
    ) async {
      final shown = await _open(tester);

      expect(find.text('Control'), findsOneWidget);
      expect(find.text('Esc'), findsNothing);
      expect(find.byType(EditableText), findsNothing);
      expect(
        tester
            .widget<FilterChip>(find.widgetWithText(FilterChip, 'Control'))
            .selected,
        false,
      );
      expect(shown.session.windowInputsReleased, isEmpty);
    });

    testWidgets('turns control on, with the keys of the screen', (
      tester,
    ) async {
      await _open(tester);

      await _turnOn(tester);

      expect(
        tester
            .widget<FilterChip>(find.widgetWithText(FilterChip, 'Control'))
            .selected,
        true,
      );
      for (final key in ['Shift', 'Ctrl', 'Option', 'Cmd', 'Esc', 'Tab']) {
        expect(find.text(key), findsOneWidget, reason: key);
      }
      expect(find.byTooltip('Bring up the keyboard'), findsOneWidget);
      expect(find.byTooltip('Left arrow'), findsOneWidget);
      expect(find.byTooltip('Up arrow'), findsOneWidget);
      expect(find.byTooltip('Down arrow'), findsOneWidget);
      expect(find.byTooltip('Right arrow'), findsOneWidget);
    });

    testWidgets('turns control off, and lets go of what is held', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await _turnOn(tester);

      expect(find.text('Esc'), findsNothing);
      expect(shown.session.windowInputsReleased, [1]);
      expect(find.byType(EditableText), findsNothing);
    });

    testWidgets('says what it does, and then what it undoes', (tester) async {
      await _open(tester);
      expect(
        find.byTooltip('Control the window from this device'),
        findsOneWidget,
      );
      expect(find.byTooltip('Stop controlling the window'), findsNothing);

      await _turnOn(tester);

      expect(find.byTooltip('Stop controlling the window'), findsOneWidget);
      expect(
        find.byTooltip('Control the window from this device'),
        findsNothing,
      );
    });
  });

  group('a touch on the picture', () {
    testWidgets('does nothing while control is off', (tester) async {
      final shown = await _open(tester);

      await tester.tapAt(_at(tester, 0.5, 0.5));
      await tester.pump(const Duration(seconds: 1));

      expect(shown.session.windowInputs, isEmpty);
    });

    testWidgets('is a click where the picture was touched', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.tapAt(_at(tester, 0.25, 0.75));
      await tester.pump();

      expect(shown.given, _click(0.25, 0.75));
      expect(shown.session.windowInputs.map((e) => e.session).toSet(), {1});
    });

    testWidgets('is not told when it is not on the picture', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.tapAt(_corner(tester) - const Offset(0, 20));
      await tester.pump(const Duration(seconds: 1));

      expect(shown.session.windowInputs, isEmpty);
    });

    testWidgets('stays a click when the finger moves a little', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      final finger = await tester.startGesture(_at(tester, 0.5, 0.5));
      await finger.moveBy(const Offset(4, 3));
      await finger.up();
      await tester.pump();

      expect(shown.given, _click(0.5, 0.5));
    });

    testWidgets('drags with the left button when the finger moves', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      final size = tester.getSize(find.byType(Texture));

      final finger = await tester.startGesture(_at(tester, 0.5, 0.5));
      await finger.moveBy(Offset(0, size.height * 0.2));
      await finger.moveBy(Offset(size.width * 0.1, 0));
      await finger.up();
      await tester.pump();

      expect(shown.given, [
        _pointer(0.5, 0.5, const WindowPointer.move()),
        _pointer(0.5, 0.5, const WindowPointer.down(button: WindowButton.left)),
        _pointer(0.5, 0.7, const WindowPointer.move()),
        _pointer(0.6, 0.7, const WindowPointer.move()),
        _pointer(0.6, 0.7, const WindowPointer.up(button: WindowButton.left)),
      ]);
    });

    testWidgets('is a click with the right button when the finger stays', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      final finger = await tester.startGesture(_at(tester, 0.4, 0.6));
      await tester.pump(const Duration(milliseconds: 500));
      await finger.up();
      await tester.pump();

      expect(shown.given, _click(0.4, 0.6, button: WindowButton.right));
    });

    testWidgets('is a scroll when two fingers move', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      final first = await tester.startGesture(
        _at(tester, 0.4, 0.5),
        pointer: 1,
      );
      final second = await tester.startGesture(
        _at(tester, 0.6, 0.5),
        pointer: 2,
      );
      await first.moveBy(const Offset(0, 20));
      await second.moveBy(const Offset(0, 20));
      await first.up();
      await second.up();
      await tester.pump();

      final scrolls = shown.given.whereType<WindowInput_Pointer>().toList();
      expect(scrolls, isNotEmpty);
      var down = 0.0;
      for (final scroll in scrolls) {
        final action = scroll.action;
        expect(action, isA<WindowPointer_Scroll>());
        action as WindowPointer_Scroll;
        expect(action.dx, 0);
        expect(scroll.x, closeTo(0.5, 0.0001));
        down += action.dy;
      }
      // The fingers moved 20 points together, and the picture has about as many pixels as the
      // screen has points, so the window moves as much as they do.
      expect(down, closeTo(20, 0.0001));
    });

    testWidgets('is a scroll that is larger for a larger window', (
      tester,
    ) async {
      final shown = await _open(tester);
      shown.session.emit(
        const Event.windowPicture(id: 1, width: 5400, height: 3375),
      );
      await tester.pump();
      await _turnOn(tester);

      final first = await tester.startGesture(
        _at(tester, 0.4, 0.5),
        pointer: 1,
      );
      final second = await tester.startGesture(
        _at(tester, 0.6, 0.5),
        pointer: 2,
      );
      await first.moveBy(const Offset(0, 10));
      await second.moveBy(const Offset(0, 10));
      await first.up();
      await second.up();
      await tester.pump();

      var down = 0.0;
      for (final scroll in shown.given.whereType<WindowInput_Pointer>()) {
        down += (scroll.action as WindowPointer_Scroll).dy;
      }
      // 5400 pixels across a picture of 900 points, and half as many points as pixels: 3 times.
      expect(down, closeTo(30, 0.0001));
    });

    testWidgets('does not zoom the picture while control is on', (
      tester,
    ) async {
      await _open(tester);
      InteractiveViewer viewer() =>
          tester.widget<InteractiveViewer>(find.byType(InteractiveViewer));
      expect(viewer().panEnabled, isTrue);
      expect(viewer().scaleEnabled, isTrue);

      await _turnOn(tester);
      expect(viewer().panEnabled, isFalse);
      expect(viewer().scaleEnabled, isFalse);

      await _turnOn(tester);
      expect(viewer().panEnabled, isTrue);
      expect(viewer().scaleEnabled, isTrue);
    });

    testWidgets('keeps the picture where it was when control is turned on', (
      tester,
    ) async {
      await _open(tester);
      final before = tester.widget<Texture>(find.byType(Texture));
      final element = tester.element(find.byType(Texture));

      await _turnOn(tester);

      expect(tester.element(find.byType(Texture)), same(element));
      expect(
        tester.widget<Texture>(find.byType(Texture)).textureId,
        before.textureId,
      );
    });

    testWidgets('is let go of when control is turned off in a drag', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      final finger = await tester.startGesture(_at(tester, 0.5, 0.5));
      await finger.moveBy(const Offset(0, 100));
      shown.forget();
      await tester.tap(find.text('Control'));
      await tester.pumpAndSettle();

      expect(shown.given, hasLength(1));
      expect(
        (shown.given.single as WindowInput_Pointer).action,
        const WindowPointer.up(button: WindowButton.left),
      );
      expect(shown.session.windowInputsReleased, [1]);

      await finger.up();
      await tester.pump();
      expect(
        shown.given,
        hasLength(1),
        reason: 'the finger is not heard after',
      );
    });

    testWidgets('lets go of the left button when the system takes the finger', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      final finger = await tester.startGesture(_at(tester, 0.5, 0.5));
      await finger.moveBy(const Offset(0, 100));
      shown.forget();

      await finger.cancel();
      await tester.pump();

      expect(shown.given, [
        _pointer(
          0.5,
          0.5 + 100 / 562.5,
          const WindowPointer.up(button: WindowButton.left),
        ),
      ]);
      expect(shown.session.windowInputsReleased, isEmpty);
      expect(find.text('Esc'), findsOneWidget, reason: 'control stays on');
    });
  });

  group('the keys of the screen', () {
    testWidgets('Escape and Tab go down and come up', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.tap(find.text('Esc'));
      await tester.tap(find.text('Tab'));

      expect(shown.given, [..._tap(HidKey.escape), ..._tap(HidKey.tab)]);
    });

    testWidgets('the arrows are the arrow keys', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.tap(find.byTooltip('Left arrow'));
      await tester.tap(find.byTooltip('Up arrow'));
      await tester.tap(find.byTooltip('Down arrow'));
      await tester.tap(find.byTooltip('Right arrow'));

      expect(shown.given, [
        ..._tap(HidKey.left),
        ..._tap(HidKey.up),
        ..._tap(HidKey.down),
        ..._tap(HidKey.right),
      ]);
    });

    testWidgets('a modifier that is chosen is for the next key only', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.tap(find.widgetWithText(FilterChip, 'Cmd'));
      await tester.pump();
      expect(
        tester
            .widget<FilterChip>(find.widgetWithText(FilterChip, 'Cmd'))
            .selected,
        isTrue,
      );
      await tester.tap(find.byTooltip('Left arrow'));
      await tester.pump();
      await tester.tap(find.byTooltip('Left arrow'));

      expect(shown.given, [
        ..._tap(HidKey.left, _command),
        ..._tap(HidKey.left),
      ]);
      expect(
        tester
            .widget<FilterChip>(find.widgetWithText(FilterChip, 'Cmd'))
            .selected,
        isFalse,
      );
    });

    testWidgets('a modifier that is chosen can be taken back', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.tap(find.widgetWithText(FilterChip, 'Cmd'));
      await tester.pump();
      await tester.tap(find.widgetWithText(FilterChip, 'Cmd'));
      await tester.pump();
      await tester.tap(find.text('Esc'));

      expect(shown.given, _tap(HidKey.escape));
    });

    testWidgets('each modifier is the one it says', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      for (final name in ['Shift', 'Ctrl', 'Option', 'Cmd']) {
        await tester.tap(find.widgetWithText(FilterChip, name));
        await tester.pump();
        await tester.tap(find.text('Tab'));
        await tester.pump();
      }

      WindowModifiers only({
        bool shift = false,
        bool control = false,
        bool option = false,
        bool command = false,
      }) => WindowModifiers(
        shift: shift,
        control: control,
        option: option,
        command: command,
        capsLock: false,
      );
      expect(shown.given, [
        ..._tap(HidKey.tab, only(shift: true)),
        ..._tap(HidKey.tab, only(control: true)),
        ..._tap(HidKey.tab, only(option: true)),
        ..._tap(HidKey.tab, only(command: true)),
      ]);
    });

    testWidgets('a modifier that is chosen is used up by a click', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.tap(find.widgetWithText(FilterChip, 'Cmd'));
      await tester.pump();
      await tester.tapAt(_at(tester, 0.5, 0.5));
      await tester.pump();
      await tester.tapAt(_at(tester, 0.5, 0.5));
      await tester.pump();

      expect(shown.given, [
        ..._click(0.5, 0.5, modifiers: _command),
        ..._click(0.5, 0.5),
      ]);
    });

    testWidgets('a modifier that is chosen lasts through a drag', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.widgetWithText(FilterChip, 'Cmd'));
      await tester.pump();

      final finger = await tester.startGesture(_at(tester, 0.5, 0.5));
      await finger.moveBy(const Offset(0, 40));
      await finger.moveBy(const Offset(0, 40));
      await finger.up();
      await tester.pump();

      expect(shown.given, isNotEmpty);
      for (final input in shown.given) {
        expect((input as WindowInput_Pointer).modifiers, _command);
      }
      await tester.tapAt(_at(tester, 0.5, 0.5));
      shown.forget();
      await tester.tap(find.text('Esc'));
      expect(shown.given, _tap(HidKey.escape));
    });
  });

  group('the keys of a keyboard', () {
    testWidgets('go to the window while control is on', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);

      expect(shown.given, [_key(0x04, text: 'a'), _key(0x04, down: false)]);
    });

    testWidgets('are left alone while control is off', (tester) async {
      final shown = await _open(tester);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);

      expect(shown.session.windowInputs, isEmpty);
    });

    testWidgets('a key that is held repeats', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyRepeatEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.arrowDown);

      expect(shown.given, [
        _key(HidKey.down),
        _key(HidKey.down, repeat: true),
        _key(HidKey.down, down: false),
      ]);
    });

    testWidgets('carry the modifiers that are down, which are no keys', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
      expect(shown.given, isEmpty);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyC, character: 'c');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyC);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);

      expect(shown.given, [
        _key(0x06, modifiers: _command, text: 'c'),
        _key(0x06, down: false, modifiers: _command),
      ]);
    });

    testWidgets('are the modifiers of a click too', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaRight);
      await tester.tapAt(_at(tester, 0.5, 0.5));
      await tester.pump();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaRight);

      expect(shown.given, _click(0.5, 0.5, modifiers: _command));
    });

    testWidgets('add to the modifiers that are chosen on the screen', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.widgetWithText(FilterChip, 'Cmd'));
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'A');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);

      const both = WindowModifiers(
        shift: true,
        control: false,
        option: false,
        command: true,
        capsLock: false,
      );
      const shift = WindowModifiers(
        shift: true,
        control: false,
        option: false,
        command: false,
        capsLock: false,
      );
      expect(shown.given, [
        _key(0x04, modifiers: both, text: 'A'),
        _key(0x04, down: false, modifiers: shift),
      ]);
    });

    testWidgets('a key that the window does not have is left to the system', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.fn);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.fn);

      expect(shown.session.windowInputs, isEmpty);
    });

    testWidgets('a key that went down before control was on is not let go of', (
      tester,
    ) async {
      final shown = await _open(tester);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');
      await _turnOn(tester);

      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);

      expect(shown.session.windowInputs, isEmpty);
    });

    testWidgets('a key that is held when control is turned off is let go of', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');

      await _turnOn(tester);
      shown.forget();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);

      expect(shown.session.windowInputsReleased, [1]);
      expect(shown.session.windowInputs, isEmpty);
    });
  });

  group('the keyboard of the screen', () {
    bool focused(WidgetTester tester) => tester
        .widget<EditableText>(find.byType(EditableText))
        .focusNode
        .hasFocus;

    Future<void> type(WidgetTester tester, String text, {int? left}) async {
      final anchor = TypingField.anchor;
      final kept = left ?? anchor.length;
      final full = anchor.substring(0, kept) + text;
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: full,
          selection: TextSelection.collapsed(offset: full.length),
        ),
      );
      await tester.pump();
    }

    testWidgets('is brought up by its button, and put down by it again', (
      tester,
    ) async {
      await _open(tester);
      await _turnOn(tester);
      expect(focused(tester), isFalse);
      expect(find.byIcon(Icons.keyboard_outlined), findsOneWidget);
      expect(find.byIcon(Icons.keyboard_hide_outlined), findsNothing);

      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.pump();
      expect(focused(tester), isTrue);
      expect(tester.testTextInput.isVisible, isTrue);
      expect(find.byIcon(Icons.keyboard_hide_outlined), findsOneWidget);
      expect(find.byIcon(Icons.keyboard_outlined), findsNothing);

      await tester.tap(find.byTooltip('Put the keyboard down'));
      await tester.pump();
      expect(focused(tester), isFalse);
      expect(find.byIcon(Icons.keyboard_outlined), findsOneWidget);
    });

    testWidgets('is put down when control is turned off', (tester) async {
      await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.pump();

      await _turnOn(tester);

      expect(find.byType(EditableText), findsNothing);
      expect(tester.testTextInput.isVisible, isFalse);
    });

    testWidgets('has nothing to correct, suggest or capitalize', (
      tester,
    ) async {
      await _open(tester);
      await _turnOn(tester);

      final field = tester.widget<EditableText>(find.byType(EditableText));
      expect(field.autocorrect, isFalse);
      expect(field.enableSuggestions, isFalse);
      expect(field.smartDashesType, SmartDashesType.disabled);
      expect(field.smartQuotesType, SmartQuotesType.disabled);
      expect(field.textCapitalization, TextCapitalization.none);
      expect(field.keyboardType, TextInputType.multiline);
      expect(field.maxLines, isNull);
    });

    testWidgets('sends what is typed as text, and is made whole again', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.pump();

      await type(tester, 'hello');

      expect(shown.given, [_key(0, text: 'hello')]);
      final state = tester.state<EditableTextState>(find.byType(EditableText));
      expect(state.textEditingValue.text, TypingField.anchor);
      expect(
        state.textEditingValue.selection.baseOffset,
        TypingField.anchor.length,
      );

      await type(tester, 'again');
      expect(shown.given, [_key(0, text: 'hello'), _key(0, text: 'again')]);
    });

    testWidgets('sends Backspace for what is deleted', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.pump();

      await type(tester, '', left: TypingField.anchor.length - 1);
      await type(tester, '', left: TypingField.anchor.length - 1);

      expect(shown.given, [
        ..._tap(HidKey.backspace),
        ..._tap(HidKey.backspace),
      ]);
      expect(
        tester
            .state<EditableTextState>(find.byType(EditableText))
            .textEditingValue
            .text,
        TypingField.anchor,
      );
    });

    testWidgets('sends Return for a line end', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.pump();

      await type(tester, 'ok\n');

      expect(shown.given, [_key(0, text: 'ok'), ..._tap(HidKey.enter)]);
    });

    testWidgets('sends what is composed when it is done, and not before', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.pump();
      final anchor = TypingField.anchor;

      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: '${anchor}ni',
          selection: TextSelection.collapsed(offset: anchor.length + 2),
          composing: TextRange(start: anchor.length, end: anchor.length + 2),
        ),
      );
      await tester.pump();
      expect(shown.given, isEmpty);

      await type(tester, '你');
      expect(shown.given, [_key(0, text: '你')]);
    });

    testWidgets('does not send a move of the cursor', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.pump();

      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: TypingField.anchor,
          selection: const TextSelection.collapsed(offset: 10),
        ),
      );
      await tester.pump();

      expect(shown.given, isEmpty);
      expect(
        tester
            .state<EditableTextState>(find.byType(EditableText))
            .textEditingValue
            .selection
            .baseOffset,
        TypingField.anchor.length,
      );
    });

    testWidgets('uses the modifiers that are chosen, and uses them up', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      await tester.tap(find.byTooltip('Bring up the keyboard'));
      await tester.tap(find.widgetWithText(FilterChip, 'Cmd'));
      await tester.pump();

      await type(tester, 'a');
      await type(tester, 'b');

      expect(shown.given, [..._tap(0x04, _command), _key(0, text: 'b')]);
    });
  });

  group('what the other device did not do', () {
    Event notice(WindowNoticeReason reason, {int id = 1}) =>
        Event.windowNotice(id: id, reason: reason, count: 2);

    testWidgets('is said over the picture, and goes away after a while', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      shown.session.emit(notice(WindowNoticeReason.noKeyboard));
      await tester.pump(Duration.zero);

      expect(
        find.text('The window does not have the keyboard on Anna’s Mac.'),
        findsOneWidget,
      );
      await tester.pump(const Duration(seconds: 3));
      expect(
        find.text('The window does not have the keyboard on Anna’s Mac.'),
        findsOneWidget,
      );
      await tester.pump(const Duration(seconds: 1));
      expect(
        find.text('The window does not have the keyboard on Anna’s Mac.'),
        findsNothing,
      );
    });

    testWidgets('is replaced by the one that follows', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      shown.session.emit(notice(WindowNoticeReason.locked));
      await tester.pump(Duration.zero);
      await tester.pump(const Duration(seconds: 3));
      shown.session.emit(notice(WindowNoticeReason.tooFast));
      await tester.pump(Duration.zero);

      expect(find.text('Anna’s Mac is locked.'), findsNothing);
      expect(
        find.text('That was faster than Anna’s Mac takes.'),
        findsOneWidget,
      );
      await tester.pump(const Duration(seconds: 3));
      expect(
        find.text('That was faster than Anna’s Mac takes.'),
        findsOneWidget,
      );
      await tester.pump(const Duration(seconds: 1));
      expect(find.text('That was faster than Anna’s Mac takes.'), findsNothing);
    });

    testWidgets('is not said while control is off', (tester) async {
      final shown = await _open(tester);

      shown.session.emit(notice(WindowNoticeReason.locked));
      await tester.pump(Duration.zero);

      expect(find.text('Anna’s Mac is locked.'), findsNothing);
    });

    testWidgets('is gone when control is turned off', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      shown.session.emit(notice(WindowNoticeReason.locked));
      await tester.pump(Duration.zero);

      await _turnOn(tester);

      expect(find.text('Anna’s Mac is locked.'), findsNothing);
      await tester.pump(const Duration(seconds: 10));
      await _turnOn(tester);
      expect(find.text('Anna’s Mac is locked.'), findsNothing);
    });

    testWidgets('lets the touches through', (tester) async {
      // A window so tall that its picture reaches the top, where the notice is.
      final shown = await _open(tester, width: 900, height: 1500);
      await _turnOn(tester);
      shown.session.emit(notice(WindowNoticeReason.locked));
      await tester.pump(Duration.zero);
      final top = tester.getTopLeft(find.text('Anna’s Mac is locked.'));

      await tester.tapAt(top + const Offset(2, 2));
      await tester.pump();

      expect(shown.given, hasLength(3));
    });

    testWidgets('is said for the window that was touched and no other', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      shown.session.emit(notice(WindowNoticeReason.locked, id: 2));
      await tester.pump(Duration.zero);

      expect(find.text('Anna’s Mac is locked.'), findsNothing);
    });
  });

  group('when the window ends or is left', () {
    testWidgets('control stops, and what is held is let go of', (tester) async {
      final shown = await _open(tester);
      await _turnOn(tester);

      shown.session.emit(
        const Event.windowEnded(id: 1, end: WindowEnd.otherDevice()),
      );
      await tester.pumpAndSettle();

      expect(find.text('Esc'), findsNothing);
      expect(find.text('Control'), findsNothing);
      expect(shown.session.windowInputsReleased, [1]);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);
      expect(shown.session.windowInputs, isEmpty);
    });

    testWidgets('the screen going away lets go of what is held', (
      tester,
    ) async {
      final shown = await _open(tester);
      await _turnOn(tester);
      final finger = await tester.startGesture(_at(tester, 0.5, 0.5));
      await finger.moveBy(const Offset(0, 60));
      shown.forget();

      await tester.pumpWidget(const SizedBox());

      expect(shown.session.windowInputsReleased, [1]);
      expect(shown.given, [
        _pointer(
          0.5,
          0.5 + 60 / 562.5,
          const WindowPointer.up(button: WindowButton.left),
        ),
      ]);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);
      expect(shown.given, hasLength(1));
    });

    testWidgets('a screen that never controlled lets go of nothing', (
      tester,
    ) async {
      final shown = await _open(tester);

      await tester.pumpWidget(const SizedBox());

      expect(shown.session.windowInputsReleased, isEmpty);
    });
  });

  group('the control, without a screen', () {
    testWidgets('lets go of what is held only once, when turned off', (
      tester,
    ) async {
      final session = FakeSession(me: _me);
      final controller = WindowsController(session, screen: FakeScreen());
      await controller.start();
      addTearDown(controller.dispose);
      final control = WindowControl(
        windows: controller,
        session: _window(),
        pictureSize: () => null,
      );
      addTearDown(control.dispose);
      var told = 0;
      control.addListener(() => told++);

      control.turnOff();
      expect(session.windowInputsReleased, isEmpty, reason: 'it was never on');
      expect(told, 0);

      control.turnOn();
      control.turnOff();
      control.turnOff();
      expect(session.windowInputsReleased, [1]);
      expect(told, 2);
    });

    testWidgets('forgets what was chosen when it is turned off', (
      tester,
    ) async {
      final rig = await _Control.start(tester);
      rig.control
        ..turnOn()
        ..chosen.toggle(Modifier.command);

      rig.control.turnOff();

      expect(rig.control.chosen.has(Modifier.command), isFalse);
    });

    testWidgets('can choose nothing once it is disposed', (tester) async {
      final rig = await _Control.start(tester);
      final control = WindowControl(
        windows: rig.windows,
        session: rig.window,
        pictureSize: () => null,
      );

      control.dispose();

      expect(() => control.chosen.toggle(Modifier.shift), throwsFlutterError);
    });

    group('the keys of a keyboard', () {
      testWidgets('are heard once, however often it was turned on', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..turnOn();

        await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');
        await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);

        expect(rig.given, [_key(0x04, text: 'a'), _key(0x04, down: false)]);
      });

      testWidgets('are heard once after it was turned off and on again', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..turnOff()
          ..turnOn();

        await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA, character: 'a');
        await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);

        expect(rig.given, [_key(0x04, text: 'a'), _key(0x04, down: false)]);
      });

      testWidgets('are not let go of if they went down before it was off', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..key(_aDown)
          ..turnOff()
          ..turnOn();
        rig.forget();

        // The window let go of the key when control went off, so this ends no press.
        expect(rig.control.key(_aUp), isFalse);
        expect(rig.given, isEmpty);
      });

      testWidgets('are taken when the window has them, and left otherwise', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.control.turnOn();

        expect(rig.control.key(_aDown), isTrue);
        expect(rig.control.key(_aRepeat), isTrue);
        expect(rig.control.key(_aUp), isTrue);
        expect(rig.control.key(_fnDown), isFalse);
        expect(rig.control.key(_fnUp), isFalse);
        expect(rig.given, [
          _key(0x04, text: 'a'),
          _key(0x04, repeat: true, text: 'a'),
          _key(0x04, down: false),
        ]);
      });

      testWidgets('are not taken, nor are the keys of the screen sent, while '
          'control is off', (tester) async {
        final rig = await _Control.start(tester);

        expect(rig.control.key(_aDown), isFalse);
        expect(rig.control.key(_aUp), isFalse);
        rig.control.tapKey(HidKey.escape);
        rig.control.type(1, 'hello');

        expect(rig.given, isEmpty);
      });

      testWidgets('use up what was chosen when they go down, not when they '
          'repeat', (tester) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..key(_aDown)
          ..chosen.toggle(Modifier.command);

        rig.control.key(_aRepeat);
        expect(rig.control.chosen.has(Modifier.command), isTrue);

        rig.control
          ..key(_aUp)
          ..key(_aDown);
        expect(rig.control.chosen.has(Modifier.command), isFalse);
      });

      testWidgets('are deleted with the modifiers that were chosen', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..chosen.toggle(Modifier.shift);

        rig.control.type(1, '');

        expect(rig.given, _tap(HidKey.backspace, _shift));
        expect(rig.control.chosen.has(Modifier.shift), isFalse);
      });
    });

    group('the fingers', () {
      testWidgets('are not taken before the picture is laid out', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.picture = null;
        rig.control.turnOn();

        rig.control.pointerDown(_fingerDown(1, const Offset(40, 30)));
        expect(rig.control.gestures.touching, isFalse);
        rig.control.pointerUp(_fingerUp(1, const Offset(40, 30)));

        expect(rig.given, isEmpty);
      });

      testWidgets('are put at the corner on a picture that has no size', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.picture = Size.zero;
        rig.control.turnOn();

        rig.control
          ..pointerDown(_fingerDown(1, const Offset(30, 40)))
          ..pointerUp(_fingerUp(1, const Offset(30, 40)));

        expect(rig.given, _click(0, 0));
      });

      testWidgets('are measured by the picture as it is then', (tester) async {
        final rig = await _Control.start(tester);
        rig.control.turnOn();

        rig.control
          ..pointerDown(_fingerDown(1, const Offset(100, 150)))
          ..pointerUp(_fingerUp(1, const Offset(100, 150)));
        rig.picture = const Size(200, 100);
        rig.control
          ..pointerDown(_fingerDown(2, const Offset(100, 50)))
          ..pointerUp(_fingerUp(2, const Offset(100, 50)));

        expect(rig.given, [..._click(0.25, 0.5), ..._click(0.5, 0.5)]);
      });

      testWidgets('use up what was chosen when one is taken away', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..chosen.toggle(Modifier.command);
        rig.control.pointerDown(_fingerDown(1, const Offset(100, 100)));

        rig.control.pointerCancel(_fingerCancelled(1, const Offset(100, 100)));

        expect(rig.control.chosen.has(Modifier.command), isFalse);
      });

      testWidgets('use up what was chosen when the last one is lifted', (
        tester,
      ) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..chosen.toggle(Modifier.command);
        rig.control
          ..pointerDown(_fingerDown(1, const Offset(100, 100)))
          ..pointerDown(_fingerDown(2, const Offset(140, 100)));

        rig.control.pointerUp(_fingerUp(1, const Offset(100, 100)));
        expect(rig.control.chosen.has(Modifier.command), isTrue);

        rig.control.pointerUp(_fingerUp(2, const Offset(140, 100)));
        expect(rig.control.chosen.has(Modifier.command), isFalse);
      });

      testWidgets('are not heard once control is off', (tester) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..pointerDown(_fingerDown(1, const Offset(100, 100)))
          ..pointerMove(_fingerMoved(1, const Offset(100, 160)))
          ..turnOff();
        rig.forget();

        rig.control
          ..pointerMove(_fingerMoved(1, const Offset(100, 200)))
          ..pointerUp(_fingerUp(1, const Offset(100, 200)))
          ..pointerDown(_fingerDown(2, const Offset(100, 100)))
          ..pointerCancel(_fingerCancelled(2, const Offset(100, 100)));

        expect(rig.given, isEmpty);
        expect(rig.control.gestures.touching, isFalse);
      });
    });

    group('the scroll of two fingers', () {
      Future<double> gain(
        WidgetTester tester, {
        required int windowWidth,
        int? pictureWidth,
      }) async {
        final rig = await _Control.start(
          tester,
          width: windowWidth,
          height: windowWidth * 5 ~/ 8,
        );
        if (pictureWidth != null) {
          rig.backend.emit(
            Event.windowPicture(
              id: 1,
              width: pictureWidth,
              height: pictureWidth * 5 ~/ 8,
            ),
          );
          await tester.pump();
        }
        rig.control
          ..turnOn()
          ..pointerDown(_fingerDown(1, const Offset(10, 10)));
        final gain = rig.control.gestures.scrollGain;
        rig.control.turnOff();
        return gain;
      }

      // The picture is 400 points across.
      testWidgets('keeps up with the fingers by the pixels of the window', (
        tester,
      ) async {
        expect(await gain(tester, windowWidth: 1600), 2.0);
      });

      testWidgets('keeps up by the pixels of the picture once it is known', (
        tester,
      ) async {
        expect(await gain(tester, windowWidth: 800, pictureWidth: 2400), 3.0);
      });

      testWidgets('is not less than the fingers move', (tester) async {
        expect(await gain(tester, windowWidth: 400), 1.0);
      });

      testWidgets('is not more than eight times what the fingers move', (
        tester,
      ) async {
        expect(await gain(tester, windowWidth: 800, pictureWidth: 20000), 8.0);
      });
    });

    testWidgets('buzzes when a finger stayed long enough for a right click', (
      tester,
    ) async {
      final calls = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          calls.add(call);
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      final rig = await _Control.start(tester);
      rig.control
        ..turnOn()
        ..pointerDown(_fingerDown(1, const Offset(100, 150)));

      await tester.pump(const Duration(milliseconds: 499));
      expect(calls, isEmpty);
      await tester.pump(const Duration(milliseconds: 1));

      expect(calls.map((call) => (call.method, call.arguments)), [
        ('HapticFeedback.vibrate', 'HapticFeedbackType.mediumImpact'),
      ]);
      expect(rig.given, _click(0.25, 0.5, button: WindowButton.right));
    });

    group('what the other device did not do', () {
      Event notice(int id) => Event.windowNotice(
        id: id,
        reason: WindowNoticeReason.locked,
        count: 2,
      );

      testWidgets('is not heard once control is off', (tester) async {
        final rig = await _Control.start(tester);
        rig.control
          ..turnOn()
          ..turnOff();
        var told = 0;
        rig.control.addListener(() => told++);

        rig.backend.emit(notice(1));
        await tester.pump(Duration.zero);

        expect(rig.control.notice, isNull);
        expect(told, 0);
      });

      testWidgets('is heard for this window and no other', (tester) async {
        final rig = await _Control.start(tester);
        rig.backend.emit(
          Event.windowStarted(session: sampleWindowSession(id: 2)),
        );
        await tester.pump();
        rig.control.turnOn();

        rig.backend.emit(notice(2));
        await tester.pump(Duration.zero);
        expect(rig.control.notice, isNull);

        rig.backend.emit(notice(1));
        await tester.pump(Duration.zero);
        expect(rig.control.notice, 'Anna’s Mac is locked.');
        await tester.pump(const Duration(seconds: 4));
      });
    });
  });

  group('the sentences about what was not done', () {
    test('are one for each reason, and say what is wrong', () {
      expect(
        describeDropped(WindowNoticeReason.controlOff, 'Anna’s Mac'),
        'Anna’s Mac has not let this device control the window.',
      );
      expect(
        describeDropped(WindowNoticeReason.paused, 'Anna’s Mac'),
        'The window is paused on Anna’s Mac.',
      );
      expect(
        describeDropped(WindowNoticeReason.locked, 'Anna’s Mac'),
        'Anna’s Mac is locked.',
      );
      expect(
        describeDropped(WindowNoticeReason.notOnScreen, 'Anna’s Mac'),
        'The window is not on the screen of Anna’s Mac.',
      );
      expect(
        describeDropped(WindowNoticeReason.tooFast, 'Anna’s Mac'),
        'That was faster than Anna’s Mac takes.',
      );
      expect(
        describeDropped(WindowNoticeReason.outOfDate, 'Anna’s Mac'),
        'The picture was out of date, so that was not done. Try again.',
      );
      expect(
        describeDropped(WindowNoticeReason.resized, 'Anna’s Mac'),
        'The window changed its size, so that was not done.',
      );
      expect(
        describeDropped(WindowNoticeReason.outsideWindow, 'Anna’s Mac'),
        'That was outside the window.',
      );
      expect(
        describeDropped(WindowNoticeReason.covered, 'Anna’s Mac'),
        'Another window is in front of that place on Anna’s Mac.',
      );
      expect(
        describeDropped(WindowNoticeReason.noKeyboard, 'Anna’s Mac'),
        'The window does not have the keyboard on Anna’s Mac.',
      );
      expect(
        describeDropped(WindowNoticeReason.appWindowInFront, 'Anna’s Mac'),
        'Another window of the same app is in front on Anna’s Mac, so a key '
        'might go to it.',
      );
      expect(
        describeDropped(WindowNoticeReason.tooManyKeys, 'Anna’s Mac'),
        'Too many keys are held down at once.',
      );
      expect(
        describeDropped(WindowNoticeReason.notPressed, 'Anna’s Mac'),
        'A key or a button was let go that was not pressed.',
      );
      expect(
        describeDropped(WindowNoticeReason.other, 'Anna’s Mac'),
        'Anna’s Mac did not do that.',
      );
    });

    test('are about every reason there is', () {
      for (final reason in WindowNoticeReason.values) {
        final sentence = describeDropped(reason, 'Anna’s Mac');
        expect(sentence, endsWith('.'), reason: '$reason');
        expect(sentence.length, greaterThan(10), reason: '$reason');
      }
    });

    test('show the name of the other device as it is shown elsewhere', () {
      const name = 'Mac\u202Efdp';
      expect(visibleText(name), isNot(name));
      expect(
        describeDropped(WindowNoticeReason.locked, name),
        '${visibleText(name)} is locked.',
      );
    });
  });
}
