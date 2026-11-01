// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';

/// What the person chose for a paired device: what it may do about the screen, and whether it may
/// hear the sound of it. [hear] is `null` when the person leaves it to the session: the device
/// hears in one that lets it control the screen, and not in one that lets it only look.
typedef StartChoice = ({StartAccess access, bool? hear});

/// Asks the person at this device what [device] may do without anyone here saying yes: ask this
/// device to start sharing its screen with it, to look at it or to look at it and control it, and
/// whether it may hear the sound of the screen, which makes this device quiet while it listens.
///
/// It returns what the person chose ([StartChoice]) when they save, also when it is as it was, and
/// `null` when they cancel. Nothing is kept here: the screen that opened it keeps what changed, and
/// asks the system for the consents that a setting needs, which may have been taken back since.
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
  /// and then it may in a session that lets it control the screen.
  final bool? hear;

  /// Whether this device can let another control its screen. The choice is left out when it
  /// cannot, unless it is the one that is made now.
  final bool canControl;

  @override
  State<StartAccessDialog> createState() => _StartAccessDialogState();
}

class _StartAccessDialogState extends State<StartAccessDialog> {
  late StartAccess _chosen = widget.current;

  /// What the person has said of the sound: a yes, a no, or nothing (`null`), which leaves it to
  /// the session. Moving the switch says a yes or a no, also where that is what the device gets
  /// anyway, so that each of the three can be kept in one go. The button under it takes it back.
  late bool? _hear = widget.hear;

  /// Whether the device hears where nothing is said, in a session that it asks for: where the
  /// screen can be controlled. It is what the switch shows until the person moves it.
  bool get _hearsByDefault => _chosen == StartAccess.control;

  /// What the person chose, which may be what was kept already.
  StartChoice get _choice => (access: _chosen, hear: _hear);

  /// What the person is told of the sound, by what would be kept of it.
  String _soundNote(String name, String computer) => switch (_hear) {
    null =>
      'Left as it is, $name hears when it controls $computer. When it only '
          'looks, it gets the screen with no sound, and $computer plays on.',
    true => '$name hears in every session, also when it only looks.',
    false =>
      '$name gets the screen with no sound, so it does not make $computer '
          'quiet.',
  };

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
            onChanged: (on) => setState(() => _hear = on),
            title: Text('Let $name hear the sound'),
            subtitle: Text(
              '$computer is quiet while $name listens. '
              '${_soundNote(name, computer)}',
            ),
          ),
          if (_hear != null)
            Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: const EdgeInsets.only(left: 4),
                child: TextButton(
                  onPressed: () => setState(() => _hear = null),
                  child: const Text('Follow the session'),
                ),
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
