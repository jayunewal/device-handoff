// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/owner_check.dart';

/// Says what a test wants the person at this device to answer when they are asked to show that it
/// is them, and notes what they were asked.
class FakeOwnerCheck extends OwnerCheck {
  FakeOwnerCheck([this.result = OwnerCheckResult.confirmed]);

  /// What `ask` gives.
  OwnerCheckResult result;

  /// The reasons that were given with each question, in order.
  final List<String> asked = [];

  /// Makes `ask` wait until [release], as it does while the person looks at the system's question.
  void hold() => _answered = Completer<void>();

  /// Lets the held `ask` answer.
  void release() => _answered?.complete();

  Completer<void>? _answered;

  @override
  Future<OwnerCheckResult> ask({required String reason}) async {
    asked.add(reason);
    await _answered?.future;
    return result;
  }
}
