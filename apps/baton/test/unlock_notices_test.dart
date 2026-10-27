// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/unlock_notices.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_notices.dart';

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

/// Starts the notices on [session], which show through [notices].
Future<UnlockNotices> _start(FakeSession session, FakeNotices notices) async {
  final unlockNotices = UnlockNotices(session, notices);
  addTearDown(unlockNotices.dispose);
  await unlockNotices.start();
  return unlockNotices;
}

Future<void> _attempt(FakeSession session, UnlockEntry entry) async {
  session.emit(Event.unlockAttempted(entry: entry));
  await pumpEventQueue();
}

void main() {
  test('tells the person that a device unlocked this one', () async {
    final session = _session();
    final notices = FakeNotices();
    await _start(session, notices);

    await _attempt(session, sampleUnlockEntry());

    expect(notices.posted, [
      (
        title: 'This device was unlocked from Anna’s iPhone',
        body:
            'If that was not you, forget Anna’s iPhone in Baton and change the '
            'password of this device.',
      ),
    ]);
  });

  test('tells the person of a try that failed after it was made', () async {
    final session = _session();
    final notices = FakeNotices();
    await _start(session, notices);

    await _attempt(
      session,
      sampleUnlockEntry(result: UnlockResult.rejected, device: 'Kitchen iPad'),
    );

    expect(notices.posted, hasLength(1));
    expect(
      notices.posted.single.title,
      'Unlocking this device from Kitchen iPad did not work',
    );
    expect(
      notices.posted.single.body,
      contains('did not accept it. Set the unlock up again.'),
    );
  });

  test('tells of each attempt that Baton says to tell of, in order', () async {
    final session = _session();
    final notices = FakeNotices();
    await _start(session, notices);

    for (final result in [
      UnlockResult.unlocked,
      UnlockResult.stale,
      UnlockResult.timedOut,
      UnlockResult.keyLost,
    ]) {
      await _attempt(session, sampleUnlockEntry(result: result));
    }

    expect(notices.posted, hasLength(4));
    expect(notices.posted.first.title, startsWith('This device was unlocked'));
    expect(
      notices.posted.skip(1).map((n) => n.title),
      everyElement(startsWith('Unlocking this device')),
    );
  });

  test(
    'says nothing of an attempt that Baton does not say to tell of',
    () async {
      final session = _session();
      final notices = FakeNotices();
      await _start(session, notices);

      // A refusal before anything was typed, a person who cancelled, and a session that ended are
      // in the history, and do not interrupt the person.
      for (final result in [
        UnlockResult.notSetUp,
        UnlockResult.notLocked,
        UnlockResult.notPrivate,
        UnlockResult.busy,
        UnlockResult.declined,
        UnlockResult.sessionEnded,
      ]) {
        await _attempt(session, sampleUnlockEntry(result: result));
      }

      expect(notices.posted, isEmpty);
    },
  );

  test('follows what the entry says and does not decide for itself', () async {
    final session = _session();
    final notices = FakeNotices();
    await _start(session, notices);

    await _attempt(
      session,
      sampleUnlockEntry(result: UnlockResult.declined, notify: true),
    );
    await _attempt(session, sampleUnlockEntry(notify: false));

    expect(notices.posted, hasLength(1));
    expect(notices.posted.single.title, contains('did not work'));
  });

  test('says nothing for any other event', () async {
    final session = _session();
    final notices = FakeNotices();
    await _start(session, notices);

    session.emit(const Event.resync());
    session.emit(const Event.windowLocked(id: 1, locked: true));
    await pumpEventQueue();

    expect(notices.posted, isEmpty);
  });

  test('goes on when the system could not show a notification', () async {
    final session = _session();
    final notices = FakeNotices()..postFailure = StateError('no center');
    await _start(session, notices);

    await _attempt(session, sampleUnlockEntry());

    expect(notices.posted, isEmpty);

    notices.postFailure = null;
    await _attempt(session, sampleUnlockEntry());

    expect(notices.posted, hasLength(1));
  });

  test(
    'goes on whatever the system throws when it could not show one',
    () async {
      final session = _session();
      final notices = FakeNotices();
      await _start(session, notices);

      for (final failure in <Object>[
        ArgumentError('bad'),
        const FormatException('odd'),
        'not even an error',
      ]) {
        notices.postFailure = failure;
        await _attempt(session, sampleUnlockEntry());
      }

      expect(notices.posted, isEmpty);

      notices.postFailure = null;
      await _attempt(session, sampleUnlockEntry());

      expect(notices.posted, hasLength(1));
    },
  );

  test('goes on when the system took no notification', () async {
    final session = _session();
    final notices = FakeNotices()..taken = false;
    await _start(session, notices);

    await _attempt(session, sampleUnlockEntry());
    await _attempt(session, sampleUnlockEntry());

    // Each was offered; the history has them all.
    expect(notices.posted, hasLength(2));
  });

  test('shows the name of the device without what turns text round', () async {
    final session = _session();
    final notices = FakeNotices();
    await _start(session, notices);

    await _attempt(session, sampleUnlockEntry(device: 'Phone\u202Egpj'));

    expect(
      notices.posted.single.title,
      'This device was unlocked from Phone\uFFFDgpj',
    );
  });

  test('stops listening when it is disposed', () async {
    final session = _session();
    final notices = FakeNotices();
    final unlockNotices = await _start(session, notices);
    expect(session.listening, isTrue);

    unlockNotices.dispose();
    await pumpEventQueue();
    await _attempt(session, sampleUnlockEntry());

    expect(notices.posted, isEmpty);
    expect(session.listening, isFalse);
  });

  test(
    'does not listen when it is disposed before the events arrive',
    () async {
      final session = _session()..holdEvents();
      final notices = FakeNotices();
      final unlockNotices = UnlockNotices(session, notices);
      final started = unlockNotices.start();

      unlockNotices.dispose();
      session.releaseEvents();
      await started;
      await pumpEventQueue();
      await _attempt(session, sampleUnlockEntry());

      expect(session.listening, isFalse);
      expect(notices.posted, isEmpty);
    },
  );
}
