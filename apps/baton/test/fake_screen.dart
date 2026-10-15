// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/awake.dart';

/// A screen that remembers what it was asked.
class FakeScreen extends ScreenKeeper {
  /// What was asked for, in order: `keepOn` and `letTurnOff`.
  final List<String> asked = [];

  /// Whether the screen is being kept on now.
  bool get on => asked.isNotEmpty && asked.last == 'keepOn';

  @override
  Future<void> keepOn() async => asked.add('keepOn');

  @override
  Future<void> letTurnOff() async => asked.add('letTurnOff');
}
