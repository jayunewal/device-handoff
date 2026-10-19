// SPDX-License-Identifier: Apache-2.0

import 'dart:io';

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

  /// Lets go of what was chosen, once the sending of it is over, however it went. Never throws.
  ///
  /// The picker of an iPhone or an iPad copies each file it hands over into the temporary folder
  /// of the app, and the copy stays there until the system clears it. A very large file would take
  /// its room twice for as long, so the copies are deleted here. Nothing else is: a path that is
  /// not a file in that folder, such as everything the open panel of a Mac gives and every folder,
  /// belongs to the person and is left alone.
  Future<void> release(List<String> paths);
}

/// The choosing that the system does: the sheet of the Files app on an iPhone or an iPad, and the
/// open panel on a Mac. The system lets Baton read what the person chose and nothing else.
class SystemFileChooser implements FileChooser {
  /// [platform] is the system to behave as, and [temporaryFolder] is where the picker keeps its
  /// copies. Tests choose both; the app uses the system it runs on and its own temporary folder.
  const SystemFileChooser({this.platform, this.temporaryFolder});

  final TargetPlatform? platform;
  final String? temporaryFolder;

  @override
  bool get canChooseFolders =>
      (platform ?? defaultTargetPlatform) == TargetPlatform.macOS;

  @override
  Future<List<String>> files() async => [
    for (final file in await selector.openFiles()) file.path,
  ];

  @override
  Future<String?> folder() => selector.getDirectoryPath();

  @override
  Future<void> release(List<String> paths) async {
    if ((platform ?? defaultTargetPlatform) != TargetPlatform.iOS) return;
    final String home;
    try {
      home = Directory(temporaryFolder ?? Directory.systemTemp.path)
          .resolveSymbolicLinksSync();
    } on FileSystemException {
      return;
    }
    for (final path in paths) {
      try {
        if (_isCopyIn(home, path)) await File(path).delete();
      } on FileSystemException {
        // It is gone already, or cannot be deleted. Nothing more can be done for a copy.
      }
    }
  }

  /// Whether [path] is a file, and not a link, in the folder [home] or one below it. [home] is
  /// given as it is on disk, with its links resolved. The folder of [path] is resolved too, so
  /// that a path which climbs out of [home], or goes through a link, is not taken for one inside.
  static bool _isCopyIn(String home, String path) {
    final type = FileSystemEntity.typeSync(path, followLinks: false);
    if (type != FileSystemEntityType.file) return false;
    final folder = File(path).parent.resolveSymbolicLinksSync();
    return folder == home || folder.startsWith('$home/');
  }
}
