// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

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

/// What the iPhone app does when something was handed to it: it says [method] to the channel, and
/// the answer is what it got back. Something must be listening: a message that nothing listens to
/// is held for whatever listens next, and is never answered.
Future<ByteData?> _say(WidgetTester tester, String method) {
  final answer = Completer<ByteData?>();
  tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    PlatformShareKeeper.channelName,
    _channel.codec.encodeMethodCall(MethodCall(method)),
    answer.complete,
  );
  return answer.future;
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
      expect(PlatformShareKeeper.news, 'handed');
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

    testWidgets('is said to have come each time the app says so', (
      tester,
    ) async {
      final heard = <void>[];
      final listening = keeper.handed.listen(heard.add);
      addTearDown(listening.cancel);

      await _say(tester, PlatformShareKeeper.news);
      await _say(tester, PlatformShareKeeper.news);
      await tester.pump();

      expect(heard, hasLength(2));
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is not told of by anything but the news', (tester) async {
      final heard = <void>[];
      final listening = keeper.handed.listen(heard.add);
      addTearDown(listening.cancel);

      final answer = await _say(tester, 'something else');
      await tester.pump();

      expect(heard, isEmpty);
      expect(
        answer,
        isNull,
        reason: 'what the app is told is that Baton does not know it',
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is told to every one that listens, and to none that left', (
      tester,
    ) async {
      final first = <void>[];
      final second = <void>[];
      final one = keeper.handed.listen(first.add);
      final two = keeper.handed.listen(second.add);
      addTearDown(two.cancel);

      await _say(tester, PlatformShareKeeper.news);
      await tester.pump();
      // Not waited for: the future of a cancel is not one of the test's own, which the test does
      // not run.
      unawaited(one.cancel());
      await _say(tester, PlatformShareKeeper.news);
      await tester.pump();

      expect(first, hasLength(1));
      expect(second, hasLength(2));
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is listened for only while something listens', (tester) async {
      expect(
        await _say(tester, PlatformShareKeeper.news),
        isNull,
        reason: 'nothing answers before anyone listens',
      );

      final listening = keeper.handed.listen((_) {});
      expect(
        await _say(tester, PlatformShareKeeper.news),
        isNotNull,
        reason: 'the channel is answered while it is listened to',
      );

      // Not waited for: the future of a cancel is not one of the test's own, which the test does
      // not run.
      unawaited(listening.cancel());
      expect(
        await _say(tester, PlatformShareKeeper.news),
        isNull,
        reason: 'and is let go of when the last one has left',
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets(
      'is not listened for on a system that cannot be handed anything',
      (tester) async {
        var closed = false;
        final listening = keeper.handed.listen(
          (_) {},
          onDone: () => closed = true,
        );
        addTearDown(listening.cancel);
        await tester.pump();

        expect(closed, isTrue, reason: 'there is nothing to wait for');
        expect(
          await _say(tester, PlatformShareKeeper.news),
          isNull,
          reason: 'and nothing listens to the channel',
        );
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );
  });
}
