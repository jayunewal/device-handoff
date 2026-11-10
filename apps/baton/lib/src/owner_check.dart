// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'format.dart';

/// How a check that the person at this device is the one it belongs to came out.
enum OwnerCheckResult {
  /// The person showed their face, or gave the passcode of this device.
  confirmed,

  /// This system asks nothing: only an iPhone or an iPad does for now.
  notNeeded,

  /// The person stopped the question, or the system took it back because Baton went away.
  cancelled,

  /// The face was not recognised, and the passcode was not given or was wrong.
  notRecognised,

  /// This device has no passcode, so there is nothing to check the person against.
  noLock,

  /// The system could not ask, or said something that Baton does not know. The person is not
  /// taken to be confirmed.
  failed;

  /// Whether what the check was for may go on.
  bool get goesOn => this == confirmed || this == notNeeded;
}

/// Asks the person who holds this device to show that it is them, before Baton does something on
/// their behalf that a stranger who picked the unlocked device up must not be able to do: it asks
/// another device for its screen, so that it can be looked at and controlled. The screens are
/// written against this, so that tests can say what the system says.
abstract class OwnerCheck {
  const OwnerCheck();

  /// Asks the person, with [reason] in the system's question, and gives the answer once they have
  /// given it. Each call asks afresh: a face that was recognised a moment ago does not stand for
  /// this one.
  Future<OwnerCheckResult> ask({required String reason});
}

/// Asks the system of an iPhone or an iPad, through the code in ios/Runner/AppDelegate.swift: for
/// the person's face (Face ID, or Touch ID on a device with no Face ID) and, when it is not
/// recognised, for the passcode of the device. Any other system is not asked, for now.
///
/// Whatever is not a clear yes is not one. A question that nothing in the app answers, or an
/// answer that Baton does not know, is [OwnerCheckResult.failed], and what the check was for does
/// not go on.
class PlatformOwnerCheck extends OwnerCheck {
  const PlatformOwnerCheck();

  /// The name of the channel and of the question, and the codes of the failures below, must be the
  /// ones in ios/Runner/AppDelegate.swift.
  static const channelName = 'io.github.jayunewal.baton/owner';
  static const question = 'ask';
  static const cancelledCode = 'cancelled';
  static const notRecognisedCode = 'not_recognised';
  static const noLockCode = 'no_lock';

  static const _channel = MethodChannel(channelName);

  @override
  Future<OwnerCheckResult> ask({required String reason}) async {
    if (defaultTargetPlatform != TargetPlatform.iOS) {
      return OwnerCheckResult.notNeeded;
    }
    try {
      // Asked for as any object, so that an answer of another kind is not a yes and is not an
      // error of its own either.
      final answer = await _channel.invokeMethod<Object>(question, reason);
      return answer == true
          ? OwnerCheckResult.confirmed
          : OwnerCheckResult.failed;
    } on PlatformException catch (e) {
      return switch (e.code) {
        cancelledCode => OwnerCheckResult.cancelled,
        notRecognisedCode => OwnerCheckResult.notRecognised,
        noLockCode => OwnerCheckResult.noLock,
        _ => OwnerCheckResult.failed,
      };
    } on MissingPluginException {
      return OwnerCheckResult.failed;
    }
  }
}

/// What the system's question says when the person asks for the screen of [device], which is the
/// name of that device. It is shown under the name of the app, so it says what is about to be done.
String ownerReason(String device, {required bool control}) {
  final name = visibleText(device);
  return control
      ? 'Look at the screen of $name and control it'
      : 'Look at the screen of $name';
}

/// What the person is told when a check ended without confirming them, in one sentence, or `null`
/// when there is nothing to tell: they were confirmed, nothing was asked, or they stopped the
/// question themselves. [device] is the name of the device that was to be asked for its screen.
String? describeOwnerRefusal(OwnerCheckResult result, String device) {
  final name = visibleText(device);
  return switch (result) {
    OwnerCheckResult.confirmed ||
    OwnerCheckResult.notNeeded ||
    OwnerCheckResult.cancelled => null,
    OwnerCheckResult.notRecognised =>
      'This device could not confirm that it is you, so $name was not asked '
          'for its screen.',
    OwnerCheckResult.noLock =>
      'This device has no passcode. Baton asks for your face or the passcode '
          'before it asks $name for its screen, so set a passcode in Settings '
          'and try again.',
    OwnerCheckResult.failed =>
      'This device could not check that it is you, so $name was not asked for '
          'its screen.',
  };
}
