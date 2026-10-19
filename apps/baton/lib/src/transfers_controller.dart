// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'awake.dart';
import 'backend.dart';
import 'file_shower.dart';

/// What came of taking the person to the files that arrived.
enum ShowResult {
  /// The system was asked to show them.
  shown,

  /// They are not there any more: the person moved or deleted them.
  gone,

  /// The system could not be asked.
  failed,
}

/// What the screens show of the files that other devices send: the requests for the person to
/// answer, the transfers they said yes to that are on their way, and how the last few ended.
///
/// The screen of a phone stays on from a request until the transfer has ended, because Baton on a
/// phone works only while it is on the screen. Nothing says how far a transfer that is being
/// received has got, so there is no step to renew the hold with: the screen stays on for
/// [awakeLimit] after the request and after the yes, and then goes back to locking as the person
/// set it. [screen] does that; tests give it one of their own.
///
/// A transfer that has ended can take the person to the files that arrived, which [shower] does;
/// tests give it one of their own.
class TransfersController extends ChangeNotifier {
  TransfersController(
    this._session, {
    ScreenKeeper screen = const PlatformScreenKeeper(),
    this._shower = const SystemFileShower(),
  }) : _awake = AwakeHold(screen);

  /// How many ended transfers are kept to be shown.
  static const keepEnded = 5;

  final Session _session;
  final AwakeHold _awake;
  final FileShower _shower;
  final StreamController<TransferRequest> _asked = StreamController.broadcast();
  StreamSubscription<Event>? _events;
  bool _disposed = false;
  final List<TransferRequest> _requests = [];
  final List<TransferRequest> _coming = [];
  final List<TransferEnd> _ended = [];

  /// Whether this device takes files, and if not, why.
  Receiving get taking => _session.receiving();

  /// Where received files are put, or `null` if this device takes none.
  String? get folder => _session.receiveFolder;

  /// The requests that wait for the person's answer, oldest first.
  List<TransferRequest> get requests => List.unmodifiable(_requests);

  /// The transfers that the person said yes to and that have not ended, oldest first.
  List<TransferRequest> get coming => List.unmodifiable(_coming);

  /// How the last transfers ended, newest first, until the person puts them away.
  List<TransferEnd> get ended => List.unmodifiable(_ended);

  /// Each request as it comes, so that a screen that covers the list can say that one waits.
  Stream<TransferRequest> get asked => _asked.stream;

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
    switch (event) {
      case Event_TransferRequested(:final request):
        if (_requests.any((r) => r.id == request.id)) return;
        _requests.add(request);
        _awake.renew();
        _asked.add(request);
        notifyListeners();
      case Event_TransferRequestEnded(:final id):
        // The other device gave up, was unpaired, or the person's time ran out.
        final before = _requests.length;
        _requests.removeWhere((r) => r.id == id);
        if (_requests.length == before) return;
        _settle();
      case Event_TransferEnded(:final end):
        _coming.removeWhere((r) => r.id == end.request);
        _ended.insert(0, end);
        if (_ended.length > keepEnded) {
          _ended.removeRange(keepEnded, _ended.length);
        }
        _settle();
      default:
        // The other events are for the screens that pair devices, list them and show the inbox.
        break;
    }
  }

  /// Says yes or no to [request]. Returns `false` if it was not waiting any more, because the
  /// other device gave up or the time ran out.
  bool answer(TransferRequest request, {required bool accept}) {
    final index = _requests.indexWhere((r) => r.id == request.id);
    if (index < 0) return false;
    final waiting = _session.answerTransfer(request.id.toInt(), accept: accept);
    _requests.removeAt(index);
    if (waiting && accept) {
      _coming.add(request);
      _awake.renew();
    }
    _settle();
    return waiting;
  }

  /// Puts away what is shown of a transfer that has ended.
  void dismiss(TransferEnd end) {
    if (_ended.remove(end)) notifyListeners();
  }

  /// Whether there is anything of the transfer [end] to take the person to: something arrived.
  bool canShow(TransferEnd end) => end.saved.isNotEmpty;

  /// Takes the person to what arrived with [end]. The files are shown, not opened.
  Future<ShowResult> show(TransferEnd end) async {
    if (!canShow(end)) return ShowResult.gone;
    try {
      return await _shower.show(end.saved) ? ShowResult.shown : ShowResult.gone;
    } on Object catch (e) {
      // The names of the files are the person's, and an error can carry them, so only what went
      // wrong is written down.
      debugPrint('Baton: showing files: ${e.runtimeType}');
      return ShowResult.failed;
    }
  }

  /// The screen can turn off again when nothing is waiting or on its way.
  void _settle() {
    if (_requests.isEmpty && _coming.isEmpty) _awake.release();
    notifyListeners();
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _awake.release();
    // A stream from the bridge finishes a cancel only when it next has an event, so do not
    // wait for it.
    unawaited(_events?.cancel());
    unawaited(_asked.close());
    super.dispose();
  }
}
