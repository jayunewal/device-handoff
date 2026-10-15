// SPDX-License-Identifier: Apache-2.0

// The class that connects the app to the Rust library, with the library itself behind it. It
// needs the library built for this computer; see bridge_test.dart.

import 'package:baton/src/backend.dart';
import 'package:baton/src/rust_backend.dart';
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
}
