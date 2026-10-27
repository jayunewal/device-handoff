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
///
/// A screen that waits for its first picture shows a bar that moves and never settles, so it is
/// opened with [settle] false: the page is pumped for a time instead.
Future<_Shown> _open(
  WidgetTester tester, {
  WindowSession? window,
  bool began = true,
  Size size = const Size(900, 1600),
  Duration? renewEvery,
  bool settle = true,
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
  if (settle) {
    await tester.pumpAndSettle();
  } else {
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 500));
  }
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

    testWidgets('is described to a screen reader as a screen when it is one', (
      tester,
    ) async {
      final handle = tester.ensureSemantics();
      await _open(
        tester,
        window: sampleWindowSession(display: true, title: null, app: null),
      );

      expect(
        find.bySemanticsLabel('The screen from Anna’s Mac'),
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
          display: false,
          title: 'Plan',
          width: 800,
          height: 600,
          control: false,
          private: false,
          sound: false,
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

    testWidgets('is not drawn for a window with no height', (tester) async {
      await _open(tester, window: sampleWindowSession(width: 800, height: 0));

      expect(find.byType(Texture), findsNothing);
      expect(find.text('This window cannot be shown here'), findsOneWidget);
    });

    double shape(WidgetTester tester) =>
        tester.widget<AspectRatio>(find.byType(AspectRatio)).aspectRatio;

    // The source says how large the picture is: one turn for the event to reach the controller,
    // and one for the screen to follow it.
    Future<void> says(
      WidgetTester tester,
      _Shown shown,
      int width,
      int height, {
      int id = 1,
    }) async {
      shown.session.emit(
        Event.windowPicture(id: id, width: width, height: height),
      );
      await tester.pump();
      await tester.pump();
    }

    testWidgets(
      'is in the shape that the source said, and in that of the window before',
      (tester) async {
        final shown = await _open(
          tester,
          window: sampleWindowSession(width: 1600, height: 1000),
        );
        expect(shape(tester), 1.6);

        // The source scales the window to what this device takes, and makes it square.
        await says(tester, shown, 1000, 1000);

        expect(shape(tester), 1.0);
      },
    );

    testWidgets('follows the source when it changes the size', (tester) async {
      final shown = await _open(tester);
      await says(tester, shown, 1000, 1000);
      expect(shape(tester), 1.0);

      // The window was made narrow, and then wide.
      await says(tester, shown, 500, 1000);
      expect(shape(tester), 0.5);
      await says(tester, shown, 2000, 500);
      expect(shape(tester), 4.0);
    });

    testWidgets('is not changed by the size of another window', (tester) async {
      final shown = await _open(
        tester,
        window: sampleWindowSession(width: 1600, height: 1000),
      );

      await says(tester, shown, 1000, 1000, id: 2);

      expect(shape(tester), 1.6);
    });

    testWidgets(
      'is drawn from the size of the picture when the window has none',
      (tester) async {
        final shown = await _open(
          tester,
          window: sampleWindowSession(width: 0, height: 600),
        );
        expect(find.byType(Texture), findsNothing);

        await says(tester, shown, 800, 600);

        expect(find.byType(Texture), findsOneWidget);
        expect(find.text('This window cannot be shown here'), findsNothing);
        expect(shape(tester), closeTo(4 / 3, 0.0001));
      },
    );

    testWidgets(
      'is as large as the screen allows in the shape of the picture',
      (tester) async {
        final shown = await _open(
          tester,
          window: sampleWindowSession(width: 1000, height: 1000),
          size: const Size(1200, 800),
        );

        await says(tester, shown, 1600, 800);

        // The picture is twice as wide as high, and the space is 1200 wide: it is as wide as
        // the space and half as high.
        final box = tester.getSize(find.byType(Texture));
        expect(box.width, closeTo(1200, 0.01));
        expect(box.height, closeTo(600, 0.01));
      },
    );
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

    testWidgets('keeps showing if the question is closed without an answer', (
      tester,
    ) async {
      final shown = await _open(tester);

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();
      expect(find.text('Stop showing this window?'), findsOneWidget);

      // A tap on the dimmed part of the screen closes the question and answers nothing.
      await tester.tapAt(const Offset(5, 5));
      await tester.pumpAndSettle();

      expect(find.text('Stop showing this window?'), findsNothing);
      expect(find.byType(Texture), findsOneWidget);
      expect(shown.session.windowsEnded, isEmpty);
      expect(shown.controller.sessions, [shown.window]);
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

  group('a whole screen', () {
    WindowSession screen({int? screen}) => sampleWindowSession(
      display: true,
      title: null,
      app: null,
      screen: screen,
    );

    testWidgets('is named a screen in the bar, and from where', (tester) async {
      await _open(tester, window: screen());

      expect(find.text('a screen'), findsOneWidget);
      expect(find.text('from Anna’s Mac'), findsOneWidget);
      expect(find.byType(Texture), findsOneWidget);
    });

    testWidgets('is asked about as a screen before it is stopped', (
      tester,
    ) async {
      await _open(tester, window: screen());

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pumpAndSettle();

      expect(find.text('Stop showing this screen?'), findsOneWidget);
      expect(find.text('Stop showing this window?'), findsNothing);
    });

    testWidgets('that has no screen to be drawn on says so as a screen', (
      tester,
    ) async {
      await _open(
        tester,
        window: const WindowSession(
          id: 1,
          role: WindowRole.viewing,
          deviceId: 'b0',
          deviceName: 'Anna’s Mac',
          display: true,
          width: 800,
          height: 600,
          control: false,
          private: false,
          sound: false,
        ),
      );

      expect(find.text('This screen cannot be shown here'), findsOneWidget);
      expect(find.text('This window cannot be shown here'), findsNothing);
    });

    testWidgets('that is over is closed as a screen', (tester) async {
      final shown = await _open(tester, window: screen());

      shown.controller.end(shown.window);
      await tester.pumpAndSettle();

      expect(find.text('The screen is closed'), findsOneWidget);
      expect(find.text('The window is closed'), findsNothing);
    });
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

  group('a screen that this device asked for', () {
    // It begins before the source has said how large it is.
    WindowSession asked({String device = 'Anna’s Mac', int? screen}) =>
        sampleWindowSession(
          display: true,
          title: null,
          app: null,
          width: 0,
          height: 0,
          device: device,
          screen: screen,
        );

    double shape(WidgetTester tester) =>
        tester.widget<AspectRatio>(find.byType(AspectRatio)).aspectRatio;

    testWidgets(
      'waits for the first picture, and says what may be the matter',
      (tester) async {
        await _open(tester, window: asked(), settle: false);

        expect(
          find.text('Waiting for the screen of Anna’s Mac'),
          findsOneWidget,
        );
        expect(
          find.text(
            'A device that is locked or asleep shows nothing. If this goes on, '
            'wake and unlock it.',
          ),
          findsOneWidget,
        );
        expect(find.byType(LinearProgressIndicator), findsOneWidget);
        expect(find.byType(Texture), findsNothing);
        expect(find.text('This screen cannot be shown here'), findsNothing);
      },
    );

    testWidgets('is drawn in the shape of the first picture', (tester) async {
      final shown = await _open(tester, window: asked(), settle: false);

      shown.session.emit(
        const Event.windowPicture(id: 1, width: 2560, height: 1600),
      );
      await tester.pump();
      await tester.pump();

      expect(find.byType(Texture), findsOneWidget);
      expect(shape(tester), 1.6);
      expect(find.byType(LinearProgressIndicator), findsNothing);
      expect(find.textContaining('Waiting for'), findsNothing);
    });

    testWidgets('can be stopped while it waits', (tester) async {
      final shown = await _open(tester, window: asked(), settle: false);

      await tester.tap(find.byTooltip('Stop showing'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 500));
      expect(find.text('Stop showing this screen?'), findsOneWidget);
      await tester.tap(find.text('Stop'));
      await tester.pumpAndSettle();

      expect(shown.session.windowsEnded, [1]);
      expect(_onHome(tester), isTrue);
    });

    testWidgets('names the device as fit to show', (tester) async {
      await _open(tester, window: asked(device: 'Mac\u202Eexe'), settle: false);

      expect(
        find.text('Waiting for the screen of Mac\uFFFDexe'),
        findsOneWidget,
      );
    });

    testWidgets('with no screen to be drawn on says so, and does not wait', (
      tester,
    ) async {
      await _open(
        tester,
        window: const WindowSession(
          id: 1,
          role: WindowRole.viewing,
          deviceId: 'b0',
          deviceName: 'Anna’s Mac',
          display: true,
          width: 0,
          height: 0,
          control: false,
          private: false,
          sound: false,
        ),
      );

      expect(find.text('This screen cannot be shown here'), findsOneWidget);
      expect(find.byType(LinearProgressIndicator), findsNothing);
    });

    testWidgets('is the only kind of window that waits for a size', (
      tester,
    ) async {
      await _open(tester, window: sampleWindowSession(width: 0, height: 0));

      expect(find.text('This window cannot be shown here'), findsOneWidget);
      expect(find.byType(LinearProgressIndicator), findsNothing);
    });
  });

  group('the settings for a session of a private screen', () {
    WindowSession privateScreen({int id = 1}) => sampleWindowSession(
      id: id,
      display: true,
      title: null,
      app: null,
      control: true,
      private: true,
    );

    final settings = find.byTooltip('Settings for this session');
    final settingsWithSomethingOn = find.byTooltip(
      'Settings for this session, something is switched on',
    );
    final screenSwitch = find.widgetWithText(
      SwitchListTile,
      'Show screen on the Mac',
    );
    final inputSwitch = find.widgetWithText(
      SwitchListTile,
      'Allow the Mac’s keyboard and trackpad',
    );

    Future<void> openSettings(WidgetTester tester) async {
      await tester.tap(find.byTooltip(RegExp('^Settings for this session')));
      await tester.pumpAndSettle();
    }

    Future<void> closeSettings(WidgetTester tester) async {
      // A tap on the dimmed part above the sheet puts it away.
      await tester.tapAt(const Offset(450, 40));
      await tester.pumpAndSettle();
    }

    bool on(WidgetTester tester, Finder tile) =>
        tester.widget<SwitchListTile>(tile).value;

    testWidgets(
      'are there only for a screen that the other device keeps private',
      (tester) async {
        await _open(tester, window: sampleWindowSession(control: true));
        expect(settings, findsNothing);

        await tester.pumpWidget(const SizedBox());
        await _open(tester, window: privateScreen());
        expect(settings, findsOneWidget);
      },
    );

    testWidgets('open over the picture with both switches off', (tester) async {
      final shown = await _open(tester, window: privateScreen());

      await openSettings(tester);

      expect(find.text('Settings for this session'), findsOneWidget);
      expect(screenSwitch, findsOneWidget);
      expect(inputSwitch, findsOneWidget);
      expect(on(tester, screenSwitch), isFalse);
      expect(on(tester, inputSwitch), isFalse);
      expect(
        find.text(
          'The Mac’s screen is dark, so nobody near it sees what you do.',
        ),
        findsOneWidget,
      );
      expect(find.text('Nobody at the Mac can use it.'), findsOneWidget);
      expect(shown.session.windowLifts, isEmpty);
    });

    testWidgets(
      'switch the screen and the keyboard and trackpad one at a time',
      (tester) async {
        final shown = await _open(tester, window: privateScreen(id: 4));
        await openSettings(tester);

        await tester.tap(screenSwitch);
        await tester.pumpAndSettle();
        expect(on(tester, screenSwitch), isTrue);
        expect(on(tester, inputSwitch), isFalse);
        expect(
          find.text('Anyone near the Mac can see what you do on it.'),
          findsOneWidget,
        );

        await tester.tap(inputSwitch);
        await tester.pumpAndSettle();
        expect(on(tester, screenSwitch), isTrue);
        expect(on(tester, inputSwitch), isTrue);
        expect(find.text('Anyone at the Mac can use it.'), findsOneWidget);

        await tester.tap(screenSwitch);
        await tester.pumpAndSettle();
        await tester.tap(inputSwitch);
        await tester.pumpAndSettle();
        expect(on(tester, screenSwitch), isFalse);
        expect(on(tester, inputSwitch), isFalse);

        // Each switch said how both stand, and the last one is how things stand.
        expect(shown.session.windowLifts, [
          (session: 4, screen: true, input: false),
          (session: 4, screen: true, input: true),
          (session: 4, screen: false, input: true),
          (session: 4, screen: false, input: false),
        ]);
      },
    );

    testWidgets(
      'can be switched on and off any number of times in one session',
      (tester) async {
        final shown = await _open(tester, window: privateScreen());
        await openSettings(tester);

        for (var round = 0; round < 5; round++) {
          await tester.tap(screenSwitch);
          await tester.pumpAndSettle();
          expect(on(tester, screenSwitch), isTrue);
          await tester.tap(screenSwitch);
          await tester.pumpAndSettle();
          expect(on(tester, screenSwitch), isFalse);
        }

        expect(shown.session.windowLifts.length, 10);
        expect(shown.session.windowLifts.last, (
          session: 1,
          screen: false,
          input: false,
        ));
      },
    );

    testWidgets(
      'are still as they were when the sheet is closed and opened again',
      (tester) async {
        await _open(tester, window: privateScreen());
        await openSettings(tester);
        await tester.tap(inputSwitch);
        await tester.pumpAndSettle();

        await closeSettings(tester);
        expect(inputSwitch, findsNothing);
        await openSettings(tester);

        expect(on(tester, screenSwitch), isFalse);
        expect(on(tester, inputSwitch), isTrue);
      },
    );

    // Either switch alone is something that is switched on.
    for (final (what, tile) in [
      ('the screen switch', screenSwitch),
      ('the keyboard and trackpad switch', inputSwitch),
    ]) {
      testWidgets('show in the bar that $what is on', (tester) async {
        await _open(tester, window: privateScreen());
        expect(settingsWithSomethingOn, findsNothing);
        expect(
          tester.widget<Badge>(find.byType(Badge)).isLabelVisible,
          isFalse,
        );

        await openSettings(tester);
        await tester.tap(tile);
        await tester.pumpAndSettle();
        await closeSettings(tester);
        expect(settingsWithSomethingOn, findsOneWidget);
        expect(tester.widget<Badge>(find.byType(Badge)).isLabelVisible, isTrue);

        await openSettings(tester);
        await tester.tap(tile);
        await tester.pumpAndSettle();
        await closeSettings(tester);
        expect(settingsWithSomethingOn, findsNothing);
        expect(
          tester.widget<Badge>(find.byType(Badge)).isLabelVisible,
          isFalse,
        );
      });
    }

    testWidgets('go on saying so while one of the two is still on', (
      tester,
    ) async {
      await _open(tester, window: privateScreen());
      await openSettings(tester);
      await tester.tap(screenSwitch);
      await tester.pumpAndSettle();
      await tester.tap(inputSwitch);
      await tester.pumpAndSettle();

      // The screen goes off again, and the keyboard and trackpad are still on.
      await tester.tap(screenSwitch);
      await tester.pumpAndSettle();
      await closeSettings(tester);
      expect(settingsWithSomethingOn, findsOneWidget);
      expect(tester.widget<Badge>(find.byType(Badge)).isLabelVisible, isTrue);

      // Then the keyboard and trackpad go off too, and nothing is on.
      await openSettings(tester);
      await tester.tap(inputSwitch);
      await tester.pumpAndSettle();
      await closeSettings(tester);
      expect(settingsWithSomethingOn, findsNothing);
      expect(tester.widget<Badge>(find.byType(Badge)).isLabelVisible, isFalse);
    });

    testWidgets(
      'keep a switch as it was, and say so, when the Mac was not told',
      (tester) async {
        final shown = await _open(tester, window: privateScreen());
        await openSettings(tester);

        shown.session.windowLiftTaken = false;
        await tester.tap(screenSwitch);
        await tester.pumpAndSettle();

        expect(on(tester, screenSwitch), isFalse);
        expect(
          find.text('The Mac did not get that, so it is as it was. Try again.'),
          findsOneWidget,
        );
        // The window goes on.
        expect(shown.controller.sessions, [shown.window]);

        // Once the Mac can be told, the switch works, and the sentence goes.
        shown.session.windowLiftTaken = true;
        await tester.tap(screenSwitch);
        await tester.pumpAndSettle();
        expect(on(tester, screenSwitch), isTrue);
        expect(find.textContaining('did not get that'), findsNothing);
      },
    );

    testWidgets('cannot be switched once the session is over', (tester) async {
      final shown = await _open(tester, window: privateScreen());
      await openSettings(tester);
      await tester.tap(screenSwitch);
      await tester.pumpAndSettle();
      await tester.tap(inputSwitch);
      await tester.pumpAndSettle();
      expect(on(tester, screenSwitch), isTrue);
      expect(on(tester, inputSwitch), isTrue);
      expect(
        find.text(
          'Both switch off again when this session ends, and the Mac locks.',
        ),
        findsOneWidget,
      );

      shown.session.emit(
        Event.windowEnded(
          id: shown.window.id.toInt(),
          end: const WindowEnd.otherDevice(),
        ),
      );
      await tester.pumpAndSettle();

      // Both are off again, and the sheet says that the session is over.
      expect(on(tester, screenSwitch), isFalse);
      expect(on(tester, inputSwitch), isFalse);
      expect(find.text('This session is over.'), findsOneWidget);

      // Neither can be switched on again, and nothing is sent to the Mac.
      final lifts = shown.session.windowLifts.length;
      await tester.tap(screenSwitch, warnIfMissed: false);
      await tester.tap(inputSwitch, warnIfMissed: false);
      await tester.pumpAndSettle();
      expect(on(tester, screenSwitch), isFalse);
      expect(on(tester, inputSwitch), isFalse);
      expect(shown.session.windowLifts.length, lifts);
    });

    testWidgets('are not offered once the session is over', (tester) async {
      final shown = await _open(tester, window: privateScreen());

      shown.session.emit(
        Event.windowEnded(
          id: shown.window.id.toInt(),
          end: const WindowEnd.otherDevice(),
        ),
      );
      await tester.pumpAndSettle();

      expect(settings, findsNothing);
    });
  });

  group('the sound of a screen', () {
    WindowSession screenWithSound({int id = 1, bool private = false}) =>
        sampleWindowSession(
          id: id,
          display: true,
          title: null,
          app: null,
          control: private,
          private: private,
          sound: true,
        );

    final off = find.byTooltip('Hear the sound of Anna’s Mac');
    final asking = find.byTooltip(
      'Waiting for the sound of Anna’s Mac, tap to stop',
    );
    final playing = find.byTooltip('Turn off the sound of Anna’s Mac');
    final silent = find.byTooltip('No sound from Anna’s Mac, tap to try again');
    final anySpeaker = find.byWidgetPredicate(
      (widget) =>
          widget is Tooltip &&
          (widget.message ?? '').contains('sound') &&
          (widget.message ?? '').contains('Anna’s Mac'),
    );

    /// Baton says how the sound stands. The screen shows it once the test has pumped.
    void hearOf(_Shown shown, WindowSound sound) {
      shown.session.emit(
        Event.windowSound(id: shown.window.id.toInt(), sound: sound),
      );
    }

    testWidgets(
      'has a speaker in the bar only for a screen that can be heard',
      (tester) async {
        await _open(tester, window: sampleWindowSession(display: true));
        expect(anySpeaker, findsNothing);

        await tester.pumpWidget(const SizedBox());
        await _open(tester, window: screenWithSound());
        expect(off, findsOneWidget);
        expect(find.byIcon(Icons.volume_off_outlined), findsOneWidget);
      },
    );

    testWidgets('is off when the session starts, and nothing is asked', (
      tester,
    ) async {
      final shown = await _open(tester, window: screenWithSound());

      expect(off, findsOneWidget);
      expect(shown.session.windowHears, isEmpty);
    });

    testWidgets(
      'is asked for with a tap, plays when Baton says so, and is turned off with another',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound(id: 4));

        await tester.tap(off);
        await tester.pumpAndSettle();
        expect(asking, findsOneWidget);
        expect(find.byIcon(Icons.volume_up_outlined), findsOneWidget);

        hearOf(shown, WindowSound.playing);
        await tester.pumpAndSettle();
        expect(playing, findsOneWidget);
        expect(find.byIcon(Icons.volume_up), findsOneWidget);

        await tester.tap(playing);
        await tester.pumpAndSettle();
        expect(off, findsOneWidget);

        expect(shown.session.windowHears, [
          (session: 4, listen: true),
          (session: 4, listen: false),
        ]);
      },
    );

    testWidgets(
      'can be stopped while it waits for the other device to answer',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());
        await tester.tap(off);
        await tester.pumpAndSettle();

        await tester.tap(asking);
        await tester.pumpAndSettle();

        expect(off, findsOneWidget);
        expect(shown.session.windowHears.last, (session: 1, listen: false));
      },
    );

    testWidgets(
      'says why the other device sends none, and asks again with a tap',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());
        await tester.tap(off);
        await tester.pumpAndSettle();

        hearOf(shown, WindowSound.noPermission);
        await tester.pumpAndSettle();
        expect(silent, findsOneWidget);
        expect(
          find.text(
            'Anna’s Mac has not let Baton record its sound. Someone at Anna’s '
            'Mac can allow it in the settings of that device. Then tap the '
            'speaker to try again.',
          ),
          findsOneWidget,
        );

        // The sentence goes as soon as the person asks again, and comes back if it is no better.
        await tester.tap(silent);
        await tester.pumpAndSettle();
        expect(asking, findsOneWidget);
        expect(find.textContaining('has not let Baton record'), findsNothing);
        hearOf(shown, WindowSound.failed);
        await tester.pumpAndSettle();
        expect(
          find.text(
            'Anna’s Mac could not record its sound. Tap the speaker to try '
            'again.',
          ),
          findsOneWidget,
        );
        hearOf(shown, WindowSound.playing);
        await tester.pumpAndSettle();
        expect(playing, findsOneWidget);
        expect(find.textContaining('could not record'), findsNothing);

        expect(shown.session.windowHears, [
          (session: 1, listen: true),
          (session: 1, listen: true),
        ]);
      },
    );

    testWidgets(
      'can be turned off from the line that says why the other device sends none',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());
        final turnOff = find.text('Turn off the sound');
        await tester.tap(off);
        await tester.pumpAndSettle();
        expect(turnOff, findsNothing);

        hearOf(shown, WindowSound.noPermission);
        await tester.pumpAndSettle();
        expect(turnOff, findsOneWidget);

        await tester.tap(turnOff);
        await tester.pumpAndSettle();

        // The line goes with the sound, the speaker is off, and the other device is told.
        expect(turnOff, findsNothing);
        expect(find.textContaining('has not let Baton record'), findsNothing);
        expect(off, findsOneWidget);
        expect(shown.session.windowHears, [
          (session: 1, listen: true),
          (session: 1, listen: false),
        ]);

        // Asking again afterwards is asking as at first.
        await tester.tap(off);
        await tester.pumpAndSettle();
        expect(asking, findsOneWidget);
        hearOf(shown, WindowSound.playing);
        await tester.pumpAndSettle();
        expect(turnOff, findsNothing);
      },
    );

    testWidgets(
      'says that the other device does not let this one hear, and asks again with a tap',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());
        await tester.tap(off);
        await tester.pumpAndSettle();

        hearOf(shown, WindowSound.notAllowed);
        await tester.pumpAndSettle();
        expect(silent, findsOneWidget);
        expect(
          find.textContaining('does not let this device hear'),
          findsOneWidget,
        );

        // The person at the other device allowed it meanwhile: the next tap is taken.
        await tester.tap(silent);
        await tester.pumpAndSettle();
        expect(asking, findsOneWidget);
        expect(
          find.textContaining('does not let this device hear'),
          findsNothing,
        );
        hearOf(shown, WindowSound.playing);
        await tester.pumpAndSettle();
        expect(playing, findsOneWidget);
        expect(shown.session.windowHears, [
          (session: 1, listen: true),
          (session: 1, listen: true),
        ]);
      },
    );

    testWidgets(
      'says that this device could not play the sound, and plays it when the person asks again',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());
        await tester.tap(off);
        await tester.pumpAndSettle();
        hearOf(shown, WindowSound.playing);
        await tester.pumpAndSettle();
        expect(playing, findsOneWidget);

        hearOf(shown, WindowSound.cannotPlay);
        await tester.pumpAndSettle();
        expect(silent, findsOneWidget);
        expect(
          find.text(
            'This device could not play the sound of Anna’s Mac. Tap the '
            'speaker to try again.',
          ),
          findsOneWidget,
        );
        // The sound is off already and the other device knows it, so the line offers to be put
        // away and nothing else.
        expect(find.text('Turn off the sound'), findsNothing);
        expect(find.text('Close'), findsOneWidget);

        await tester.tap(silent);
        await tester.pumpAndSettle();
        expect(asking, findsOneWidget);
        expect(find.textContaining('could not play'), findsNothing);
        hearOf(shown, WindowSound.playing);
        await tester.pumpAndSettle();
        expect(playing, findsOneWidget);
        expect(shown.session.windowHears, [
          (session: 1, listen: true),
          (session: 1, listen: true),
        ]);
      },
    );

    testWidgets(
      'puts the line about a sound that this device could not play away with Close alone, and asks nothing',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());
        await tester.tap(off);
        await tester.pumpAndSettle();
        hearOf(shown, WindowSound.cannotPlay);
        await tester.pumpAndSettle();
        final line = find.textContaining('could not play');
        expect(line, findsOneWidget);

        await tester.tap(find.text('Close'));
        await tester.pumpAndSettle();

        // The line goes, the speaker still says there is no sound, and nobody was told anything:
        // this device did that when it could not play.
        expect(line, findsNothing);
        expect(find.text('Close'), findsNothing);
        expect(silent, findsOneWidget);
        expect(shown.session.windowHears, [(session: 1, listen: true)]);
      },
    );

    testWidgets(
      'puts the line about the silence away with Close, and shows it again when the sound stands differently',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());
        final close = find.text('Close');
        final line = find.textContaining('has not let Baton record');
        await tester.tap(off);
        await tester.pumpAndSettle();
        hearOf(shown, WindowSound.noPermission);
        await tester.pumpAndSettle();
        expect(line, findsOneWidget);
        expect(close, findsOneWidget);

        await tester.tap(close);
        await tester.pumpAndSettle();

        // The line goes. The speaker still says there is no sound, and nothing was asked.
        expect(line, findsNothing);
        expect(close, findsNothing);
        expect(find.text('Turn off the sound'), findsNothing);
        expect(silent, findsOneWidget);
        expect(shown.session.windowHears, [(session: 1, listen: true)]);

        // The same news does not bring it back.
        hearOf(shown, WindowSound.noPermission);
        await tester.pumpAndSettle();
        expect(line, findsNothing);

        // A person who asks again and gets the same answer is told it.
        await tester.tap(silent);
        await tester.pumpAndSettle();
        expect(line, findsNothing);
        hearOf(shown, WindowSound.noPermission);
        await tester.pumpAndSettle();
        expect(line, findsOneWidget);
        expect(close, findsOneWidget);

        // Another reason brings it back too, after it was put away again.
        await tester.tap(close);
        await tester.pumpAndSettle();
        hearOf(shown, WindowSound.failed);
        await tester.pumpAndSettle();
        expect(find.textContaining('could not record'), findsOneWidget);
      },
    );

    // A tall screen that can be controlled, so that the picture reaches the top of the view, under
    // the lines over it, and a touch on a line lands on the picture if the line lets it through.
    WindowSession tallScreen() => sampleWindowSession(
      display: true,
      title: null,
      app: null,
      width: 900,
      height: 4000,
      control: true,
      sound: true,
    );

    testWidgets(
      'keeps a touch on the line that says why from going through to the window',
      (tester) async {
        final shown = await _open(tester, window: tallScreen());
        await tester.tap(find.widgetWithText(FilterChip, 'Control'));
        await tester.pump();
        await tester.tap(off);
        await tester.pumpAndSettle();
        hearOf(shown, WindowSound.failed);
        await tester.pumpAndSettle();
        final line = find
            .ancestor(
              of: find.textContaining('could not record'),
              matching: find.byType(Material),
            )
            .first;

        // A touch on the picture is given to the window, so this test sees the difference.
        await tester.tapAt(const Offset(450, 900));
        await tester.pump();
        expect(shown.session.windowInputs, isNotEmpty);
        shown.session.windowInputs.clear();

        // One at the edge of the line, where there are neither words nor button, is not.
        await tester.tapAt(Offset(450, tester.getTopLeft(line).dy + 3));
        await tester.pump();
        expect(shown.session.windowInputs, isEmpty);
      },
    );

    testWidgets(
      'lets a touch on the line about what the other device did not do through to the window',
      (tester) async {
        final shown = await _open(tester, window: tallScreen());
        await tester.tap(find.widgetWithText(FilterChip, 'Control'));
        await tester.pump();
        shown.session.emit(
          Event.windowNotice(
            id: 1,
            reason: WindowNoticeReason.locked,
            count: 1,
          ),
        );
        await tester.pump(Duration.zero);
        await tester.pump();
        final line = find
            .ancestor(
              of: find.text('Anna’s Mac is locked.'),
              matching: find.byType(Material),
            )
            .first;

        // This line has no button, so a touch on it is a touch on the picture under it.
        await tester.tapAt(Offset(450, tester.getTopLeft(line).dy + 3));
        await tester.pump();
        expect(shown.session.windowInputs, isNotEmpty);
      },
    );

    testWidgets('has a sentence for each reason that there is no sound', (
      tester,
    ) async {
      final shown = await _open(tester, window: screenWithSound());
      await tester.tap(off);
      await tester.pumpAndSettle();

      for (final (sound, text) in [
        (
          WindowSound.window,
          'Sound goes with a whole screen, and this is a window on Anna’s Mac.',
        ),
        (
          WindowSound.failed,
          'Anna’s Mac could not record its sound. Tap the speaker to try again.',
        ),
        (
          WindowSound.other,
          'Anna’s Mac cannot send its sound now. Tap the speaker to try again.',
        ),
        (
          WindowSound.cannotPlay,
          'This device could not play the sound of Anna’s Mac. Tap the '
              'speaker to try again.',
        ),
        (
          WindowSound.notAllowed,
          'Anna’s Mac does not let this device hear it. Someone at Anna’s Mac '
              'can allow it in the settings for this device. Then tap the '
              'speaker to try again.',
        ),
      ]) {
        hearOf(shown, sound);
        await tester.pumpAndSettle();
        expect(find.text(text), findsOneWidget, reason: '$sound');
      }
    });

    testWidgets(
      'puts the sentence about the sound under the one about what the other device did not do',
      (tester) async {
        final shown = await _open(
          tester,
          window: sampleWindowSession(
            display: true,
            title: null,
            app: null,
            control: true,
            sound: true,
          ),
        );
        await tester.tap(find.widgetWithText(FilterChip, 'Control'));
        await tester.pump();
        await tester.tap(off);
        await tester.pumpAndSettle();

        shown.session.emit(
          Event.windowNotice(
            id: 1,
            reason: WindowNoticeReason.locked,
            count: 1,
          ),
        );
        hearOf(shown, WindowSound.failed);
        await tester.pump(Duration.zero);
        await tester.pump();

        final dropped = find.text('Anna’s Mac is locked.');
        final sound = find.text(
          'Anna’s Mac could not record its sound. Tap the speaker to try again.',
        );
        expect(dropped, findsOneWidget);
        expect(sound, findsOneWidget);
        expect(
          tester.getTopLeft(dropped).dy,
          lessThan(tester.getTopLeft(sound).dy),
        );
      },
    );

    testWidgets(
      'says so, and keeps things as they were, when the other device could not be asked',
      (tester) async {
        final shown = await _open(tester, window: screenWithSound());

        shown.session.windowHearTaken = false;
        await tester.tap(off);
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 500));

        expect(
          find.text('Baton could not ask Anna’s Mac for its sound. Try again.'),
          findsOneWidget,
        );
        expect(off, findsOneWidget);
        // The window goes on.
        expect(shown.controller.sessions, [shown.window]);
      },
    );

    testWidgets(
      'is asked for at once on a private screen, and shows that it waits',
      (tester) async {
        final shown = await _open(
          tester,
          window: screenWithSound(private: true),
        );

        expect(shown.session.windowHears, [(session: 1, listen: true)]);
        expect(asking, findsOneWidget);
        hearOf(shown, WindowSound.playing);
        await tester.pumpAndSettle();
        expect(playing, findsOneWidget);
      },
    );

    testWidgets('is not offered once the session is over', (tester) async {
      final shown = await _open(tester, window: screenWithSound());
      await tester.tap(off);
      await tester.pumpAndSettle();
      hearOf(shown, WindowSound.noPermission);
      await tester.pumpAndSettle();

      shown.session.emit(
        Event.windowEnded(
          id: shown.window.id.toInt(),
          end: const WindowEnd.otherDevice(),
        ),
      );
      await tester.pumpAndSettle();

      expect(anySpeaker, findsNothing);
      // Nor is there a sentence about the sound of a window that is over.
      expect(find.textContaining('has not let Baton record'), findsNothing);
    });
  });
}
