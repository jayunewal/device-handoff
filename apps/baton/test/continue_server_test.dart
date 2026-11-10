// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/continue_server.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_continue_reader.dart';

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

const _report = ContinueItems(
  page: ContinuePage(url: 'https://example.com/report', title: 'The report'),
  clipboard: 'hello',
);

/// Starts a server on [session], which reads with [reader].
Future<ContinueServer> _start(
  FakeSession session,
  FakeContinueReader reader,
) async {
  final server = ContinueServer(session, reader);
  addTearDown(server.dispose);
  await server.start();
  return server;
}

Future<void> _ask(FakeSession session, int id) async {
  session.emit(Event.continueAsked(id: id, from: 'Anna’s iPhone'));
  await pumpEventQueue();
}

void main() {
  test('gives a device that asks what the person is working on', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report);
    await _start(session, reader);

    await _ask(session, 7);

    expect(session.continueAnswers, [(request: 7, items: _report)]);
    expect(reader.reads, 1);
  });

  test('reads nothing until a device asks', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report);
    await _start(session, reader);
    await pumpEventQueue();

    expect(reader.reads, 0);
    expect(session.continueAnswers, isEmpty);
  });

  test(
    'gives an empty answer as it is: there is nothing to continue',
    () async {
      final session = _session();
      await _start(session, FakeContinueReader());

      await _ask(session, 1);

      expect(session.continueAnswers, [
        (request: 1, items: const ContinueItems()),
      ]);
    },
  );

  test(
    'says that it could not read, which is not that there is nothing',
    () async {
      final session = _session();
      final reader = FakeContinueReader(items: null);
      await _start(session, reader);

      await _ask(session, 2);

      expect(session.continueAnswers, [(request: 2, items: null)]);
    },
  );

  test('says that it could not read when reading fails', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report)
      ..readFailure = StateError('the Mac app is gone');
    await _start(session, reader);

    await _ask(session, 3);

    expect(session.continueAnswers, [(request: 3, items: null)]);
  });

  test('answers each question by itself, with what is open then', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report);
    await _start(session, reader);

    await _ask(session, 1);
    reader.items = const ContinueItems(clipboard: 'something else');
    await _ask(session, 2);

    expect(session.continueAnswers, [
      (request: 1, items: _report),
      (request: 2, items: const ContinueItems(clipboard: 'something else')),
    ]);
    expect(reader.reads, 2);
  });

  test('answers a question that was waiting for the Mac to read', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report)..holdRead();
    await _start(session, reader);

    await _ask(session, 4);
    expect(session.continueAnswers, isEmpty);

    reader.releaseRead();
    await pumpEventQueue();

    expect(session.continueAnswers, [(request: 4, items: _report)]);
  });

  test('starts no second read while one is out, and says so at once', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report)..holdRead();
    await _start(session, reader);

    await _ask(session, 1);
    // The library gave up on the first, and the device asks again.
    await _ask(session, 2);
    await _ask(session, 3);

    expect(reader.reads, 1, reason: 'the system is not asked behind its back');
    expect(session.continueAnswers, [
      (request: 2, items: null),
      (request: 3, items: null),
    ]);

    // The first comes back, late, and is handed over all the same: the library turns it down if
    // nobody waits for it.
    reader.releaseRead();
    await pumpEventQueue();
    expect(session.continueAnswers.last, (request: 1, items: _report));
  });

  test('reads again once the one that was out has come back', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report)..holdRead();
    await _start(session, reader);
    await _ask(session, 1);
    reader.releaseRead();
    await pumpEventQueue();

    reader.items = const ContinueItems(clipboard: 'a newer line');
    await _ask(session, 2);

    expect(reader.reads, 2);
    expect(session.continueAnswers.last, (
      request: 2,
      items: const ContinueItems(clipboard: 'a newer line'),
    ));
  });

  test('reads again after a read that failed', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report)
      ..readFailure = StateError('the Mac app is gone');
    await _start(session, reader);
    await _ask(session, 1);
    expect(session.continueAnswers, [(request: 1, items: null)]);

    reader.readFailure = null;
    await _ask(session, 2);

    expect(reader.reads, 2);
    expect(session.continueAnswers.last, (request: 2, items: _report));
  });

  test('is not troubled by an answer that comes too late', () async {
    final session = _session();
    session.continueAnswerTaken = false;
    await _start(session, FakeContinueReader(items: _report));

    await _ask(session, 5);

    // The library turned it down, because the device had been told already. Nothing else to do.
    expect(session.continueAnswers, hasLength(1));
  });

  test('ignores the other events', () async {
    final session = _session();
    final reader = FakeContinueReader(items: _report);
    await _start(session, reader);

    session.emit(const Event.devicesChanged());
    session.emit(const Event.resync());
    await pumpEventQueue();

    expect(reader.reads, 0);
    expect(session.continueAnswers, isEmpty);
  });

  test(
    'answers nothing once it is closed, and has let go of the events',
    () async {
      final session = _session();
      final reader = FakeContinueReader(items: _report);
      final server = await _start(session, reader);
      expect(session.listening, isTrue);

      server.dispose();
      await pumpEventQueue();
      expect(session.listening, isFalse);

      await _ask(session, 6);
      expect(reader.reads, 0);
      expect(session.continueAnswers, isEmpty);
    },
  );

  test('does not listen if it is closed before the events are ready', () async {
    final session = _session()..holdEvents();
    final reader = FakeContinueReader(items: _report);
    final server = ContinueServer(session, reader);

    final started = server.start();
    server.dispose();
    session.releaseEvents();
    await started;
    await pumpEventQueue();

    expect(session.listening, isFalse);
    await _ask(session, 8);
    expect(reader.reads, 0);
  });
}
