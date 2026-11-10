// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'format.dart';

/// What happened to the last thing the person asked for, in a sentence to show them.
class Outcome {
  const Outcome(this.message, {required this.ok, this.waiting = false});

  final String message;

  /// Whether it worked. A send that waits did, too: nothing went wrong that the person has to
  /// act on.
  final bool ok;

  /// Whether what the person sent waits in the outbox, because the device could not be reached
  /// (`SendResult.waiting`). It is not sent yet, and the sentence says so.
  final bool waiting;
}

/// Sends a link or some text to one paired device, and checks that the device can be reached.
class SendController extends ChangeNotifier {
  SendController(this._session, this.device);

  final Session _session;

  /// The device that things are sent to.
  final Device device;

  bool _sending = false;
  bool _checking = false;
  bool _disposed = false;
  Outcome? _outcome;

  /// Whether something is on its way to the device.
  bool get sending => _sending;

  /// Whether the device is being asked if it is there.
  bool get checking => _checking;

  /// What happened to the last send or check, until the text is changed.
  Outcome? get outcome => _outcome;

  /// The address that [text] is sent as, if it is a link: a web address with nothing around
  /// it. `null` if it goes as text, as a line that only has a link in it among other words
  /// does.
  String? linkIn(String text) {
    final trimmed = text.trim();
    if (trimmed.isEmpty || trimmed.contains(RegExp(r'\s'))) return null;
    return _session.webAddress(trimmed);
  }

  /// Forgets what happened to the last send or check, because the text is not the same any
  /// more.
  void edited() {
    if (_outcome == null) return;
    _outcome = null;
    notifyListeners();
  }

  /// Sends [text] to the device, as a link if it is one and as text if not. Returns whether it
  /// left the box: it was sent, or the device could not be reached and it waits in the outbox,
  /// to be sent as soon as the device can be. [outcome] says which, and what to tell the person
  /// if it did not.
  Future<bool> send(String text) async {
    if (_sending || text.trim().isEmpty) return false;
    final link = linkIn(text);
    final what = link != null ? 'link' : 'text';
    _sending = true;
    _outcome = null;
    notifyListeners();
    try {
      final result = link != null
          ? await _session.sendLink(device.id, link)
          : await _session.sendText(device.id, text);
      _outcome = switch (result) {
        SendResult.delivered => Outcome(
          'Sent the $what to ${device.name}.',
          ok: true,
        ),
        SendResult.waiting => Outcome(
          '${device.name} cannot be reached now, so the $what waits here for up '
          'to a day. Baton sends it as soon as ${device.name} can be reached.',
          ok: true,
          waiting: true,
        ),
      };
      return true;
    } on BatonError catch (e) {
      _outcome = Outcome(sentence(e.message), ok: false);
      return false;
    } finally {
      _sending = false;
      if (!_disposed) notifyListeners();
    }
  }

  /// Asks the device if it is there, and how the traffic gets to it.
  Future<void> check() async {
    if (_checking) return;
    _checking = true;
    _outcome = null;
    notifyListeners();
    try {
      final reach = await _session.ping(device.id);
      _outcome = Outcome(describeReach(device.name, reach), ok: true);
    } on BatonError catch (e) {
      _outcome = Outcome(sentence(e.message), ok: false);
    } finally {
      _checking = false;
      if (!_disposed) notifyListeners();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
