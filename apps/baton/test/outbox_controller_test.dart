// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/outbox_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

const _ipad =
    'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1';
const _phone =
    'd2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2d2';

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

Future<OutboxController> _started(FakeSession session) async {
  final controller = OutboxController(session);
  addTearDown(controller.dispose);
  await controller.start();
  return controller;
}

void main() {
  test('reads what waits when it starts, the longest wait first', () async {
    final session = _session();
    session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    session.queue(
      deviceId: _ipad,
      deviceName: 'Kitchen iPad',
      summary: 'some text (5 characters)',
    );
    final controller = OutboxController(session);
    addTearDown(controller.dispose);
    expect(controller.items, isEmpty, reason: 'not read yet');

    await controller.start();

    expect(controller.items.map((item) => item.summary), [
      'a link (example.com)',
      'some text (5 characters)',
    ]);
  });

  test(
    'reads again when the list changes, and tells the screen each time',
    () async {
      final session = _session();
      final controller = await _started(session);
      var heard = 0;
      controller.addListener(() => heard++);

      final item = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
      await pumpEventQueue();
      expect(controller.items, [item]);
      expect(heard, 1);

      session.waiting.clear();
      session.emit(const Event.outboxChanged());
      await pumpEventQueue();
      expect(controller.items, isEmpty);
      expect(heard, 2);
    },
  );

  test('reads again after events were missed', () async {
    final session = _session();
    final controller = await _started(session);

    // Something changed that the controller was not told of, as when events were dropped.
    session.waiting.add(
      const OutboxItem(
        id: 7,
        deviceId: _ipad,
        deviceName: 'Kitchen iPad',
        summary: 'a link (example.org)',
        queuedAt: 1760000000,
        expiresAt: 1760086400,
      ),
    );
    session.emit(const Event.resync());
    await pumpEventQueue();

    expect(controller.items.map((item) => item.id), [7]);
  });

  test('a list that cannot be read stays as it was', () async {
    final session = _session();
    final item = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    final controller = await _started(session);
    var heard = 0;
    controller.addListener(() => heard++);
    // Baton has stopped, which only happens as the app closes.
    session.outboxFailure = const BatonError(
      kind: ErrorKind.stopped,
      message: 'Baton has stopped',
    );

    session.emit(const Event.resync());
    await pumpEventQueue();

    expect(controller.items, [item]);
    expect(heard, 0, reason: 'nothing changed, so nobody is told');
  });

  test('says which items wait for one device and no other', () async {
    final session = _session();
    session.queue(
      deviceId: _ipad,
      deviceName: 'Kitchen iPad',
      summary: 'a link (a.example)',
    );
    session.queue(
      deviceId: _phone,
      deviceName: 'Pocket Phone',
      summary: 'a link (b.example)',
    );
    session.queue(
      deviceId: _ipad,
      deviceName: 'Kitchen iPad',
      summary: 'a link (c.example)',
    );
    final controller = await _started(session);

    expect(controller.waitingFor(_ipad).map((item) => item.summary), [
      'a link (a.example)',
      'a link (c.example)',
    ]);
    expect(controller.waitingFor(_phone).map((item) => item.summary), [
      'a link (b.example)',
    ]);
    expect(controller.waitingFor('e3' * 32), isEmpty);
  });

  test('tells how an item ended, and the list no longer has it', () async {
    final session = _session();
    final first = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    final second = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    final controller = await _started(session);
    final ended = <OutboxSettled>[];
    controller.settled.listen(ended.add);

    session.settle(
      first,
      OutboxEnd.delivered,
      'Sent a link (example.com) to Kitchen iPad.',
    );
    await pumpEventQueue();
    session.settle(
      second,
      OutboxEnd.expired,
      'Gave up on sending a link (example.com) to Kitchen iPad: it could not be '
      'reached for a day. Send it again when Kitchen iPad is open.',
    );
    await pumpEventQueue();

    expect(ended.map((settled) => settled.end), [
      OutboxEnd.delivered,
      OutboxEnd.expired,
    ]);
    expect(ended.first.message, 'Sent a link (example.com) to Kitchen iPad.');
    expect(controller.items, isEmpty);
  });

  test('giving an item up says that it will not be sent', () async {
    final session = _session();
    final item = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    final controller = await _started(session);
    final ended = <OutboxSettled>[];
    controller.settled.listen(ended.add);

    final said = await controller.cancel(item);

    expect(session.cancelled, [item.id]);
    expect(
      said,
      'Cancelled. Baton will not send a link (example.com) to Kitchen iPad.',
    );
    expect(controller.items, isEmpty, reason: 'the list is right at once');
    await pumpEventQueue();
    expect(ended, isEmpty, reason: 'an item that was given up on is not told');
  });

  test('giving up an item that was sent a moment ago says nothing', () async {
    final session = _session();
    final item = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    final controller = await _started(session);
    // It went while the person was reaching for the button.
    session.waiting.clear();

    expect(await controller.cancel(item), isNull);

    expect(controller.items, isEmpty);
  });

  test('says why an item could not be given up, in a sentence', () async {
    final session = _session()
      ..cancelFailure = const BatonError(
        kind: ErrorKind.stopped,
        message: 'Baton has stopped',
      );
    final item = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    final controller = await _started(session);

    expect(await controller.cancel(item), 'Baton has stopped.');

    expect(controller.items, [item], reason: 'it still waits');
  });

  test('shows a name that hides or turns text round plainly', () async {
    final session = _session();
    final item = session.queue(
      deviceId: _ipad,
      deviceName: 'Kitchen\u202Eiriad',
      summary: 'a link (example\u200B.com)',
    );
    final controller = await _started(session);

    final said = await controller.cancel(item);

    expect(
      said,
      'Cancelled. Baton will not send a link (example\uFFFD.com) to '
      'Kitchen\uFFFDiriad.',
    );
  });

  test(
    'does not listen if it is disposed of before the events are ready',
    () async {
      final session = _session()..holdEvents();
      final controller = OutboxController(session);

      final started = controller.start();
      await pumpEventQueue();
      controller.dispose();
      session.releaseEvents();
      await started;
      await pumpEventQueue();

      expect(session.listening, isFalse);
    },
  );

  test('stops listening when it is disposed of', () async {
    final session = _session();
    final controller = OutboxController(session);
    await controller.start();
    expect(session.listening, isTrue);

    controller.dispose();
    await pumpEventQueue();

    expect(session.listening, isFalse);
  });

  test('an event that comes as it is disposed of does no harm', () async {
    final session = _session();
    final controller = OutboxController(session);
    await controller.start();
    final ended = <OutboxSettled>[];
    controller.settled.listen(ended.add, onDone: () {});

    controller.dispose();
    session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
    await pumpEventQueue();

    expect(ended, isEmpty);
  });

  test(
    'a cancel that is still under way as it is disposed of does no harm',
    () async {
      final session = _session();
      final item = session.queue(deviceId: _ipad, deviceName: 'Kitchen iPad');
      final controller = OutboxController(session);
      await controller.start();
      session.holdCancel();

      final cancelling = controller.cancel(item);
      await pumpEventQueue();
      // The person left the screen before Baton answered.
      controller.dispose();
      session.releaseCancel();

      // Telling a disposed notifier's listeners is an error, so the list is not read again.
      expect(
        await cancelling,
        'Cancelled. Baton will not send a link (example.com) to Kitchen iPad.',
      );
      expect(session.cancelled, [item.id]);
    },
  );
}
