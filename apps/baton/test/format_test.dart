// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/format.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final now = DateTime(2026, 10, 15, 12, 30);
  int ago(Duration d) => now.subtract(d).millisecondsSinceEpoch ~/ 1000;

  test('a device that was never heard from has not been seen', () {
    expect(lastSeen(null, now), 'not seen yet');
  });

  test('how long ago a device was heard from is said in the nearest unit', () {
    expect(lastSeen(ago(Duration.zero), now), 'seen just now');
    expect(lastSeen(ago(const Duration(seconds: 59)), now), 'seen just now');
    expect(lastSeen(ago(const Duration(minutes: 1)), now), 'seen 1 minute ago');
    expect(
      lastSeen(ago(const Duration(minutes: 59, seconds: 59)), now),
      'seen 59 minutes ago',
    );
    expect(lastSeen(ago(const Duration(hours: 1)), now), 'seen 1 hour ago');
    expect(lastSeen(ago(const Duration(hours: 23)), now), 'seen 23 hours ago');
    expect(lastSeen(ago(const Duration(days: 1)), now), 'seen 1 day ago');
    expect(lastSeen(ago(const Duration(days: 6)), now), 'seen 6 days ago');
  });

  test('a device heard from a week ago or more is dated', () {
    expect(lastSeen(ago(const Duration(days: 7)), now), 'seen on Oct 8');
    expect(
      lastSeen(ago(const Duration(days: 300)), now),
      'seen on Dec 19, 2025',
    );
  });

  test(
    'a clock that is a little behind does not make a device from the future',
    () {
      expect(
        lastSeen(
          now.add(const Duration(minutes: 2)).millisecondsSinceEpoch ~/ 1000,
          now,
        ),
        'seen just now',
      );
    },
  );

  test('every system has a name and an icon', () {
    for (final platform in Platform.values) {
      expect(platformName(platform), isNotEmpty);
      expect(platformIcon(platform).codePoint, isNonZero);
    }
  });
}
