// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';
import '../windows_controller.dart';

/// What the person answered when asked, as they end a session, what is to become of the device
/// they control.
enum EndAnswer {
  /// End the session as every session ends: the device locks itself.
  lock,

  /// End the session and ask the device to stay as it is, and not lock itself.
  leave,
}

/// Ends [session], which is shown here, the way the person chooses. A screen whose device can be
/// left as it is (`WindowSession.leave`) asks first: lock the device, which is what ends every
/// session, leave it as it is, or keep the session. Any other session ends without a question.
///
/// The question is asked every time and the answer is kept nowhere, because a stored "leave it"
/// would be one consent given once for a device that stays open at every end after it. Only the
/// answer given here leaves the device unlocked: every other end of a session locks it.
///
/// Returns whether the person ended the session. It is `false` when they chose to keep it, and
/// when it ended while the question was open: the screen then says how it ended.
Future<bool> endAskingToLock(
  BuildContext context,
  WindowsController controller,
  WindowSession session,
) async {
  if (!session.leave) {
    controller.end(session);
    return true;
  }
  final answer = await showDialog<EndAnswer>(
    context: context,
    builder: (context) =>
        LockOrLeaveDialog(controller: controller, session: session),
  );
  switch (answer) {
    case EndAnswer.lock:
      controller.end(session);
      return true;
    case EndAnswer.leave:
      controller.endLeaving(session);
      return true;
    case null:
      return false;
  }
}

/// The question that [endAskingToLock] asks, with the three answers: Lock, which is the default
/// and ends the session as ever, Leave it, which ends it and asks the device to stay as it is, and
/// Cancel, which keeps the session. It returns the [EndAnswer], or `null` for Cancel and for a tap
/// beside the question.
///
/// The question closes by itself when the session ends while it is open, since a "leave it" would
/// then be an answer to a session that is over, and the person could take it to have left the
/// device open when the device locked as it does at every other end.
class LockOrLeaveDialog extends StatefulWidget {
  const LockOrLeaveDialog({
    required this.controller,
    required this.session,
    super.key,
  });

  final WindowsController controller;

  /// The screen that is asked about. It has to be one that the device can leave as it is.
  final WindowSession session;

  @override
  State<LockOrLeaveDialog> createState() => _LockOrLeaveDialogState();
}

class _LockOrLeaveDialogState extends State<LockOrLeaveDialog> {
  /// The route that this question is on, to tell whether it still is the one on top.
  ModalRoute<Object?>? _route;

  bool get _running =>
      widget.controller.sessions.any((s) => s.id == widget.session.id);

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_changed);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _route = ModalRoute.of(context);
  }

  @override
  void dispose() {
    widget.controller.removeListener(_changed);
    super.dispose();
  }

  /// The session ended with the question open: there is nothing left to answer.
  void _changed() {
    if (!_running) _answer(null);
  }

  /// Closes the question with [answer], and only while it is the route on top. A question that was
  /// answered or dismissed already is on its way out and no longer the one on top, so a second
  /// answer takes nothing away, least of all the screen under it.
  void _answer(EndAnswer? answer) {
    final route = _route;
    if (route == null || !route.isCurrent) return;
    Navigator.of(context).pop(answer);
  }

  @override
  Widget build(BuildContext context) {
    final device = visibleText(widget.session.deviceName);
    return AlertDialog(
      title: Text('Lock $device?'),
      content: Text(
        '$device locks when you stop showing it. If you leave it, anyone at '
        '$device can use it until it is locked.',
      ),
      actions: [
        TextButton(onPressed: () => _answer(null), child: const Text('Cancel')),
        TextButton(
          onPressed: () => _answer(EndAnswer.leave),
          child: const Text('Leave it'),
        ),
        FilledButton(
          autofocus: true,
          onPressed: () => _answer(EndAnswer.lock),
          child: const Text('Lock'),
        ),
      ],
    );
  }
}
