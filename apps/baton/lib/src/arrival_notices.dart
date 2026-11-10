// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'always_on.dart';
import 'backend.dart';
import 'format.dart';
import 'notice_access.dart';

/// Tells the person at this device, with a notification of the system, that something came for
/// them while they cannot see Baton: a link or some text that a paired device sent, a device that
/// asks to send files, and a device that offers to show a window or its screen. A Mac keeps Baton
/// running with no window (the item in the menu bar, macos/Runner/AppDelegate.swift), and the notes
/// at the bottom of the window that say the same are not seen then, nor while another app is in
/// front.
///
/// A notification says who and what kind of thing and never what: it can be a password, an address
/// with a token in it or the name of a private file, and a notification shows on a screen that
/// anyone may look at. A click on it opens the window, where the inbox and the questions that wait
/// for a yes are. Nothing here answers a question for the person.
///
/// A person who did not allow notifications is told nothing, which `NoticeKeeper.post` says by
/// returning `false`: the Permissions screen is where they are asked.
class ArrivalNotices {
  ArrivalNotices({
    required Stream<Received> arrivals,
    required Stream<TransferRequest> files,
    required Stream<WindowRequest> windows,
    required this._notices,
    required this._window,
  }) {
    _subscriptions = [
      arrivals.listen(_arrived),
      files.listen(_wantsToSend),
      windows.listen(_wantsToShow),
    ];
  }

  final NoticeKeeper _notices;
  final AlwaysOnKeeper _window;
  late final List<StreamSubscription<Object?>> _subscriptions;
  bool _disposed = false;

  void _arrived(Received received) {
    final what = switch (received.item) {
      ReceivedItem_Link() => 'a link',
      ReceivedItem_Text() => 'some text',
    };
    unawaited(
      _tell(
        title: '${visibleText(received.fromName)} sent you $what',
        body: 'Open Baton to see it in the inbox.',
      ),
    );
  }

  void _wantsToSend(TransferRequest request) {
    unawaited(
      _tell(
        title: '${visibleText(request.deviceName)} wants to send you files',
        body: 'Open Baton to say yes or no.',
      ),
    );
  }

  void _wantsToShow(WindowRequest request) {
    unawaited(
      _tell(
        title:
            '${visibleText(request.deviceName)} wants to show you '
            '${request.display ? 'its screen' : 'a window'}',
        body: 'Open Baton to say yes or no.',
      ),
    );
  }

  Future<void> _tell({required String title, required String body}) async {
    try {
      // The person who is looking at Baton is told by the window, and is not told twice.
      if (await _window.looking() || _disposed) return;
      await _notices.post(title: title, body: body);
    } on Object catch (e) {
      // A notification that did not show is not worth stopping for: what came is in the inbox,
      // or waits as a question in the window, all the same.
      debugPrint('Baton: notice: $e');
    }
  }

  void dispose() {
    _disposed = true;
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
  }
}
