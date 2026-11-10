// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/always_on.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(PlatformAlwaysOnKeeper.channelName);

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
  group('what keeps Baton running on a Mac', () {
    const keeper = PlatformAlwaysOnKeeper();

    test('is asked through the channel that the Mac app answers', () {
      expect(
        PlatformAlwaysOnKeeper.channelName,
        'io.github.jayunewal.baton/always_on',
      );
      expect(PlatformAlwaysOnKeeper.signInQuestion, 'signIn');
      expect(PlatformAlwaysOnKeeper.setSignInQuestion, 'setSignIn');
      expect(
        PlatformAlwaysOnKeeper.openSignInSettingsQuestion,
        'openSignInSettings',
      );
      expect(PlatformAlwaysOnKeeper.readyQuestion, 'ready');
      expect(PlatformAlwaysOnKeeper.lookingQuestion, 'looking');
    });

    testWidgets('says whether Baton opens at sign-in, as the app says it', (
      tester,
    ) async {
      var answer = 'on';
      final calls = _listen(tester, (_) => answer);

      expect(await keeper.signIn(), SignInOpening.on);
      answer = 'off';
      expect(await keeper.signIn(), SignInOpening.off);
      answer = 'needsApproval';
      expect(await keeper.signIn(), SignInOpening.needsApproval);
      answer = 'unavailable';
      expect(await keeper.signIn(), SignInOpening.unavailable);

      expect(
        [for (final call in calls) call.method],
        ['signIn', 'signIn', 'signIn', 'signIn'],
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('does not open at sign-in on an answer that is not known', (
      tester,
    ) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, 42, true, '', 'yes', 'On']) {
        answer = unknown;
        expect(
          await keeper.signIn(),
          SignInOpening.unavailable,
          reason: '$unknown',
        );
        expect(
          await keeper.setSignIn(true),
          SignInOpening.unavailable,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('turns it on and off, and says where it then stands', (
      tester,
    ) async {
      var answer = 'on';
      final calls = _listen(tester, (_) => answer);

      expect(await keeper.setSignIn(true), SignInOpening.on);
      answer = 'needsApproval';
      expect(await keeper.setSignIn(true), SignInOpening.needsApproval);
      answer = 'off';
      expect(await keeper.setSignIn(false), SignInOpening.off);

      expect(
        [for (final call in calls) call.method],
        ['setSignIn', 'setSignIn', 'setSignIn'],
      );
      expect([for (final call in calls) call.arguments], [true, true, false]);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('passes on what the system said when it refuses', (
      tester,
    ) async {
      _listen(
        tester,
        (_) => throw PlatformException(
          code: 'sign_in_failed',
          message: 'Operation not permitted',
        ),
      );

      await expectLater(
        keeper.setSignIn(true),
        throwsA(
          isA<AlwaysOnFailure>().having(
            (e) => e.message,
            'message',
            'Operation not permitted',
          ),
        ),
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('says something when the system says nothing', (tester) async {
      _listen(tester, (_) => throw PlatformException(code: 'sign_in_failed'));

      await expectLater(
        keeper.setSignIn(true),
        throwsA(
          isA<AlwaysOnFailure>().having(
            (e) => e.message,
            'message',
            'The system did not say why.',
          ),
        ),
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('opens Settings, and says whether it did', (tester) async {
      Object? answer = true;
      final calls = _listen(tester, (_) => answer);

      expect(await keeper.openSignInSettings(), isTrue);
      answer = false;
      expect(await keeper.openSignInSettings(), isFalse);
      answer = 'true';
      expect(await keeper.openSignInSettings(), isFalse);

      expect(
        [for (final call in calls) call.method],
        ['openSignInSettings', 'openSignInSettings', 'openSignInSettings'],
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('says that the app is up', (tester) async {
      final calls = _listen(tester, (_) => null);

      await keeper.ready();

      expect([for (final call in calls) call.method], ['ready']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('says whether the person can see Baton, as the app says it', (
      tester,
    ) async {
      Object? answer = true;
      final calls = _listen(tester, (_) => answer);

      expect(await keeper.looking(), isTrue);
      answer = false;
      expect(await keeper.looking(), isFalse);

      expect([for (final call in calls) call.method], ['looking', 'looking']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('takes the person for one who cannot see Baton when the app '
        'does not say that they can', (tester) async {
      Object? answer;
      _listen(tester, (_) => answer);

      // A notification that was not needed is better than none that was.
      for (final unknown in [null, 42, 'yes', 'true', '']) {
        answer = unknown;
        expect(await keeper.looking(), isFalse, reason: '$unknown');
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('cannot be asked when the app has no code for it, or fails', (
      tester,
    ) async {
      _listen(tester, (_) => throw MissingPluginException());
      expect(await keeper.signIn(), SignInOpening.unavailable);
      expect(await keeper.setSignIn(true), SignInOpening.unavailable);
      expect(await keeper.openSignInSettings(), isFalse);
      expect(await keeper.looking(), isFalse);
      await keeper.ready();

      _listen(tester, (_) => throw PlatformException(code: 'broken'));
      expect(await keeper.signIn(), SignInOpening.unavailable);
      expect(await keeper.openSignInSettings(), isFalse);
      expect(await keeper.looking(), isFalse);
      await keeper.ready();
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('is not asked of any system but a Mac', (tester) async {
      final calls = _listen(tester, (_) => 'on');

      expect(await keeper.signIn(), SignInOpening.unavailable);
      expect(await keeper.setSignIn(true), SignInOpening.unavailable);
      expect(await keeper.openSignInSettings(), isFalse);
      await keeper.ready();
      // Where there is no window to hide, the person is taken to be looking, so that no
      // notification is made for a window that is there.
      expect(await keeper.looking(), isTrue);

      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.all(excluding: {TargetPlatform.macOS}));
  });
}
