// SPDX-License-Identifier: Apache-2.0

import 'dart:async';
import 'dart:io';

import 'package:baton/src/rust_backend.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';

/// Where the library built for this computer is: the path in the BATON_FFI_LIB environment
/// variable, else the one `cargo build -p baton-ffi` makes.
String libraryPath() {
  final given = Platform.environment['BATON_FFI_LIB'];
  if (given != null && given.isNotEmpty) {
    return given;
  }
  final file = Platform.isMacOS
      ? 'libbaton_ffi.dylib'
      : Platform.isWindows
      ? 'baton_ffi.dll'
      : 'libbaton_ffi.so';
  return '../../target/debug/$file';
}

/// A folder for one device, with settings that keep it to this computer: no relay, no
/// discovery and no port mapping. It is deleted when the test ends.
Future<Directory> deviceFolder() async {
  final folder = await Directory.systemTemp.createTemp('baton-test-');
  addTearDown(() => folder.delete(recursive: true));
  await File('${folder.path}/config.toml').writeAsString(
    '[network]\nrelay = "disabled"\nlan_discovery = false\nport_mapping = false\n',
  );
  return folder;
}

/// The app's way of reaching the Rust library, for the library built for this computer.
RustBackend backendIn(Directory folder) => RustBackend(
  dataDir: folder.path,
  library: () => ExternalLibrary.open(libraryPath()),
);

/// Waits until [done] is true, checking each time [notifier] says that something changed.
Future<void> until(
  ChangeNotifier notifier,
  bool Function() done, {
  Duration timeout = const Duration(seconds: 30),
  String Function()? describe,
}) async {
  if (done()) return;
  final reached = Completer<void>();
  void check() {
    if (done() && !reached.isCompleted) reached.complete();
  }

  notifier.addListener(check);
  try {
    await reached.future.timeout(
      timeout,
      onTimeout: () => throw TimeoutException(
        'Waited ${timeout.inSeconds} seconds. ${describe?.call() ?? ''}',
      ),
    );
  } finally {
    notifier.removeListener(check);
  }
}
