// SPDX-License-Identifier: Apache-2.0

// The C interface through which the platform code of the app shows a window is in the library
// itself, by name, where a program that loads the library finds it. The library has it because
// it uses the crate that holds it. A change that stopped using that crate would take the
// functions out of the library, and this test is where that shows first.
//
// The library is the one that bridge_test.dart loads.

import 'dart:ffi';

import 'package:flutter_test/flutter_test.dart';

import 'support.dart';

void main() {
  test('the library carries the C interface of the platform code', () {
    final library = DynamicLibrary.open(libraryPath());

    final version = library.lookupFunction<Uint32 Function(), int Function()>(
      'baton_stream_abi_version',
    );
    expect(version(), 1);

    // Called with what a platform could get wrong, the two others answer with a number and do
    // nothing: no table to register, and a link that is not open.
    final register = library
        .lookupFunction<
          Int32 Function(Pointer<Void>),
          int Function(Pointer<Void>)
        >('baton_stream_register_viewer');
    expect(register(nullptr), -1);
    final failed = library
        .lookupFunction<Int32 Function(Uint64), int Function(int)>(
          'baton_stream_link_decode_failed',
        );
    expect(failed(0), -4);
  });
}
