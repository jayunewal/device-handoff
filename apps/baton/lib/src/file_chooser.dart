// SPDX-License-Identifier: Apache-2.0

import 'package:file_selector/file_selector.dart' as selector;
import 'package:flutter/foundation.dart';

/// Lets the person choose the files and folders to send. The screens are written against this,
/// so that tests can choose without a system dialog.
abstract interface class FileChooser {
  /// Whether [folder] works on this system: the Mac can choose a folder, and the picker of an
  /// iPhone or an iPad chooses files only.
  bool get canChooseFolders;

  /// Asks the person for files. The paths are empty if they closed the dialog without choosing.
  Future<List<String>> files();

  /// Asks the person for a folder. `null` if they closed the dialog without choosing.
  Future<String?> folder();
}

/// The choosing that the system does: the sheet of the Files app on an iPhone or an iPad, and the
/// open panel on a Mac. The system lets Baton read what the person chose and nothing else.
class SystemFileChooser implements FileChooser {
  const SystemFileChooser();

  @override
  bool get canChooseFolders => defaultTargetPlatform == TargetPlatform.macOS;

  @override
  Future<List<String>> files() async => [
    for (final file in await selector.openFiles()) file.path,
  ];

  @override
  Future<String?> folder() => selector.getDirectoryPath();
}
