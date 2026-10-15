// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'format.dart';

/// What happened to the last thing the person asked for, in a sentence to show them.
class Outcome {
  const Outcome(this.message, {required this.ok});

  final String message;

  /// Whether it worked.
  final bool ok;
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
  /// was sent. [outcome] says what to tell the person either way.
  Future<bool> send(String text) async {
    if (_sending || text.trim().isEmpty) return false;
    final link = linkIn(text);
    _sending = true;
    _outcome = null;
    notifyListeners();
    try {
      if (link != null) {
        await _session.sendLink(device.id, link);
      } else {
        await _session.sendText(device.id, text);
      }
      _outcome = Outcome(
        'Sent the ${link != null ? 'link' : 'text'} to ${device.name}.',
        ok: true,
      );
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
