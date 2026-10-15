// SPDX-License-Identifier: Apache-2.0

// The iOS and macOS builds cannot run on the computers that most tests run on, so this checks
// the one change to the copy of Cargokit that they depend on, which a new copy would lose
// without anything failing until someone built the app for an iPhone or a Mac.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test('Cargokit looks for the library under the name that Cargo gives it', () {
    final source = File(
      'rust_builder/cargokit/build_tool/lib/src/artifacts_provider.dart',
    ).readAsStringSync();
    expect(
      source,
      contains("libraryName = libraryName.replaceAll('-', '_');"),
      reason: 'the package is called baton-ffi, and Cargo makes libbaton_ffi.a of it',
    );
  });
}
