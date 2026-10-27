// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';

/// What the person chose for a paired device: what it may do about the screen, and whether it may
/// hear the sound of it. [hear] is `null` when the person leaves it to the device to hear if it
/// may control the screen, and not otherwise.
typedef StartChoice = ({StartAccess access, bool? hear});

/// Asks the person at this device what [device] may do without anyone here saying yes: ask this
/// device to start sharing its screen with it, to look at it or to look at it and control it, and
/// whether it may hear the sound of the screen, which makes this device quiet while it listens.
///
/// It returns what the person chose ([StartChoice]), and `null` when they leave it as it was.
/// Nothing is kept here: the screen that opened it keeps the answer.
class StartAccessDialog extends StatefulWidget {
  const StartAccessDialog({
    required this.device,
    required this.thisDevice,
    required this.current,
    required this.hear,
    required this.canControl,
    super.key,
  });

  /// The paired device that asks.
  final Device device;

  /// The system that this device runs, to name it in words: "this Mac".
  final Platform thisDevice;

  /// What the device may do now.
  final StartAccess current;

  /// What was kept of the device hearing the sound: `true`, `false`, or `null` when nobody said,
  /// and then it may if it may control the screen.
  final bool? hear;

  /// Whether this device can let another control its screen. The choice is left out when it
  /// cannot, unless it is the one that is made now.
  final bool canControl;

  @override
  State<StartAccessDialog> createState() => _StartAccessDialogState();
}

class _StartAccessDialogState extends State<StartAccessDialog> {
  late StartAccess _chosen = widget.current;
  late bool? _hear = widget.hear;

  /// Whether the person has moved the switch. Until they have, it shows what the device gets
  /// without being told: the sound where the screen can be controlled.
  bool _touched = false;

  bool get _hearsByDefault => _chosen == StartAccess.control;

  /// What to keep, or `null` when it is what was kept. Saying what the device gets anyway is
  /// saying nothing, so that a device that is moved to another choice later follows it.
  StartChoice? get _choice {
    final hear = _touched && _hear == _hearsByDefault ? null : _hear;
    if (_chosen == widget.current && hear == widget.hear) return null;
    return (access: _chosen, hear: hear);
  }

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
          SwitchListTile(
            value: _hear ?? _hearsByDefault,
            onChanged: (on) => setState(() {
              _hear = on;
              _touched = true;
            }),
            title: Text('Let $name hear the sound'),
            subtitle: Text(
              '$computer is quiet while $name listens. Without this, $name '
              'shows the screen with no sound, and $computer plays on.',
            ),
          ),
          const SizedBox(height: 12),
          Text(
            'The screen shows only while $computer is awake and unlocked. '
            'Turning this off does not end a screen that is shared, or its '
            'sound: end it with Stop. Baton must be allowed to record the '
            'screen to show it: when you save, $computer asks for that if it '
            'has not been given. Say yes while you are here, then quit Baton '
            'and open it again.',
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
          onPressed: () => Navigator.of(context).pop(_choice),
          child: const Text('Save'),
        ),
      ],
    );
  }
}
