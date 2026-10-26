// SPDX-License-Identifier: Apache-2.0

import 'dart:io';

import 'package:baton/src/control_access.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(PlatformControlAccess.channelName);

/// Answers the channel with [answer], and notes what it was asked, in order.
List<String> _listen(
  WidgetTester tester,
  Object? Function(MethodCall call) answer,
) {
  final calls = <String>[];
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(_channel, (
    call,
  ) async {
    calls.add(call.method);
    return answer(call);
  });
  addTearDown(
    () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      _channel,
      null,
    ),
  );
  return calls;
}

void main() {
  group('the consent of the system to control windows', () {
    const access = PlatformControlAccess();

    test('are asked through the plugin that the questions name', () {
      expect(
        PlatformControlAccess.channelName,
        'io.github.jayunewal.baton/stream',
      );
      expect(PlatformControlAccess.stateQuestion, 'controlAccess');
      expect(PlatformControlAccess.requestQuestion, 'requestControlAccess');
    });

    testWidgets('are read as the plugin says them, on a Mac', (tester) async {
      var answer = 'allowed';
      final calls = _listen(tester, (_) => answer);

      expect(await access.state(), ControlAccess.allowed);
      answer = 'needed';
      expect(await access.state(), ControlAccess.needed);
      answer = 'unavailable';
      expect(await access.state(), ControlAccess.unavailable);

      expect(calls, ['controlAccess', 'controlAccess', 'controlAccess']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are not allowed by an answer that is not known', (
      tester,
    ) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, 42, true, '', 'yes', 'Allowed']) {
        answer = unknown;
        expect(
          await access.state(),
          ControlAccess.unavailable,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are asked for by the plugin, on a Mac', (tester) async {
      final calls = _listen(tester, (_) => 'needed');

      await access.request();

      expect(calls, ['requestControlAccess']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are unavailable when the plugin does not answer', (
      tester,
    ) async {
      // The app has no code for it, and the channel says so.
      _listen(tester, (_) => throw MissingPluginException());

      expect(await access.state(), ControlAccess.unavailable);
      await access.request();
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are unavailable when the plugin fails', (tester) async {
      _listen(tester, (_) => throw PlatformException(code: 'broken'));

      expect(await access.state(), ControlAccess.unavailable);
      await access.request();
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are not asked of any system but a Mac', (tester) async {
      final calls = _listen(tester, (_) => 'allowed');

      expect(await access.state(), ControlAccess.unavailable);
      await access.request();

      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.all(excluding: {TargetPlatform.macOS}));
  });

  group('the consent of the system to record the screen', () {
    const access = PlatformScreenAccess();

    test('are asked through the plugin, with the questions it names', () {
      expect(PlatformScreenAccess.stateQuestion, 'screenAccess');
      expect(PlatformScreenAccess.requestQuestion, 'requestScreenAccess');
    });

    testWidgets('are read as the plugin says them, on a Mac', (tester) async {
      var answer = 'allowed';
      final calls = _listen(tester, (_) => answer);

      expect(await access.state(), ScreenAccess.allowed);
      answer = 'needed';
      expect(await access.state(), ScreenAccess.needed);
      answer = 'unavailable';
      expect(await access.state(), ScreenAccess.unavailable);

      expect(calls, ['screenAccess', 'screenAccess', 'screenAccess']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are not allowed by an answer that is not known', (
      tester,
    ) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, 42, true, '', 'yes', 'Allowed']) {
        answer = unknown;
        expect(
          await access.state(),
          ScreenAccess.unavailable,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are asked for by the plugin, on a Mac', (tester) async {
      final calls = _listen(tester, (_) => 'needed');

      await access.request();

      expect(calls, ['requestScreenAccess']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are unavailable when the plugin does not answer', (
      tester,
    ) async {
      _listen(tester, (_) => throw MissingPluginException());

      expect(await access.state(), ScreenAccess.unavailable);
      await access.request();
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are unavailable when the plugin fails', (tester) async {
      _listen(tester, (_) => throw PlatformException(code: 'broken'));

      expect(await access.state(), ScreenAccess.unavailable);
      await access.request();
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are not asked of any system but a Mac', (tester) async {
      final calls = _listen(tester, (_) => 'allowed');

      expect(await access.state(), ScreenAccess.unavailable);
      await access.request();

      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.all(excluding: {TargetPlatform.macOS}));
  });

  // The Swift cannot be run here, and a word of it that the app does not say is a question that
  // nobody answers: the app would take the person's yes for a system that cannot do it. So the
  // words are looked for in the Swift, in both folders of the plugin.
  group('the plugin that answers', () {
    String read(String system, String file) =>
        File('rust_builder/$system/Classes/$file')
            .readAsStringSync()
            .replaceAll('\r\n', '\n');

    for (final system in ['macos', 'ios']) {
      test('has the channel and the questions the app asks ($system)', () {
        final plugin = read(system, 'BatonStreamPlugin.swift');

        expect(
          plugin,
          contains(
            'static let channelName = "${PlatformControlAccess.channelName}"',
          ),
        );
        expect(
          plugin,
          contains('case "${PlatformControlAccess.stateQuestion}":'),
        );
        expect(
          plugin,
          contains('case "${PlatformControlAccess.requestQuestion}":'),
        );
        expect(
          plugin,
          contains('case "${PlatformScreenAccess.stateQuestion}":'),
        );
        expect(
          plugin,
          contains('case "${PlatformScreenAccess.requestQuestion}":'),
        );
      });

      test('says only the words the app reads ($system)', () {
        final rules = read(system, 'BatonInput.swift');

        final body = RegExp(r'enum ControlAccess: String[^{]*\{([^}]*)\}')
            .firstMatch(rules)
            ?.group(1);
        expect(body, isNotNull, reason: 'the enum of the answers is gone');
        final words = RegExp(
          r'^\s*case (\w+)',
          multiLine: true,
        ).allMatches(body!).map((match) => match.group(1)).toSet();
        expect(words, ControlAccess.values.map((state) => state.name).toSet());
      });

      test('says only the words the app reads of the screen ($system)', () {
        final rules = read(system, 'BatonInput.swift');

        final body = RegExp(r'enum ScreenAccess: String[^{]*\{([^}]*)\}')
            .firstMatch(rules)
            ?.group(1);
        expect(body, isNotNull, reason: 'the enum of the answers is gone');
        final words = RegExp(
          r'^\s*case (\w+)',
          multiLine: true,
        ).allMatches(body!).map((match) => match.group(1)).toSet();
        expect(words, ScreenAccess.values.map((state) => state.name).toSet());
      });
    }
  });
}
