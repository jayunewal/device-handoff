// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/notice_access.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(PlatformNoticeKeeper.channelName);

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
  group('the notifications of the system', () {
    const notices = PlatformNoticeKeeper();

    test('are asked through the channel that the Mac app answers', () {
      expect(
        PlatformNoticeKeeper.channelName,
        'io.github.jayunewal.baton/notices',
      );
      expect(PlatformNoticeKeeper.stateQuestion, 'state');
      expect(PlatformNoticeKeeper.requestQuestion, 'request');
      expect(PlatformNoticeKeeper.postQuestion, 'post');
    });

    testWidgets('are read as the app says them, on a Mac', (tester) async {
      var answer = 'allowed';
      final calls = _listen(tester, (_) => answer);

      expect(await notices.state(), NoticeAccess.allowed);
      answer = 'needed';
      expect(await notices.state(), NoticeAccess.needed);
      answer = 'denied';
      expect(await notices.state(), NoticeAccess.denied);
      answer = 'unavailable';
      expect(await notices.state(), NoticeAccess.unavailable);

      expect(
        [for (final call in calls) call.method],
        ['state', 'state', 'state', 'state'],
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are not allowed by an answer that is not known', (
      tester,
    ) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, 42, true, '', 'yes', 'Allowed']) {
        answer = unknown;
        expect(
          await notices.state(),
          NoticeAccess.unavailable,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are asked for by the app, and its answer is the new state', (
      tester,
    ) async {
      final calls = _listen(tester, (_) => 'denied');

      expect(await notices.request(), NoticeAccess.denied);

      expect([for (final call in calls) call.method], ['request']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets(
      'show a title and a body, and say whether the system took them',
      (tester) async {
        var taken = true;
        final calls = _listen(tester, (_) => taken);

        expect(await notices.post(title: 'A title', body: 'A body'), isTrue);
        taken = false;
        expect(await notices.post(title: 'Another', body: 'Body'), isFalse);

        expect(calls.first.method, 'post');
        expect(calls.first.arguments, {'title': 'A title', 'body': 'A body'});
        expect(calls.last.arguments, {'title': 'Another', 'body': 'Body'});
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );

    testWidgets('are not shown on an answer that is not a yes', (tester) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, 'true', 1, 'yes']) {
        answer = unknown;
        expect(
          await notices.post(title: 't', body: 'b'),
          isFalse,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are unavailable when the app does not answer', (tester) async {
      // The app has no code for it, and the channel says so.
      _listen(tester, (_) => throw MissingPluginException());

      expect(await notices.state(), NoticeAccess.unavailable);
      expect(await notices.request(), NoticeAccess.unavailable);
      expect(await notices.post(title: 't', body: 'b'), isFalse);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are unavailable when the app fails', (tester) async {
      _listen(tester, (_) => throw PlatformException(code: 'broken'));

      expect(await notices.state(), NoticeAccess.unavailable);
      expect(await notices.request(), NoticeAccess.unavailable);
      expect(await notices.post(title: 't', body: 'b'), isFalse);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('are not asked of any system but a Mac', (tester) async {
      final calls = _listen(tester, (_) => 'allowed');

      expect(await notices.state(), NoticeAccess.unavailable);
      expect(await notices.request(), NoticeAccess.unavailable);
      expect(await notices.post(title: 't', body: 'b'), isFalse);

      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.all(excluding: {TargetPlatform.macOS}));
  });
}
