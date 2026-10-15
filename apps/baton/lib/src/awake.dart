// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Keeps the screen of this device on.
///
/// A phone or a tablet turns its screen off, and locks, after a while without a touch, and Baton
/// leaves that as the person set it. The one time it does not is while two devices pair: the
/// person is looking at the other device, and Baton on a phone works only while it is on the
/// screen, so a screen that went off in the middle of pairing would take the pairing with it.
/// Screens ask through an [AwakeHold], which keeps it to that.
abstract class ScreenKeeper {
  const ScreenKeeper();

  /// Keeps the screen on.
  Future<void> keepOn();

  /// Lets the screen turn off after a while, as the person set it.
  Future<void> letTurnOff();
}

/// Asks the operating system. Only iPhones and iPads are asked for now, by the code in
/// ios/Runner/AppDelegate.swift. On a computer the screen is the person's to manage, and this
/// does nothing.
class PlatformScreenKeeper extends ScreenKeeper {
  const PlatformScreenKeeper();

  /// The channel that the iOS app answers on. The name must be the one in
  /// ios/Runner/AppDelegate.swift.
  static const channelName = 'io.github.jayunewal.baton/screen';

  static const _channel = MethodChannel(channelName);

  /// How many holds are open. A phone has one setting for the screen, and the holds of two
  /// screens can overlap: the one that is closing lets go after the one that has just opened
  /// asked. So the setting is turned on by the first hold and off by the last, in whatever
  /// order they come. Each [AwakeHold] asks once and lets go once, so the count stays level.
  static int _holds = 0;

  /// Forgets the holds that an earlier test left open.
  @visibleForTesting
  static void forgetHolds() => _holds = 0;

  @override
  Future<void> keepOn() async {
    _holds++;
    if (_holds == 1) await _call('keepOn');
  }

  @override
  Future<void> letTurnOff() async {
    if (_holds == 0) return;
    _holds--;
    if (_holds == 0) await _call('letTurnOff');
  }

  /// Whether the system is set to keep the screen on, as the system itself says. The tests that
  /// run the app on a phone use it to see that what Baton asked for happened.
  Future<bool> isKeptOn() async => await _call('isKeptOn') == true;

  Future<Object?> _call(String method) async {
    if (defaultTargetPlatform != TargetPlatform.iOS) return null;
    try {
      return await _channel.invokeMethod<Object>(method);
    } on PlatformException catch (e) {
      // Pairing works with a screen that turns off, if the person keeps it on. It is not worth
      // stopping for.
      debugPrint('Baton: screen: $e');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: screen: $e');
    }
    return null;
  }
}

/// The longest that one [AwakeHold] keeps the screen on without a step forward. No step of a
/// pairing lasts longer by itself: a code is shown for five minutes at most, the person has under
/// three to answer a request, and reaching the other device and hearing its answer take less
/// still. A screen that has gone this long with nothing happening, such as a camera that waits for
/// a code that nobody holds up, has been forgotten, and the device goes back to locking as the
/// person set it.
const awakeLimit = Duration(minutes: 5);

/// What a screen holds for as long as it needs the screen of the device to stay on. [hold] and
/// [release] can be called as often as the screen likes: only the first of a run of either does
/// anything.
///
/// A hold does not last for ever. After [awakeLimit] without a step forward ([renew]) the screen
/// is let go, and stays let go until the screen takes a step or [release] has been called, so
/// that a screen that forgot to let go, or that the person walked away from, cannot leave a
/// phone that never locks.
class AwakeHold {
  AwakeHold(this._screen);

  final ScreenKeeper _screen;
  Timer? _limit;
  bool _holding = false;
  bool _spent = false;

  /// Keeps the screen on, unless this hold has already run out. Asking again changes nothing, the
  /// limit included: a step forward is [renew].
  void hold() {
    if (_holding || _spent) return;
    _holding = true;
    _limit = Timer(awakeLimit, _runOut);
    unawaited(_screen.keepOn());
  }

  /// A step forward in the pairing, so that someone is there. The limit starts again, and a hold
  /// that had run out, or had not begun, keeps the screen on.
  void renew() {
    _spent = false;
    if (_holding) {
      _limit?.cancel();
      _limit = Timer(awakeLimit, _runOut);
    } else {
      hold();
    }
  }

  /// Lets the screen turn off again, and makes this hold ready for another use.
  void release() {
    _spent = false;
    _limit?.cancel();
    _limit = null;
    if (!_holding) return;
    _holding = false;
    unawaited(_screen.letTurnOff());
  }

  void _runOut() {
    _limit = null;
    _spent = true;
    _holding = false;
    unawaited(_screen.letTurnOff());
  }
}
