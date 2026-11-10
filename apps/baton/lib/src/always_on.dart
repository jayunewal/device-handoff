// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Whether this Mac opens Baton when its person signs in. A Baton that stays in the menu bar, and
/// is opened at sign-in, is one that the person's other devices can reach without anyone opening
/// it first.
enum SignInOpening {
  /// Baton opens when the person signs in.
  on,

  /// It does not. The person can turn it on.
  off,

  /// The system waits for the person: they turned it off in System Settings, under General and
  /// Login Items, or have yet to say yes there. Only they can change that, and
  /// [AlwaysOnKeeper.openSignInSettings] takes them there.
  needsApproval,

  /// This device cannot be asked to: it is not a Mac, or the Mac is older than macOS 13.
  unavailable,
}

/// The system refused to turn opening at sign-in on or off. [message] is what the system said, for
/// the person to read.
class AlwaysOnFailure implements Exception {
  const AlwaysOnFailure(this.message);

  final String message;

  @override
  String toString() => 'AlwaysOnFailure: $message';
}

/// What keeps Baton running on a Mac: the item in the menu bar, the window that hides when it is
/// closed, and opening at sign-in. The screens are written against this, so that tests can say what
/// the system says.
abstract class AlwaysOnKeeper {
  const AlwaysOnKeeper();

  /// Whether Baton opens at sign-in. The person may change it at any time in System Settings, so it
  /// is asked again each time it matters.
  Future<SignInOpening> signIn();

  /// Turns opening at sign-in on or off and says where it then stands, which is not the answer that
  /// was asked for where the person has to say yes in System Settings. Throws an
  /// [AlwaysOnFailure] when the system refuses.
  Future<SignInOpening> setSignIn(bool on);

  /// Opens the pane of System Settings where the person allows Baton to open at sign-in. Says
  /// whether it opened.
  Future<bool> openSignInSettings();

  /// Says that the first screen of the running app is up. A Baton that a login item opened keeps
  /// its window out of sight until then, and hides it after.
  Future<void> ready();

  /// Whether the person can see Baton now: it is the app in front, and its window is on the screen
  /// and not covered. What came for the person is told by a notification of the system to one who
  /// cannot see Baton, because the window that would show it is hidden, behind another app or
  /// gone. Where there is no window to hide, which is anywhere but a Mac, the person is taken to be
  /// looking.
  Future<bool> looking();
}

/// Asks the operating system, through the code of the Mac app in macos/Runner/AppDelegate.swift
/// and macos/Runner/MainFlutterWindow.swift. Only a Mac is asked; every other system says that it
/// cannot, and there is nothing to keep running there.
class PlatformAlwaysOnKeeper extends AlwaysOnKeeper {
  const PlatformAlwaysOnKeeper();

  /// The channel that the Mac app answers on. The name, the questions, and the words of the
  /// answers, must be those in macos/Runner/MainFlutterWindow.swift.
  static const channelName = 'io.github.jayunewal.baton/always_on';
  static const signInQuestion = 'signIn';
  static const setSignInQuestion = 'setSignIn';
  static const openSignInSettingsQuestion = 'openSignInSettings';
  static const readyQuestion = 'ready';
  static const lookingQuestion = 'looking';

  static const _channel = MethodChannel(channelName);

  @override
  Future<SignInOpening> signIn() async => _opening(await _ask(signInQuestion));

  @override
  Future<SignInOpening> setSignIn(bool on) async {
    if (defaultTargetPlatform != TargetPlatform.macOS) {
      return SignInOpening.unavailable;
    }
    try {
      return _opening(
        await _channel.invokeMethod<Object>(setSignInQuestion, on),
      );
    } on PlatformException catch (e) {
      throw AlwaysOnFailure(e.message ?? 'The system did not say why.');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: always on: $setSignInQuestion: $e');
      return SignInOpening.unavailable;
    }
  }

  @override
  Future<bool> openSignInSettings() async =>
      await _ask(openSignInSettingsQuestion) == true;

  @override
  Future<void> ready() async {
    await _ask(readyQuestion);
  }

  @override
  Future<bool> looking() async {
    // Only a Mac has a window that goes out of sight, and only a Mac shows these notifications.
    if (defaultTargetPlatform != TargetPlatform.macOS) return true;
    // A Mac app that does not answer is taken as one that cannot be seen: a notification that
    // was not needed is better than none that was.
    return await _ask(lookingQuestion) == true;
  }

  SignInOpening _opening(Object? answer) => switch (answer) {
    'on' => SignInOpening.on,
    'off' => SignInOpening.off,
    'needsApproval' => SignInOpening.needsApproval,
    _ => SignInOpening.unavailable,
  };

  /// Puts [question] to the Mac app and gives its answer, or `null` where there is none: this is
  /// not a Mac, or the app has no code for the question, or it failed. Opening at sign-in is a
  /// convenience, and not worth stopping for.
  Future<Object?> _ask(String question) async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return null;
    try {
      return await _channel.invokeMethod<Object>(question);
    } on PlatformException catch (e) {
      debugPrint('Baton: always on: $question: $e');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: always on: $question: $e');
    }
    return null;
  }
}
