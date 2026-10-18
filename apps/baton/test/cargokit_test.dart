// SPDX-License-Identifier: Apache-2.0

// The iOS and macOS builds cannot run on the computers that most tests run on, so this checks
// the changes to the copy of Cargokit that they depend on, which a new copy would lose without
// anything failing until someone built the app for an iPhone or a Mac.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

String _read(String file) =>
    File('rust_builder/cargokit/build_tool/lib/src/$file').readAsStringSync();

void main() {
  // The package is called baton-ffi, and Cargo makes libbaton_ffi.a of it.
  test('Cargokit looks for the library under the name that Cargo gives it', () {
    expect(
      _read('artifacts_provider.dart'),
      contains("libraryName = libraryName.replaceAll('-', '_');"),
    );
  });

  test('Cargokit leaves the library under the name that the pod links', () {
    expect(
      _read('build_pod.dart'),
      contains("environment.crateInfo.packageName.replaceAll('-', '_');"),
    );
  });

  // CI keeps this folder from one run to the next, which saves building every dependency again.
  test('Cargokit builds in the build folder of the app', () {
    final script = File('rust_builder/cargokit/build_pod.sh');
    expect(
      script.readAsStringSync(),
      contains(r'/${FLUTTER_BUILD_DIR:-build}/cargokit"'),
    );
  });
}
