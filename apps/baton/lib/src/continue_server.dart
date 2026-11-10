// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'continue_reader.dart';

/// Answers a paired device that asks what the person at this device is working on, to continue it
/// there (docs/adr/0015-continue-from-another-device.md).
///
/// The library has already checked that the device is paired and that the person here allowed it,
/// and it has said so to this class as `Event_ContinueAsked`. The page that is in front in the
/// browser and the text on the clipboard are read now, when the question comes, and handed to the
/// library, which gives them to the device. Nothing is read in advance, and nothing is kept: not
/// here, and not in the library.
///
/// It listens for as long as the app runs, also with no window on the screen: this is how a phone
/// reaches a Mac that nobody sits at, with only its menu bar item.
class ContinueServer {
  ContinueServer(this._session, this._reader);

  final Session _session;
  final ContinueReader _reader;
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
    if (event case Event_ContinueAsked(:final id)) {
      unawaited(_answer(id));
    }
  }

  Future<void> _answer(int request) async {
    ContinueItems? items;
    try {
      items = await _reader.read();
    } on Object catch (e) {
      // The device that asked is told that it could not be read, which is what `null` says.
      debugPrint('Baton: continuing: ${e.runtimeType}');
    }
    // The library gives a few seconds to read. An answer that comes later is turned down, and the
    // device has been told already.
    try {
      _session.answerContinue(request, items);
    } on Object catch (e) {
      debugPrint('Baton: continuing: ${e.runtimeType}');
    }
  }

  void dispose() {
    _disposed = true;
    // A stream from the bridge finishes a cancel only when it next has an event, so do not wait
    // for it.
    unawaited(_events?.cancel());
  }
}
