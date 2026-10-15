// SPDX-License-Identifier: Apache-2.0

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'format.dart';

/// What the inbox shows: what other devices have sent, kept up to date from a running Baton.
class InboxController extends ChangeNotifier {
  InboxController(this._session);

  final Session _session;
  final StreamController<Received> _arrivals = StreamController.broadcast();
  StreamSubscription<Event>? _events;
  bool _disposed = false;
  bool _reading = false;
  bool _readAgain = false;
  bool _looking = false;
  List<Received>? _items;
  String? _problem;

  // The number of the newest item that was there when the app opened, or that the person has
  // had on screen since. Numbers are never reused, so every listed item above it is new, also
  // one whose event was missed (see `Event_Resync`). An item that is no longer listed is not
  // counted.
  int _seen = 0;

  // The numbers of the items whose events came while the inbox was not on screen. An event can
  // come after the first read has listed its item, and that read counts what it lists as
  // already there, so such an item is told apart by its event as well.
  final Set<int> _fresh = {};

  /// What other devices have sent, newest first, or `null` until it has been read.
  List<Received>? get items => _items;

  /// What to tell the person if the inbox could not be read, as a sentence. `null` if it could.
  String? get problem => _problem;

  /// How many of the items listed came since the person last had the inbox on screen.
  int get unseen {
    final items = _items;
    if (items == null) return _fresh.length;
    return items.where((item) => _isNew(item.number.toInt())).length;
  }

  bool _isNew(int number) => number > _seen || _fresh.contains(number);

  /// Each item as it comes, so that a screen other than the inbox can say that something
  /// arrived. The item is for that and no more: it can be a password, so a notice says who
  /// sent what kind of thing, not what it is.
  Stream<Received> get arrivals => _arrivals.stream;

  /// Whether the inbox is on screen. Set it while it is: what comes then is seen as it comes,
  /// so it is not counted as new.
  bool get looking => _looking;

  set looking(bool value) {
    if (_looking == value) return;
    _looking = value;
    if (value) {
      _fresh.clear();
      _seen = math.max(_seen, _newest(_items ?? const []));
    }
    notifyListeners();
  }

  /// Starts listening for events and reads the inbox.
  Future<void> start() async {
    // Listening comes first, so that nothing that comes while the inbox is read is lost.
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
    await _read();
  }

  /// Reads the inbox again, after it could not be read.
  Future<void> reload() {
    _problem = null;
    notifyListeners();
    return _read();
  }

  void _onEvent(Event event) {
    if (_disposed) return;
    switch (event) {
      case Event_Received(:final received):
        if (!_looking) _fresh.add(received.number.toInt());
        _arrivals.add(received);
        unawaited(_read());
      case Event_Resync():
        unawaited(_read());
      default:
        // The other events are for the screens that pair devices and list them.
        break;
    }
  }

  /// Takes [item] out of the inbox. Returns what to tell the person if that did not work, and
  /// `null` if it did.
  Future<String?> remove(Received item) async {
    try {
      await _session.removeReceived(item.number.toInt());
    } on BatonError catch (e) {
      return sentence(e.message);
    }
    _fresh.remove(item.number.toInt());
    unawaited(_read());
    return null;
  }

  /// Takes everything out of the inbox. Returns what to tell the person if that did not work,
  /// and `null` if it did.
  Future<String?> clear() async {
    try {
      await _session.clearInbox();
    } on BatonError catch (e) {
      return sentence(e.message);
    }
    _fresh.clear();
    unawaited(_read());
    return null;
  }

  /// The address that a screen may open for [item], or `null` if there is none: it is not a
  /// link, or not a web address. Open that address, and only that.
  String? addressToOpen(Received item) => switch (item.item) {
    ReceivedItem_Link(:final url) => _session.webAddress(url),
    ReceivedItem_Text() => null,
  };

  static int _newest(Iterable<Received> items) =>
      items.fold(0, (newest, item) => math.max(newest, item.number.toInt()));

  Future<void> _read() async {
    if (_reading) {
      // Something changed after the read that is going on began, so read again after it.
      _readAgain = true;
      return;
    }
    _reading = true;
    try {
      do {
        _readAgain = false;
        final items = await _session.inbox();
        if (_disposed) return;
        if (_items == null) {
          // What was there when the app opened is not new, but what came as it was read is.
          _seen = _newest(
            items.where((item) => !_fresh.contains(item.number.toInt())),
          );
        }
        if (_looking) _seen = math.max(_seen, _newest(items));
        _items = items;
        _problem = null;
        notifyListeners();
      } while (_readAgain);
    } on BatonError catch (e) {
      if (_disposed) return;
      if (e.kind == ErrorKind.stopped) {
        // Baton has stopped, which only happens as the app closes.
        debugPrint('Baton: inbox: ${e.message}');
      } else {
        _problem = sentence(e.message);
        notifyListeners();
      }
    } finally {
      _reading = false;
    }
  }

  @override
  void dispose() {
    _disposed = true;
    // A stream from the bridge finishes a cancel only when it next has an event, so do not
    // wait for it.
    unawaited(_events?.cancel());
    unawaited(_arrivals.close());
    super.dispose();
  }
}
