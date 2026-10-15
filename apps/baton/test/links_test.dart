// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/links.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('opens a web address with the system, whole', () async {
    final asked = <Uri>[];

    final opened = await openInBrowser(
      'https://example.com/plan?a=1&b=2#top',
      launch: (uri) async {
        asked.add(uri);
        return true;
      },
    );

    expect(opened, isTrue);
    expect(asked.map((uri) => uri.toString()), [
      'https://example.com/plan?a=1&b=2#top',
    ]);
  });

  test('says so when the system would not open it', () async {
    expect(
      await openInBrowser('http://example.com', launch: (_) async => false),
      isFalse,
    );
  });

  test('never hands the system anything but a web address', () async {
    var asked = 0;
    for (final address in [
      'tel:+15551234',
      'sms:+15551234',
      'mailto:anna@example.com',
      'file:///etc/hosts',
      'javascript:alert(1)',
      'data:text/html,<script>alert(1)</script>',
      'ftp://example.com/a',
      'baton://pair/abc',
      'https:///no-host',
      'https://',
      '//example.com/a',
      'example.com/a',
      'not a link',
      '',
    ]) {
      final opened = await openInBrowser(
        address,
        launch: (_) async {
          asked++;
          return true;
        },
      );
      expect(opened, isFalse, reason: address);
    }

    expect(asked, 0);
  });

  test(
    'a web address that cannot be read is not opened, and is no error',
    () async {
      var asked = 0;

      final opened = await openInBrowser(
        'https://[::1',
        launch: (_) async {
          asked++;
          return true;
        },
      );

      expect(opened, isFalse);
      expect(asked, 0);
    },
  );

  test(
    'a system that fails is told as "did not open", not as an error',
    () async {
      final opened = await openInBrowser(
        'https://example.com/plan',
        launch: (_) => throw PlatformException(
          code: 'ACTIVITY_NOT_FOUND',
          message: 'No Activity found to handle Intent',
        ),
      );

      expect(opened, isFalse);
    },
  );
}
