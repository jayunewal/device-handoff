// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/stay_awake.dart';

/// Says what a test wants the Mac to say about staying awake while plugged in, and notes what it
/// was asked to do.
class FakeStayAwake extends StayAwakeKeeper {
  FakeStayAwake([this.stay = StayAwake.off]);

  /// What `state` gives. `set` changes it, as the Mac does.
  StayAwake stay;

  /// What the Mac says to a request to stay awake, when it can be asked at all: `holding` where it
  /// is plugged in, `waiting` where it runs on its battery, or `refused` where the system would not.
  StayAwake onAnswer = StayAwake.holding;

  /// What `set` was given, in order.
  final List<bool> sets = [];

  /// How many times the app asked where it stands.
  int asks = 0;

  /// When set, `set` answers only when this does, so that a test can look at the screen while a
  /// change is on its way.
  Completer<void>? hold;

  @override
  Future<StayAwake> state() async {
    asks++;
    return stay;
  }

  @override
  Future<StayAwake> set(bool on) async {
    sets.add(on);
    await hold?.future;
    if (stay != StayAwake.unavailable) {
      stay = on ? onAnswer : StayAwake.off;
    }
    return stay;
  }
}
