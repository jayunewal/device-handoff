// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';

/// Asks the person at this device whether [device] may ask what they are working on, to continue
/// it there: the page that is in front in Safari or Chrome, and the text on the clipboard. Nobody
/// here is asked when it does, so this is the one place that says yes, and it is off until then.
///
/// It returns what the person chose when they save, also when it is as it was, and `null` when
/// they cancel. Nothing is kept here: the screen that opened it keeps what changed, and asks the
/// system for the permission to read the browsers.
class ContinueAccessDialog extends StatefulWidget {
  const ContinueAccessDialog({
    required this.device,
    required this.thisDevice,
    required this.allowed,
    super.key,
  });

  /// The paired device that asks.
  final Device device;

  /// The system that this device runs, to name it in words: "this Mac".
  final Platform thisDevice;

  /// Whether the device may ask now.
  final bool allowed;

  @override
  State<ContinueAccessDialog> createState() => _ContinueAccessDialogState();
}

class _ContinueAccessDialogState extends State<ContinueAccessDialog> {
  late bool _allowed = widget.allowed;

  @override
  Widget build(BuildContext context) {
    final name = visibleText(widget.device.name);
    final computer = 'this ${platformName(widget.thisDevice)}';
    final theme = Theme.of(context);
    return AlertDialog(
      scrollable: true,
      title: Text('When $name asks what you are working on'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SwitchListTile(
            value: _allowed,
            onChanged: (on) => setState(() => _allowed = on),
            title: Text('Let $name continue from $computer'),
            subtitle: Text(
              'When $name asks, Baton gives it the address and title of the '
              'page that is in front in Safari or Chrome, and the text on the '
              'clipboard, as they are at that moment. Nothing is kept, and no '
              'one here is asked.',
            ),
          ),
          const SizedBox(height: 12),
          Text(
            '$computer gives nothing while it is locked or its screen is '
            'off, and never gives text that a password manager marked as '
            'secret. Baton leaves out the private windows of Chrome. It cannot '
            'promise that for Safari, so turn this off if you keep private '
            'pages open that $name should not see.\n\n'
            'The first time, macOS asks whether Baton may control Safari and '
            'Chrome. Baton uses that only to read the address and title of the '
            'page in front. If you say no, the clipboard still works.',
            style: theme.textTheme.bodySmall,
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_allowed),
          child: const Text('Save'),
        ),
      ],
    );
  }
}
