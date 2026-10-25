// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';

/// Asks the person at this device what [device] may do without anyone here saying yes: ask this
/// device to start sharing its screen with it, to look at it or to look at it and control it.
///
/// It returns what the person chose, and `null` when they leave it as it was. Nothing is kept
/// here: the screen that opened it keeps the answer.
class StartAccessDialog extends StatefulWidget {
  const StartAccessDialog({
    required this.device,
    required this.thisDevice,
    required this.current,
    required this.canControl,
    super.key,
  });

  /// The paired device that asks.
  final Device device;

  /// The system that this device runs, to name it in words: "this Mac".
  final Platform thisDevice;

  /// What the device may do now.
  final StartAccess current;

  /// Whether this device can let another control its screen. The choice is left out when it
  /// cannot, unless it is the one that is made now.
  final bool canControl;

  @override
  State<StartAccessDialog> createState() => _StartAccessDialogState();
}

class _StartAccessDialogState extends State<StartAccessDialog> {
  late StartAccess _chosen = widget.current;

  @override
  Widget build(BuildContext context) {
    final name = visibleText(widget.device.name);
    final computer = 'this ${platformName(widget.thisDevice)}';
    final theme = Theme.of(context);
    return AlertDialog(
      scrollable: true,
      title: Text('When $name asks for the screen of $computer'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          RadioGroup<StartAccess>(
            groupValue: _chosen,
            onChanged: (access) {
              if (access != null) setState(() => _chosen = access);
            },
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                const RadioListTile<StartAccess>(
                  value: StartAccess.off,
                  title: Text('Do not show it'),
                  subtitle: Text(
                    'It is shown only when you share it from here.',
                  ),
                ),
                RadioListTile<StartAccess>(
                  value: StartAccess.look,
                  title: const Text('Show it to look at'),
                  subtitle: Text(
                    '$name can look at everything on the screen, and no one '
                    'here is asked.',
                  ),
                ),
                if (widget.canControl || widget.current == StartAccess.control)
                  RadioListTile<StartAccess>(
                    value: StartAccess.control,
                    title: const Text('Show it to look at and control'),
                    subtitle: Text(
                      '$name can also click and type, and no one here is '
                      'asked.',
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(height: 12),
          Text(
            'The screen shows only while $computer is awake and unlocked. '
            'Turning this off does not end a screen that is shared: end it '
            'with Stop. The first time a screen is asked for, $computer asks '
            'whether Baton may record it. Say yes while you are here.',
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
          onPressed: () =>
              Navigator.of(context)
                  .pop(_chosen == widget.current ? null : _chosen),
          child: const Text('Save'),
        ),
      ],
    );
  }
}
