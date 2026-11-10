// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/shares.dart';

/// Says what a test wants to have been handed to Baton, and notes how often it was asked.
class FakeShares extends ShareKeeper {
  FakeShares([Iterable<String> handed = const []]) : _handed = [...handed];

  final List<String> _handed;
  final StreamController<void> _told = StreamController<void>.broadcast();

  /// How many times `take` was called.
  int asked = 0;

  /// Something that a shortcut hands over from now on. The app is not told, as when it comes back
  /// to the screen and asks.
  void hand(String text) => _handed.add(text);

  /// Something that a shortcut hands over from now on, and the app is told so, as when the action
  /// runs while the app is already in front and nothing else in it changes.
  void handAndTell(String text) {
    hand(text);
    _told.add(null);
  }

  /// Whether anything listens for being told.
  bool get listening => _told.hasListener;

  /// Makes `take` wait until [release], as the app does while the system answers.
  void hold() => _answered = Completer<void>();

  /// Lets the held `take` answer.
  void release() => _answered?.complete();

  Completer<void>? _answered;

  @override
  Stream<void> get handed => _told.stream;

  @override
  Future<List<String>> take() async {
    asked++;
    await _answered?.future;
    final taken = [..._handed];
    _handed.clear();
    return taken;
  }
}
