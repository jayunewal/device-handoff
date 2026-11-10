// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'format.dart';

/// What the screen that continues from a device shows.
sealed class ContinueView {
  const ContinueView();
}

/// The device is being asked.
final class ContinueAsking extends ContinueView {
  const ContinueAsking();
}

/// The device gave what its person is working on: a page, text, or both.
final class ContinueFound extends ContinueView {
  const ContinueFound({this.page, this.address, this.text});

  /// The page that is in front in the browser of the device, as it was when the device was asked.
  final ContinuePage? page;

  /// What to open for [page]: `Session.webAddress` of its address, or `null` where that is not a
  /// web address, and then the page is shown and not opened. Another device chose the address, so
  /// it is this and never the address as it arrived that is handed to the system.
  final String? address;

  /// The text on the clipboard of the device. Copy this, not what is shown of it.
  final String? text;
}

/// The device gave nothing, and said so: it is locked or asleep, or has no page open and nothing
/// copied. It is not a failure.
final class ContinueNothing extends ContinueView {
  const ContinueNothing();
}

/// The device could not be asked, or did not give anything, and [sentence] says why and what to
/// do.
final class ContinueStuck extends ContinueView {
  const ContinueStuck(this.sentence, {this.mayRetry = false});

  final String sentence;

  /// Whether asking again in a moment may work, which a refusal by the person at the device does
  /// not.
  final bool mayRetry;
}

/// Asks a paired device what its person is working on, to continue it here: the page in front in
/// its browser, and the text on its clipboard. The device was allowed beforehand by the person at
/// it, so nobody there is asked, and what comes back is read when it is asked and kept nowhere,
/// not even here after the screen is closed (docs/adr/0015-continue-from-another-device.md).
class ContinueController extends ChangeNotifier {
  ContinueController(this._session, this.device);

  final Session _session;

  /// The device that is asked.
  final Device device;

  ContinueView _view = const ContinueAsking();
  int _turn = 0;
  bool _disposed = false;

  /// What to show now.
  ContinueView get view => _view;

  /// Asks the device. Asking again while it is being asked starts over: only the latest answer is
  /// shown.
  Future<void> ask() async {
    final turn = ++_turn;
    _set(const ContinueAsking());
    ContinueView view;
    try {
      view = _viewOf(await _session.continueFrom(device.id));
    } on BatonError catch (e) {
      view = _problem(e);
    }
    if (_disposed || turn != _turn) return;
    _set(view);
  }

  /// How the device is called here. It chose its own name, so it is made fit to show.
  String get _name => visibleText(device.name);

  ContinueView _viewOf(ContinueOutcome outcome) => switch (outcome) {
    ContinueOutcome_Items(:final items) => _found(items),
    ContinueOutcome_NotAllowed() => ContinueStuck(
      '$_name has not let this device continue from it. At $_name, open '
      'Baton, open the menu of this device in the list, choose “Let this '
      'device continue from this Mac” and turn it on.',
    ),
    ContinueOutcome_Busy() => ContinueStuck(
      '$_name is busy right now. Try again in a moment.',
      mayRetry: true,
    ),
    ContinueOutcome_Unavailable() => ContinueStuck(
      '$_name could not read what it has open just now. If this keeps '
      'happening, check at $_name that Baton may control Safari and Chrome, in '
      'System Settings, under Privacy & Security, Automation.',
      mayRetry: true,
    ),
    ContinueOutcome_Unsupported() => ContinueStuck(
      '$_name has a version of Baton that cannot do this yet.',
    ),
  };

  ContinueView _found(ContinueItems items) {
    final page = items.page;
    final text = items.clipboard;
    if (page == null && text == null) return const ContinueNothing();
    return ContinueFound(
      page: page,
      address: page == null ? null : _session.webAddress(page.url),
      text: text,
    );
  }

  ContinueView _problem(BatonError e) => switch (e.kind) {
    ErrorKind.notPaired => ContinueStuck(
      'This device is not paired with $_name any more.',
    ),
    ErrorKind.unreachable => ContinueStuck(
      'Baton could not reach $_name. Check that it is awake and online.',
      mayRetry: true,
    ),
    _ => ContinueStuck(sentence(e.message), mayRetry: true),
  };

  void _set(ContinueView view) {
    _view = view;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
