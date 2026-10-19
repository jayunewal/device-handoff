// SPDX-License-Identifier: Apache-2.0

// Taking the person to the files that arrived: what the app asks of the system, and what it makes
// of the answers. The system is Finder on a Mac and the Files app on an iPhone or an iPad, asked
// by the code in macos/Runner/MainFlutterWindow.swift and ios/Runner/AppDelegate.swift. Nothing
// here can run that code, so the integration tests ask it for a file that is not there.

import 'package:baton/src/file_shower.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(SystemFileShower.channelName);

/// Makes the app answer with [answer], and notes what it was asked: the question, then the paths.
List<List<String>> _answerWith(Future<Object?> Function() answer) {
  final asked = <List<String>>[];
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  messenger.setMockMethodCallHandler(_channel, (call) async {
    asked.add([call.method, ...List<String>.from(call.arguments as List)]);
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

  test(
    'the system is asked to show the files at the paths, in order',
    () async {
      final asked = _answerWith(() async => true);

      final shown = await const SystemFileShower().show([
        '/Users/anna/Downloads/Holiday',
        '/Users/anna/Downloads/notes 2.txt',
      ]);

      expect(shown, isTrue);
      expect(asked, [
        [
          SystemFileShower.question,
          '/Users/anna/Downloads/Holiday',
          '/Users/anna/Downloads/notes 2.txt',
        ],
      ]);
    },
  );

  test('files that are not there any more are said to be gone', () async {
    _answerWith(() async => false);

    expect(
      await const SystemFileShower().show(['/Users/anna/Downloads/a.txt']),
      isFalse,
    );
  });

  test('with nothing to show the system is not asked', () async {
    final asked = _answerWith(() async => true);

    expect(await const SystemFileShower().show([]), isFalse);
    expect(asked, isEmpty);
  });

  test('it is told when the system cannot be asked', () async {
    _answerWith(
      () async => throw PlatformException(
        code: 'not_opened',
        message: 'The Files app did not open.',
      ),
    );
    await expectLater(
      const SystemFileShower().show(['/a']),
      _stateErrorSaying(
        equals(
          'The system would not show the files: The Files app did not open.',
        ),
      ),
    );

    _answerWith(() async => throw PlatformException(code: 'bad_paths'));
    await expectLater(
      const SystemFileShower().show(['/a']),
      _stateErrorSaying(contains('would not show the files: bad_paths')),
    );

    _answerWith(() async => null);
    await expectLater(
      const SystemFileShower().show(['/a']),
      _stateErrorSaying(contains('did not say whether')),
    );
  });

  test('it is told when the app does not answer at all', () async {
    _answerWith(() async => throw MissingPluginException());

    await expectLater(
      const SystemFileShower().show(['/a']),
      _stateErrorSaying(contains('cannot ask the system to show files')),
    );
  });
}
