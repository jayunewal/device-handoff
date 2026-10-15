// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/awake.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_screen.dart';

void main() {
  group('a hold on the screen', () {
    test('keeps the screen on once, however often it is asked', () {
      final screen = FakeScreen();
      AwakeHold(screen)
        ..hold()
        ..hold();

      expect(screen.asked, ['keepOn']);
    });

    test('lets the screen go once, and only if it was kept on', () {
      final screen = FakeScreen();
      final hold = AwakeHold(screen)..release();
      expect(screen.asked, isEmpty);

      hold
        ..hold()
        ..release()
        ..release();
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('lets the screen go when the limit has passed, not before', (
      tester,
    ) async {
      final screen = FakeScreen();
      AwakeHold(screen).hold();

      await tester.pump(awakeLimit - const Duration(seconds: 1));
      expect(screen.on, isTrue);

      await tester.pump(const Duration(seconds: 2));
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('stays let go after the limit, until it is released', (
      tester,
    ) async {
      final screen = FakeScreen();
      final hold = AwakeHold(screen)..hold();
      await tester.pump(awakeLimit + const Duration(seconds: 1));
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      // A screen that only asks again gets nothing. Taking a step is something else.
      hold.hold();
      await tester.pump(awakeLimit * 2);
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      // Letting go for good, as it does when the pairing is over, makes it ready for another.
      hold
        ..release()
        ..hold();
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
      await tester.pump(awakeLimit + const Duration(seconds: 1));
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn', 'letTurnOff']);
    });

    testWidgets('counts the limit from the first ask, not from each', (
      tester,
    ) async {
      final screen = FakeScreen();
      final hold = AwakeHold(screen)..hold();

      await tester.pump(awakeLimit - const Duration(minutes: 1));
      hold.hold();
      await tester.pump(const Duration(minutes: 2));

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('lets go only once if it is released before the limit', (
      tester,
    ) async {
      final screen = FakeScreen();
      AwakeHold(screen)
        ..hold()
        ..release();

      await tester.pump(awakeLimit * 2);

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('counts the limit again from each step forward', (
      tester,
    ) async {
      final screen = FakeScreen();
      final hold = AwakeHold(screen)..hold();

      // Each step comes just before the limit would have been reached, and the pairing is
      // still going on after the time that one hold would have had.
      for (var step = 0; step < 3; step++) {
        await tester.pump(awakeLimit - const Duration(minutes: 1));
        hold.renew();
      }
      await tester.pump(awakeLimit - const Duration(seconds: 1));
      expect(screen.asked, ['keepOn'], reason: 'asked once, and still on');

      await tester.pump(const Duration(seconds: 2));
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('keeps the screen on again at a step after it ran out', (
      tester,
    ) async {
      final screen = FakeScreen();
      final hold = AwakeHold(screen)..hold();
      await tester.pump(awakeLimit + const Duration(seconds: 1));
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      hold.renew();
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);

      // And that is a new run, with a limit of its own.
      await tester.pump(awakeLimit + const Duration(seconds: 1));
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn', 'letTurnOff']);
    });

    test('keeps the screen on at a step when it did not hold yet', () {
      final screen = FakeScreen();
      final hold = AwakeHold(screen)..renew();
      expect(screen.asked, ['keepOn']);

      hold
        ..release()
        ..renew();
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
    });

    testWidgets('asks once and lets go once, however many steps there are', (
      tester,
    ) async {
      final screen = FakeScreen();
      AwakeHold(screen)
        ..hold()
        ..renew()
        ..renew()
        ..release();

      await tester.pump(awakeLimit * 2);

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });
  });

  group('the screen of the device', () {
    const channel = MethodChannel(PlatformScreenKeeper.channelName);

    // The count of holds belongs to the app, not to a test.
    setUp(PlatformScreenKeeper.forgetHolds);

    /// Answers the channel and notes what it was asked, in order.
    List<String> listen(WidgetTester tester, {bool kept = true}) {
      final calls = <String>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        calls.add(call.method);
        return call.method == 'isKeptOn' ? kept : null;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      return calls;
    }

    testWidgets('is asked through the app on an iPhone or iPad', (
      tester,
    ) async {
      final calls = listen(tester);
      const keeper = PlatformScreenKeeper();

      await keeper.keepOn();
      expect(await keeper.isKeptOn(), isTrue);
      await keeper.letTurnOff();

      expect(calls, ['keepOn', 'isKeptOn', 'letTurnOff']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is turned off by the last of several holds, not the first', (
      tester,
    ) async {
      final calls = listen(tester);
      const keeper = PlatformScreenKeeper();

      await keeper.keepOn();
      await keeper.keepOn();
      expect(calls, ['keepOn'], reason: 'the app was asked once');

      await keeper.letTurnOff();
      expect(calls, ['keepOn'], reason: 'the other hold still needs it');

      await keeper.letTurnOff();
      expect(calls, ['keepOn', 'letTurnOff']);

      // A hold that comes after all have let go is the first of a new run.
      await keeper.keepOn();
      expect(calls, ['keepOn', 'letTurnOff', 'keepOn']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('stays on when a screen opens as another one is closing', (
      tester,
    ) async {
      final calls = listen(tester);
      // Back from "Show a code" and at once to "Scan or enter a code": the screen that opens
      // asks before the one that closes, which has to finish going, lets go.
      final leaving = AwakeHold(const PlatformScreenKeeper())..hold();
      final arriving = AwakeHold(const PlatformScreenKeeper())..hold();
      leaving.release();
      await tester.pump();
      expect(calls, ['keepOn']);

      arriving.release();
      await tester.pump();
      expect(calls, ['keepOn', 'letTurnOff']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('has nothing to turn off when nothing was kept on', (
      tester,
    ) async {
      final calls = listen(tester);
      const keeper = PlatformScreenKeeper();

      await keeper.letTurnOff();
      await keeper.keepOn();
      await keeper.letTurnOff();
      await keeper.letTurnOff();

      expect(calls, ['keepOn', 'letTurnOff']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is read from what the system says, not from what was asked', (
      tester,
    ) async {
      listen(tester, kept: false);
      const keeper = PlatformScreenKeeper();

      await keeper.keepOn();

      expect(await keeper.isKeptOn(), isFalse);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('carries on when the app does not answer', (tester) async {
      // The app has no code for it, and the channel says so.
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        (call) async => throw MissingPluginException(),
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      const keeper = PlatformScreenKeeper();

      await keeper.keepOn();
      await keeper.letTurnOff();

      expect(await keeper.isKeptOn(), isFalse);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('carries on when the app says it could not', (tester) async {
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        (call) async => throw PlatformException(code: 'broken'),
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      const keeper = PlatformScreenKeeper();

      await keeper.keepOn();
      await keeper.letTurnOff();

      expect(await keeper.isKeptOn(), isFalse);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

    testWidgets('is left to the person on every other system', (tester) async {
      final calls = listen(tester);
      const keeper = PlatformScreenKeeper();

      await keeper.keepOn();
      await keeper.letTurnOff();

      expect(await keeper.isKeptOn(), isFalse);
      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.all(excluding: {TargetPlatform.iOS}));
  });
}
