// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

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

  /// Says, each time, that something was handed to Baton while the app is running. The action can
  /// run while the app is already in front, which changes nothing that the app would see, so the
  /// app has to be told. It says only that there is something to [take], and nothing about what.
  Stream<void> get handed => const Stream<void>.empty();
}

/// Asks the iPhone or iPad app, through the code in ios/Runner/SendWithBaton.swift. Any other
/// system has nothing handed to it, and says so without asking.
///
/// What the app holds is in memory only, so a text that nobody took before the app was closed is
/// gone. What was handed over can be a password or an address with a token in it, so none of it
/// is written down here.
class PlatformShareKeeper extends ShareKeeper {
  const PlatformShareKeeper();

  /// The name of the channel, of the question and of the news must be the ones in
  /// ios/Runner/SendWithBaton.swift. Baton asks the question, and the iPhone app says the news.
  static const channelName = 'io.github.jayunewal.baton/shares';
  static const question = 'take';
  static const news = 'handed';

  static const _channel = MethodChannel(channelName);

  // The channel has one handler, so there is one stream for the app, and the channel is listened to
  // only while something listens to the stream.
  static final StreamController<void> _handed =
      StreamController<void>.broadcast(
        onListen: () => _channel.setMethodCallHandler(_told),
        onCancel: () => _channel.setMethodCallHandler(null),
      );

  static Future<Object?> _told(MethodCall call) async {
    // What is not the news is not something that the app knows, which is what the app is told.
    if (call.method != news) throw MissingPluginException();
    _handed.add(null);
    return null;
  }

  /// Nothing is handed to any other system, and it is not listened for.
  @override
  Stream<void> get handed => defaultTargetPlatform == TargetPlatform.iOS
      ? _handed.stream
      : const Stream<void>.empty();

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
