// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/arrival_notices.dart';
import 'package:baton/src/backend.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_always_on.dart';
import 'fake_backend.dart';
import 'fake_notices.dart';

const _link = ReceivedItem.link(
  url: 'https://example.com/reset?token=s3cret',
  title: 'Reset your password',
);

/// The streams that the controllers of the home screen give, and what the notices are made of.
class _Rig {
  _Rig() {
    notices = ArrivalNotices(
      arrivals: arrivals.stream,
      files: files.stream,
      windows: windows.stream,
      notices: keeper,
      window: window,
    );
    addTearDown(_close);
  }

  Future<void> _close() async {
    notices.dispose();
    await arrivals.close();
    await files.close();
    await windows.close();
  }

  final arrivals = StreamController<Received>.broadcast();
  final files = StreamController<TransferRequest>.broadcast();
  final windows = StreamController<WindowRequest>.broadcast();
  final keeper = FakeNotices();
  final window = FakeAlwaysOn()..seen = false;
  late final ArrivalNotices notices;
}

void main() {
  group('while the person cannot see Baton', () {
    test(
      'a link that arrived is told, with who sent it and not what it is',
      () async {
        final rig = _Rig();

        rig.arrivals.add(sampleReceived(1, item: _link));
        await pumpEventQueue();

        expect(rig.keeper.posted, [
          (
            title: 'Anna’s iPhone sent you a link',
            body: 'Open Baton to see it in the inbox.',
          ),
        ]);
        final said = '${rig.keeper.posted.single}';
        expect(said, isNot(contains('example.com')));
        expect(said, isNot(contains('s3cret')));
        expect(said, isNot(contains('password')));
      },
    );

    test('text that arrived is told as text, and never what it says', () async {
      final rig = _Rig();

      rig.arrivals.add(
        sampleReceived(
          2,
          from: 'Desk iPad',
          item: const ReceivedItem.text(text: 'the safe code is 4417'),
        ),
      );
      await pumpEventQueue();

      expect(rig.keeper.posted.single.title, 'Desk iPad sent you some text');
      expect('${rig.keeper.posted.single}', isNot(contains('4417')));
    });

    test(
      'a device that asks to send files is told, without a file name',
      () async {
        final rig = _Rig();

        rig.files.add(
          sampleTransfer(
            from: 'Anna’s iPhone',
            preview: const [
              PreviewName(
                name: 'medical-results.pdf',
                folder: false,
                files: 0,
                bytes: 100,
              ),
            ],
          ),
        );
        await pumpEventQueue();

        expect(rig.keeper.posted, [
          (
            title: 'Anna’s iPhone wants to send you files',
            body: 'Open Baton to say yes or no.',
          ),
        ]);
        expect('${rig.keeper.posted.single}', isNot(contains('medical')));
      },
    );

    test('a device that offers a window or its screen is told which', () async {
      final rig = _Rig();

      rig.windows.add(sampleWindowRequest);
      rig.windows.add(
        const WindowRequest(
          id: 4,
          deviceId: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
          deviceName: 'Anna’s Mac',
          display: true,
          width: 2560,
          height: 1440,
          control: false,
        ),
      );
      await pumpEventQueue();

      expect(
        [for (final notice in rig.keeper.posted) notice.title],
        [
          'Anna’s Mac wants to show you a window',
          'Anna’s Mac wants to show you its screen',
        ],
      );
      expect(rig.keeper.posted.first.body, 'Open Baton to say yes or no.');
      expect(
        '${rig.keeper.posted}',
        isNot(contains('Budget')),
        reason: 'the title of the window can be private',
      );
    });

    test('a name that hides or turns text round is shown plainly', () async {
      final rig = _Rig();

      rig.arrivals.add(sampleReceived(3, from: 'Pay\u202Egpj.exe'));
      await pumpEventQueue();

      final title = rig.keeper.posted.single.title;
      expect(title, isNot(contains('\u202E')));
      expect(title, endsWith('sent you some text'));
    });

    test('each thing that comes is told by itself', () async {
      final rig = _Rig();

      rig.arrivals.add(sampleReceived(1));
      rig.arrivals.add(sampleReceived(2, item: _link));
      rig.files.add(sampleTransfer());
      await pumpEventQueue();

      expect(rig.keeper.posted, hasLength(3));
    });
  });

  group('while the person can see Baton', () {
    test('nothing is told, because the window tells them', () async {
      final rig = _Rig();
      rig.window.seen = true;

      rig.arrivals.add(sampleReceived(1));
      rig.files.add(sampleTransfer());
      rig.windows.add(sampleWindowRequest);
      await pumpEventQueue();

      expect(rig.keeper.posted, isEmpty);
    });

    test('it is asked at the time, and not once for all', () async {
      final rig = _Rig();
      rig.window.seen = true;
      rig.arrivals.add(sampleReceived(1));
      await pumpEventQueue();
      expect(rig.keeper.posted, isEmpty);

      // The person put another app in front.
      rig.window.seen = false;
      rig.arrivals.add(sampleReceived(2));
      await pumpEventQueue();

      expect(rig.keeper.posted, hasLength(1));
    });
  });

  group('when the notification cannot be shown', () {
    test('a system that did not take it is no trouble', () async {
      final rig = _Rig();
      rig.keeper.taken = false;

      rig.arrivals.add(sampleReceived(1));
      await pumpEventQueue();

      // The item is in the inbox all the same, and the next one is told as before.
      rig.keeper.taken = true;
      rig.arrivals.add(sampleReceived(2));
      await pumpEventQueue();
      expect(rig.keeper.posted, hasLength(2));
    });

    test('one that fails is no trouble either', () async {
      final rig = _Rig();
      rig.keeper.postFailure = StateError('the Mac app is gone');

      rig.arrivals.add(sampleReceived(1));
      rig.files.add(sampleTransfer());
      await pumpEventQueue();

      rig.keeper.postFailure = null;
      rig.arrivals.add(sampleReceived(2));
      await pumpEventQueue();
      expect(rig.keeper.posted, hasLength(1));
    });
  });

  group('when it is closed', () {
    test('nothing more is told, and the streams are let go of', () async {
      final rig = _Rig();
      rig.notices.dispose();

      rig.arrivals.add(sampleReceived(1));
      rig.files.add(sampleTransfer());
      rig.windows.add(sampleWindowRequest);
      await pumpEventQueue();

      expect(rig.keeper.posted, isEmpty);
      expect(rig.arrivals.hasListener, isFalse);
      expect(rig.files.hasListener, isFalse);
      expect(rig.windows.hasListener, isFalse);
    });
  });
}
