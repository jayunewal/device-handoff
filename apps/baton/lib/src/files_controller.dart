// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'awake.dart';
import 'backend.dart';
import 'format.dart';
import 'send_controller.dart' show Outcome;

/// Sends files and folders to one paired device, and shows how far the transfer has got.
///
/// The screen of a phone stays on while a transfer is going, because Baton on a phone works only
/// while it is on the screen. [screen] does that; tests give it one of their own. The hold is
/// renewed each time the transfer takes a step, so a transfer that has stood still for
/// [awakeLimit] lets the screen go.
class FilesController extends ChangeNotifier {
  FilesController(
    this._session,
    this.device, {
    ScreenKeeper screen = const PlatformScreenKeeper(),
    this._every = const Duration(milliseconds: 250),
  }) : _awake = AwakeHold(screen);

  final Session _session;

  /// The device that the files are sent to.
  final Device device;

  final AwakeHold _awake;
  final Duration _every;
  FileTransfer? _transfer;
  Timer? _timer;
  bool _starting = false;
  bool _disposed = false;
  FileProgress? _progress;
  Outcome? _outcome;
  String _what = '';

  /// Whether files are being sent, or are about to be.
  bool get busy => _starting || _transfer != null;

  /// How far the transfer has got, while it is going. `null` when none is.
  FileProgress? get progress => _progress;

  /// What is being sent, in a few words: the name of the first thing, and how many more there are.
  String get what => _what;

  /// How much of the transfer is done, from 0 to 1, or `null` while there is nothing to measure:
  /// before the files go out, and while Baton waits to try again.
  double? get fraction {
    final progress = _progress;
    if (progress == null) return null;
    return switch (progress.stage) {
      SendStage.sending =>
        progress.bytes.toInt() == 0
            ? null
            : (progress.sent.toInt() / progress.bytes.toInt()).clamp(0.0, 1.0),
      SendStage.finishing || SendStage.over => 1.0,
      SendStage.connecting || SendStage.asking || SendStage.waiting => null,
    };
  }

  /// What happened to the last transfer, until the next one starts. `null` if the person gave it
  /// up themselves: nothing needs saying about that.
  Outcome? get outcome => _outcome;

  /// Sends the files and folders at [paths]. Returns whether every file arrived; [outcome] says
  /// what to tell the person either way.
  Future<bool> send(List<String> paths) async {
    if (busy || paths.isEmpty) return false;
    _starting = true;
    _outcome = null;
    _progress = null;
    _what = _describe(paths);
    _awake.hold();
    notifyListeners();
    final FileTransfer transfer;
    try {
      transfer = await _session.sendFiles(device.id, paths);
    } on BatonError catch (e) {
      _starting = false;
      _awake.release();
      _outcome = Outcome(sentence(e.message), ok: false);
      _changed();
      return false;
    }
    if (_disposed) {
      // The screen went away while the transfer was starting.
      transfer.cancel();
      _awake.release();
      return false;
    }
    _starting = false;
    _transfer = transfer;
    _progress = transfer.progress();
    _timer = Timer.periodic(_every, (_) => _poll(transfer));
    notifyListeners();
    try {
      final sent = await transfer.outcome();
      _outcome = Outcome(describeSent(sent, device.name), ok: sent.failed == 0);
      return sent.failed == 0;
    } on BatonError catch (e) {
      _outcome = e.kind == ErrorKind.cancelled
          ? null
          : Outcome(sentence(e.message), ok: false);
      return false;
    } finally {
      _timer?.cancel();
      _timer = null;
      _transfer = null;
      _progress = null;
      _awake.release();
      _changed();
    }
  }

  /// Gives the transfer up, and tells the other device if it can. [outcome] stays empty.
  void cancel() => _transfer?.cancel();

  void _poll(FileTransfer transfer) {
    if (_disposed || !identical(_transfer, transfer)) return;
    final now = transfer.progress();
    if (now == _progress) return;
    _progress = now;
    // A step forward: someone is there, and the screen stays on for as long as it takes.
    _awake.renew();
    notifyListeners();
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  /// "photo.jpg", or "photo.jpg and 2 more".
  static String _describe(List<String> paths) {
    final first = paths.first
        .split(RegExp(r'[/\\]'))
        .where((s) => s.isNotEmpty);
    final name = first.isEmpty ? paths.first : first.last;
    return paths.length == 1 ? name : '$name and ${paths.length - 1} more';
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    _timer = null;
    // Leaving the screen gives a transfer that is going up.
    _transfer?.cancel();
    _awake.release();
    super.dispose();
  }
}
