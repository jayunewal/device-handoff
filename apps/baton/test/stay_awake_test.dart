// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/stay_awake.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(PlatformStayAwakeKeeper.channelName);

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
  group('staying awake while plugged in', () {
    const keeper = PlatformStayAwakeKeeper();

    test('is asked through the channel that the Mac app answers', () {
      expect(
        PlatformStayAwakeKeeper.channelName,
        'io.github.jayunewal.baton/stay_awake',
      );
      expect(PlatformStayAwakeKeeper.stateQuestion, 'state');
      expect(PlatformStayAwakeKeeper.setQuestion, 'set');
    });

    testWidgets('says where it stands, in the words of the Mac app', (
      tester,
    ) async {
      var answer = 'off';
      final calls = _listen(tester, (_) => answer);

      expect(await keeper.state(), StayAwake.off);
      answer = 'holding';
      expect(await keeper.state(), StayAwake.holding);
      answer = 'waiting';
      expect(await keeper.state(), StayAwake.waiting);
      answer = 'refused';
      expect(await keeper.state(), StayAwake.refused);

      expect(
        [for (final call in calls) call.method],
        ['state', 'state', 'state', 'state'],
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('cannot be asked of an answer that is not known', (
      tester,
    ) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, 42, true, '', 'yes', 'Off', 'unavailable']) {
        answer = unknown;
        expect(await keeper.state(), StayAwake.unavailable, reason: '$unknown');
        expect(
          await keeper.set(true),
          StayAwake.unavailable,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets(
      'asks for it and lets go of it, and says where it then stands',
      (tester) async {
        var answer = 'holding';
        final calls = _listen(tester, (_) => answer);

        expect(await keeper.set(true), StayAwake.holding);
        answer = 'waiting';
        expect(await keeper.set(true), StayAwake.waiting);
        answer = 'off';
        expect(await keeper.set(false), StayAwake.off);

        expect([for (final call in calls) call.method], ['set', 'set', 'set']);
        expect([for (final call in calls) call.arguments], [true, true, false]);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );

    testWidgets('says where it stands when a change was not answered', (
      tester,
    ) async {
      // A Mac app that does not know the change has changed nothing: what the person is told is the
      // state that the Mac is in, and not that they got what they asked for.
      final calls = _listen(
        tester,
        (call) => call.method == 'state' ? 'off' : null,
      );

      expect(await keeper.set(true), StayAwake.off);

      expect([for (final call in calls) call.method], ['set', 'state']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('cannot be asked when the app has no code for it, or fails', (
      tester,
    ) async {
      _listen(tester, (_) => throw MissingPluginException());
      expect(await keeper.state(), StayAwake.unavailable);
      expect(await keeper.set(true), StayAwake.unavailable);

      _listen(tester, (_) => throw PlatformException(code: 'broken'));
      expect(await keeper.state(), StayAwake.unavailable);
      expect(await keeper.set(true), StayAwake.unavailable);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('is not asked of any system but a Mac', (tester) async {
      final calls = _listen(tester, (_) => 'holding');

      expect(await keeper.state(), StayAwake.unavailable);
      expect(await keeper.set(true), StayAwake.unavailable);

      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.all(excluding: {TargetPlatform.macOS}));
  });
}
