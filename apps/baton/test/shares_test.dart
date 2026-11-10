// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/shares.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(PlatformShareKeeper.channelName);

/// Answers the channel with [answer], and notes what it was asked, in order.
List<MethodCall> _listen(
  WidgetTester tester,
  Object? Function(MethodCall call) answer,
) {
  final calls = <MethodCall>[];
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(_channel, (
    call,
  ) async {
    calls.add(call);
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
  group('what a shortcut handed to Baton', () {
    const keeper = PlatformShareKeeper();

    test('is asked through the channel that the iPhone app answers', () {
      expect(
        PlatformShareKeeper.channelName,
        'io.github.jayunewal.baton/shares',
      );
      expect(PlatformShareKeeper.question, 'take');
    });

    testWidgets('is taken from the app, oldest first', (tester) async {
      final calls = _listen(
        tester,
        (_) => ['https://example.com/a', 'some words'],
      );

      expect(await keeper.take(), ['https://example.com/a', 'some words']);

      expect([for (final call in calls) call.method], ['take']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is nothing when nothing was handed over', (tester) async {
      _listen(tester, (_) => <Object?>[]);
      expect(await keeper.take(), isEmpty);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('leaves out an answer that is not text, or has none in it', (
      tester,
    ) async {
      _listen(
        tester,
        (_) => <Object?>['kept', '', '   \n', 42, null, true, 'kept too'],
      );
      expect(await keeper.take(), ['kept', 'kept too']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is nothing for an answer of another kind', (tester) async {
      for (final unusual in [null, 'text', 7, <String, Object>{}]) {
        _listen(tester, (_) => unusual);
        expect(await keeper.take(), isEmpty, reason: '$unusual');
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets(
      'is nothing, and not an error, when the app has no code for it',
      (tester) async {
        _listen(tester, (_) => throw MissingPluginException());
        expect(await keeper.take(), isEmpty);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );

    testWidgets('is nothing, and not an error, when the app fails', (
      tester,
    ) async {
      _listen(tester, (_) => throw PlatformException(code: 'broken'));
      expect(await keeper.take(), isEmpty);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is not asked of a system that cannot be handed anything', (
      tester,
    ) async {
      final calls = _listen(tester, (_) => ['https://example.com/']);

      expect(await keeper.take(), isEmpty);

      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));
  });
}
