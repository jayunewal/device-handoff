// SPDX-License-Identifier: Apache-2.0

// Where the app keeps its files. An iPhone that runs iOS 27 starts an app without HOME in its
// environment, and Baton, which looked for HOME, did not start on it. A Mac gives HOME and the
// computer that runs these tests may or may not, so the tests give the app an environment of
// their own, and a system that answers.
// The tests of the folder itself need the library built for this computer; see bridge_test.dart.

import 'dart:io';

import 'package:baton/src/rust_backend.dart';
import 'package:baton/src/support_folder.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support.dart';

const _channel = MethodChannel(SupportFolder.channelName);

/// The environment that iOS 27 gives an app.
const _noHome = <String, String>{};

/// Makes the tests that follow run as [platform].
void _runAs(TargetPlatform platform) {
  debugDefaultTargetPlatformOverride = platform;
  addTearDown(() => debugDefaultTargetPlatformOverride = null);
}

/// Makes the iOS app answer the question about its folder with [answer], and notes the
/// questions it was asked.
List<String> _answerWith(Future<Object?> Function() answer) {
  final asked = <String>[];
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  messenger.setMockMethodCallHandler(_channel, (call) async {
    asked.add(call.method);
    return answer();
  });
  addTearDown(() => messenger.setMockMethodCallHandler(_channel, null));
  return asked;
}

/// What a [StateError] says.
Matcher _stateErrorSaying(Matcher message) =>
    throwsA(isA<StateError>().having((e) => e.message, 'message', message));

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('on an iPhone or an iPad', () {
    setUp(() => _runAs(TargetPlatform.iOS));

    test(
      'the system says where the folder is, with no HOME to go by',
      () async {
        final asked = _answerWith(
          () async => '/var/mobile/Containers/Data/Application/ID/Library/Application Support',
        );

        expect(
          await const SupportFolder(environment: _noHome).path(),
          '/var/mobile/Containers/Data/Application/ID/Library/Application Support',
        );
        expect(asked, [SupportFolder.question]);
      },
    );

    test('HOME is not looked at, whatever it says', () async {
      _answerWith(() async => '/container/Library/Application Support');

      for (final home in ['/somewhere/else', '', '/']) {
        expect(
          await SupportFolder(environment: {'HOME': home}).path(),
          '/container/Library/Application Support',
          reason: 'HOME is "$home"',
        );
      }
    });

    test('it is told when the system cannot say', () async {
      _answerWith(
        () async =>
            throw PlatformException(code: 'no_folder', message: 'no room'),
      );
      await expectLater(
        const SupportFolder(environment: _noHome).path(),
        _stateErrorSaying(
          contains('would not say where the app keeps its files: no room'),
        ),
      );

      _answerWith(() async => null);
      await expectLater(
        const SupportFolder(environment: _noHome).path(),
        _stateErrorSaying(contains('did not say where')),
      );

      _answerWith(() async => '');
      await expectLater(
        const SupportFolder(environment: _noHome).path(),
        _stateErrorSaying(contains('did not say where')),
      );
    });

    test('it is told when the app does not answer at all', () async {
      _answerWith(() async => throw MissingPluginException());

      await expectLater(
        const SupportFolder(environment: _noHome).path(),
        _stateErrorSaying(contains('cannot ask the system')),
      );
    });

    test('the folder for received files is asked of the system', () async {
      final asked = _answerWith(
        () async => '/var/mobile/Containers/Data/Application/ID/Documents',
      );

      expect(
        await const SupportFolder(environment: _noHome).receiveFolder(),
        '/var/mobile/Containers/Data/Application/ID/Documents',
      );
      expect(asked, [SupportFolder.receiveQuestion]);
    });

    test(
      'it is told when the system cannot say where received files go',
      () async {
        _answerWith(
          () async =>
              throw PlatformException(code: 'no_folder', message: 'no room'),
        );
        await expectLater(
          const SupportFolder(environment: _noHome).receiveFolder(),
          _stateErrorSaying(
            contains('would not say where received files go: no room'),
          ),
        );

        _answerWith(() async => null);
        await expectLater(
          const SupportFolder(environment: _noHome).receiveFolder(),
          _stateErrorSaying(contains('did not say where received files go')),
        );

        _answerWith(() async => '');
        await expectLater(
          const SupportFolder(environment: _noHome).receiveFolder(),
          _stateErrorSaying(contains('did not say where received files go')),
        );

        _answerWith(() async => throw MissingPluginException());
        await expectLater(
          const SupportFolder(environment: _noHome).receiveFolder(),
          _stateErrorSaying(
            contains('cannot ask the system where received files go'),
          ),
        );
      },
    );

    test('Baton keeps its files in a folder of its own inside it', () async {
      final support = await Directory.systemTemp.createTemp('baton-support-');
      addTearDown(() => support.delete(recursive: true));
      _answerWith(() async => support.path);
      final backend = RustBackend(
        supportFolder: const SupportFolder(environment: _noHome),
        library: () => ExternalLibrary.open(libraryPath()),
      );

      // Before the fix, this was "Bad state: The home folder of the app is not known."
      expect(await backend.deviceName(), isNull);
      await backend.setDeviceName('Desk iPhone');
      expect(await backend.deviceName(), 'Desk iPhone');

      final folder = Directory('${support.path}/${RustBackend.folderName}');
      expect(folder.existsSync(), isTrue);
      expect(File('${folder.path}/config.toml').existsSync(), isTrue);
    });
  });

  group('on a Mac', () {
    setUp(() => _runAs(TargetPlatform.macOS));

    test(
      'HOME says where the folder is, and the system is not asked',
      () async {
        final asked = _answerWith(() async => '/not/used');

        expect(
          await const SupportFolder(
            environment: {'HOME': '/Users/me/Library/Containers/baton/Data'},
          ).path(),
          '/Users/me/Library/Containers/baton/Data/Library/Application Support',
        );
        expect(asked, isEmpty);
      },
    );

    test('the folder for received files is asked of the system, not made from HOME', () async {
      final asked = _answerWith(() async => '/Users/me/Downloads');

      expect(
        await const SupportFolder(
          environment: {'HOME': '/Users/me/Library/Containers/baton/Data'},
        ).receiveFolder(),
        '/Users/me/Downloads',
      );
      expect(asked, [SupportFolder.receiveQuestion]);
    });

    test(
      'it is told when the system cannot say where received files go',
      () async {
        _answerWith(
          () async =>
              throw PlatformException(code: 'no_folder', message: 'no room'),
        );
        await expectLater(
          const SupportFolder(environment: {'HOME': '/Users/me'})
              .receiveFolder(),
          _stateErrorSaying(
            contains('would not say where received files go: no room'),
          ),
        );

        _answerWith(() async => null);
        await expectLater(
          const SupportFolder(environment: {'HOME': '/Users/me'})
              .receiveFolder(),
          _stateErrorSaying(contains('did not say where received files go')),
        );
      },
    );

    test('a missing or empty HOME is said to be unknown', () async {
      for (final environment in [
        _noHome,
        const {'HOME': ''},
      ]) {
        await expectLater(
          SupportFolder(environment: environment).path(),
          _stateErrorSaying(equals('The home folder of the app is not known.')),
          reason: '$environment',
        );
      }
    });
  });

  test('another system is not supported yet', () async {
    _runAs(TargetPlatform.linux);

    await expectLater(
      const SupportFolder(environment: {'HOME': '/home/me'}).path(),
      throwsUnsupportedError,
    );
    await expectLater(
      const SupportFolder(environment: {'HOME': '/home/me'}).receiveFolder(),
      throwsUnsupportedError,
    );
  });
}
