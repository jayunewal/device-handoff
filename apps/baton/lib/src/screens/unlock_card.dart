// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';
import '../unlock_text.dart';
import '../windows_controller.dart';

/// What covers the picture of a screen that is locked at the device that shares it: that it is
/// locked, and where the person can unlock it from here, the Unlock button, how the unlock goes
/// and how the last one ended. It takes the touches that land on it, because a locked device drops
/// them.
///
/// The button is there when the session takes an unlock (`WindowSession.unlock`): the person at
/// the other device set it up for this one, the screen is kept private, and this device can ask
/// for a face. A session that does not says so and names the way to get one, without guessing at
/// which of the three is missing.
///
/// Tapping Unlock asks for Face ID or Touch ID on this device, and nothing is sent to the other
/// device until it is recognised. The sentences are in `unlock_text.dart`.
class LockedCard extends StatelessWidget {
  const LockedCard({
    required this.controller,
    required this.session,
    required this.onUnlock,
    super.key,
  });

  final WindowsController controller;

  /// The screen that is locked.
  final WindowSession session;

  /// Taps the Unlock button. The screen that shows the card asks the controller and tells the
  /// person when Baton did not take the request.
  final VoidCallback onUnlock;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final name = visibleText(session.deviceName);
    final unlock = controller.unlockOf(session);
    final going = describeUnlocking(unlock.stage, session.deviceName);
    final result = unlock.result;
    final ended = unlock.stage == UnlockStage.ended && result != null;
    final canAsk =
        session.unlock &&
        (unlock.stage == UnlockStage.idle ||
            (ended && canTryUnlockAgain(result)));
    // The device said that it is unlocked and its picture has not come back yet.
    final unlocked = ended && result == UnlockResult.unlocked;
    // The sentence under the title: what is going on, how the last try ended, or what the person
    // can do.
    final detail =
        going ??
        (unlocked
            ? 'Its picture comes back in a moment.'
            : ended
            ? describeUnlockEnd(result, session.deviceName)
            : session.unlock
            ? 'Face ID or Touch ID on this device confirms that it is you. '
                  'Then $name types its password itself.'
            : 'This session cannot unlock it. Unlock it at $name, or set up '
                  '“Let this device unlock this Mac” in Baton there and '
                  'control $name from here.');
    return ColoredBox(
      color: Colors.black.withValues(alpha: 0.72),
      child: Center(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(24),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(Icons.lock_outline, size: 40, color: Colors.white),
                const SizedBox(height: 12),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    unlocked ? '$name is unlocked' : '$name is locked',
                    textAlign: TextAlign.center,
                    style: theme.textTheme.titleMedium?.copyWith(
                      color: Colors.white,
                    ),
                  ),
                ),
                const SizedBox(height: 8),
                Semantics(
                  liveRegion: true,
                  child: Text(
                    detail,
                    textAlign: TextAlign.center,
                    style: theme.textTheme.bodyMedium?.copyWith(
                      color: Colors.white70,
                    ),
                  ),
                ),
                if (going != null) ...[
                  const SizedBox(height: 16),
                  SizedBox(
                    width: 200,
                    child: LinearProgressIndicator(semanticsLabel: going),
                  ),
                ],
                if (canAsk) ...[
                  const SizedBox(height: 16),
                  FilledButton.icon(
                    onPressed: onUnlock,
                    icon: const Icon(Icons.lock_open),
                    label: Text(ended ? 'Try again' : 'Unlock'),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}
