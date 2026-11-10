// SPDX-License-Identifier: Apache-2.0

import 'dart:io';

import 'package:baton/src/owner_check.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(PlatformOwnerCheck.channelName);

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
  group('the check that it is the person at this device', () {
    const check = PlatformOwnerCheck();

    test('is asked through the app, with the question and codes it names', () {
      expect(PlatformOwnerCheck.channelName, 'io.github.jayunewal.baton/owner');
      expect(PlatformOwnerCheck.question, 'ask');
      expect(PlatformOwnerCheck.cancelledCode, 'cancelled');
      expect(PlatformOwnerCheck.notRecognisedCode, 'not_recognised');
      expect(PlatformOwnerCheck.noLockCode, 'no_lock');
    });

    test('lets only a yes, or a system that asks nothing, go on', () {
      expect(OwnerCheckResult.confirmed.goesOn, isTrue);
      expect(OwnerCheckResult.notNeeded.goesOn, isTrue);
      for (final result in [
        OwnerCheckResult.cancelled,
        OwnerCheckResult.notRecognised,
        OwnerCheckResult.noLock,
        OwnerCheckResult.failed,
      ]) {
        expect(result.goesOn, isFalse, reason: result.name);
      }
    });

    testWidgets('is confirmed when the system says yes, on an iPhone or iPad', (
      tester,
    ) async {
      final calls = _listen(tester, (_) => true);

      expect(
        await check.ask(reason: 'Look at the screen of Anna’s Mac'),
        OwnerCheckResult.confirmed,
      );

      expect(calls, hasLength(1));
      expect(calls.single.method, 'ask');
      expect(calls.single.arguments, 'Look at the screen of Anna’s Mac');
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is asked again each time, and is not remembered', (
      tester,
    ) async {
      final calls = _listen(tester, (_) => true);

      await check.ask(reason: 'one');
      await check.ask(reason: 'two');

      expect(calls.map((call) => call.arguments), ['one', 'two']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('tells a cancel, a face that is not recognised and no passcode '
        'apart', (tester) async {
      var code = 'cancelled';
      _listen(tester, (_) => throw PlatformException(code: code));

      expect(await check.ask(reason: 'x'), OwnerCheckResult.cancelled);
      code = 'not_recognised';
      expect(await check.ask(reason: 'x'), OwnerCheckResult.notRecognised);
      code = 'no_lock';
      expect(await check.ask(reason: 'x'), OwnerCheckResult.noLock);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is not confirmed by anything that is not a clear yes', (
      tester,
    ) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, false, 'yes', 1, 'true', '']) {
        answer = unknown;
        expect(
          await check.ask(reason: 'x'),
          OwnerCheckResult.failed,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is not confirmed when the system fails or has no answer', (
      tester,
    ) async {
      var failure = PlatformException(code: 'failed');
      _listen(tester, (_) => throw failure);

      expect(await check.ask(reason: 'x'), OwnerCheckResult.failed);
      failure = PlatformException(code: 'bad_arguments');
      expect(await check.ask(reason: 'x'), OwnerCheckResult.failed);
      failure = PlatformException(code: 'something the app made up');
      expect(await check.ask(reason: 'x'), OwnerCheckResult.failed);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets(
      'is not confirmed when nothing in the app answers the question',
      (tester) async {
        // The app has no code for it, and the channel says so.
        _listen(tester, (_) => throw MissingPluginException());

        expect(await check.ask(reason: 'x'), OwnerCheckResult.failed);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );

    testWidgets('asks nothing of a system that is not an iPhone or an iPad', (
      tester,
    ) async {
      final calls = _listen(tester, (_) => true);

      expect(await check.ask(reason: 'x'), OwnerCheckResult.notNeeded);

      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.all(excluding: {TargetPlatform.iOS}));
  });

  group('the words of the check', () {
    test(
      'say what is asked for, with the name of the device made fit to show',
      () {
        expect(
          ownerReason('Anna’s Mac', control: false),
          'Look at the screen of Anna’s Mac',
        );
        expect(
          ownerReason('Anna’s Mac', control: true),
          'Look at the screen of Anna’s Mac and control it',
        );
        expect(
          ownerReason('Mac\u202Eevil', control: false),
          'Look at the screen of Mac\uFFFDevil',
        );
      },
    );

    test('are left unsaid for a yes, for nothing asked and for a cancel', () {
      for (final result in [
        OwnerCheckResult.confirmed,
        OwnerCheckResult.notNeeded,
        OwnerCheckResult.cancelled,
      ]) {
        expect(
          describeOwnerRefusal(result, 'Anna’s Mac'),
          isNull,
          reason: result.name,
        );
      }
    });

    test('say that nothing was asked of the device, and why', () {
      expect(
        describeOwnerRefusal(OwnerCheckResult.notRecognised, 'Anna’s Mac'),
        'This device could not confirm that it is you, so Anna’s Mac was not '
        'asked for its screen.',
      );
      expect(
        describeOwnerRefusal(OwnerCheckResult.failed, 'Anna’s Mac'),
        'This device could not check that it is you, so Anna’s Mac was not '
        'asked for its screen.',
      );
      final noLock = describeOwnerRefusal(
        OwnerCheckResult.noLock,
        'Anna’s Mac',
      );
      expect(noLock, contains('no passcode'));
      expect(noLock, contains('set a passcode in Settings'));
      expect(noLock, contains('Anna’s Mac'));
    });

    test('show the name of the device made fit to show', () {
      for (final result in [
        OwnerCheckResult.notRecognised,
        OwnerCheckResult.noLock,
        OwnerCheckResult.failed,
      ]) {
        final sentence = describeOwnerRefusal(result, 'Mac\u202Eevil')!;
        expect(sentence, contains('Mac\uFFFDevil'), reason: result.name);
        expect(sentence, isNot(contains('\u202E')), reason: result.name);
      }
    });
  });

  group('the iPhone app', () {
    final swift = File('ios/Runner/AppDelegate.swift').readAsStringSync();

    test('answers the question on the channel, with the codes of the app', () {
      // If the names differ, nothing answers, and every screen that is asked for is refused as
      // one that could not be checked: safe, and of no use.
      expect(swift, contains('name: "${PlatformOwnerCheck.channelName}"'));
      expect(swift, contains('case "${PlatformOwnerCheck.question}":'));
      for (final code in [
        PlatformOwnerCheck.cancelledCode,
        PlatformOwnerCheck.notRecognisedCode,
        PlatformOwnerCheck.noLockCode,
      ]) {
        expect(swift, contains('FlutterError(code: "$code"'), reason: code);
      }
    });

    test('registers the channel with the others', () {
      expect(
        swift,
        contains(
          '    checkTheOwner(messenger: engineBridge.applicationRegistrar.messenger())',
        ),
      );
    });

    test('asks for the face or the passcode, afresh for each question', () {
      // A face that was recognised a moment ago must not stand for this question, and a device
      // that is held by someone else has to be asked, whatever the last person did.
      expect(swift, contains('evaluatePolicy(.deviceOwnerAuthentication,'));
      expect(swift, contains('canEvaluatePolicy(.deviceOwnerAuthentication,'));
      expect(
        swift,
        contains('touchIDAuthenticationAllowableReuseDuration = 0'),
      );
      final body = swift.substring(swift.indexOf('private enum OwnerCheck {'));
      expect(
        body.substring(0, body.indexOf('private func answerAboutPermissions')),
        contains('let context = LAContext()'),
        reason: 'a context of its own for each question',
      );
    });

    test('says why it uses Face ID for this too', () {
      final plist = File('ios/Runner/Info.plist').readAsStringSync();
      final text = RegExp(
        r'<key>NSFaceIDUsageDescription</key>\s*<string>([^<]*)</string>',
      ).firstMatch(plist)!.group(1)!;
      expect(text, contains('before this device opens or controls your Mac'));
      expect(text, contains('unlock your Mac'));
    });
  });
}
