// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/awake.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/files_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';
import 'support.dart';

final _kitchenIpad = Device(
  id: 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: [_kitchenIpad],
);

FilesController _controller(FakeSession session, {FakeScreen? screen}) {
  final controller = FilesController(
    session,
    _kitchenIpad,
    screen: screen ?? FakeScreen(),
    every: const Duration(milliseconds: 5),
  );
  addTearDown(controller.dispose);
  return controller;
}

const _holiday = FilesSent(files: 1, bytes: 2000000, failed: 0, leftOut: 0);

void main() {
  test(
    'files go to the device, and the person hears that they arrived',
    () async {
      final session = _session();
      final controller = _controller(session);

      final sent = controller.send(['/Users/me/holiday.jpg']);
      await until(controller, () => controller.progress != null);

      expect(controller.busy, isTrue);
      expect(controller.what, 'holiday.jpg');
      expect(session.filesSent.map((sent) => sent.id), [_kitchenIpad.id]);
      expect(session.filesSent.map((sent) => sent.paths), [
        ['/Users/me/holiday.jpg'],
      ]);

      session.transfer.finish(_holiday);

      expect(await sent, isTrue);
      expect(controller.busy, isFalse);
      expect(controller.progress, isNull);
      expect(controller.outcome!.ok, isTrue);
      expect(
        controller.outcome!.message,
        'Sent 1 file (2.0 MB) to Kitchen iPad.',
      );
    },
  );

  test('what is sent is named by the first thing and how many more', () async {
    for (final (paths, what) in [
      (['/Users/me/holiday.jpg'], 'holiday.jpg'),
      (['/Users/me/Photos/'], 'Photos'),
      (['C:\\Users\\me\\notes.txt'], 'notes.txt'),
      (['/a.txt', '/b.txt', '/c.txt'], 'a.txt and 2 more'),
      (['/'], '/'),
    ]) {
      final session = _session();
      final controller = _controller(session);

      unawaited(controller.send(paths));
      await until(controller, () => controller.busy);

      expect(controller.what, what, reason: '$paths');
    }
  });

  test(
    'the bar follows the bytes that went out, and waits when it cannot',
    () async {
      final session = _session();
      final controller = _controller(session);
      unawaited(controller.send(['/a.bin']));
      await until(controller, () => controller.progress != null);

      // Nothing is measured while Baton reaches the device or waits for the answer.
      expect(controller.fraction, isNull);
      session.transfer.advance(stage: SendStage.asking);
      await until(
        controller,
        () => controller.progress!.stage == SendStage.asking,
      );
      expect(controller.fraction, isNull);

      session.transfer.advance(
        stage: SendStage.sending,
        bytes: 1000,
        sent: 250,
      );
      await until(controller, () => controller.fraction == 0.25);

      session.transfer.advance(stage: SendStage.waiting, sent: 0);
      await until(
        controller,
        () => controller.progress!.stage == SendStage.waiting,
      );
      expect(controller.fraction, isNull);

      // A transfer of nothing but empty files has no bytes to count.
      session.transfer.advance(stage: SendStage.sending, bytes: 0);
      await until(
        controller,
        () => controller.progress!.stage == SendStage.sending,
      );
      expect(controller.fraction, isNull);

      session.transfer.advance(
        stage: SendStage.finishing,
        bytes: 1000,
        sent: 1000,
      );
      await until(controller, () => controller.fraction == 1.0);
    },
  );

  test(
    'a start that fails is told in a sentence, and nothing is left going',
    () async {
      final session = _session()
        ..sendFilesFailure = const BatonError(
          kind: ErrorKind.badItem,
          message: 'there is nothing to send in what you chose',
        );
      final screen = FakeScreen();
      final controller = _controller(session, screen: screen);

      expect(await controller.send(['/Users/me/empty']), isFalse);

      expect(controller.busy, isFalse);
      expect(
        controller.outcome!.message,
        'There is nothing to send in what you chose.',
      );
      expect(controller.outcome!.ok, isFalse);
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    },
  );

  test(
    'a device that says no, or cannot be reached, is told as it was said',
    () async {
      for (final (error, kind) in [
        (
          const BatonError(
            kind: ErrorKind.declined,
            message: 'Kitchen iPad did not take the files.',
          ),
          ErrorKind.declined,
        ),
        (
          const BatonError(
            kind: ErrorKind.unreachable,
            message: 'Could not reach Kitchen iPad.',
          ),
          ErrorKind.unreachable,
        ),
      ]) {
        final session = _session();
        final controller = _controller(session);
        final sent = controller.send(['/a.bin']);
        await until(controller, () => controller.progress != null);

        session.transfer.fail(error);

        expect(await sent, isFalse, reason: '$kind');
        expect(controller.outcome!.message, error.message, reason: '$kind');
        expect(controller.outcome!.ok, isFalse, reason: '$kind');
        expect(controller.busy, isFalse, reason: '$kind');
      }
    },
  );

  test('files that did not all arrive are not reported as sent', () async {
    final session = _session();
    final controller = _controller(session);
    final sent = controller.send(['/Users/me/Photos']);
    await until(controller, () => controller.progress != null);

    session.transfer.finish(
      const FilesSent(files: 3, bytes: 9000000, failed: 1, leftOut: 2),
    );

    expect(await sent, isFalse);
    expect(controller.outcome!.ok, isFalse);
    expect(
      controller.outcome!.message,
      'Sent 2 of 3 files to Kitchen iPad. 1 file could not be saved there. '
      '2 items in what you chose were left out: links inside folders, files '
      'that could not be read and files that are too large.',
    );
  });

  test(
    'giving the transfer up says nothing, because the person knows',
    () async {
      final session = _session();
      final controller = _controller(session);
      final sent = controller.send(['/a.bin']);
      await until(controller, () => controller.progress != null);

      controller.cancel();

      expect(await sent, isFalse);
      expect(session.transfer.cancelled, isTrue);
      expect(controller.outcome, isNull);
      expect(controller.busy, isFalse);
    },
  );

  test(
    'one transfer goes at a time, and nothing is a transfer of nothing',
    () async {
      final session = _session();
      final controller = _controller(session);

      expect(await controller.send(const []), isFalse);
      expect(session.filesSent, isEmpty);

      final first = controller.send(['/a.bin']);
      await until(controller, () => controller.progress != null);
      expect(await controller.send(['/b.bin']), isFalse);
      expect(session.filesSent.map((sent) => sent.paths), [
        ['/a.bin'],
      ]);

      session.transfer.finish(_holiday);
      await first;
      expect(controller.busy, isFalse);
    },
  );

  test('a second send can start when the first has ended', () async {
    final session = _session();
    final controller = _controller(session);
    final first = controller.send(['/a.bin']);
    await until(controller, () => controller.progress != null);
    session.transfer.finish(_holiday);
    await first;

    final second = controller.send(['/b.bin']);
    await until(controller, () => controller.progress != null);

    expect(controller.outcome, isNull, reason: 'the last outcome is cleared');
    session.transfer.finish(_holiday);
    expect(await second, isTrue);
  });

  test('leaving the screen gives a transfer that is going up', () async {
    final session = _session();
    final screen = FakeScreen();
    final controller = FilesController(
      session,
      _kitchenIpad,
      screen: screen,
      every: const Duration(milliseconds: 5),
    );
    final sent = controller.send(['/a.bin']);
    await until(controller, () => controller.progress != null);

    controller.dispose();

    expect(session.transfer.cancelled, isTrue);
    expect(await sent, isFalse);
    expect(screen.asked, ['keepOn', 'letTurnOff']);
  });

  test('leaving the screen while the transfer starts gives it up as soon as it has started', () async {
    final session = _session()..holdSendFiles();
    final screen = FakeScreen();
    final controller = FilesController(session, _kitchenIpad, screen: screen);
    final sent = controller.send(['/a.bin']);
    await pumpEventQueue();
    expect(controller.busy, isTrue);

    controller.dispose();
    session.releaseSendFiles();

    expect(await sent, isFalse);
    expect(session.transfer.cancelled, isTrue);
    expect(screen.asked, ['keepOn', 'letTurnOff']);
  });

  group('the screen of a phone', () {
    test(
      'stays on while files go, and goes back to locking when they have',
      () async {
        final session = _session();
        final screen = FakeScreen();
        final controller = _controller(session, screen: screen);
        expect(screen.asked, isEmpty, reason: 'nothing to keep on yet');

        final sent = controller.send(['/a.bin']);
        await until(controller, () => controller.progress != null);
        expect(screen.asked, ['keepOn']);

        session.transfer.finish(_holiday);
        await sent;
        expect(screen.asked, ['keepOn', 'letTurnOff']);
      },
    );

    testWidgets(
      'stays on for as long as the transfer takes steps, and no longer',
      (tester) async {
        final session = _session();
        final screen = FakeScreen();
        final controller = FilesController(
          session,
          _kitchenIpad,
          screen: screen,
        );
        addTearDown(controller.dispose);
        unawaited(controller.send(['/big.bin']));
        await tester.pump();
        session.transfer.advance(stage: SendStage.sending, bytes: 1000);

        // A step every two minutes goes on for longer than the limit of one hold.
        for (var step = 1; step <= 4; step++) {
          await tester.pump(const Duration(minutes: 2));
          session.transfer.advance(sent: step * 100);
          await tester.pump(const Duration(milliseconds: 300));
        }
        expect(screen.asked, ['keepOn']);

        // A transfer that stands still lets the screen go, as a pairing that stands still does.
        await tester.pump(awakeLimit + const Duration(seconds: 1));
        expect(screen.asked, ['keepOn', 'letTurnOff']);

        session.transfer.finish(_holiday);
        await tester.pump();
        expect(screen.asked, ['keepOn', 'letTurnOff'], reason: 'once only');
      },
    );
  });
}
