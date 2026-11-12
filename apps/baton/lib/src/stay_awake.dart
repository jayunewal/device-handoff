// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Where "stay awake while plugged in" stands on this Mac.
///
/// A Mac that sleeps cannot be reached by the other devices of the person, and nothing wakes it
/// from outside its own network. So the person may ask Baton to keep the Mac from sleeping for as
/// long as it is plugged in. The screen is not held: it turns off as the person set it, and a
/// laptop with its lid shut can still sleep, whatever any app asks.
enum StayAwake {
  /// This is not a Mac, or the Mac app does not know the question. There is nothing to ask for.
  unavailable,

  /// The person did not ask for it. The Mac sleeps as System Settings says.
  off,

  /// The person asked, and the Mac is plugged in: Baton keeps it from going to sleep.
  holding,

  /// The person asked, and the Mac runs on its battery, so it can sleep. Baton holds it awake again
  /// as soon as it is plugged in.
  waiting,

  /// The person asked, and Baton cannot do it now: the system would not give the hold. It tries
  /// again when the power changes, and when the person asks again.
  refused,
}

/// Says whether a Mac stays awake while it is plugged in, and changes it. The screens are written
/// against this, so that tests can say what the Mac says.
abstract class StayAwakeKeeper {
  const StayAwakeKeeper();

  /// Where staying awake stands now. The power of the Mac changes under it, so it is asked again
  /// each time it matters.
  Future<StayAwake> state();

  /// Asks for staying awake, or lets the Mac sleep as it was set, and says where that then stands.
  /// The choice is kept by the Mac app, which holds the Mac from the start of the app, before any
  /// screen is up.
  Future<StayAwake> set(bool on);
}

/// Asks the operating system, through the code of the Mac app in
/// rust_builder/macos/Classes/BatonStreamPlugin.swift and BatonStayAwakeMac.swift. Only a Mac is
/// asked; every other system says that it cannot, and there is nothing to hold there.
class PlatformStayAwakeKeeper extends StayAwakeKeeper {
  const PlatformStayAwakeKeeper();

  /// The channel that the Mac app answers on. The name, the questions, and the words of the
  /// answers, must be those in BatonStreamPlugin.swift and BatonStayAwake.swift.
  static const channelName = 'io.github.jayunewal.baton/stay_awake';
  static const stateQuestion = 'state';
  static const setQuestion = 'set';

  static const _channel = MethodChannel(channelName);

  @override
  Future<StayAwake> state() async => _awake(await _ask(stateQuestion));

  @override
  Future<StayAwake> set(bool on) async {
    final answer = await _ask(setQuestion, on);
    // A Mac app that does not answer a change has changed nothing, so the person is told where
    // things stand.
    return answer == null ? state() : _awake(answer);
  }

  StayAwake _awake(Object? answer) => switch (answer) {
    'off' => StayAwake.off,
    'holding' => StayAwake.holding,
    'waiting' => StayAwake.waiting,
    'refused' => StayAwake.refused,
    _ => StayAwake.unavailable,
  };

  /// Puts [question] to the Mac app and gives its answer, or `null` where there is none: this is
  /// not a Mac, or the app has no code for the question, or it failed. Staying awake is a
  /// convenience, and not worth stopping for.
  Future<Object?> _ask(String question, [Object? argument]) async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return null;
    try {
      return await _channel.invokeMethod<Object>(question, argument);
    } on PlatformException catch (e) {
      debugPrint('Baton: stay awake: $question: $e');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: stay awake: $question: $e');
    }
    return null;
  }
}
