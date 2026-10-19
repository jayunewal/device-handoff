// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/file_shower.dart';

/// A system that shows files by remembering what it was asked to show.
class FakeShower implements FileShower {
  /// What it was asked to show, in order.
  final List<List<String>> asked = [];

  /// Whether what it is asked to show is there. When it is not, it says so.
  bool there = true;

  /// What it fails with, instead of answering, when the system cannot be asked.
  StateError? failure;

  @override
  Future<bool> show(List<String> paths) async {
    asked.add(paths);
    final failure = this.failure;
    if (failure != null) throw failure;
    return there;
  }
}
