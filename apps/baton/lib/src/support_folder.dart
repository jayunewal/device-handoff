// SPDX-License-Identifier: Apache-2.0

// Where the system keeps an app's own files, which is where Baton makes its folder.

import 'dart:io' as io;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// The `Library/Application Support` folder of the app: where an app keeps the files that are not
/// the person's documents. Baton makes its folder inside it.
///
/// An iPhone or an iPad is asked, by the code in ios/Runner/AppDelegate.swift, and the
/// environment is not read there: iOS 27 starts an app without `HOME` in it, and a Baton that
/// looked for `HOME` could not start. A Mac is told by `HOME`, which for a sandboxed app is the
/// folder of its sandbox, and which is always there.
class SupportFolder {
  /// [environment] is what the system told the app about its surroundings. Without it, that is
  /// the environment of the process. A test gives its own, to see what the app does when there
  /// is no `HOME`, which a simulator and a Mac both have.
  const SupportFolder({this._environment});

  /// The channel that the iOS app answers on. The name, and the one of the question, must be the
  /// ones in ios/Runner/AppDelegate.swift.
  static const channelName = 'io.github.jayunewal.baton/folder';

  /// What the iOS app is asked on [channelName].
  static const question = 'applicationSupport';

  static const _channel = MethodChannel(channelName);

  final Map<String, String>? _environment;

  /// The path of the folder, with no slash at its end.
  ///
  /// Throws a [StateError] when the system cannot say where it is, and an [UnsupportedError]
  /// where Baton does not run yet.
  Future<String> path() async => switch (defaultTargetPlatform) {
    TargetPlatform.iOS => await _fromTheSystem(),
    TargetPlatform.macOS => _underHome(),
    _ => throw UnsupportedError('Baton does not run on this system yet.'),
  };

  Future<String> _fromTheSystem() async {
    final String? folder;
    try {
      folder = await _channel.invokeMethod<String>(question);
    } on PlatformException catch (e) {
      throw StateError(
        'The system would not say where the app keeps its files: '
        '${e.message ?? e.code}',
      );
    } on MissingPluginException {
      throw StateError(
        'The app cannot ask the system where it keeps its files.',
      );
    }
    if (folder == null || folder.isEmpty) {
      throw StateError('The system did not say where the app keeps its files.');
    }
    return folder;
  }

  String _underHome() {
    final home = (_environment ?? io.Platform.environment)['HOME'];
    if (home == null || home.isEmpty) {
      throw StateError('The home folder of the app is not known.');
    }
    return '$home/Library/Application Support';
  }
}
