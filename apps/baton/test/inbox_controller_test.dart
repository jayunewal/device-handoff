// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/inbox_controller.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

const _link = ReceivedItem.link(
  url: 'https://example.com/a',
  title: 'An example',
);

Future<InboxController> _started(FakeSession session) async {
  final controller = InboxController(session);
  addTearDown(controller.dispose);
  await controller.start();
  return controller;
}

void main() {
  test(
    'reads what is in the inbox, newest first, and none of it is new',
    () async {
      final session = _session()
        ..received = [sampleReceived(2), sampleReceived(1)];
      final controller = InboxController(session);
      addTearDown(controller.dispose);
      expect(controller.items, isNull, reason: 'not read yet');

      await controller.start();

      expect(controller.items!.map((item) => item.number), [2, 1]);
      expect(controller.unseen, 0);
      expect(controller.problem, isNull);
    },
  );

  test('an item that arrives is shown, counted as new and announced', () async {
    final session = _session();
    final controller = await _started(session);
    final announced = <Received>[];
    controller.arrivals.listen(announced.add);

    session.arrive(sampleReceived(1, item: _link));
    await pumpEventQueue();

    expect(controller.items!.map((item) => item.number), [1]);
    expect(controller.unseen, 1);
    expect(announced.map((item) => item.number), [1]);

    session.arrive(sampleReceived(2));
    await pumpEventQueue();

    expect(controller.items!.map((item) => item.number), [2, 1]);
    expect(controller.unseen, 2);
  });

  test('one that arrives while the inbox is first read is not lost', () async {
    final session = _session()..received = [sampleReceived(1)];
    session.holdInbox();
    final controller = InboxController(session);
    addTearDown(controller.dispose);

    final started = controller.start();
    await pumpEventQueue();
    // The read has begun and is waiting. The item comes now, and is not in what it gives.
    session.arrive(sampleReceived(2));
    await pumpEventQueue();
    session.releaseInbox();
    await started;
    await pumpEventQueue();

    expect(controller.items!.map((item) => item.number), [2, 1]);
    expect(controller.unseen, 1, reason: 'only the one that came is new');
    expect(session.inboxReads, 2, reason: 'it read again, and not more');
  });

  test(
    'while the inbox is on screen what arrives is seen as it comes',
    () async {
      final session = _session();
      final controller = await _started(session);
      session.arrive(sampleReceived(1));
      await pumpEventQueue();
      expect(controller.unseen, 1);

      controller.looking = true;
      expect(controller.unseen, 0, reason: 'looking at it makes it seen');
      session.arrive(sampleReceived(2));
      await pumpEventQueue();
      expect(controller.unseen, 0);
      expect(controller.items!.map((item) => item.number), [2, 1]);

      controller.looking = false;
      session.arrive(sampleReceived(3));
      await pumpEventQueue();
      expect(controller.unseen, 1);
    },
  );

  test('the screens hear when the count of new items changes', () async {
    final session = _session();
    final controller = await _started(session);
    var heard = 0;
    controller.addListener(() => heard++);

    controller.looking = true;
    expect(heard, 1);
    controller.looking = true;
    expect(heard, 1, reason: 'nothing changed');
  });

  test('reads the inbox again when events were missed', () async {
    final session = _session();
    final controller = await _started(session);
    session.received = [sampleReceived(5)];

    session.emit(const Event.resync());
    await pumpEventQueue();

    expect(controller.items!.map((item) => item.number), [5]);
    expect(controller.unseen, 0, reason: 'what was missed is not announced');
  });

  test('takes one item out, and stops counting it as new', () async {
    final session = _session();
    final controller = await _started(session);
    session.arrive(sampleReceived(1));
    session.arrive(sampleReceived(2));
    await pumpEventQueue();
    expect(controller.unseen, 2);

    final problem = await controller.remove(controller.items!.last);
    await pumpEventQueue();

    expect(problem, isNull);
    expect(session.removed, [1]);
    expect(controller.items!.map((item) => item.number), [2]);
    expect(controller.unseen, 1);
  });

  test('takes everything out', () async {
    final session = _session()
      ..received = [sampleReceived(2), sampleReceived(1)];
    final controller = await _started(session);
    session.arrive(sampleReceived(3));
    await pumpEventQueue();

    final problem = await controller.clear();
    await pumpEventQueue();

    expect(problem, isNull);
    expect(session.clears, 1);
    expect(controller.items, isEmpty);
    expect(controller.unseen, 0);
  });

  test('says why an item could not be taken out, and keeps it', () async {
    final session = _session()..received = [sampleReceived(1)];
    final controller = await _started(session);
    session.inboxFailure = const BatonError(
      kind: ErrorKind.storage,
      message: 'the inbox file cannot be written',
    );

    expect(
      await controller.remove(controller.items!.single),
      'The inbox file cannot be written.',
    );
    expect(await controller.clear(), 'The inbox file cannot be written.');
    expect(controller.items!.map((item) => item.number), [1]);
  });

  test(
    'says when the inbox cannot be read, and reads it again on request',
    () async {
      final session = _session()..received = [sampleReceived(1)];
      session.inboxFailure = const BatonError(
        kind: ErrorKind.storage,
        message: 'the inbox file is damaged',
      );
      final controller = await _started(session);

      expect(controller.problem, 'The inbox file is damaged.');
      expect(controller.items, isNull);

      session.inboxFailure = null;
      var heardProblemGo = false;
      controller.addListener(
        () => heardProblemGo |= controller.problem == null,
      );
      await controller.reload();

      expect(heardProblemGo, isTrue);
      expect(controller.problem, isNull);
      expect(controller.items!.map((item) => item.number), [1]);
    },
  );

  test('does not call a stopped Baton a problem', () async {
    final session = _session();
    session.inboxFailure = const BatonError(
      kind: ErrorKind.stopped,
      message: 'Baton has stopped',
    );
    final controller = await _started(session);

    expect(controller.problem, isNull);
  });

  test('offers an address to open only for a link to a web page', () async {
    final session = _session();
    final controller = await _started(session);
    Received link(String url) =>
        sampleReceived(1, item: ReceivedItem.link(url: url));

    expect(
      controller.addressToOpen(link('https://example.com/a')),
      'https://example.com/a',
    );
    expect(
      controller.addressToOpen(link('HTTP://Example.com/a')),
      'http://example.com/a',
      reason: 'it is the address that was checked that is opened',
    );
    expect(controller.addressToOpen(link('tel:+15551234')), isNull);
    expect(controller.addressToOpen(link('file:///etc/hosts')), isNull);
    expect(controller.addressToOpen(link('javascript:alert(1)')), isNull);
    expect(controller.addressToOpen(link('not a link')), isNull);
    expect(
      controller.addressToOpen(
        sampleReceived(2, item: const ReceivedItem.text(text: 'https://a.b/')),
      ),
      isNull,
      reason: 'text is never opened, even if it looks like a link',
    );
  });

  test('listens to the events until it is disposed of', () async {
    final session = _session();
    final controller = InboxController(session);
    await controller.start();
    expect(session.listening, isTrue);

    controller.dispose();
    await pumpEventQueue();

    expect(session.listening, isFalse);
  });

  test(
    'does not listen if it is disposed of before the events are ready',
    () async {
      final session = _session()..holdEvents();
      final controller = InboxController(session);

      final started = controller.start();
      await pumpEventQueue();
      controller.dispose();
      session.releaseEvents();
      await started;
      await pumpEventQueue();

      expect(session.listening, isFalse);
      expect(session.inboxReads, 0);
    },
  );

  test('an item that arrives as it is disposed of does no harm', () async {
    final session = _session();
    final controller = InboxController(session);
    await controller.start();
    final announced = <Received>[];
    controller.arrivals.listen(announced.add, onDone: () {});

    controller.dispose();
    session.arrive(sampleReceived(1));
    await pumpEventQueue();

    expect(announced, isEmpty);
  });
}
