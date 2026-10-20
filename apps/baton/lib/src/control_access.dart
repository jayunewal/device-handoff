// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Whether this device may let another device control a window that it shares.
enum ControlAccess {
  /// The person has let Baton control windows of this device, so a window can be shared with
  /// control.
  allowed,

  /// The system could let Baton, and the person has not yet. It asks, or sends the person to the
  /// place where they say yes, when [ControlAccessKeeper.request] is called.
  needed,

  /// This device cannot share a window with control: its system is too old for it, or it shares
  /// nothing.
  unavailable,
}

/// The consent of the system to control windows of this device, which the person gives once. The
/// screens are written against this, so that tests can say what the system says.
abstract class ControlAccessKeeper {
  const ControlAccessKeeper();

  /// What the system says now. The person may change it at any time in the settings of the
  /// system, so it is asked again each time it matters.
  Future<ControlAccess> state();

  /// Asks the system to let Baton control windows. The system shows its own question, or the
  /// person has to say yes in its settings: either way the answer comes later, and [state] tells.
  Future<void> request();
}

/// Asks the operating system, through the plugin that shares windows
/// (macos/Classes/BatonStreamPlugin.swift). Only a Mac is asked for now; the other systems share
/// no windows, and this says that they cannot.
class PlatformControlAccess extends ControlAccessKeeper {
  const PlatformControlAccess();

  /// The channel that the plugin answers on. The name, and the ones of the questions and
  /// answers, must be those in macos/Classes/BatonStreamPlugin.swift.
  static const channelName = 'io.github.jayunewal.baton/stream';
  static const stateQuestion = 'controlAccess';
  static const requestQuestion = 'requestControlAccess';

  static const _channel = MethodChannel(channelName);

  @override
  Future<ControlAccess> state() async {
    return switch (await _call(stateQuestion)) {
      'allowed' => ControlAccess.allowed,
      'needed' => ControlAccess.needed,
      _ => ControlAccess.unavailable,
    };
  }

  @override
  Future<void> request() async {
    await _call(requestQuestion);
  }

  Future<Object?> _call(String question) async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return null;
    try {
      return await _channel.invokeMethod<Object>(question);
    } on PlatformException catch (e) {
      // Sharing a window to be looked at works without it. It is not worth stopping for.
      debugPrint('Baton: control access: $e');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: control access: $e');
    }
    return null;
  }
}
