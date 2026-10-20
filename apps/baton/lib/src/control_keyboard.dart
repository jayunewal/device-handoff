// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import 'control_keys.dart';

/// The field that brings up the keyboard of the screen for a window that is controlled, and tells
/// what the person types on it.
///
/// It cannot be seen, and it keeps no text. It holds only [TypingField.anchor], so that the
/// keyboard has characters to delete when the person presses Backspace, and is made to hold it
/// again after each thing the person did. Focus on [focusNode] brings the keyboard up, and taking
/// the focus away puts it down.
class SoftKeyboardField extends StatefulWidget {
  const SoftKeyboardField({
    required this.focusNode,
    required this.onTyped,
    super.key,
  });

  final FocusNode focusNode;

  /// The person deleted [deleted] characters and then typed [text], which may be none of either.
  /// A line end in the text is the Return key. Text that is being composed, as it is for Chinese
  /// or Japanese, is told when it is done.
  final void Function(int deleted, String text) onTyped;

  @override
  State<SoftKeyboardField> createState() => _SoftKeyboardFieldState();
}

class _SoftKeyboardFieldState extends State<SoftKeyboardField> {
  static final TextEditingValue _fresh = TextEditingValue(
    text: TypingField.anchor,
    selection: TextSelection.collapsed(offset: TypingField.anchor.length),
  );

  final TextEditingController _controller = TextEditingController.fromValue(
    _fresh,
  );

  @override
  void initState() {
    super.initState();
    _controller.addListener(_changed);
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  void _changed() {
    final value = _controller.value;
    if (value == _fresh) return;
    final done = TypingField.read(value);
    if (done == null) return;
    // The field is made whole before anything else is done, and the change that this makes is
    // not a change of what the person did.
    _controller.value = _fresh;
    if (done.deleted > 0 || done.typed.isNotEmpty) {
      widget.onTyped(done.deleted, done.typed);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ExcludeSemantics(
      child: SizedBox(
        width: 1,
        height: 1,
        child: Opacity(
          opacity: 0,
          child: EditableText(
            controller: _controller,
            focusNode: widget.focusNode,
            style: const TextStyle(fontSize: 1),
            cursorColor: Colors.transparent,
            backgroundCursorColor: Colors.transparent,
            // The Return key is a key of the window, and not the end of the typing.
            keyboardType: TextInputType.multiline,
            textInputAction: TextInputAction.newline,
            maxLines: null,
            // What is typed is for another device, which makes what it needs of it: the
            // keyboard has nothing to correct, to suggest or to make into typographic marks.
            autocorrect: false,
            enableSuggestions: false,
            smartDashesType: SmartDashesType.disabled,
            smartQuotesType: SmartQuotesType.disabled,
            textCapitalization: TextCapitalization.none,
            enableInteractiveSelection: false,
            showCursor: false,
          ),
        ),
      ),
    );
  }
}

/// The keys that a keyboard of a phone does not have, for a window that is controlled: the
/// modifier keys, to choose before the next thing the person does, the keys Escape and Tab, the
/// arrows, and the button that brings up the keyboard of the screen. They are in one line where
/// they fit and in more where they do not, so that none is out of sight.
class ControlKeys extends StatelessWidget {
  const ControlKeys({
    required this.chosen,
    required this.onKey,
    required this.keyboardShown,
    required this.onKeyboard,
    super.key,
  });

  /// The modifier keys that are chosen.
  final ChosenModifiers chosen;

  /// A key of the bar was pressed, with the usage of the keyboard page that it has.
  final void Function(int code) onKey;

  /// Whether the keyboard of the screen is up.
  final bool keyboardShown;

  /// The person asks for the keyboard of the screen, or to put it down.
  final VoidCallback onKeyboard;

  /// The names of the modifier keys on the bar. The one that is called Control on the screen's
  /// switch is Ctrl here, so that the two are not taken for each other.
  static const _modifiers = {
    Modifier.shift: 'Shift',
    Modifier.control: 'Ctrl',
    Modifier.option: 'Option',
    Modifier.command: 'Cmd',
  };

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Theme.of(context).colorScheme.surfaceContainerHigh,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: ListenableBuilder(
          listenable: chosen,
          builder: (context, _) => Wrap(
            spacing: 8,
            runSpacing: 4,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              IconButton.filledTonal(
                isSelected: keyboardShown,
                icon: const Icon(Icons.keyboard_outlined),
                selectedIcon: const Icon(Icons.keyboard_hide_outlined),
                tooltip: keyboardShown
                    ? 'Put the keyboard down'
                    : 'Bring up the keyboard',
                onPressed: onKeyboard,
              ),
              for (final MapEntry(key: modifier, value: name)
                  in _modifiers.entries)
                FilterChip(
                  label: Text(name),
                  selected: chosen.has(modifier),
                  onSelected: (_) => chosen.toggle(modifier),
                ),
              ActionChip(
                label: const Text('Esc'),
                onPressed: () => onKey(HidKey.escape),
              ),
              ActionChip(
                label: const Text('Tab'),
                onPressed: () => onKey(HidKey.tab),
              ),
              _arrow(Icons.arrow_back, 'Left arrow', HidKey.left),
              _arrow(Icons.arrow_upward, 'Up arrow', HidKey.up),
              _arrow(Icons.arrow_downward, 'Down arrow', HidKey.down),
              _arrow(Icons.arrow_forward, 'Right arrow', HidKey.right),
            ],
          ),
        ),
      ),
    );
  }

  Widget _arrow(IconData icon, String name, int code) =>
      IconButton(icon: Icon(icon), tooltip: name, onPressed: () => onKey(code));
}
