// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'format.dart';

/// What waits to be sent to a device that could not be reached, kept up to date from a running
/// Baton, and how each item ends (`docs/adr/0016-outbox.md`).
///
/// The list says what each item is and never what it says: the person knows what they sent, and a
/// list that shows the text of a password is one to look over a shoulder at.
class OutboxController extends ChangeNotifier {
  OutboxController(this._session);

  final Session _session;
  final StreamController<OutboxSettled> _settled = StreamController.broadcast();
  StreamSubscription<Event>? _events;
  bool _disposed = false;
  List<OutboxItem> _items = const [];

  /// What waits, the item that has waited longest first.
  List<OutboxItem> get items => _items;

  /// What waits for the device [deviceId], the item that has waited longest first.
  List<OutboxItem> waitingFor(String deviceId) => [
    for (final item in _items)
      if (item.deviceId == deviceId) item,
  ];

  /// Each item as it ends, for a screen to tell the person with the sentence as it is: the device
  /// took it, did not take it, or could not be reached for a day. An item that the person cancelled,
  /// or whose device they unpaired, is not told.
  Stream<OutboxSettled> get settled => _settled.stream;

  /// Starts listening for events and reads the outbox.
  Future<void> start() async {
    // Listening comes first, so that nothing that happens while the outbox is read is lost.
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
    _read();
  }

  void _onEvent(Event event) {
    // No event comes after `dispose`: it cancels the subscription before anything else.
    switch (event) {
      case Event_OutboxChanged() || Event_Resync():
        _read();
      case Event_OutboxSettled(:final settled):
        _settled.add(settled);
      default:
        // The other events are for the screens that pair devices and list them.
        break;
    }
  }

  /// Gives up on sending [item]. Returns what to tell the person, as a sentence: that it will not
  /// be sent, or why this did not work. Returns `null` when there is nothing to tell, because the
  /// item was sent a moment before: the person is told that as it ends (see [settled]).
  Future<String?> cancel(OutboxItem item) async {
    final bool found;
    try {
      found = await _session.cancelOutbox(item.id.toInt());
    } on BatonError catch (e) {
      return sentence(e.message);
    }
    // `Event_OutboxChanged` comes too. The list is read here so that it is right at once, whether
    // or not that has been heard yet.
    _read();
    if (!found) return null;
    return 'Cancelled. Baton will not send ${visibleText(item.summary)} to '
        '${visibleText(item.deviceName)}.';
  }

  void _read() {
    if (_disposed) return;
    try {
      _items = _session.outbox();
    } on Object catch (e) {
      // The list stays as it was. Baton has stopped, which only happens as the app closes.
      debugPrint('Baton: outbox: ${e.runtimeType}');
      return;
    }
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    // A stream from the bridge finishes a cancel only when it next has an event, so do not wait for
    // it.
    unawaited(_events?.cancel());
    unawaited(_settled.close());
    super.dispose();
  }
}
