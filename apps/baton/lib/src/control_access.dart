// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Whether this device may let another device control a window or a screen that it shares.
enum ControlAccess {
  /// The person has let Baton control this device, so a window or a screen can be shared with
  /// control.
  allowed,

  /// The system could let Baton, and the person has not yet. It asks, or sends the person to the
  /// place where they say yes, when [ControlAccessKeeper.request] is called.
  needed,

  /// This device cannot share a window or a screen with control: its system is too old for it, or
  /// it shares nothing.
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

  @override
  Future<ControlAccess> state() async {
    return switch (await _ask(stateQuestion)) {
      'allowed' => ControlAccess.allowed,
      'needed' => ControlAccess.needed,
      _ => ControlAccess.unavailable,
    };
  }

  @override
  Future<void> request() async {
    await _ask(requestQuestion);
  }
}

/// Whether this device lets Baton record its screen with no picker of the system, which it must
/// do to give its screen to a device that asks for it.
enum ScreenAccess {
  /// The person has let Baton record the screen, so a device that asks for it is given it.
  allowed,

  /// The system could let Baton, and the person has not yet. It asks, or sends the person to the
  /// place where they say yes, when [ScreenAccessKeeper.request] is called. The system takes the
  /// answer into effect only when Baton is opened again.
  needed,

  /// This device gives no screen to a device that asks for it: it shares nothing, or its system
  /// has no way to ask.
  unavailable,
}

/// The consent of the system to record the screen of this device, which the person gives once.
/// The screens are written against this, so that tests can say what the system says.
abstract class ScreenAccessKeeper {
  const ScreenAccessKeeper();

  /// What the system says now. The person may change it at any time in the settings of the
  /// system, so it is asked again each time it matters.
  Future<ScreenAccess> state();

  /// Asks the system to let Baton record the screen. The system shows its own question, or the
  /// person has to say yes in its settings: either way the answer comes later, and [state] tells.
  Future<void> request();
}

/// Asks the operating system, through the same plugin as [PlatformControlAccess]. Only a Mac is
/// asked; the other systems give no screen to a device that asks, and this says that they cannot.
class PlatformScreenAccess extends ScreenAccessKeeper {
  const PlatformScreenAccess();

  /// The questions and the answers must be those in macos/Classes/BatonStreamPlugin.swift.
  static const stateQuestion = 'screenAccess';
  static const requestQuestion = 'requestScreenAccess';

  @override
  Future<ScreenAccess> state() async {
    return switch (await _ask(stateQuestion)) {
      'allowed' => ScreenAccess.allowed,
      'needed' => ScreenAccess.needed,
      _ => ScreenAccess.unavailable,
    };
  }

  @override
  Future<void> request() async {
    await _ask(requestQuestion);
  }
}

const _channel = MethodChannel(PlatformControlAccess.channelName);

/// Puts [question] to the plugin and gives its answer, or `null` where there is none: this is not
/// a Mac, or the plugin has no code for the question, or it failed.
Future<Object?> _ask(String question) async {
  if (defaultTargetPlatform != TargetPlatform.macOS) return null;
  try {
    return await _channel.invokeMethod<Object>(question);
  } on PlatformException catch (e) {
    // Neither consent is needed to share a window to be looked at, and a screen that is asked for
    // without the consent is refused with a sentence of its own. It is not worth stopping for.
    debugPrint('Baton: $question: $e');
  } on MissingPluginException catch (e) {
    debugPrint('Baton: $question: $e');
  }
  return null;
}
