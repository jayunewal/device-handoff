// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/format.dart';
import 'package:flutter/foundation.dart' show TargetPlatform;
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

  test('the time left is counted down in minutes and seconds', () {
    final start = DateTime.fromMillisecondsSinceEpoch(1760529600 * 1000);
    int expiresAfter(int seconds) => 1760529600 + seconds;
    expect(timeLeft(expiresAfter(300), start), '5:00');
    expect(timeLeft(expiresAfter(299), start), '4:59');
    expect(timeLeft(expiresAfter(61), start), '1:01');
    expect(timeLeft(expiresAfter(9), start), '0:09');
    expect(timeLeft(expiresAfter(0), start), '0:00');
    expect(
      timeLeft(expiresAfter(-30), start),
      '0:00',
      reason: 'not below zero',
    );
    // A part of a second still counts as the second it is in.
    final late = start.add(const Duration(milliseconds: 400));
    expect(timeLeft(expiresAfter(300), late), '5:00');
    expect(
      timeLeft(expiresAfter(300), late.add(const Duration(milliseconds: 700))),
      '4:59',
    );
  });

  test('a message from the library is made to read as a sentence', () {
    expect(
      sentence('pairing was not accepted: the other device declined'),
      'Pairing was not accepted: the other device declined.',
    );
    expect(sentence('Already a sentence.'), 'Already a sentence.');
    expect(sentence('Is it?'), 'Is it?');
    expect(sentence('  spaces around  '), 'Spaces around.');
    expect(
      sentence('ends with a bracket (like this)'),
      'Ends with a bracket (like this).',
    );
    expect(sentence(''), '');
    expect(sentence('   '), '');
  });

  test('how long ago something came is said without the word "seen"', () {
    expect(since(ago(Duration.zero), now), 'just now');
    expect(since(ago(const Duration(minutes: 5)), now), '5 minutes ago');
    expect(since(ago(const Duration(hours: 2)), now), '2 hours ago');
    expect(since(ago(const Duration(days: 3)), now), '3 days ago');
    expect(since(ago(const Duration(days: 7)), now), 'on Oct 8');
    expect(since(ago(const Duration(days: 300)), now), 'on Dec 19, 2025');
  });

  test('what a ping found is told with the way the traffic went', () {
    String told(Route route) =>
        describeReach('Kitchen iPad', Reach(roundTripMs: 24, route: route));
    expect(
      told(Route.lan),
      'Kitchen iPad answered in 24 ms, over your local network.',
    );
    expect(
      told(Route.direct),
      'Kitchen iPad answered in 24 ms, directly over the internet.',
    );
    expect(
      told(Route.relay),
      'Kitchen iPad answered in 24 ms, through the relay, which is slower.',
    );
    expect(told(Route.unknown), 'Kitchen iPad answered in 24 ms.');
  });

  group('text from another device is made fit to show', () {
    test('ordinary text, in any script, is left as it is', () {
      const text = 'Buy milk\n\tand eggs — 牛乳 — חלב — 🥛';
      expect(visibleText(text), text);
    });

    test('line ends are the same everywhere', () {
      expect(visibleText('one\r\ntwo\rthree\nfour'), 'one\ntwo\nthree\nfour');
    });

    test('characters that turn text round are shown as a mark', () {
      // A line that reads as a harmless address, until the override is taken away.
      const fake = 'https://example.com/\u202Egpj.exe';
      expect(visibleText(fake), 'https://example.com/\uFFFDgpj.exe');
      for (final c in [0x202A, 0x202B, 0x202C, 0x202D, 0x202E]) {
        expect(visibleText('a${String.fromCharCode(c)}b'), 'a\uFFFDb');
      }
      for (final c in [0x2066, 0x2067, 0x2068, 0x2069]) {
        expect(visibleText('a${String.fromCharCode(c)}b'), 'a\uFFFDb');
      }
    });

    test('direction marks, which have no glyph, are shown as a mark too', () {
      // They move digits and punctuation around them without any override: with the right-to-left
      // marks, "Total $10.12" is laid out as "Total $1012." although what is copied is the same.
      for (final c in [0x200E, 0x200F, 0x061C]) {
        expect(visibleText('a${String.fromCharCode(c)}b'), 'a\uFFFDb');
      }
      expect(
        visibleText('Total \$10\u200F.\u200F12'),
        'Total \$10\uFFFD.\uFFFD12',
      );
    });

    test('other characters that have no glyph are shown as a mark', () {
      // The zero-width space; the word joiner and the invisible operators; the deprecated
      // format characters; the byte order mark; the annotation marks.
      final invisible = [
        0x200B,
        for (var c = 0x2060; c <= 0x2064; c++) c,
        for (var c = 0x206A; c <= 0x206F; c++) c,
        0xFEFF,
        for (var c = 0xFFF9; c <= 0xFFFB; c++) c,
      ];
      for (final c in invisible) {
        expect(
          visibleText('a${String.fromCharCode(c)}b'),
          'a\uFFFDb',
          reason: 'U+${c.toRadixString(16)}',
        );
      }
    });

    test('the joiners that emoji and several scripts need are left alone', () {
      const family = '👨\u200D👩\u200D👧';
      const persian = 'می\u200Cخواهم';
      const heart = '❤️';
      expect(visibleText(family), family);
      expect(visibleText(persian), persian);
      expect(visibleText(heart), heart);
    });

    test('control characters that cannot be seen are shown as a mark', () {
      expect(
        visibleText('a\u0000b\u0007c\u001Bd\u007Fe\u0085f'),
        'a\uFFFDb\uFFFDc\uFFFDd\uFFFDe\uFFFDf',
      );
    });
  });

  group('sizes and counts of files', () {
    test('a size is written the way the Mac and the iPhone write one', () {
      expect(fileSize(0), '0 bytes');
      expect(fileSize(1), '1 byte');
      expect(fileSize(999), '999 bytes');
      expect(fileSize(1000), '1.0 KB');
      expect(fileSize(1049), '1.0 KB');
      expect(fileSize(1060), '1.1 KB');
      expect(fileSize(9949), '9.9 KB');
      expect(fileSize(12345), '12 KB');
      // Above ten a size is rounded once, to the whole unit, and not first to a tenth.
      expect(fileSize(12460), '12 KB');
      expect(fileSize(999499), '999 KB');
      expect(fileSize(3200000), '3.2 MB');
      expect(fileSize(2000000000), '2.0 GB');
      expect(fileSize(31000000000), '31 GB');
      expect(fileSize(4200000000000), '4.2 TB');
    });

    test('a size that rounds up to the next unit is written in it', () {
      expect(fileSize(999500), '1.0 MB');
      expect(fileSize(999999), '1.0 MB');
      expect(fileSize(9960), '10 KB');
      expect(fileSize(999500000), '1.0 GB');
      // There is no unit after the terabyte.
      expect(fileSize(1000000000000000), '1000 TB');
    });

    test(
      'files, folders and items are counted in the singular and the plural',
      () {
        expect(fileCount(0), '0 files');
        expect(fileCount(1), '1 file');
        expect(fileCount(2), '2 files');
        expect(folderCount(1), '1 folder');
        expect(folderCount(3), '3 folders');
        expect(itemCount(1), '1 item');
        expect(itemCount(2), '2 items');
      },
    );

    test('what a transfer holds is said in a few words', () {
      expect(
        describeContents(files: 1, folders: 0, bytes: 2000000),
        '1 file, 2.0 MB',
      );
      expect(
        describeContents(files: 12, folders: 2, bytes: 31000000000),
        '12 files in 2 folders, 31 GB',
      );
      expect(
        describeContents(files: 3, folders: 1, bytes: 0),
        '3 files in 1 folder, 0 bytes',
      );
    });

    test('every stage of a transfer that is sent has a sentence', () {
      for (final stage in SendStage.values) {
        expect(describeStage(stage, 'Kitchen iPad'), isNotEmpty);
      }
      expect(
        describeStage(SendStage.asking, 'Kitchen iPad'),
        'Waiting for Kitchen iPad to say yes…',
      );
      expect(
        describeStage(SendStage.finishing, 'Kitchen iPad'),
        'Kitchen iPad is putting the files in place…',
      );
    });

    test(
      'how a transfer that was sent ended is said with what was left out',
      () {
        FilesSent sent({int files = 3, int failed = 0, int leftOut = 0}) =>
            FilesSent(
              files: files,
              bytes: 3200000,
              failed: failed,
              leftOut: leftOut,
            );

        expect(
          describeSent(sent(files: 1), 'Kitchen iPad'),
          'Sent 1 file (3.2 MB) to Kitchen iPad.',
        );
        expect(
          describeSent(sent(), 'Kitchen iPad'),
          'Sent 3 files (3.2 MB) to Kitchen iPad.',
        );
        expect(
          describeSent(sent(failed: 1), 'Kitchen iPad'),
          'Sent 2 of 3 files to Kitchen iPad. 1 file could not be saved there.',
        );
        expect(
          describeSent(sent(failed: 3), 'Kitchen iPad'),
          'Kitchen iPad could not keep the files.',
        );
        expect(
          describeSent(sent(files: 1, failed: 1), 'Kitchen iPad'),
          'Kitchen iPad could not keep the file.',
        );
        expect(
          describeSent(sent(leftOut: 1), 'Kitchen iPad'),
          'Sent 3 files (3.2 MB) to Kitchen iPad. 1 item in what you chose was '
          'left out: links inside folders, files that could not be read and '
          'files that are too large.',
        );
      },
    );

    test('where received files are is said for the system they are on', () {
      expect(
        describeReceiveFolder('/var/mobile/Documents', TargetPlatform.iOS),
        'the Baton folder in the Files app',
      );
      expect(
        describeReceiveFolder('/Users/anna/Downloads', TargetPlatform.macOS),
        'your Downloads folder',
      );
      // The folder of the sandbox, which the person would not look in, is named in full.
      expect(
        describeReceiveFolder(
          '/Users/anna/Library/Containers/io.github.jayunewal.baton/Data/Library/Foo',
          TargetPlatform.macOS,
        ),
        'the folder '
        '/Users/anna/Library/Containers/io.github.jayunewal.baton/Data/Library/Foo',
      );
      expect(
        describeReceiveFolder('/home/anna/Downloads', TargetPlatform.linux),
        'the folder /home/anna/Downloads',
      );
    });
  });
}
