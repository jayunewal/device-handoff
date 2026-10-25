// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/awake.dart';

/// A screen that remembers what it was asked.
///
/// Like [PlatformScreenKeeper], it is kept on by the first hold and turned off by the last, in
/// whatever order they come: the hold of a screen that opens and the one of the screen that asked
/// for it overlap.
class FakeScreen extends ScreenKeeper {
  /// What was asked for, in order: `keepOn` and `letTurnOff`.
  final List<String> asked = [];

  int _holds = 0;

  /// Whether the screen is being kept on now.
  bool get on => _holds > 0;

  @override
  Future<void> keepOn() async {
    asked.add('keepOn');
    _holds++;
  }

  @override
  Future<void> letTurnOff() async {
    asked.add('letTurnOff');
    if (_holds > 0) _holds--;
  }
}
