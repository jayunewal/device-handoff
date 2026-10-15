// SPDX-License-Identifier: Apache-2.0

import 'dart:io';

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
