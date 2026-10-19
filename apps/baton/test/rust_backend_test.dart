// SPDX-License-Identifier: Apache-2.0

// The class that connects the app to the Rust library, with the library itself behind it. It
// needs the library built for this computer; see bridge_test.dart.

import 'dart:async';
import 'dart:io';

import 'package:baton/src/backend.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:baton/src/support_folder.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support.dart';

void main() {
  // Keep this first: once the library has loaded it stays loaded in this process.
  test('a library that cannot be loaded can be tried for again', () async {
    final broken = RustBackend(
      dataDir: '/nonexistent',
      library: () => throw ArgumentError('no such library'),
    );
    await expectLater(broken.deviceName(), throwsArgumentError);

    // The next try finds it.
    final folder = await deviceFolder();
    final backend = backendIn(folder);
    expect(await backend.deviceName(), isNull);
  });

  test('chooses a name, starts Baton under it and stops it', () async {
    final folder = await deviceFolder();
    final backend = backendIn(folder);

    expect(await backend.deviceName(), isNull);
    await expectLater(
      backend.setDeviceName('   '),
      throwsA(
        isA<BatonError>().having((e) => e.kind, 'kind', ErrorKind.badName),
      ),
    );
    expect(await backend.deviceName(), isNull);

    await backend.setDeviceName('Desk Linux');
    expect(await backend.deviceName(), 'Desk Linux');
    expect(backend.suggestedName(), isNotEmpty);

    final session = await backend.start();
    expect(session.me.name, 'Desk Linux');
    expect(session.me.platform, Platform.linux);
    expect(await session.devices(), isEmpty);
    expect((await session.network()).lanDiscovery, isFalse);

    // The folder belongs to the running Baton until it stops.
    await expectLater(
      backendIn(folder).start(),
      throwsA(
        isA<BatonError>().having(
          (e) => e.kind,
          'kind',
          ErrorKind.alreadyRunning,
        ),
      ),
    );
    final ended = (await session.events()).isEmpty;
    await session.stop();
    expect(await ended, isTrue, reason: 'the events end when Baton stops');
    final again = await backendIn(folder).start();
    expect(again.me.id, session.me.id, reason: 'the same device after a stop');
    await again.stop();
  });

  test('a system that cannot say where received files go takes none', () async {
    // This computer is not one that Baton runs on yet, so the folder for received files is not
    // known. Pairing and sending do not need it, so Baton starts, and says why it takes no files.
    final backend = backendIn(await deviceFolder());

    final session = await backend.start();
    addTearDown(session.stop);

    expect(
      session.receiving(),
      isA<Receiving_Unusable>().having(
        (receiving) => receiving.message,
        'message',
        'Baton does not run on this system yet.',
      ),
    );
    expect(session.receiveFolder, isNull);
  });

  test(
    'a system that will not say where received files go takes none',
    () async {
      TestWidgetsFlutterBinding.ensureInitialized();
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      const channel = MethodChannel(SupportFolder.channelName);
      messenger.setMockMethodCallHandler(channel, (call) async {
        throw PlatformException(code: 'no_folder', message: 'no room');
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      final backend = backendIn(await deviceFolder());

      final session = await backend.start();
      addTearDown(session.stop);

      expect(
        session.receiving(),
        isA<Receiving_Unusable>().having(
          (receiving) => receiving.message,
          'message',
          contains('would not say where received files go: no room'),
        ),
      );
      expect(session.receiveFolder, isNull);
    },
  );

  test('a folder for received files that is given is used', () async {
    final received = await Directory.systemTemp.createTemp('baton-received-');
    addTearDown(() => received.delete(recursive: true));
    final backend = backendIn(await deviceFolder(), receiveDir: received);

    final session = await backend.start();
    addTearDown(session.stop);

    expect(session.receiving(), isA<Receiving_On>());
    expect(session.receiveFolder, received.path);
  });

  test('a folder for received files that cannot be used is said so', () async {
    final received = await Directory.systemTemp.createTemp('baton-received-');
    addTearDown(() => received.delete(recursive: true));
    final backend = backendIn(
      await deviceFolder(),
      receiveDir: Directory('${received.path}/not-there'),
    );

    final session = await backend.start();
    addTearDown(session.stop);

    expect(
      session.receiving(),
      isA<Receiving_Unusable>().having(
        (receiving) => receiving.message,
        'message',
        'The folder for received files is not there.',
      ),
    );
    expect(session.receiveFolder, isNull);
  });

  group('files between two devices', () {
    test('a request is taken or refused as the person answers it', () async {
      final received = await _emptyFolder();
      final taker = await _device('Device A', receiveDir: received);
      final sender = await _device('Device B');
      final events = StreamIterator(await taker.events());
      addTearDown(() => unawaited(events.cancel()));
      await _pair(taker, events, sender);
      final file = await _file('hello.txt', 'hello from B');

      // A no keeps nothing, and the sender is told.
      final refused = await sender.sendFiles(taker.me.id, [file.path]);
      final asked = (await _next<Event_TransferRequested>(events)).request;
      expect(taker.answerTransfer(asked.id, accept: false), isTrue);
      await expectLater(
        refused.outcome(),
        throwsA(
          isA<BatonError>().having((e) => e.kind, 'kind', ErrorKind.declined),
        ),
      );
      expect(_kept(received), isEmpty);

      // A yes takes the files in, as they were.
      final sending = await sender.sendFiles(taker.me.id, [file.path]);
      final request = (await _next<Event_TransferRequested>(events)).request;
      expect(taker.answerTransfer(request.id, accept: true), isTrue);
      final sent = await sending.outcome();
      expect((sent.files, sent.bytes, sent.failed), (1, 12, 0));
      expect(
        File('${received.path}/hello.txt').readAsStringSync(),
        'hello from B',
      );
    });

    test(
      'a send that is given up takes the question off the other screen',
      () async {
        final received = await _emptyFolder();
        final taker = await _device('Device A', receiveDir: received);
        final sender = await _device('Device B');
        final events = StreamIterator(await taker.events());
        addTearDown(() => unawaited(events.cancel()));
        await _pair(taker, events, sender);
        final file = await _file('hello.txt', 'hello from B');
        final sending = await sender.sendFiles(taker.me.id, [file.path]);
        final request = (await _next<Event_TransferRequested>(events)).request;

        sending.cancel();

        expect(
          (await _next<Event_TransferRequestEnded>(events)).id,
          request.id,
        );
        await expectLater(
          sending.outcome(),
          throwsA(
            isA<BatonError>().having(
              (e) => e.kind,
              'kind',
              ErrorKind.cancelled,
            ),
          ),
        );
        expect(_kept(received), isEmpty);
      },
    );
  });
}

/// How long to wait for something that should happen at once.
const _wait = Duration(seconds: 30);

/// Starts Baton under [name] in a folder of its own, and stops it when the test is over. It puts
/// the files that other devices send in [receiveDir], and takes none without one.
Future<Session> _device(String name, {Directory? receiveDir}) async {
  final backend = backendIn(await deviceFolder(), receiveDir: receiveDir);
  await backend.setDeviceName(name);
  final session = await backend.start();
  addTearDown(session.stop);
  return session;
}

/// An empty folder that is deleted when the test is over.
Future<Directory> _emptyFolder() async {
  final folder = await Directory.systemTemp.createTemp('baton-backend-');
  addTearDown(() => folder.delete(recursive: true));
  return folder;
}

/// A file with [text] in it, in a folder that is deleted when the test is over.
Future<File> _file(String name, String text) async {
  final folder = await _emptyFolder();
  return File('${folder.path}/$name').writeAsString(text);
}

/// What is in [folder], by name, leaving out the folders that Baton makes for files that are on
/// their way.
List<String> _kept(Directory folder) => [
  for (final entry in folder.listSync())
    if (!entry.path
        .substring(folder.path.length + 1)
        .startsWith('.baton-incoming-'))
      entry.path.substring(folder.path.length + 1),
];

/// The next event that is an [E], skipping any others. Fails if none comes in time, or the
/// stream ends first.
Future<E> _next<E extends Event>(StreamIterator<Event> events) async {
  while (await events.moveNext().timeout(_wait)) {
    final event = events.current;
    if (event is E) {
      return event;
    }
  }
  throw StateError('The stream ended while waiting for a $E.');
}

/// Pairs [joiner] with [offerer], who makes the offer and hears of it on [events].
Future<void> _pair(
  Session offerer,
  StreamIterator<Event> events,
  Session joiner,
) async {
  final offer = await offerer.offerPairing();
  final joining = StreamIterator(joiner.joinPairing(offer.ticket));
  addTearDown(() => unawaited(joining.cancel()));
  final asked = await _next<Event_PairingRequested>(events);
  expect(offerer.answerPairing(asked.request.id, accept: true), isTrue);
  while (await joining.moveNext().timeout(_wait)) {
    if (joining.current is JoinProgress_Paired) {
      return;
    }
  }
  fail('The joining ended without the devices being paired.');
}
