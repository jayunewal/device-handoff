// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/unlock_text.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

const _mac = 'Anna’s Mac';
const _phone = 'Anna’s iPhone';

void main() {
  group('while an unlock goes', () {
    test(
      'says what is happening at each step, and nothing before or after',
      () {
        expect(describeUnlocking(UnlockStage.idle, _mac), isNull);
        expect(describeUnlocking(UnlockStage.ended, _mac), isNull);
        expect(
          describeUnlocking(UnlockStage.starting, _mac),
          'Asking Anna’s Mac to unlock…',
        );
        expect(
          describeUnlocking(UnlockStage.asking, _mac),
          'Waiting for Face ID or Touch ID…',
        );
        expect(
          describeUnlocking(UnlockStage.typing, _mac),
          'Anna’s Mac is typing its password…',
        );
      },
    );

    test(
      'shows the name of the other device without what turns text round',
      () {
        expect(
          describeUnlocking(UnlockStage.typing, 'Mac\u202Egpj'),
          'Mac\uFFFDgpj is typing its password…',
        );
      },
    );
  });

  group('when an unlock that the person asked for has ended', () {
    test('says something about every result on one line', () {
      for (final result in UnlockResult.values) {
        final text = describeUnlockEnd(result, _mac);
        expect(text, isNotEmpty, reason: '$result');
        expect(text, isNot(contains('\n')), reason: '$result');
      }
    });

    test('names the other device where the result is about it', () {
      const about = [
        UnlockResult.unlocked,
        UnlockResult.notSetUp,
        UnlockResult.notLocked,
        UnlockResult.notPrivate,
        UnlockResult.busy,
        UnlockResult.stale,
        UnlockResult.rejected,
        UnlockResult.timedOut,
        UnlockResult.failed,
        UnlockResult.other,
        UnlockResult.noQuestion,
        UnlockResult.noResult,
        UnlockResult.sessionEnded,
      ];
      for (final result in about) {
        expect(
          describeUnlockEnd(result, _mac),
          contains(_mac),
          reason: '$result',
        );
      }
    });

    test('says that the device is unlocked', () {
      expect(
        describeUnlockEnd(UnlockResult.unlocked, _mac),
        '$_mac is unlocked.',
      );
    });

    test('tells a person whose set-up ended where to set it up again', () {
      for (final result in [
        UnlockResult.notSetUp,
        UnlockResult.stale,
        UnlockResult.keyLost,
      ]) {
        expect(
          describeUnlockEnd(result, _mac),
          endsWith('Set it up again in Baton on $_mac.'),
          reason: '$result',
        );
      }
      expect(
        describeUnlockEnd(UnlockResult.rejected, _mac),
        endsWith(
          'If the password of $_mac changed, set it up again in Baton on $_mac.',
        ),
      );
    });

    test('tells a person whose face was not taken to try again', () {
      expect(
        describeUnlockEnd(UnlockResult.declined, _mac),
        'Not unlocked. This device did not confirm that it was you, or you '
        'cancelled. Try again.',
      );
    });

    test('tells a person who may have unlocked it to look at the screen', () {
      for (final result in [UnlockResult.noResult, UnlockResult.other]) {
        expect(
          describeUnlockEnd(result, _mac),
          contains('Look at'),
          reason: '$result',
        );
      }
    });

    test('lets the person try again only where another tap can help', () {
      const again = {
        UnlockResult.notPrivate,
        UnlockResult.busy,
        UnlockResult.timedOut,
        UnlockResult.failed,
        UnlockResult.other,
        UnlockResult.declined,
        UnlockResult.gaveUp,
        UnlockResult.noQuestion,
        UnlockResult.noResult,
      };
      for (final result in UnlockResult.values) {
        expect(
          canTryUnlockAgain(result),
          again.contains(result),
          reason: '$result',
        );
      }
    });
  });

  group('in the history of this device', () {
    test(
      'says something about every result on one line, and names the device',
      () {
        for (final result in UnlockResult.values) {
          final text = describeAttempt(
            sampleUnlockEntry(device: _phone, result: result),
          );
          expect(text, contains(_phone), reason: '$result');
          expect(text, isNot(contains('\n')), reason: '$result');
        }
      },
    );

    test('says that the device unlocked this one', () {
      expect(
        describeAttempt(sampleUnlockEntry(device: _phone)),
        'Anna’s iPhone unlocked this device.',
      );
    });

    test('says that nothing was typed where nothing was', () {
      for (final result in [
        UnlockResult.notSetUp,
        UnlockResult.notLocked,
        UnlockResult.notPrivate,
        UnlockResult.stale,
        UnlockResult.timedOut,
        UnlockResult.declined,
        UnlockResult.keyLost,
        UnlockResult.gaveUp,
        UnlockResult.noQuestion,
        UnlockResult.sessionEnded,
      ]) {
        expect(
          describeAttempt(sampleUnlockEntry(result: result)).toLowerCase(),
          contains('nothing was typed'),
          reason: '$result',
        );
      }
    });

    test('says that the password was typed and not accepted', () {
      expect(
        describeAttempt(sampleUnlockEntry(result: UnlockResult.rejected)),
        'The password was typed for Anna’s iPhone and this device did not '
        'accept it. Set the unlock up again.',
      );
    });

    test('tells the person to set the unlock up again where it ended', () {
      for (final result in [
        UnlockResult.stale,
        UnlockResult.rejected,
        UnlockResult.keyLost,
      ]) {
        expect(
          describeAttempt(sampleUnlockEntry(result: result)),
          endsWith('Set the unlock up again.'),
          reason: '$result',
        );
      }
    });

    test('shows the name without what turns text round', () {
      expect(
        describeAttempt(sampleUnlockEntry(device: 'Phone\u202Egpj')),
        'Phone\uFFFDgpj unlocked this device.',
      );
    });
  });

  group('in a notification', () {
    test('tells of an unlock, and what to do if it was not the person', () {
      final notice = describeNotice(sampleUnlockEntry(device: _phone));

      expect(notice.title, 'This device was unlocked from Anna’s iPhone');
      expect(
        notice.body,
        'If that was not you, forget Anna’s iPhone in Baton and change the '
        'password of this device.',
      );
    });

    test('tells of a try that failed, with what the history says of it', () {
      final entry = sampleUnlockEntry(
        device: _phone,
        result: UnlockResult.rejected,
      );

      final notice = describeNotice(entry);

      expect(
        notice.title,
        'Unlocking this device from Anna’s iPhone did not work',
      );
      expect(notice.body, describeAttempt(entry));
    });

    test('shows the name without what turns text round', () {
      final notice = describeNotice(
        sampleUnlockEntry(device: 'Phone\u202Egpj'),
      );

      expect(notice.title, contains('Phone\uFFFDgpj'));
      expect(notice.body, contains('Phone\uFFFDgpj'));
    });
  });

  group('about notifications that are not shown', () {
    test('says nothing where they are shown, or will be asked for', () {
      expect(describeNoticeAccess(NoticeAccess.allowed, 'Mac'), isNull);
      expect(describeNoticeAccess(NoticeAccess.needed, 'Mac'), isNull);
    });

    test('says where to allow them when the person said no', () {
      expect(
        describeNoticeAccess(NoticeAccess.denied, 'Mac'),
        'Notifications are off for Baton, so it cannot tell you when this Mac '
        'is unlocked. Allow them in System Settings, under Notifications.',
      );
    });

    test('says where to look instead when the system cannot show them', () {
      expect(
        describeNoticeAccess(NoticeAccess.unavailable, 'Mac'),
        'This Mac cannot show notifications for Baton, so look at the recent '
        'attempts to see when it is unlocked.',
      );
    });
  });
}
