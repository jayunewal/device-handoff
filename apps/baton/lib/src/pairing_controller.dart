// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'backend.dart';

/// Where "Show a code" is.
enum OfferPhase {
  /// Getting a code ready. Baton gives the relay a few seconds to answer, so that the code
  /// works from other networks.
  starting,

  /// The code is on the screen, waiting for another device to ask.
  showing,

  /// Another device asked. The person compares the six digits with the other device and
  /// answers.
  asking,

  /// The person answered. What is left is to hear how it went.
  answering,

  /// A device was paired.
  paired,

  /// The code stopped working without anything being paired; [OfferController.outcome] says
  /// why.
  over,

  /// A code could not be made; [OfferController.problem] says why.
  failed,
}

/// What the "Show a code" screen shows: an invitation to pair, and what becomes of it.
class OfferController extends ChangeNotifier {
  OfferController(this._session);

  final Session _session;
  StreamSubscription<Event>? _events;
  bool _disposed = false;
  OfferPhase _phase = OfferPhase.starting;
  Offer? _offer;
  // The requests that have not been answered, oldest first. Baton only ever lets one device
  // pair with an offer, but several may ask before the person has answered the first.
  final List<PairingRequest> _requests = [];
  Device? _device;
  OfferOutcome? _outcome;
  String _problem = '';
  bool _accepted = false;

  OfferPhase get phase => _phase;

  /// The invitation, from [OfferPhase.showing] until it ends.
  Offer? get offer => _offer;

  /// The device to answer, while [phase] is [OfferPhase.asking].
  PairingRequest? get request => _requests.isEmpty ? null : _requests.first;

  /// How many more devices are waiting for an answer behind [request].
  int get waiting => _requests.isEmpty ? 0 : _requests.length - 1;

  /// The device that was paired, when [phase] is [OfferPhase.paired].
  Device? get device => _device;

  /// How the invitation ended, when [phase] is [OfferPhase.over].
  OfferOutcome? get outcome => _outcome;

  /// What to tell the person, when [phase] is [OfferPhase.failed]. It is one sentence.
  String get problem => _problem;

  /// Starts listening for what happens to the invitation and makes one.
  Future<void> start() async {
    // Listening comes first, so that nothing that happens to the invitation is missed.
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
    await _make();
  }

  /// Makes another invitation, after the last one ended or could not be made.
  Future<void> again() async {
    if (_phase != OfferPhase.over && _phase != OfferPhase.failed) return;
    await _make();
  }

  Future<void> _make() async {
    _requests.clear();
    _offer = null;
    _outcome = null;
    _device = null;
    _accepted = false;
    _set(OfferPhase.starting);
    try {
      final offer = await _session.offerPairing();
      if (_disposed) {
        // Made for a screen that has gone. Do not leave it open.
        _session.cancelOffer();
        return;
      }
      _offer = offer;
      _set(OfferPhase.showing);
    } on BatonError catch (e) {
      _problem = e.message;
      _set(OfferPhase.failed);
    }
  }

  /// Answers the device that asks: `true` if the six digits are the same on both screens.
  void answer({required bool accept}) {
    final asked = request;
    if (_phase != OfferPhase.asking || asked == null) return;
    _requests.removeAt(0);
    if (_session.answerPairing(asked.id, accept: accept)) {
      _accepted = accept;
      _set(OfferPhase.answering);
    } else {
      // It stopped waiting just now, and the events say why.
      _set(_requests.isEmpty ? OfferPhase.showing : OfferPhase.asking);
    }
  }

  void _onEvent(Event event) {
    if (_disposed) return;
    // Until the invitation is made, nothing that is said about invitations is about this one:
    // it may be the end of an earlier one that was withdrawn as this screen opened.
    if (_phase == OfferPhase.starting || _phase == OfferPhase.failed) return;
    switch (event) {
      case Event_PairingRequested(:final request):
        if (_phase == OfferPhase.paired || _phase == OfferPhase.over) return;
        _requests.add(request);
        if (_phase == OfferPhase.showing) {
          _set(OfferPhase.asking);
        } else {
          // Another device is asking behind the one on the screen.
          notifyListeners();
        }
      case Event_PairingRequestEnded(:final id):
        // Only a request nobody answered ends like this.
        _requests.removeWhere((r) => r.id == id);
        if (_phase == OfferPhase.asking && _requests.isEmpty) {
          _set(OfferPhase.showing);
        } else {
          // The one in front may have changed.
          notifyListeners();
        }
      case Event_OfferEnded(:final outcome):
        if (_phase == OfferPhase.paired || _phase == OfferPhase.over) return;
        _requests.clear();
        switch (outcome) {
          case OfferOutcome_Paired(:final device):
            _device = device;
            _set(OfferPhase.paired);
          default:
            _outcome = outcome;
            _set(OfferPhase.over);
        }
      default:
        // The other events are for the screens that list the devices and show the inbox.
        break;
    }
  }

  void _set(OfferPhase phase) {
    if (_disposed) return;
    _phase = phase;
    notifyListeners();
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    // An invitation that nobody can see should not stay open. One that has just been said yes to
    // is left alone: it ends by itself, and a withdrawal that got there first would win.
    final open =
        _phase == OfferPhase.showing ||
        _phase == OfferPhase.asking ||
        (_phase == OfferPhase.answering && !_accepted);
    if (open) _session.cancelOffer();
    // A stream from the bridge finishes a cancel only when it next has an event, so do not
    // wait for it.
    unawaited(_events?.cancel());
    super.dispose();
  }
}

/// Where "Scan or enter a code" is.
enum JoinPhase {
  /// Waiting for a code, from the camera or pasted.
  entering,

  /// Reaching the other device.
  connecting,

  /// The other device was reached. The person on it compares the six digits with this screen
  /// and says yes there.
  comparing,

  /// The other device said yes.
  paired,

  /// The pairing did not happen; [JoinController.problem] says why.
  failed,
}

/// What the "Scan or enter a code" screen shows: joining the invitation that another device
/// shows.
class JoinController extends ChangeNotifier {
  JoinController(this._session);

  final Session _session;
  StreamSubscription<JoinProgress>? _progress;
  bool _disposed = false;
  JoinPhase _phase = JoinPhase.entering;
  String _code = '';
  Device? _device;
  String _problem = '';
  // What was last turned down as not a code, so that a camera that keeps seeing the same
  // picture does not start over for it again and again.
  String? _turnedDown;

  JoinPhase get phase => _phase;

  /// The six digits to compare, as `514 643`, from [JoinPhase.comparing] on.
  String get code => _code;

  /// The device that was paired, when [phase] is [JoinPhase.paired].
  Device? get device => _device;

  /// What to tell the person: why the pairing failed, or what was wrong with the text that
  /// was given while [phase] is [JoinPhase.entering]. It is one sentence, or empty.
  String get problem => _problem;

  /// Starts to pair with the device that made [ticket], which the person pasted or typed.
  void join(String ticket) {
    if (_phase != JoinPhase.entering || ticket.trim().isEmpty) return;
    _problem = '';
    _set(JoinPhase.connecting);
    // The try before this one, if it was turned down, may not have ended yet. Nothing it says
    // from here on is about this one. Cancelling a bridge stream finishes only when it next
    // yields (see docs/architecture.md), so this does not wait for it.
    unawaited(_progress?.cancel());
    _progress = _session
        .joinPairing(ticket)
        .listen(
          (progress) => _onProgress(progress, ticket),
          onDone: _onDone,
          onError: (Object e) {
            debugPrint('Baton: pairing: $e');
            _problem = 'Something unexpected went wrong: $e';
            _set(JoinPhase.failed);
          },
        );
  }

  /// Starts to pair with the device whose QR code the camera read as [text]. A camera keeps
  /// seeing what is in front of it, so text that was turned down as not a code is not tried
  /// again until something else is in front of the camera.
  void scanned(String text) {
    if (text == _turnedDown) return;
    join(text);
  }

  void _onProgress(JoinProgress progress, String ticket) {
    switch (progress) {
      case JoinProgress_Code(:final code):
        _code = code;
        _set(JoinPhase.comparing);
      case JoinProgress_Paired(:final device):
        _device = device;
        _set(JoinPhase.paired);
      case JoinProgress_Failed(:final error):
        _problem = error.message;
        if (error.kind == ErrorKind.badTicket) {
          // The person's to fix: another picture or another paste.
          _turnedDown = ticket;
          _set(JoinPhase.entering);
        } else {
          _set(JoinPhase.failed);
        }
    }
  }

  void _onDone() {
    // The stream ends after its last item. If it ends without one, do not leave the person
    // waiting.
    if (_phase == JoinPhase.connecting || _phase == JoinPhase.comparing) {
      _problem = 'Pairing stopped before it was finished.';
      _set(JoinPhase.failed);
    }
  }

  /// Goes back to waiting for a code, after a failure.
  void tryAgain() {
    if (_phase != JoinPhase.failed) return;
    _problem = '';
    _turnedDown = null;
    _set(JoinPhase.entering);
  }

  void _set(JoinPhase phase) {
    if (_disposed) return;
    _phase = phase;
    notifyListeners();
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    if (_phase == JoinPhase.connecting || _phase == JoinPhase.comparing) {
      _session.cancelJoin();
    }
    unawaited(_progress?.cancel());
    super.dispose();
  }
}
