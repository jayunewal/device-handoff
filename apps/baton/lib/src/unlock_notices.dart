// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'notice_access.dart';
import 'unlock_text.dart';

/// Tells the person at this device, with a notification of the system, of each attempt to unlock
/// it that is worth knowing of: one that unlocked it, and one that got as far as a try and failed
/// (`UnlockEntry.notify`, a rule that Baton keeps once). A refusal that came before anything was
/// typed, a person who cancelled and a session that ended say nothing worth a notification, and
/// they are in the history all the same.
///
/// This is the other half of the unlock: it is the person at this device who set it up, and they
/// are not looking at it while a paired device unlocks it. The notification names the device and
/// says how the attempt ended, and holds nothing of the password, the keys or the screen. A missed
/// event is in the history, which is read again.
class UnlockNotices {
  UnlockNotices(this._session, this._notices);

  final Session _session;
  final NoticeKeeper _notices;
  StreamSubscription<Event>? _events;
  bool _disposed = false;

  /// Starts listening for events.
  Future<void> start() async {
    final events = await _session.events();
    if (_disposed) {
      // Nobody is left to read them. A stream is told to stop only when someone cancels it.
      unawaited(events.listen(null).cancel());
      return;
    }
    _events = events.listen(
      _onEvent,
      onError: (Object e) => debugPrint('Baton: events: $e'),
    );
  }

  void _onEvent(Event event) {
    if (_disposed) return;
    if (event case Event_UnlockAttempted(:final entry) when entry.notify) {
      unawaited(_tell(entry));
    }
  }

  Future<void> _tell(UnlockEntry entry) async {
    final text = describeNotice(entry);
    try {
      await _notices.post(title: text.title, body: text.body);
    } on Object catch (e) {
      // The history has it all the same.
      debugPrint('Baton: notice: $e');
    }
  }

  void dispose() {
    _disposed = true;
    // A stream from the bridge finishes a cancel only when it next has an event, so do not wait
    // for it.
    unawaited(_events?.cancel());
  }
}
