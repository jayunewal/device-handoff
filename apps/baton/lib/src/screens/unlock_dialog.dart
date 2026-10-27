// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../controller.dart';
import '../format.dart';
import '../notice_access.dart';
import '../unlock_text.dart';

/// What the person did in [UnlockDialog], for the screen that opened it.
enum UnlockDialogResult {
  /// The unlock is set up for the device.
  setUp,

  /// The unlock for the device was turned off.
  turnedOff,
}

/// Lets the person at this device set the unlock up for a paired device, so that the device can
/// unlock this one while it is locked (docs/adr/0013-unlock-from-a-phone.md), and turn it off, and
/// shows the last attempts of that device to unlock this one.
///
/// Setting it up takes the password of the person who uses this device. Baton checks it against
/// the account, and keeps it in a box that only the other device opens, after its person's face.
/// The field hides what is typed, lets nothing suggest or correct it, and is emptied as soon as
/// the password has been given to Baton, whatever came of it. The password is a string of the app
/// for as long as it is typed, which Flutter cannot wipe; the record says so.
///
/// It returns what the person did, and `null` when they closed it. Nothing is kept here: the
/// controller does the work, and the screen that opened this tells the person what came of it.
class UnlockDialog extends StatefulWidget {
  const UnlockDialog({
    required this.controller,
    required this.device,
    required this.thisDevice,
    required this.notices,
    this.now = DateTime.now,
    super.key,
  });

  final DevicesController controller;

  /// The paired device that would unlock this one.
  final Device device;

  /// The system that this device runs, to name it in words: "this Mac".
  final Platform thisDevice;

  /// What the system says about notifications, which tell the person that the device unlocked this
  /// one. The dialog says so when they are off.
  final NoticeKeeper notices;

  /// The time to measure "5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  @override
  State<UnlockDialog> createState() => _UnlockDialogState();
}

class _UnlockDialogState extends State<UnlockDialog> {
  final TextEditingController _password = TextEditingController();

  /// Whether Baton is at work on something that closing this would leave the person not knowing
  /// the end of.
  bool _working = false;

  /// A sentence about what did not work, or `null`.
  String? _problem;

  /// What the system says about notifications, once it has been asked.
  NoticeAccess? _access;

  @override
  void initState() {
    super.initState();
    _askAccess();
  }

  Future<void> _askAccess() async {
    final access = await widget.notices.state();
    if (mounted) setState(() => _access = access);
  }

  @override
  void dispose() {
    _password.dispose();
    super.dispose();
  }

  Future<void> _setUp() async {
    final password = _password.text;
    if (_working || password.isEmpty) return;
    setState(() {
      _working = true;
      _problem = null;
    });
    final result = await widget.controller.setUpUnlock(widget.device, password);
    // Baton has the password now, or has refused it. Either way the field lets go of it: a typing
    // mistake is typed again, and nothing waits in the field.
    _password.clear();
    if (!mounted) return;
    switch (result) {
      case UnlockSetUp_Done():
        Navigator.of(context).pop(UnlockDialogResult.setUp);
      case UnlockSetUp_WrongPassword():
        setState(() {
          _working = false;
          _problem =
              'That is not the password of this '
              '${platformName(widget.thisDevice)}, so nothing was kept. Type '
              'it again.';
        });
      case UnlockSetUp_NotDone(:final message):
        setState(() {
          _working = false;
          _problem = message;
        });
    }
  }

  Future<void> _turnOff() async {
    if (_working) return;
    setState(() {
      _working = true;
      _problem = null;
    });
    final problem = await widget.controller.turnOffUnlock(widget.device);
    if (!mounted) return;
    if (problem == null) {
      Navigator.of(context).pop(UnlockDialogResult.turnedOff);
    } else {
      setState(() {
        _working = false;
        _problem = problem;
      });
    }
  }

  /// Where the unlock stands, in a sentence. [name] is the device that would unlock this one.
  String _describeStatus(
    UnlockStatus? status,
    String name,
    String computer,
    DateTime now,
  ) => switch (status) {
    null => '$name is not paired with this $computer any more.',
    UnlockStatus_Off() => 'Not set up. $name cannot unlock this $computer.',
    UnlockStatus_On(since: final at) =>
      'Set up ${since(at.toInt(), now)}. $name can unlock this $computer '
          'while it is locked, when you control it from $name and Face ID or '
          'Touch ID there confirms that it is you.',
    UnlockStatus_Ended(since: final at) =>
      'Set up ${since(at.toInt(), now)}, and it ended: the password was not '
          'accepted, or $name lost its key. Set it up again below.',
  };

  @override
  Widget build(BuildContext context) {
    final name = visibleText(widget.device.name);
    final computer = platformName(widget.thisDevice);
    final theme = Theme.of(context);
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) {
        final status = widget.controller.unlockStatus(widget.device);
        final attempts = widget.controller.unlockAttempts(widget.device);
        final now = widget.now();
        return PopScope(
          // Closing while Baton works would leave the person not knowing what came of it.
          canPop: !_working,
          child: AlertDialog(
            scrollable: true,
            title: Text('Unlock this $computer from $name'),
            content: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  _describeStatus(status, name, computer, now),
                  style: theme.textTheme.bodyMedium,
                ),
                const SizedBox(height: 12),
                Text(
                  'Baton does not keep your password where it can be read. It is '
                  'locked in a box on this $computer, and the key to the box is '
                  'given out only by Face ID or Touch ID on $name, every time. '
                  'It works in a session where you control this $computer from '
                  '$name and its screen is kept private. If you change the '
                  'password of this $computer, set this up again.',
                  style: theme.textTheme.bodySmall,
                ),
                const SizedBox(height: 16),
                TextField(
                  controller: _password,
                  enabled: !_working,
                  obscureText: true,
                  autocorrect: false,
                  enableSuggestions: false,
                  enableIMEPersonalizedLearning: false,
                  textInputAction: TextInputAction.done,
                  onChanged: (_) => setState(() {}),
                  onSubmitted: (_) => _setUp(),
                  decoration: InputDecoration(
                    labelText: 'Password of this $computer',
                    helperText: 'The one you type to unlock it.',
                    border: const OutlineInputBorder(),
                  ),
                ),
                if (_working) ...[
                  const SizedBox(height: 12),
                  const LinearProgressIndicator(semanticsLabel: 'Working'),
                ],
                if (_problem case final problem?) ...[
                  const SizedBox(height: 12),
                  Semantics(
                    liveRegion: true,
                    child: Text(
                      problem,
                      style: theme.textTheme.bodyMedium?.copyWith(
                        color: theme.colorScheme.error,
                      ),
                    ),
                  ),
                ],
                if (_access case final access?)
                  if (describeNoticeAccess(access, computer)
                      case final sentence?) ...[
                    const SizedBox(height: 12),
                    Text(sentence, style: theme.textTheme.bodySmall),
                  ],
                const SizedBox(height: 16),
                Text(
                  'Recent attempts from $name',
                  style: theme.textTheme.titleSmall,
                ),
                const SizedBox(height: 4),
                if (attempts.isEmpty)
                  Text('None yet.', style: theme.textTheme.bodySmall)
                else
                  for (final attempt in attempts)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 6),
                      child: Text(
                        '${describeAttempt(attempt)} '
                        '(${since(attempt.at.toInt(), now)})',
                        style: theme.textTheme.bodySmall,
                      ),
                    ),
              ],
            ),
            actions: [
              TextButton(
                onPressed: _working ? null : () => Navigator.of(context).pop(),
                child: const Text('Close'),
              ),
              if (status is UnlockStatus_On || status is UnlockStatus_Ended)
                TextButton(
                  onPressed: _working ? null : _turnOff,
                  child: const Text('Turn off'),
                ),
              FilledButton(
                onPressed: _working || _password.text.isEmpty ? null : _setUp,
                child: Text(
                  status is UnlockStatus_On || status is UnlockStatus_Ended
                      ? 'Set up again'
                      : 'Set up',
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
