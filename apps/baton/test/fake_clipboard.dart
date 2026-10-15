// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// The clipboard in a widget test. [holds] is what a paste finds on it, and [copied] is what the
/// app last put on it. It is taken away again when the test ends.
class FakeClipboard {
  FakeClipboard(WidgetTester tester, {this.holds}) {
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        switch (call.method) {
          case 'Clipboard.getData':
            final text = holds;
            return text == null ? null : <String, Object?>{'text': text};
          case 'Clipboard.setData':
            copied = (call.arguments as Map)['text'] as String?;
        }
        return null;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
  }

  /// What a paste finds, or `null` for a clipboard with nothing on it.
  String? holds;

  /// What was last copied to it, or `null` if nothing was.
  String? copied;
}
