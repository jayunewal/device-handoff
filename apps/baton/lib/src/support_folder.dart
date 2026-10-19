// SPDX-License-Identifier: Apache-2.0

// Where the system keeps an app's own files, which is where Baton makes its folder, and where the
// files that other devices send are put.

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
///
/// The folder for the files that other devices send is the person's to see, so it is asked of the
/// system on a Mac as well, by the code in macos/Runner/MainFlutterWindow.swift: the folder of a
/// sandbox is not one they would look in.
class SupportFolder {
  /// [environment] is what the system told the app about its surroundings. Without it, that is
  /// the environment of the process. A test gives its own, so that what the app does with a
  /// `HOME` of any kind, or with none, does not depend on the computer that runs the test.
  const SupportFolder({this._environment});

  /// The channel that the iOS app and the macOS app answer on. The name, and the ones of the
  /// questions, must be the ones in ios/Runner/AppDelegate.swift and in
  /// macos/Runner/MainFlutterWindow.swift.
  static const channelName = 'io.github.jayunewal.baton/folder';

  /// What the iOS app is asked on [channelName]: where it keeps its own files.
  static const question = 'applicationSupport';

  /// What both apps are asked on [channelName]: where the files that other devices send are put.
  static const receiveQuestion = 'receiveFolder';

  static const _channel = MethodChannel(channelName);

  final Map<String, String>? _environment;

  /// The path of the folder, with no slash at its end.
  ///
  /// Throws a [StateError] when the system cannot say where it is, and an [UnsupportedError]
  /// where Baton does not run yet.
  Future<String> path() async => switch (defaultTargetPlatform) {
    TargetPlatform.iOS => await _fromTheSystem(
      question,
      'where the app keeps its files',
    ),
    TargetPlatform.macOS => _underHome(),
    _ => throw UnsupportedError('Baton does not run on this system yet.'),
  };

  /// The path of the folder where the files that other devices send are put, with no slash at its
  /// end: the Baton folder that the Files app shows, on an iPhone or an iPad, and the Downloads
  /// folder of the person on a Mac. The folder exists and can be written to.
  ///
  /// Throws a [StateError] when the system cannot say where it is, and an [UnsupportedError]
  /// where Baton does not run yet.
  Future<String> receiveFolder() async => switch (defaultTargetPlatform) {
    TargetPlatform.iOS || TargetPlatform.macOS => await _fromTheSystem(
      receiveQuestion,
      'where received files go',
    ),
    _ => throw UnsupportedError('Baton does not run on this system yet.'),
  };

  Future<String> _fromTheSystem(String question, String what) async {
    final String? folder;
    try {
      folder = await _channel.invokeMethod<String>(question);
    } on PlatformException catch (e) {
      throw StateError(
        'The system would not say $what: ${e.message ?? e.code}',
      );
    } on MissingPluginException {
      throw StateError('The app cannot ask the system $what.');
    }
    if (folder == null || folder.isEmpty) {
      throw StateError('The system did not say $what.');
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
