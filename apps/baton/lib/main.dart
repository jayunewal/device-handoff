// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import 'src/app.dart';
import 'src/rust_backend.dart';

void main() {
  runApp(BatonApp(backend: RustBackend()));
}
