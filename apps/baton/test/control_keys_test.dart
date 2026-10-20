// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/control_keys.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _shift = WindowModifiers(
  shift: true,
  control: false,
  option: false,
  command: false,
  capsLock: false,
);

const _control = WindowModifiers(
  shift: false,
  control: true,
  option: false,
  command: false,
  capsLock: false,
);

const _option = WindowModifiers(
  shift: false,
  control: false,
  option: true,
  command: false,
  capsLock: false,
);

const _command = WindowModifiers(
  shift: false,
  control: false,
  option: false,
  command: true,
  capsLock: false,
);

const _capsLock = WindowModifiers(
  shift: false,
  control: false,
  option: false,
  command: false,
  capsLock: true,
);

/// The code of a key, as the other device reads it: the usage on the keyboard page.
int _usage(PhysicalKeyboardKey key) => key.usbHidUsage & 0xFFFF;

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

/// A key that went down and came up.
List<WindowInput> _tap(int code, [WindowModifiers modifiers = noModifiers]) => [
  _key(code, modifiers: modifiers),
  _key(code, down: false, modifiers: modifiers),
];

KeyDownEvent _down(
  PhysicalKeyboardKey physical,
  LogicalKeyboardKey logical, [
  String? character,
]) => KeyDownEvent(
  physicalKey: physical,
  logicalKey: logical,
  character: character,
  timeStamp: Duration.zero,
);

void main() {
  group('the usages of the keys that the screen sends', () {
    test('are those that the platform gives the keys', () {
      expect(HidKey.enter, _usage(PhysicalKeyboardKey.enter));
      expect(HidKey.escape, _usage(PhysicalKeyboardKey.escape));
      expect(HidKey.backspace, _usage(PhysicalKeyboardKey.backspace));
      expect(HidKey.tab, _usage(PhysicalKeyboardKey.tab));
      expect(HidKey.right, _usage(PhysicalKeyboardKey.arrowRight));
      expect(HidKey.left, _usage(PhysicalKeyboardKey.arrowLeft));
      expect(HidKey.down, _usage(PhysicalKeyboardKey.arrowDown));
      expect(HidKey.up, _usage(PhysicalKeyboardKey.arrowUp));
    });
  });

  group('the code of a key', () {
    test('is its usage on the keyboard page', () {
      expect(keyboardCode(PhysicalKeyboardKey.keyA), 0x04);
      expect(keyboardCode(PhysicalKeyboardKey.keyZ), 0x1D);
      expect(keyboardCode(PhysicalKeyboardKey.enter), 0x28);
      expect(keyboardCode(PhysicalKeyboardKey.space), 0x2C);
      expect(keyboardCode(PhysicalKeyboardKey.f1), 0x3A);
      expect(keyboardCode(PhysicalKeyboardKey.capsLock), 0x39);
      expect(keyboardCode(PhysicalKeyboardKey.arrowUp), 0x52);
      expect(keyboardCode(PhysicalKeyboardKey.contextMenu), 0x65);
    });

    test('is none for a key that has no usage on that page', () {
      // The page is in the bits above the usage: a media key is on the page of consumer
      // controls, and the Fn key of a Mac on none, and neither is a key of the keyboard.
      expect(PhysicalKeyboardKey.mediaPlay.usbHidUsage >> 16, isNot(0x07));
      expect(keyboardCode(PhysicalKeyboardKey.mediaPlay), isNull);
      expect(keyboardCode(PhysicalKeyboardKey.fn), isNull);
      expect(keyboardCode(const PhysicalKeyboardKey(0x00000004)), isNull);
      expect(keyboardCode(const PhysicalKeyboardKey(0x00080004)), isNull);
      expect(keyboardCode(const PhysicalKeyboardKey(0x01070004)), isNull);
    });

    test('is none for what a keyboard says when it has too many keys down', () {
      for (final usage in [0x00, 0x01, 0x02, 0x03]) {
        expect(keyboardCode(PhysicalKeyboardKey(0x00070000 + usage)), isNull);
      }
      expect(keyboardCode(const PhysicalKeyboardKey(0x00070004)), 0x04);
    });

    test('is none for a modifier key, which the other events carry', () {
      for (final key in [
        PhysicalKeyboardKey.controlLeft,
        PhysicalKeyboardKey.shiftLeft,
        PhysicalKeyboardKey.altLeft,
        PhysicalKeyboardKey.metaLeft,
        PhysicalKeyboardKey.controlRight,
        PhysicalKeyboardKey.shiftRight,
        PhysicalKeyboardKey.altRight,
        PhysicalKeyboardKey.metaRight,
      ]) {
        expect(keyboardCode(key), isNull, reason: key.debugName);
      }
      // The key before them is the last one that is sent, and the page ends after them.
      expect(keyboardCode(const PhysicalKeyboardKey(0x000700DF)), 0xDF);
      expect(keyboardCode(const PhysicalKeyboardKey(0x000700E0)), isNull);
      expect(keyboardCode(const PhysicalKeyboardKey(0x000700E7)), isNull);
      expect(keyboardCode(const PhysicalKeyboardKey(0x000700E8)), isNull);
    });
  });

  group('a key event of a keyboard', () {
    test('that goes down is a key that goes down, with its text', () {
      expect(
        windowKey(
          _down(PhysicalKeyboardKey.keyA, LogicalKeyboardKey.keyA, 'a'),
          _command,
        ),
        _key(0x04, modifiers: _command, text: 'a'),
      );
    });

    test('that goes down has no text if it made none', () {
      expect(
        windowKey(
          _down(PhysicalKeyboardKey.arrowUp, LogicalKeyboardKey.arrowUp),
          noModifiers,
        ),
        _key(0x52),
      );
    });

    test('that repeats is a key that goes down again, and says so', () {
      expect(
        windowKey(
          const KeyRepeatEvent(
            physicalKey: PhysicalKeyboardKey.keyA,
            logicalKey: LogicalKeyboardKey.keyA,
            character: 'a',
            timeStamp: Duration.zero,
          ),
          noModifiers,
        ),
        _key(0x04, repeat: true, text: 'a'),
      );
    });

    test('that comes up is a key that comes up, without text', () {
      expect(
        windowKey(
          const KeyUpEvent(
            physicalKey: PhysicalKeyboardKey.keyA,
            logicalKey: LogicalKeyboardKey.keyA,
            timeStamp: Duration.zero,
          ),
          _shift,
        ),
        _key(0x04, down: false, modifiers: _shift),
      );
    });

    test('of a key that is not sent is nothing', () {
      expect(
        windowKey(
          _down(PhysicalKeyboardKey.fn, LogicalKeyboardKey.fn),
          noModifiers,
        ),
        isNull,
      );
      expect(
        windowKey(
          _down(PhysicalKeyboardKey.shiftLeft, LogicalKeyboardKey.shiftLeft),
          _shift,
        ),
        isNull,
      );
      expect(
        windowKey(
          const KeyUpEvent(
            physicalKey: PhysicalKeyboardKey.metaLeft,
            logicalKey: LogicalKeyboardKey.metaLeft,
            timeStamp: Duration.zero,
          ),
          noModifiers,
        ),
        isNull,
      );
    });
  });

  group('the modifiers of a hardware keyboard', () {
    testWidgets('are those of the keys that are down', (tester) async {
      final keyboard = HardwareKeyboard.instance;
      expect(modifiersOf(keyboard), noModifiers);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      expect(modifiersOf(keyboard), _shift);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlRight);
      expect(modifiersOf(keyboard), _control);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlRight);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      expect(modifiersOf(keyboard), _option);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.metaRight);
      expect(modifiersOf(keyboard), _command);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.metaRight);

      expect(modifiersOf(keyboard), noModifiers);
    });

    testWidgets('say whether caps lock is on', (tester) async {
      final keyboard = HardwareKeyboard.instance;
      await tester.sendKeyDownEvent(LogicalKeyboardKey.capsLock);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.capsLock);
      expect(modifiersOf(keyboard), _capsLock);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.capsLock);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.capsLock);
      expect(modifiersOf(keyboard), noModifiers);
    });
  });

  group('the modifiers that the person chose on the screen', () {
    test('are none at first', () {
      final chosen = ChosenModifiers();
      for (final modifier in Modifier.values) {
        expect(chosen.has(modifier), isFalse);
      }
      expect(chosen.now(), noModifiers);
    });

    test('are chosen one by one, and taken back by choosing again', () {
      final chosen = ChosenModifiers();
      var told = 0;
      chosen.addListener(() => told++);

      chosen.toggle(Modifier.command);
      expect(told, 1);
      expect(chosen.has(Modifier.command), isTrue);
      expect(chosen.has(Modifier.shift), isFalse);
      expect(chosen.now(), _command);

      chosen.toggle(Modifier.shift);
      expect(told, 2);
      expect(
        chosen.now(),
        const WindowModifiers(
          shift: true,
          control: false,
          option: false,
          command: true,
          capsLock: false,
        ),
      );

      chosen.toggle(Modifier.command);
      expect(told, 3);
      expect(chosen.has(Modifier.command), isFalse);
      expect(chosen.now(), _shift);

      // The last one taken back is told as well: the keys of the screen are drawn from it.
      chosen.toggle(Modifier.shift);
      expect(told, 4);
      expect(chosen.now(), noModifiers);
    });

    test('each make their own modifier', () {
      final chosen = ChosenModifiers();
      expect((chosen..toggle(Modifier.shift)).now(), _shift);
      expect(
        (chosen
              ..toggle(Modifier.shift)
              ..toggle(Modifier.control))
            .now(),
        _control,
      );
      expect(
        (chosen
              ..toggle(Modifier.control)
              ..toggle(Modifier.option))
            .now(),
        _option,
      );
      expect(
        (chosen
              ..toggle(Modifier.option)
              ..toggle(Modifier.command))
            .now(),
        _command,
      );
    });

    test('add to the modifiers of a keyboard, and caps lock is its own', () {
      final chosen = ChosenModifiers()..toggle(Modifier.option);
      expect(
        chosen.now(_capsLock),
        const WindowModifiers(
          shift: false,
          control: false,
          option: true,
          command: false,
          capsLock: true,
        ),
      );
      expect(chosen.now(_shift).shift, isTrue);
      expect(chosen.now(_shift).option, isTrue);
      expect(chosen.now(_control).control, isTrue);
      expect(chosen.now(_command).command, isTrue);
      expect(chosen.now(_option), _option);
    });

    test('are not needed for the modifiers of a keyboard, each its own', () {
      final nothing = ChosenModifiers();
      for (final hardware in [_shift, _control, _option, _command]) {
        expect(nothing.now(hardware), hardware);
      }
      expect(nothing.now(_capsLock), _capsLock);
    });

    test('are used up by what was done, and nothing is told if none were', () {
      final chosen = ChosenModifiers();
      var told = 0;
      chosen.addListener(() => told++);

      chosen.spend();
      expect(told, 0);

      chosen
        ..toggle(Modifier.command)
        ..toggle(Modifier.shift);
      expect(told, 2);
      chosen.spend();
      expect(told, 3);
      expect(chosen.now(), noModifiers);
      chosen.spend();
      expect(told, 3);
    });
  });

  group('a key that goes down and comes up', () {
    test('is two events with the same modifiers', () {
      expect(keyTap(HidKey.enter, noModifiers), _tap(0x28));
      expect(keyTap(0x06, _command), _tap(0x06, _command));
    });
  });

  group('text typed on the keyboard of the screen', () {
    test('is nothing when there is none', () {
      expect(typed('', noModifiers), isEmpty);
      expect(typed('', _command), isEmpty);
    });

    test('is text with no key, in one event', () {
      expect(typed('Hello, world', noModifiers), [
        _key(0, text: 'Hello, world'),
      ]);
    });

    test('keeps the characters that are made of several', () {
      expect(typed('é 👍🏽 🇳🇱', noModifiers), [
        _key(0, text: 'é 👍🏽 🇳🇱'),
      ]);
    });

    test('is text when only Shift or caps lock is down', () {
      expect(typed('Ab', _shift), [_key(0, modifiers: _shift, text: 'Ab')]);
      expect(typed('ab', _capsLock), [
        _key(0, modifiers: _capsLock, text: 'ab'),
      ]);
    });

    test('has the Return key for a line end, however it is written', () {
      expect(typed('a\nb', noModifiers), [
        _key(0, text: 'a'),
        ..._tap(HidKey.enter),
        _key(0, text: 'b'),
      ]);
      expect(typed('a\rb', noModifiers), [
        _key(0, text: 'a'),
        ..._tap(HidKey.enter),
        _key(0, text: 'b'),
      ]);
      expect(typed('a\r\nb', noModifiers), [
        _key(0, text: 'a'),
        ..._tap(HidKey.enter),
        _key(0, text: 'b'),
      ]);
      expect(typed('\n\n', noModifiers), [
        ..._tap(HidKey.enter),
        ..._tap(HidKey.enter),
      ]);
    });

    test('has the Tab key for a tab', () {
      expect(typed('a\tb', noModifiers), [
        _key(0, text: 'a'),
        ..._tap(HidKey.tab),
        _key(0, text: 'b'),
      ]);
      expect(typed('\t', _shift), _tap(HidKey.tab, _shift));
    });

    test(
      'is the keys of a shortcut when Command, Control or Option is down',
      () {
        expect(typed('c', _command), _tap(0x06, _command));
        expect(typed('v', _control), _tap(0x19, _control));
        expect(typed('z', _option), _tap(0x1D, _option));
      },
    );

    test('is the keys of a shortcut for each of the characters', () {
      expect(typed('ab', _command), [
        ..._tap(0x04, _command),
        ..._tap(0x05, _command),
      ]);
    });

    test('has Shift down for a capital letter of a shortcut', () {
      const shifted = WindowModifiers(
        shift: true,
        control: false,
        option: false,
        command: true,
        capsLock: false,
      );
      expect(typed('Z', _command), _tap(0x1D, shifted));
      expect(typed('z', shifted), _tap(0x1D, shifted));
    });

    test('keeps the other modifiers when Shift is added for a shortcut', () {
      const everything = WindowModifiers(
        shift: false,
        control: true,
        option: true,
        command: true,
        capsLock: true,
      );
      const withShift = WindowModifiers(
        shift: true,
        control: true,
        option: true,
        command: true,
        capsLock: true,
      );
      expect(typed('A', everything), _tap(0x04, withShift));
      expect(typed('a', everything), _tap(0x04, everything));
    });

    test('is a letter of a shortcut by the letter and not the case', () {
      expect(typed('a', _command).map((e) => (e as WindowInput_Key).code), [
        0x04,
        0x04,
      ]);
      expect(typed('A', _command).map((e) => (e as WindowInput_Key).code), [
        0x04,
        0x04,
      ]);
    });

    test('sends the digits and the marks of a shortcut as their keys', () {
      const shifted = WindowModifiers(
        shift: true,
        control: false,
        option: false,
        command: true,
        capsLock: false,
      );
      final plain = <String, PhysicalKeyboardKey>{
        '1': PhysicalKeyboardKey.digit1,
        '2': PhysicalKeyboardKey.digit2,
        '3': PhysicalKeyboardKey.digit3,
        '4': PhysicalKeyboardKey.digit4,
        '5': PhysicalKeyboardKey.digit5,
        '6': PhysicalKeyboardKey.digit6,
        '7': PhysicalKeyboardKey.digit7,
        '8': PhysicalKeyboardKey.digit8,
        '9': PhysicalKeyboardKey.digit9,
        '0': PhysicalKeyboardKey.digit0,
        ' ': PhysicalKeyboardKey.space,
        '-': PhysicalKeyboardKey.minus,
        '=': PhysicalKeyboardKey.equal,
        '[': PhysicalKeyboardKey.bracketLeft,
        ']': PhysicalKeyboardKey.bracketRight,
        r'\': PhysicalKeyboardKey.backslash,
        ';': PhysicalKeyboardKey.semicolon,
        "'": PhysicalKeyboardKey.quote,
        '`': PhysicalKeyboardKey.backquote,
        ',': PhysicalKeyboardKey.comma,
        '.': PhysicalKeyboardKey.period,
        '/': PhysicalKeyboardKey.slash,
      };
      final withShift = <String, PhysicalKeyboardKey>{
        '!': PhysicalKeyboardKey.digit1,
        '@': PhysicalKeyboardKey.digit2,
        '#': PhysicalKeyboardKey.digit3,
        r'$': PhysicalKeyboardKey.digit4,
        '%': PhysicalKeyboardKey.digit5,
        '^': PhysicalKeyboardKey.digit6,
        '&': PhysicalKeyboardKey.digit7,
        '*': PhysicalKeyboardKey.digit8,
        '(': PhysicalKeyboardKey.digit9,
        ')': PhysicalKeyboardKey.digit0,
        '_': PhysicalKeyboardKey.minus,
        '+': PhysicalKeyboardKey.equal,
        '{': PhysicalKeyboardKey.bracketLeft,
        '}': PhysicalKeyboardKey.bracketRight,
        '|': PhysicalKeyboardKey.backslash,
        ':': PhysicalKeyboardKey.semicolon,
        '"': PhysicalKeyboardKey.quote,
        '~': PhysicalKeyboardKey.backquote,
        '<': PhysicalKeyboardKey.comma,
        '>': PhysicalKeyboardKey.period,
        '?': PhysicalKeyboardKey.slash,
      };
      for (final MapEntry(key: character, value: key) in plain.entries) {
        expect(
          typed(character, _command),
          _tap(_usage(key), _command),
          reason: 'the character $character',
        );
      }
      for (final MapEntry(key: character, value: key) in withShift.entries) {
        expect(
          typed(character, _command),
          _tap(_usage(key), shifted),
          reason: 'the character $character',
        );
      }
    });

    test('sends the letters of a shortcut as their keys', () {
      final keys = [
        PhysicalKeyboardKey.keyA,
        PhysicalKeyboardKey.keyB,
        PhysicalKeyboardKey.keyC,
        PhysicalKeyboardKey.keyD,
        PhysicalKeyboardKey.keyE,
        PhysicalKeyboardKey.keyF,
        PhysicalKeyboardKey.keyG,
        PhysicalKeyboardKey.keyH,
        PhysicalKeyboardKey.keyI,
        PhysicalKeyboardKey.keyJ,
        PhysicalKeyboardKey.keyK,
        PhysicalKeyboardKey.keyL,
        PhysicalKeyboardKey.keyM,
        PhysicalKeyboardKey.keyN,
        PhysicalKeyboardKey.keyO,
        PhysicalKeyboardKey.keyP,
        PhysicalKeyboardKey.keyQ,
        PhysicalKeyboardKey.keyR,
        PhysicalKeyboardKey.keyS,
        PhysicalKeyboardKey.keyT,
        PhysicalKeyboardKey.keyU,
        PhysicalKeyboardKey.keyV,
        PhysicalKeyboardKey.keyW,
        PhysicalKeyboardKey.keyX,
        PhysicalKeyboardKey.keyY,
        PhysicalKeyboardKey.keyZ,
      ];
      const letters = 'abcdefghijklmnopqrstuvwxyz';
      for (var i = 0; i < letters.length; i++) {
        expect(
          typed(letters[i], _control),
          _tap(_usage(keys[i]), _control),
          reason: letters[i],
        );
      }
    });

    test('stays text for a character that is no key, even with Command', () {
      expect(typed('é', _command), [_key(0, modifiers: _command, text: 'é')]);
      expect(typed('é和😀', _control), [
        _key(0, modifiers: _control, text: 'é和😀'),
      ]);
      // The Kelvin sign and the dotted capital I make a letter when they are lower-cased, but
      // are not the key of one.
      expect(typed('K', _command), [_key(0, modifiers: _command, text: 'K')]);
      expect(typed('İ', _command), [_key(0, modifiers: _command, text: 'İ')]);
    });

    test('is a run of text between the keys of a shortcut', () {
      expect(typed('ab é c', _command), [
        ..._tap(0x04, _command),
        ..._tap(0x05, _command),
        ..._tap(0x2C, _command),
        _key(0, modifiers: _command, text: 'é'),
        ..._tap(0x2C, _command),
        ..._tap(0x06, _command),
      ]);
    });

    test('is the Return key for a line end when a shortcut is typed', () {
      expect(typed('a\nb', _command), [
        ..._tap(0x04, _command),
        ..._tap(HidKey.enter, _command),
        ..._tap(0x05, _command),
      ]);
    });
  });

  group('the field that the keyboard of the screen types into', () {
    TextEditingValue value(
      String text, {
      TextRange composing = TextRange.empty,
    }) => TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
      composing: composing,
    );

    test('holds many characters that are not seen, and nothing else', () {
      expect(TypingField.anchor.length, 32);
      expect(TypingField.anchor.codeUnits.toSet(), {0x200B});
    });

    test('says nothing was done while it is as it was made', () {
      expect(TypingField.read(value(TypingField.anchor)), (
        deleted: 0,
        typed: '',
      ));
    });

    test('says what was typed after the characters that are not seen', () {
      expect(TypingField.read(value('${TypingField.anchor}abc')), (
        deleted: 0,
        typed: 'abc',
      ));
      expect(TypingField.read(value('${TypingField.anchor}é\n')), (
        deleted: 0,
        typed: 'é\n',
      ));
    });

    test('says how many characters were deleted', () {
      final one = TypingField.anchor.substring(1);
      expect(TypingField.read(value(one)), (deleted: 1, typed: ''));
      final five = TypingField.anchor.substring(0, 27);
      expect(TypingField.read(value(five)), (deleted: 5, typed: ''));
      expect(TypingField.read(value('${five}x')), (deleted: 5, typed: 'x'));
    });

    test('says what was typed wherever in the characters it was put', () {
      final anchor = TypingField.anchor;
      expect(
        TypingField.read(
          value('${anchor.substring(0, 10)}ab${anchor.substring(10)}'),
        ),
        (deleted: 0, typed: 'ab'),
      );
      expect(
        TypingField.read(
          value('${anchor.substring(0, 10)}ab${anchor.substring(11)}'),
        ),
        (deleted: 1, typed: 'ab'),
      );
    });

    test(
      'says that all of the characters were deleted when nothing is left',
      () {
        expect(TypingField.read(value('')), (deleted: 32, typed: ''));
      },
    );

    test('says that the text was replaced when all of the anchor is gone', () {
      // The keyboard took the whole text and put other text in its place, which deletes nothing
      // from the other device.
      expect(TypingField.read(value('abc')), (deleted: 0, typed: 'abc'));
    });

    test('has no more deleted than it was made of', () {
      expect(
        TypingField.read(value('${TypingField.anchor}${TypingField.anchor}')),
        (deleted: 0, typed: ''),
      );
    });

    test('leaves out the characters that are not seen from what was typed', () {
      expect(TypingField.read(value('${TypingField.anchor}a​b​')), (
        deleted: 0,
        typed: 'ab',
      ));
    });

    test('says nothing while the text is composed', () {
      expect(
        TypingField.read(
          value(
            '${TypingField.anchor}ni',
            composing: const TextRange(start: 32, end: 34),
          ),
        ),
        isNull,
      );
    });

    test(
      'says what it holds when the composing is over, empty or no range',
      () {
        expect(
          TypingField.read(
            value(
              '${TypingField.anchor}你',
              composing: const TextRange(start: -1, end: 3),
            ),
          ),
          (deleted: 0, typed: '你'),
        );
        expect(
          TypingField.read(
            value(
              '${TypingField.anchor}你',
              composing: const TextRange.collapsed(33),
            ),
          ),
          (deleted: 0, typed: '你'),
        );
        expect(TypingField.read(value('${TypingField.anchor}你')), (
          deleted: 0,
          typed: '你',
        ));
      },
    );
  });
}
