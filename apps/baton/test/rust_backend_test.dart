// SPDX-License-Identifier: Apache-2.0

// The class that connects the app to the Rust library, with the library itself behind it. It
// needs the library built for this computer; see bridge_test.dart.

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
}
