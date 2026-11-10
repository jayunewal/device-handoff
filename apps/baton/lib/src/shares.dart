// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// What was handed to Baton from outside it, to send to a paired device: a link or some text that
/// a shortcut gave to the action "Send with Baton", which a shortcut can show in the share sheet.
/// The home screen is written against this, so that tests can say what was handed over.
abstract class ShareKeeper {
  const ShareKeeper();

  /// The links and texts that were handed to Baton since the last call, oldest first. Each is
  /// given once, and is forgotten by whatever kept it. Empty when there is nothing.
  Future<List<String>> take();
}

/// Asks the iPhone or iPad app, through the code in ios/Runner/SendWithBaton.swift. Any other
/// system has nothing handed to it, and says so without asking.
///
/// What the app holds is in memory only, so a text that nobody took before the app was closed is
/// gone. What was handed over can be a password or an address with a token in it, so none of it
/// is written down here.
class PlatformShareKeeper extends ShareKeeper {
  const PlatformShareKeeper();

  /// The name of the channel and of the question must be the ones in
  /// ios/Runner/SendWithBaton.swift.
  static const channelName = 'io.github.jayunewal.baton/shares';
  static const question = 'take';

  static const _channel = MethodChannel(channelName);

  @override
  Future<List<String>> take() async {
    if (defaultTargetPlatform != TargetPlatform.iOS) return const [];
    try {
      // Asked for as any object, so that an answer of another kind is nothing and not an error of
      // its own either.
      final answer = await _channel.invokeMethod<Object>(question);
      if (answer is! List) return const [];
      return [
        for (final item in answer)
          if (item is String && item.trim().isNotEmpty) item,
      ];
    } on PlatformException catch (e) {
      debugPrint('Baton: shares: ${e.code}');
    } on MissingPluginException {
      debugPrint('Baton: shares: no code for it');
    }
    return const [];
  }
}
