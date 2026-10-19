// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/window_screen.dart';
import 'package:baton/src/windows_controller.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

/// Everything a test of the screen holds on to.
class _Shown {
  _Shown(this.session, this.controller, this.screen, this.window);

  final FakeSession session;
  final WindowsController controller;
  final FakeScreen screen;
  final WindowSession window;
}

/// Opens the screen of [window] over a page that says "Home", as the home screen does, once
/// Baton has said that the window began (unless [began] is false). Returns when the screen is
/// there. Without a [renewEvery] the screen renews the hold on the device's screen as often as
/// it does in the app.
Future<_Shown> _open(
  WidgetTester tester, {
  WindowSession? window,
  bool began = true,
  Size size = const Size(900, 1600),
  Duration? renewEvery,
}) async {
  final shown = window ?? sampleWindowSession();
  final session = FakeSession(
    me: const Me(
      id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
      name: 'Kitchen iPad',
      platform: Platform.ipadOs,
      fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
    ),
  );
  final screen = FakeScreen();
  final controller = WindowsController(session, screen: FakeScreen());
  await controller.start();
  addTearDown(controller.dispose);
  if (began) {
    session.emit(Event.windowStarted(session: shown));
    await tester.pump();
  }
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: Builder(
        builder: (context) => Scaffold(
          body: Center(
            child: TextButton(
              onPressed: () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => renewEvery == null
                      ? WindowScreen(
                          controller: controller,
                          session: shown,
                          screen: screen,
                        )
                      : WindowScreen(
                          controller: controller,
                          session: shown,
                          screen: screen,
                          renewEvery: renewEvery,
                        ),
                ),
              ),
              child: const Text('Home'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Home'));
  await tester.pumpAndSettle();
  return _Shown(session, controller, screen, shown);
}

bool _onHome(WidgetTester tester) => find.text('Home').evaluate().isNotEmpty;

void main() {
  group('the picture', () {
    testWidgets(
      'is drawn with the texture of the screen, in the shape of the window',
      (tester) async {
        await _open(
          tester,
          window: sampleWindowSession(screen: 12, width: 1600, height: 1000),
        );

        final texture = tester.widget<Texture>(find.byType(Texture));
        expect(texture.textureId, 12);
        expect(
          texture.filterQuality,
          FilterQuality.medium,
          reason: 'a window is drawn smaller than it is, and is smoothed',
        );
        expect(
          tester.widget<AspectRatio>(find.byType(AspectRatio)).aspectRatio,
          1.6,
        );
      },
    );

    testWidgets(
      'is drawn from a texture that has the number 0, as the first one on an iPhone has',
      (tester) async {
        await _open(tester, window: sampleWindowSession(screen: 0));

        expect(tester.widget<Texture>(find.byType(Texture)).textureId, 0);
        expect(find.text('This window cannot be shown here'), findsNothing);
      },
    );

    testWidgets('is as large as the screen allows, with its shape kept', (
      tester,
    ) async {
      // A screen wider than the window: the picture is as tall as the space, and not as wide.
      await _open(
        tester,
        window: sampleWindowSession(width: 1000, height: 1000),
        size: const Size(1200, 800),
      );

      final box = tester.getSize(find.byType(Texture));
      expect(box.width, closeTo(box.height, 0.01));
      expect(box.height, greaterThan(500));
    });

    testWidgets('can be zoomed with two fingers', (tester) async {
      await _open(tester);

      final viewer = tester.widget<InteractiveViewer>(
        find.byType(InteractiveViewer),
      );
      expect(viewer.maxScale, 6);
      expect(viewer.minScale, 1);
    });

    testWidgets('is described to a screen reader', (tester) async {
      final handle = tester.ensureSemantics();
      await _open(tester);

      expect(
        find.bySemanticsLabel('The window Budget.numbers from Anna’s Mac'),
        findsOneWidget,
      );
      handle.dispose();
    });

    testWidgets('is not drawn for a window that has no screen', (tester) async {
      await _open(
        tester,
        window: const WindowSession(
          id: 1,
          role: WindowRole.viewing,
          deviceId: 'b0',
          deviceName: 'Anna’s Mac',
          title: 'Plan',
          width: 800,
          height: 600,
          control: false,
        ),
      );

      expect(find.byType(Texture), findsNothing);
      expect(find.text('This window cannot be shown here'), findsOneWidget);
      expect(
        find.text('Baton did not get a screen to draw it on.'),
        findsOneWidget,
      );
    });

    testWidgets('is not drawn for a window with no size', (tester) async {
      await _open(tester, window: sampleWindowSession(width: 0, height: 600));

      expect(find.byType(Texture), findsNothing);
      expect(find.text('This window cannot be shown here'), findsOneWidget);
    });
  });

  group('the bar at the top', () {
    testWidgets('names the window and the device that shows it', (
      tester,
    ) async {
      await _open(tester);

      expect(find.text('Budget.numbers'), findsOneWidget);
      expect(find.text('from Anna’s Mac'), findsOneWidget);
    });

    testWidgets(
      'names a window with no title after its application, or calls it a window',
      (tester) async {
        await _open(tester, window: sampleWindowSession(title: null));
        expect(find.text('Numbers'), findsOneWidget);

        await tester.pumpWidget(const SizedBox());
        await _open(
          tester,
          window: sampleWindowSession(title: null, app: null),
        );
        expect(find.text('a window'), findsOneWidget);
      },
    );

    testWidgets('has the words of the other device made fit to show', (
      tester,
    ) async {
      await _open(
        tester,
        window: sampleWindowSession(
          title: 'Plan\u202Etxt',
          device: 'Mac\u202Eexe',
        ),
      );

      expect(find.text('Plan\uFFFDtxt'), findsOneWidget);
      expect(find.text('from Mac\uFFFDexe'), findsOneWidget);
    });
  });

  group('the screen of the device', () {
    testWidgets(
      'stays on for as long as the window shows, however long that is',
      (tester) async {
        final shown = await _open(tester);
        expect(shown.screen.asked, ['keepOn']);

        // Nobody touches it, and a hold that was not renewed would run out after five minutes.
        for (var minute = 0; minute < 20; minute++) {
          await tester.pump(const Duration(minutes: 1));
        }

        expect(shown.screen.asked, ['keepOn']);
        await tester.pumpWidget(const SizedBox());
      },
    );

    testWidgets('is renewed as often as it is asked to be, not more', (
      tester,
    ) async {
      // With a renewal every ten minutes, the hold of five minutes runs out in between.
      final shown = await _open(
        tester,
        renewEvery: const Duration(minutes: 10),
      );

      await tester.pump(const Duration(minutes: 6));

      expect(shown.screen.asked, ['keepOn', 'letTurnOff']);

      await tester.pump(const Duration(minutes: 5));

      expect(shown.screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('goes back to locking when the window ends', (tester) async {
      final shown = await _open(tester);

      shown.session.emit(
        const Event.windowEnded(id: 1, end: WindowEnd.otherDevice()),
      );
      await tester.pump();

      expect(shown.screen.asked, ['keepOn', 'letTurnOff']);

      // And it stays so: nothing is renewed after the end.
      await tester.pump(const Duration(minutes: 30));
      expect(shown.screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('goes back to locking when the person stops the window', (
      tester,
    ) async {
      final shown = await _open(tester);

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Stop'));
      await tester.pumpAndSettle();

      expect(shown.screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets(
      'is not held for a window that was over before the screen came',
      (tester) async {
        final shown = await _open(tester, began: false);

        expect(shown.screen.asked, isEmpty);
        expect(find.text('The window is closed'), findsOneWidget);
      },
    );

    testWidgets('is let go of when the screen is left by the system', (
      tester,
    ) async {
      final shown = await _open(tester);

      await tester.pumpWidget(const SizedBox());

      expect(shown.screen.asked, ['keepOn', 'letTurnOff']);
    });
  });

  group('leaving while the window shows', () {
    testWidgets('names the other device as fit to show', (tester) async {
      await _open(tester, window: sampleWindowSession(device: 'Mac\u202Eexe'));

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();

      expect(
        find.text('Mac\uFFFDexe has to share it again to show it here.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Keep showing'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('asks first, and keeps showing if the person says so', (
      tester,
    ) async {
      final shown = await _open(tester);

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();

      expect(find.text('Stop showing this window?'), findsOneWidget);
      expect(
        find.text('Anna’s Mac has to share it again to show it here.'),
        findsOneWidget,
      );

      await tester.tap(find.text('Keep showing'));
      await tester.pumpAndSettle();

      expect(find.text('Stop showing this window?'), findsNothing);
      expect(find.byType(Texture), findsOneWidget);
      expect(shown.session.windowsEnded, isEmpty);
      expect(_onHome(tester), isFalse);
    });

    testWidgets('ends the window and goes back if the person says stop', (
      tester,
    ) async {
      final shown = await _open(tester);

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Stop'));
      await tester.pumpAndSettle();

      expect(shown.session.windowsEnded, [1]);
      expect(shown.controller.sessions, isEmpty);
      expect(_onHome(tester), isTrue);
      expect(find.byType(Texture), findsNothing);
    });

    testWidgets('asks the same when the system takes the person back', (
      tester,
    ) async {
      final shown = await _open(tester);

      final navigator = tester.state<NavigatorState>(find.byType(Navigator));
      await navigator.maybePop();
      await tester.pumpAndSettle();

      expect(find.text('Stop showing this window?'), findsOneWidget);
      expect(shown.session.windowsEnded, isEmpty);
      expect(_onHome(tester), isFalse);
    });

    testWidgets('names the device that has to share the window again', (
      tester,
    ) async {
      await _open(tester, window: sampleWindowSession(device: 'Desk PC'));

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();

      expect(
        find.text('Desk PC has to share it again to show it here.'),
        findsOneWidget,
      );
    });

    testWidgets(
      'does not end a window that ended while the question was asked',
      (tester) async {
        final shown = await _open(tester);
        await tester.tap(find.byTooltip('Stop showing'));
        await tester.pumpAndSettle();

        shown.session.emit(
          const Event.windowEnded(id: 1, end: WindowEnd.otherDevice()),
        );
        await tester.pump();
        await tester.tap(find.text('Stop'));
        await tester.pumpAndSettle();

        // The window was over; Baton is told all the same, and says there was nothing to end.
        expect(_onHome(tester), isTrue);
      },
    );
  });

  group('a window that ended', () {
    testWidgets(
      'says that the other device stopped it, and is closed with a button',
      (tester) async {
        final shown = await _open(tester);

        shown.session.emit(
          const Event.windowEnded(id: 1, end: WindowEnd.otherDevice()),
        );
        await tester.pumpAndSettle();

        expect(find.byType(Texture), findsNothing);
        expect(
          find.text('Anna’s Mac stopped sharing Budget.numbers'),
          findsOneWidget,
        );

        await tester.tap(find.text('Close'));
        await tester.pumpAndSettle();

        expect(find.text('Stop showing this window?'), findsNothing);
        expect(_onHome(tester), isTrue);
        expect(
          shown.controller.notes,
          isEmpty,
          reason: 'the person has seen it',
        );
      },
    );

    testWidgets('says that the connection was lost', (tester) async {
      final shown = await _open(tester);

      shown.session.emit(const Event.windowEnded(id: 1, end: WindowEnd.lost()));
      await tester.pumpAndSettle();

      expect(
        find.text('The connection to Anna’s Mac was lost'),
        findsOneWidget,
      );
      expect(
        find.text('The window stopped. Ask Anna’s Mac to share it again.'),
        findsOneWidget,
      );
    });

    testWidgets('says what went wrong', (tester) async {
      final shown = await _open(tester);

      shown.session.emit(
        const Event.windowEnded(
          id: 1,
          end: WindowEnd.failed(message: 'the picture could not be read'),
        ),
      );
      await tester.pumpAndSettle();

      expect(find.text('The window from Anna’s Mac stopped'), findsOneWidget);
      expect(find.text('The picture could not be read.'), findsOneWidget);
    });

    testWidgets('is left without a question, by the bar as well', (
      tester,
    ) async {
      final shown = await _open(tester);
      shown.session.emit(
        const Event.windowEnded(id: 1, end: WindowEnd.otherDevice()),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip('Close'), findsOneWidget);
      await tester.tap(find.byTooltip('Close'));
      await tester.pumpAndSettle();

      expect(find.text('Stop showing this window?'), findsNothing);
      expect(_onHome(tester), isTrue);
      expect(shown.controller.notes, isEmpty);
    });

    testWidgets(
      'is left without a question when the system takes the person back',
      (tester) async {
        final shown = await _open(tester);
        shown.session.emit(
          const Event.windowEnded(id: 1, end: WindowEnd.lost()),
        );
        await tester.pumpAndSettle();

        final navigator = tester.state<NavigatorState>(find.byType(Navigator));
        await navigator.maybePop();
        await tester.pumpAndSettle();

        expect(find.text('Stop showing this window?'), findsNothing);
        expect(_onHome(tester), isTrue);
        expect(shown.controller.notes, isEmpty);
      },
    );

    testWidgets('that this device ended says only that it is closed', (
      tester,
    ) async {
      final shown = await _open(tester);

      shown.controller.end(shown.window);
      await tester.pumpAndSettle();

      expect(find.text('The window is closed'), findsOneWidget);
      expect(find.byType(Texture), findsNothing);
    });

    testWidgets('keeps the notes of other windows when it is closed', (
      tester,
    ) async {
      final shown = await _open(tester);
      shown.session.emit(
        Event.windowStarted(
          session: sampleWindowSession(id: 2, role: WindowRole.sharing),
        ),
      );
      shown.session.emit(
        const Event.windowEnded(id: 2, end: WindowEnd.otherDevice()),
      );
      shown.session.emit(
        const Event.windowEnded(id: 1, end: WindowEnd.otherDevice()),
      );
      await tester.pumpAndSettle();
      expect(shown.controller.notes, hasLength(2));

      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();

      expect(shown.controller.notes, hasLength(1));
      expect((shown.controller.notes.single as WindowOver).session.id, 2);
    });
  });
}
