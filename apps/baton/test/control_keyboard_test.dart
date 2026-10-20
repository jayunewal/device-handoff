// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/control_keyboard.dart';
import 'package:baton/src/control_keys.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// What the field told, and what it held at the time.
class _Told {
  final List<(int, String)> typed = [];
  final List<String> held = [];
}

/// Shows the field with the focus on it, so that it has the keyboard.
Future<_Told> _show(WidgetTester tester) async {
  final told = _Told();
  final focus = FocusNode();
  addTearDown(focus.dispose);
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SoftKeyboardField(
          focusNode: focus,
          onTyped: (deleted, text) {
            told.typed.add((deleted, text));
            told.held.add(
              tester
                  .state<EditableTextState>(find.byType(EditableText))
                  .textEditingValue
                  .text,
            );
          },
        ),
      ),
    ),
  );
  focus.requestFocus();
  await tester.pump();
  return told;
}

/// The person types [text] after the first [kept] characters of what the field holds.
Future<void> _type(WidgetTester tester, String text, {int? kept}) async {
  final anchor = TypingField.anchor;
  final value = anchor.substring(0, kept ?? anchor.length) + text;
  tester.testTextInput.updateEditingValue(
    TextEditingValue(
      text: value,
      selection: TextSelection.collapsed(offset: value.length),
    ),
  );
  await tester.pump();
}

void main() {
  group('the field of the keyboard', () {
    testWidgets('tells what was typed, and holds the anchor again by then', (
      tester,
    ) async {
      final told = await _show(tester);

      await _type(tester, 'hi');

      expect(told.typed, [(0, 'hi')]);
      expect(told.held, [TypingField.anchor]);
    });

    testWidgets('tells what was deleted, and what was typed instead', (
      tester,
    ) async {
      final told = await _show(tester);

      await _type(tester, '', kept: TypingField.anchor.length - 3);
      await _type(tester, 'ok', kept: TypingField.anchor.length - 1);

      expect(told.typed, [(3, ''), (1, 'ok')]);
    });

    testWidgets('tells nothing for a move of the cursor', (tester) async {
      final told = await _show(tester);

      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: TypingField.anchor,
          selection: const TextSelection.collapsed(offset: 10),
        ),
      );
      await tester.pump();

      expect(told.typed, isEmpty);
    });

    testWidgets('tells nothing while text is being composed', (tester) async {
      final told = await _show(tester);
      final anchor = TypingField.anchor;

      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: '${anchor}ni',
          selection: TextSelection.collapsed(offset: anchor.length + 2),
          composing: TextRange(start: anchor.length, end: anchor.length + 2),
        ),
      );
      await tester.pump();

      expect(told.typed, isEmpty);
    });

    testWidgets('has the Return key of a text, and nothing to show or select', (
      tester,
    ) async {
      await _show(tester);

      final field = tester.widget<EditableText>(find.byType(EditableText));
      expect(field.textInputAction, TextInputAction.newline);
      expect(field.enableInteractiveSelection, isFalse);
      expect(field.showCursor, isFalse);
      final opacity = find.descendant(
        of: find.byType(SoftKeyboardField),
        matching: find.byType(Opacity),
      );
      expect(tester.widget<Opacity>(opacity.first).opacity, 0);
    });

    testWidgets('is not read out by a screen reader', (tester) async {
      await _show(tester);

      final excluding = find.descendant(
        of: find.byType(SoftKeyboardField),
        matching: find.byType(ExcludeSemantics),
      );
      expect(excluding, findsWidgets);
      expect(tester.widget<ExcludeSemantics>(excluding.first).excluding, true);
    });
  });
}
