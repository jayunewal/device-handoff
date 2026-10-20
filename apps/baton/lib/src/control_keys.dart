// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart' show StringCharacters;

import 'backend.dart';

// What the keys of a keyboard, a real one or the one on the screen, become as events for the
// window of another device.
//
// A key is sent as the USB HID usage of the keyboard page that it has on the keyboard, with the
// modifiers that are down after it, and with the text it produced if there is some: the other
// device translates the usage to its own codes. Text that came with no key, from the on-screen
// keyboard or from dictation, is sent as text. What may be sent is decided in Rust, once; this
// only has to say what the person did.

/// The usages of the keyboard page that the screen sends on its own, for the keys it shows.
abstract final class HidKey {
  static const int enter = 0x28;
  static const int escape = 0x29;
  static const int backspace = 0x2A;
  static const int tab = 0x2B;
  static const int right = 0x4F;
  static const int left = 0x50;
  static const int down = 0x51;
  static const int up = 0x52;
}

/// No modifier key is down.
const WindowModifiers noModifiers = WindowModifiers(
  shift: false,
  control: false,
  option: false,
  command: false,
  capsLock: false,
);

/// The modifier keys that a person can choose on the screen.
enum Modifier { shift, control, option, command }

/// The modifier keys that the person chose on the screen, for the next thing they do: a key, a
/// click, a stroke of text. The screen has no Command key of its own, and this is how it gets
/// one. They are used up by the thing that was done, so that the next key is not a shortcut by
/// accident.
class ChosenModifiers extends ChangeNotifier {
  final Set<Modifier> _chosen = {};

  /// Whether [modifier] is chosen.
  bool has(Modifier modifier) => _chosen.contains(modifier);

  /// Chooses [modifier], or takes it back.
  void toggle(Modifier modifier) {
    if (!_chosen.remove(modifier)) _chosen.add(modifier);
    notifyListeners();
  }

  /// What is down now: the modifiers chosen on the screen and those of the keyboard [hardware]
  /// has.
  WindowModifiers now([WindowModifiers hardware = noModifiers]) =>
      WindowModifiers(
        shift: hardware.shift || has(Modifier.shift),
        control: hardware.control || has(Modifier.control),
        option: hardware.option || has(Modifier.option),
        command: hardware.command || has(Modifier.command),
        capsLock: hardware.capsLock,
      );

  /// The thing that was done used up what was chosen.
  void spend() {
    if (_chosen.isEmpty) return;
    _chosen.clear();
    notifyListeners();
  }
}

/// The modifier keys that [keyboard] says are down.
WindowModifiers modifiersOf(HardwareKeyboard keyboard) => WindowModifiers(
  shift: keyboard.isShiftPressed,
  control: keyboard.isControlPressed,
  option: keyboard.isAltPressed,
  command: keyboard.isMetaPressed,
  capsLock: keyboard.lockModesEnabled.contains(KeyboardLockMode.capsLock),
);

/// The usage of the keyboard page that [key] has, to be sent as the code of an event, or `null`
/// for a key that is not sent: one that has no usage there, like the Globe key or a media key,
/// whose bits mean another key on the other device if they are taken for a usage of this page,
/// and the modifier keys, which are no events of their own because the modifiers that are down
/// come with every event.
int? keyboardCode(PhysicalKeyboardKey key) {
  final usage = key.usbHidUsage;
  if (usage >> 16 != _keyboardPage) return null;
  final code = usage & 0xFFFF;
  // 0x01 to 0x03 are the states of a keyboard that has too many keys down, not keys, and the
  // usages from 0xE0 to 0xE7 are the modifier keys, the last ones of the page.
  return code >= 0x04 && code < _firstModifier ? code : null;
}

/// The page of the usages that are keys of a keyboard.
const int _keyboardPage = 0x07;

/// The usage of the first modifier key, the left Control key.
const int _firstModifier = 0xE0;

/// What a key event of a keyboard is for the window, or `null` if the key is not sent (see
/// [keyboardCode]). [modifiers] are those that are down after the event.
WindowInput_Key? windowKey(KeyEvent event, WindowModifiers modifiers) {
  final code = keyboardCode(event.physicalKey);
  if (code == null) return null;
  final down = event is! KeyUpEvent;
  return WindowInput_Key(
    code: code,
    down: down,
    modifiers: modifiers,
    repeat: event is KeyRepeatEvent,
    // A key up never has a character.
    text: event.character,
  );
}

/// A key that goes down and comes up, with [modifiers] down.
List<WindowInput> keyTap(int code, WindowModifiers modifiers) => [
  for (final down in [true, false])
    WindowInput.key(
      code: code,
      down: down,
      modifiers: modifiers,
      repeat: false,
    ),
];

/// What typing [text] on the keyboard of the screen is for the window, with [modifiers] down.
///
/// A line end is the Return key and a tab is the Tab key, since a field of the other device does
/// not take them as text. Text is text, typed as it is, unless a modifier other than Shift is
/// down: a letter with Command is a shortcut, and a shortcut is made of the key and not of the
/// letter, so a character that a key has is sent as the key. Rust cuts long text into pieces.
List<WindowInput> typed(String text, WindowModifiers modifiers) {
  final events = <WindowInput>[];
  final run = StringBuffer();
  void endRun() {
    if (run.isEmpty) return;
    events.add(
      WindowInput.key(
        code: 0,
        down: true,
        modifiers: modifiers,
        repeat: false,
        text: run.toString(),
      ),
    );
    run.clear();
  }

  final shortcut = modifiers.control || modifiers.option || modifiers.command;
  for (final character in text.characters) {
    final stroke = shortcut ? _stroke(character) : null;
    if (character == '\n' || character == '\r' || character == '\r\n') {
      endRun();
      events.addAll(keyTap(HidKey.enter, modifiers));
    } else if (character == '\t') {
      endRun();
      events.addAll(keyTap(HidKey.tab, modifiers));
    } else if (stroke != null) {
      endRun();
      events.addAll(
        keyTap(stroke.code, stroke.shift ? _withShift(modifiers) : modifiers),
      );
    } else {
      run.write(character);
    }
  }
  endRun();
  return events;
}

WindowModifiers _withShift(WindowModifiers modifiers) => WindowModifiers(
  shift: true,
  control: modifiers.control,
  option: modifiers.option,
  command: modifiers.command,
  capsLock: modifiers.capsLock,
);

/// The keys of the letters and the digits, from usage 0x04, and the characters that the digits
/// make with Shift, on the keyboard that every device has in common.
const String _letters = 'abcdefghijklmnopqrstuvwxyz';
const String _digits = '1234567890';
const String _shiftedDigits = '!@#\$%^&*()';

/// The other characters of the keys of a US keyboard: the usage, the character without Shift and
/// the one with it.
const Map<int, (String, String)> _marks = {
  0x2C: (' ', ' '),
  0x2D: ('-', '_'),
  0x2E: ('=', '+'),
  0x2F: ('[', '{'),
  0x30: (']', '}'),
  0x31: ('\\', '|'),
  0x33: (';', ':'),
  0x34: ("'", '"'),
  0x35: ('`', '~'),
  0x36: (',', '<'),
  0x37: ('.', '>'),
  0x38: ('/', '?'),
};

/// The key of a US keyboard that makes [character], and whether Shift is down for it, or `null`
/// for a character that no key of that keyboard makes.
({int code, bool shift})? _stroke(String character) {
  // Only the characters of ASCII are keys of that keyboard. Some others lower-case to one of its
  // letters, like the Kelvin sign, and are not that key.
  if (character.length != 1 || character.codeUnitAt(0) >= 0x80) return null;
  final lower = character.toLowerCase();
  final letter = _letters.indexOf(lower);
  if (letter >= 0) return (code: 0x04 + letter, shift: lower != character);
  final digit = _digits.indexOf(character);
  if (digit >= 0) return (code: 0x1E + digit, shift: false);
  final shifted = _shiftedDigits.indexOf(character);
  if (shifted >= 0) return (code: 0x1E + shifted, shift: true);
  for (final MapEntry(key: code, value: (plain, withShift)) in _marks.entries) {
    if (character == plain) return (code: code, shift: false);
    if (character == withShift) return (code: code, shift: true);
  }
  return null;
}

/// The field that the on-screen keyboard types into, which has to be there for the keyboard to
/// show, and what it holds.
///
/// The field holds only [anchor], so that a Backspace, which deletes nothing from an empty field
/// and is told to nobody, deletes one of its characters and is seen. Whatever is typed goes after
/// it. [read] says what the field's value means, and the field is made to hold [anchor] again.
abstract final class TypingField {
  /// A character that takes no room and shows nothing: a zero-width space.
  static final String _unseen = String.fromCharCode(0x200B);

  /// What the field holds when nothing has been typed: many characters that are not seen, so that
  /// the Backspace of a key that is held, which is several before the field can be made whole
  /// again, is not lost.
  static final String anchor = _unseen * 32;

  /// What the field's [value] says was done: how many characters were deleted, and the text that
  /// was typed. `null` while the text is being composed, as it is for Chinese or Japanese or
  /// while the keyboard guesses a word: the person has not finished it, and what is typed is
  /// what stays.
  static ({int deleted, String typed})? read(TextEditingValue value) {
    final composing = value.composing;
    if (composing.isValid && !composing.isCollapsed) return null;
    final text = value.text;
    final typed = text.replaceAll(_unseen, '');
    final left = text.length - typed.length;
    // A value that lost every character of the anchor and holds text was replaced, and not
    // deleted from: nothing is deleted from the other device for it.
    final replaced = left == 0 && typed.isNotEmpty;
    final deleted = replaced ? 0 : anchor.length - left;
    return (deleted: deleted > 0 ? deleted : 0, typed: typed);
  }
}
