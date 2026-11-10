// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/backend.dart';
import 'package:baton/src/continue_reader.dart';

/// Says what a test wants the Mac to have open, and what the system says about the browsers, and
/// notes what it was asked.
class FakeContinueReader extends ContinueReader {
  FakeContinueReader({this.items = const ContinueItems()});

  /// What `read` gives. `null` is a read that could not be done.
  ContinueItems? items;

  /// What `read` throws, if it should fail.
  Object? readFailure;

  /// How many times `read` was called.
  int reads = 0;

  /// Makes `read` wait until [releaseRead], as it does while a browser answers.
  void holdRead() => _readReady = Completer<void>();

  /// Lets the held `read` answer.
  void releaseRead() => _readReady?.complete();

  Completer<void>? _readReady;

  /// Where each browser stands. A browser that is not here is not on this Mac: `unavailable`.
  final Map<Browser, BrowserAccess> access = {};

  /// What the person says when the system asks them about a browser that it has not asked about:
  /// a yes, unless a test says otherwise. It is asked only of a browser that is open.
  final Map<Browser, BrowserAccess> answers = {};

  /// What `browserAccess` was asked, in order.
  final List<({Browser browser, bool ask})> accessAsked = [];

  @override
  Future<ContinueItems?> read() async {
    reads++;
    await _readReady?.future;
    final failure = readFailure;
    if (failure != null) throw failure;
    return items;
  }

  @override
  Future<BrowserAccess> browserAccess(
    Browser browser, {
    bool ask = false,
  }) async {
    accessAsked.add((browser: browser, ask: ask));
    if (ask && access[browser] == BrowserAccess.needed) {
      access[browser] = answers[browser] ?? BrowserAccess.allowed;
    }
    return access[browser] ?? BrowserAccess.unavailable;
  }
}
