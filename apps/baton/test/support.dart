// SPDX-License-Identifier: Apache-2.0

import 'dart:async';
import 'dart:io';

import 'package:baton/src/rust_backend.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qr/qr.dart';
import 'package:zxing2/qrcode.dart';

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

/// What a decoder reads from [code], drawn [scale] pixels to a square with the blank border
/// that cameras need: what the camera of another device would see.
///
/// Use it only on text that does not change from one run to the next. The decoder here, a Dart
/// port of ZXing, cannot read about one valid code in a hundred, whatever the size it is drawn
/// at: the C++ original, tried on 400 random tickets, read all of them. A test that read the
/// ticket of a live Baton would fail now and then for no reason that is the app's.
String readQr(QrImage code, {int scale = 4}) {
  const border = 4;
  final side = (code.moduleCount + 2 * border) * scale;
  final pixels = Int32List(side * side)..fillRange(0, side * side, 0xFFFFFFFF);
  for (var row = 0; row < code.moduleCount; row++) {
    for (var column = 0; column < code.moduleCount; column++) {
      if (!code.isDark(row, column)) continue;
      for (var y = 0; y < scale; y++) {
        for (var x = 0; x < scale; x++) {
          final at = (row + border) * scale + y;
          pixels[at * side + (column + border) * scale + x] = 0xFF000000;
        }
      }
    }
  }
  return readPicture(side, side, pixels);
}

/// Reads the QR code in a picture of [width] by [height] pixels, one `0xAARRGGBB` number each.
String readPicture(int width, int height, Int32List pixels) {
  final source = RGBLuminanceSource(width, height, pixels);
  return QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source))).text;
}
