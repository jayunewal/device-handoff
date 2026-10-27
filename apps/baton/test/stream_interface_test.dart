// SPDX-License-Identifier: Apache-2.0

// The C interface through which the platform code of the app shows a window, and shares one, is
// in the library itself, by name, where a program that loads the library finds it. The library
// has it because it uses the crate that holds it. A change that stopped using that crate would
// take the functions out of the library, and this test is where that shows first.
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
    expect(version(), 2);

    // Called with what a platform could get wrong, the others answer with a number and do
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
    final soundFailed = library
        .lookupFunction<Int32 Function(Uint64), int Function(int)>(
          'baton_stream_link_sound_failed',
        );
    expect(soundFailed(0), -4);
  });

  test('the library carries the functions for sharing a window', () {
    final library = DynamicLibrary.open(libraryPath());

    // Called with what a platform could get wrong, each answers with a number and does nothing:
    // no table to register, a request that nobody waits for, and a link that is not open (0 is
    // never one).
    final register = library
        .lookupFunction<
          Int32 Function(Pointer<Void>),
          int Function(Pointer<Void>)
        >('baton_stream_register_sharer');
    expect(register(nullptr), -1);
    final pickDone = library
        .lookupFunction<
          Int32 Function(Uint64, Pointer<Void>, Pointer<Void>),
          int Function(int, Pointer<Void>, Pointer<Void>)
        >('baton_stream_pick_done');
    expect(pickDone(0, nullptr, nullptr), -4);

    final clock = library
        .lookupFunction<
          Int32 Function(Uint64, Pointer<Uint64>),
          int Function(int, Pointer<Uint64>)
        >('baton_stream_source_clock');
    expect(clock(0, nullptr), -4);
    final begin = library
        .lookupFunction<
          Int32 Function(Uint64, Pointer<Uint32>),
          int Function(int, Pointer<Uint32>)
        >('baton_stream_source_begin');
    expect(begin(0, nullptr), -4);
    final cancel = library
        .lookupFunction<Int32 Function(Uint64), int Function(int)>(
          'baton_stream_source_cancel',
        );
    expect(cancel(0), -4);
    final recoveryPending = library
        .lookupFunction<
          Int32 Function(Uint64, Pointer<Uint32>),
          int Function(int, Pointer<Uint32>)
        >('baton_stream_source_recovery_pending');
    expect(recoveryPending(0, nullptr), -4);
    final sent = library
        .lookupFunction<
          Int32 Function(
            Uint64,
            Pointer<Void>,
            Pointer<Uint32>,
            Pointer<Uint64>,
          ),
          int Function(int, Pointer<Void>, Pointer<Uint32>, Pointer<Uint64>)
        >('baton_stream_source_sent');
    expect(sent(0, nullptr, nullptr, nullptr), -4);
    final newEpoch = library
        .lookupFunction<
          Int32 Function(Uint64, Pointer<Void>),
          int Function(int, Pointer<Void>)
        >('baton_stream_source_new_epoch');
    expect(newEpoch(0, nullptr), -4);
    final pause = library
        .lookupFunction<Int32 Function(Uint64, Uint32), int Function(int, int)>(
          'baton_stream_source_pause',
        );
    expect(pause(0, 0), -4);
    final resume = library
        .lookupFunction<Int32 Function(Uint64), int Function(int)>(
          'baton_stream_source_resume',
        );
    expect(resume(0), -4);
    final end = library
        .lookupFunction<
          Int32 Function(Uint64, Pointer<Void>),
          int Function(int, Pointer<Void>)
        >('baton_stream_source_end');
    expect(end(0, nullptr), -4);
  });
}
