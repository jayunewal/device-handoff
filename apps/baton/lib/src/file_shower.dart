// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/services.dart';

/// Takes the person to files that have arrived, so that they can open them, move them or send
/// them on. The screens are written against this, so that tests can show without leaving the app.
abstract interface class FileShower {
  /// Shows the files and folders at [paths], each a full path, as the system shows files.
  ///
  /// Returns `false` when none of them is there any more, because the person moved or deleted
  /// them, and `true` when the system was asked to show them.
  ///
  /// Throws a [StateError] when the system cannot be asked. Its message can hold a path, so it is
  /// not for a log.
  Future<bool> show(List<String> paths);
}

/// What the system does, asked through the code in macos/Runner/MainFlutterWindow.swift and in
/// ios/Runner/AppDelegate.swift.
///
/// A Mac opens Finder with the items selected, in the folder that holds them. An iPhone or an
/// iPad opens the Files app at the folder that holds them, which for a file that arrived is the
/// Baton folder, and at the folder itself when a single folder arrived. Baton shows files and
/// never opens them: what a file does when it is opened is for the person to choose.
class SystemFileShower implements FileShower {
  const SystemFileShower();

  /// The channel that the iOS app and the macOS app answer on. The name, and the one of the
  /// question, must be the ones in ios/Runner/AppDelegate.swift and in
  /// macos/Runner/MainFlutterWindow.swift.
  static const channelName = 'io.github.jayunewal.baton/files';

  /// What both apps are asked on [channelName]: to show the files at the paths that come with it.
  static const question = 'show';

  static const _channel = MethodChannel(channelName);

  @override
  Future<bool> show(List<String> paths) async {
    if (paths.isEmpty) return false;
    final bool? shown;
    try {
      shown = await _channel.invokeMethod<bool>(question, paths);
    } on PlatformException catch (e) {
      throw StateError(
        'The system would not show the files: ${e.message ?? e.code}',
      );
    } on MissingPluginException {
      throw StateError('The app cannot ask the system to show files.');
    }
    if (shown == null) {
      throw StateError('The system did not say whether it showed the files.');
    }
    return shown;
  }
}
