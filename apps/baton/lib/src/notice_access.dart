// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Whether Baton may show notifications on this device, which it does to tell the person that a
/// paired device unlocked this one, or tried to (docs/adr/0013-unlock-from-a-phone.md).
enum NoticeAccess {
  /// The person allowed notifications from Baton.
  allowed,

  /// The system has not asked yet. [NoticeKeeper.request] asks.
  needed,

  /// The person said no. Only the settings of the system change that, and the system asks nothing
  /// again.
  denied,

  /// This device shows no notifications for Baton: it is not a Mac, or the system cannot.
  unavailable,
}

/// The notifications of the system, which the person allows once. The screens are written against
/// this, so that tests can say what the system says and see what was shown.
abstract class NoticeKeeper {
  const NoticeKeeper();

  /// What the system says now. The person may change it at any time in the settings of the
  /// system, so it is asked again each time it matters.
  Future<NoticeAccess> state();

  /// Asks the system to let Baton show notifications. The system shows its own question, once;
  /// the answer comes back. A person who said no is not asked again.
  Future<NoticeAccess> request();

  /// Shows a notification with [title] and [body]. Returns whether the system took it, which is not
  /// so where the person said no.
  Future<bool> post({required String title, required String body});
}

/// Asks the operating system, through the code of the Mac app in
/// macos/Runner/MainFlutterWindow.swift. Only a Mac shows the notifications for now, because only
/// a Mac is unlocked; the other systems say that they cannot.
class PlatformNoticeKeeper extends NoticeKeeper {
  const PlatformNoticeKeeper();

  /// The channel that the Mac app answers on. The name, the questions, and the words of the
  /// answers, must be those in macos/Runner/MainFlutterWindow.swift.
  static const channelName = 'io.github.jayunewal.baton/notices';
  static const stateQuestion = 'state';
  static const requestQuestion = 'request';
  static const postQuestion = 'post';

  static const _channel = MethodChannel(channelName);

  @override
  Future<NoticeAccess> state() async => _access(await _ask(stateQuestion));

  @override
  Future<NoticeAccess> request() async => _access(await _ask(requestQuestion));

  @override
  Future<bool> post({required String title, required String body}) async {
    final taken = await _ask(postQuestion, {'title': title, 'body': body});
    return taken == true;
  }

  NoticeAccess _access(Object? answer) => switch (answer) {
    'allowed' => NoticeAccess.allowed,
    'needed' => NoticeAccess.needed,
    'denied' => NoticeAccess.denied,
    _ => NoticeAccess.unavailable,
  };

  /// Puts [question] to the Mac app and gives its answer, or `null` where there is none: this is
  /// not a Mac, or the app has no code for the question, or it failed. A notification that cannot
  /// be shown is not worth stopping for: the history of the unlock has the attempt all the same.
  Future<Object?> _ask(
    String question, [
    Map<String, String>? arguments,
  ]) async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return null;
    try {
      return await _channel.invokeMethod<Object>(question, arguments);
    } on PlatformException catch (e) {
      debugPrint('Baton: notices: $question: $e');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: notices: $question: $e');
    }
    return null;
  }
}
