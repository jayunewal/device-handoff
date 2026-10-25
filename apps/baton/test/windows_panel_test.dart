// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/windows_panel.dart';
import 'package:baton/src/windows_controller.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

const _ipad = Device(
  id: 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
  lastSeenAt: 1760529600,
);

const _mac = Device(
  id: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  name: 'Anna’s Mac',
  platform: Platform.macOs,
  fingerprint: 'mnop-qrst-uvwx-yzab-cdef',
  pairedAt: 1760000000,
  lastSeenAt: 1760529600,
);

/// A device whose name turns the text after it round, as a name that another person chose can.
const _oddPad = Device(
  id: 'c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2',
  name: 'Pad\u202Eexe',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
  lastSeenAt: 1760529600,
);

/// Shows the panel at the bottom of a screen of [size], as the home screen does, and returns
/// the session behind it and the controller. [opened] hears the windows that the person asks to
/// be taken to.
Future<(FakeSession, WindowsController)> _show(
  WidgetTester tester, {
  Size size = const Size(900, 1600),
  List<WindowSession>? opened,
}) async {
  final session = FakeSession(
    me: const Me(
      id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
      name: 'Desk Mac',
      platform: Platform.macOs,
      fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
    ),
    pairedDevices: [_ipad],
  );
  final controller = WindowsController(session, screen: FakeScreen());
  await controller.start();
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: _Host(controller: controller, opened: opened ?? []),
    ),
  );
  return (session, controller);
}

/// The screen that owns the controller and shows the panel at its bottom, as the home screen
/// does: the controller is let go of when the screen is.
class _Host extends StatefulWidget {
  const _Host({required this.controller, required this.opened});

  final WindowsController controller;
  final List<WindowSession> opened;

  @override
  State<_Host> createState() => _HostState();
}

class _HostState extends State<_Host> {
  @override
  void dispose() {
    widget.controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    body: const SizedBox.expand(),
    bottomNavigationBar: WindowsPanel(
      controller: widget.controller,
      onOpen: widget.opened.add,
    ),
  );
}

Future<void> _offer(
  WidgetTester tester,
  FakeSession session,
  WindowRequest request,
) async {
  session.emit(Event.windowRequested(request: request));
  await tester.pump();
}

Future<void> _begin(
  WidgetTester tester,
  FakeSession session,
  WindowSession window,
) async {
  session.emit(Event.windowStarted(session: window));
  await tester.pump();
}

Future<void> _end(
  WidgetTester tester,
  FakeSession session,
  int id,
  WindowEnd end,
) async {
  session.emit(Event.windowEnded(id: id, end: end));
  await tester.pump();
}

/// An offer from a device whose name turns the text after it round.
const _oddOffer = WindowRequest(
  id: 5,
  deviceId: 'd2',
  deviceName: 'Mac\u202Eexe',
  display: false,
  width: 800,
  height: 600,
  control: false,
);

/// The whole screen of a Mac, which has no title and no application.
const _screenOffer = WindowRequest(
  id: 6,
  deviceId: 'd3',
  deviceName: 'Anna’s Mac',
  display: true,
  width: 3024,
  height: 1964,
  control: false,
);

const _offer2 = WindowRequest(
  id: 4,
  deviceId: 'd1',
  deviceName: 'Desk PC',
  display: false,
  width: 800,
  height: 600,
  control: false,
);

void main() {
  testWidgets('takes no room when there is nothing to show', (tester) async {
    await _show(tester);

    expect(find.byType(Card), findsNothing);
    expect(tester.getSize(find.byType(WindowsPanel)).height, 0);
  });

  group('an offer', () {
    testWidgets('shows who offers the window and what it is', (tester) async {
      final (session, _) = await _show(tester);

      await _offer(tester, session, sampleWindowRequest);

      expect(
        find.text('Anna’s Mac wants to show you a window'),
        findsOneWidget,
      );
      expect(find.text('Budget.numbers · Numbers'), findsOneWidget);
      expect(
        find.text(
          'You will see it live on this screen. You can only look at it.',
        ),
        findsOneWidget,
      );
      expect(find.text('Decline'), findsOneWidget);
      expect(find.text('Show it'), findsOneWidget);
    });

    testWidgets(
      'says that the window can be controlled when it is offered so',
      (tester) async {
        final (session, _) = await _show(tester);

        await _offer(
          tester,
          session,
          const WindowRequest(
            id: 7,
            deviceId: 'd4',
            deviceName: 'Anna’s Mac',
            display: false,
            title: 'Plan',
            app: 'Notes',
            width: 800,
            height: 600,
            control: true,
          ),
        );

        expect(
          find.text(
            'You will see it live on this screen, and you can control it: what '
            'you touch and type here is done there.',
          ),
          findsOneWidget,
        );
        expect(find.textContaining('only look'), findsNothing);
        expect(find.text('Show it'), findsOneWidget);
      },
    );

    testWidgets('names the title alone, or the application alone', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      await _offer(
        tester,
        session,
        const WindowRequest(
          id: 4,
          deviceId: 'd1',
          deviceName: 'Desk PC',
          display: false,
          title: 'Plan',
          width: 800,
          height: 600,
          control: false,
        ),
      );
      expect(find.text('Plan'), findsOneWidget);
      expect(find.textContaining('·'), findsNothing);

      await _offer(
        tester,
        session,
        const WindowRequest(
          id: 5,
          deviceId: 'd2',
          deviceName: 'Den PC',
          display: false,
          app: 'Terminal',
          width: 800,
          height: 600,
          control: false,
        ),
      );
      expect(find.text('Terminal'), findsOneWidget);
      expect(find.textContaining('·'), findsNothing);
    });

    testWidgets('has a title on several lines shown on one', (tester) async {
      final (session, _) = await _show(tester);

      await _offer(
        tester,
        session,
        const WindowRequest(
          id: 4,
          deviceId: 'd1',
          deviceName: 'Desk PC',
          display: false,
          title: 'Plan\n  for\r\nMarch',
          app: ' Pages ',
          width: 800,
          height: 600,
          control: false,
        ),
      );

      expect(find.text('Plan for March · Pages'), findsOneWidget);
    });

    testWidgets('from a window with no title and no application says who', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      await _offer(tester, session, _offer2);

      expect(find.text('Desk PC wants to show you a window'), findsOneWidget);
      expect(find.textContaining('·'), findsNothing);
      // The heading, the line about looking, and the two buttons: no line for the window.
      expect(
        find.descendant(of: find.byType(Card), matching: find.byType(Text)),
        findsNWidgets(4),
      );
    });

    testWidgets('is said yes to, and the card gives way to one that opens it', (
      tester,
    ) async {
      final (session, controller) = await _show(tester);
      await _offer(tester, session, sampleWindowRequest);

      await tester.tap(find.text('Show it'));
      await tester.pump();

      expect(session.windowAnswers, [(request: 3, accept: true)]);
      expect(find.text('Show it'), findsNothing);
      expect(find.text('Opening the window from Anna’s Mac…'), findsOneWidget);
      expect(find.byType(LinearProgressIndicator), findsOneWidget);

      await _begin(tester, session, sampleWindowSession());

      expect(find.text('Opening the window from Anna’s Mac…'), findsNothing);
      expect(controller.opening, isNull);
    });

    testWidgets('that is said yes to names the other device as fit to show', (
      tester,
    ) async {
      final (session, _) = await _show(tester);
      await _offer(tester, session, _oddOffer);

      await tester.tap(find.text('Show it'));
      await tester.pump();

      expect(
        find.text('Opening the window from Mac\uFFFDexe…'),
        findsOneWidget,
      );
    });

    testWidgets('is said no to, and the card goes', (tester) async {
      final (session, _) = await _show(tester);
      await _offer(tester, session, sampleWindowRequest);

      await tester.tap(find.text('Decline'));
      await tester.pump();

      expect(session.windowAnswers, [(request: 3, accept: false)]);
      expect(find.byType(Card), findsNothing);
    });

    testWidgets('of two windows shows both, oldest first', (tester) async {
      final (session, _) = await _show(tester);

      await _offer(tester, session, sampleWindowRequest);
      await _offer(tester, session, _offer2);

      final first = tester.getTopLeft(
        find.text('Anna’s Mac wants to show you a window'),
      );
      final second = tester.getTopLeft(
        find.text('Desk PC wants to show you a window'),
      );
      expect(first.dy, lessThan(second.dy));
    });

    testWidgets('has the words of the other device made fit to show', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      // U+202E turns the text after it round, and a title can be made to read as something else.
      await _offer(
        tester,
        session,
        const WindowRequest(
          id: 4,
          deviceId: 'd1',
          deviceName: 'Mac\u202Eexe',
          display: false,
          title: 'Plan\u202Etxt',
          app: 'App\u200Bname',
          width: 800,
          height: 600,
          control: false,
        ),
      );

      expect(
        find.text('Mac\uFFFDexe wants to show you a window'),
        findsOneWidget,
      );
      expect(find.text('Plan\uFFFDtxt · App\uFFFDname'), findsOneWidget);
    });
  });

  group('a window that this device offers', () {
    testWidgets('shows who it is for until they have answered', (tester) async {
      final (session, controller) = await _show(tester);
      session.holdShare();

      final done = controller.share(_ipad);
      await tester.pump();

      expect(
        find.text('Sharing a window or screen with Kitchen iPad'),
        findsOneWidget,
      );
      expect(
        find.text(
          'Choose a window or a screen. Kitchen iPad is then asked whether to show it.',
        ),
        findsOneWidget,
      );
      expect(
        tester
            .widget<LinearProgressIndicator>(
              find.byType(LinearProgressIndicator),
            )
            .semanticsLabel,
        'Sharing a window or screen',
      );

      session.releaseShare();
      await done;
      await tester.pump();

      expect(find.byType(Card), findsNothing);
    });

    testWidgets('names the device as fit to show', (tester) async {
      final (session, controller) = await _show(tester);
      session.holdShare();

      final done = controller.share(_oddPad);
      await tester.pump();

      expect(
        find.text('Sharing a window or screen with Pad\uFFFDexe'),
        findsOneWidget,
      );
      expect(
        find.text(
          'Choose a window or a screen. Pad\uFFFDexe is then asked whether to show it.',
        ),
        findsOneWidget,
      );

      session.releaseShare();
      await done;
      await tester.pump();
    });
  });

  group('a window that this device offers to be controlled', () {
    testWidgets(
      'says that a window or a screen is chosen, and that it can be controlled',
      (tester) async {
        final (session, controller) = await _show(tester);
        session.holdShare();

        final done = controller.share(_ipad, control: true);
        await tester.pump();

        expect(controller.offeringControl, isTrue);
        expect(
          find.text(
            'Sharing a window or screen with Kitchen iPad, to look at and '
            'control',
          ),
          findsOneWidget,
        );
        expect(
          find.text(
            'Choose a window or a screen. Kitchen iPad is then asked whether '
            'to show it, and can control it.',
          ),
          findsOneWidget,
        );
        expect(
          find.text('Sharing a window or screen with Kitchen iPad'),
          findsNothing,
        );
        expect(
          tester
              .widget<LinearProgressIndicator>(
                find.byType(LinearProgressIndicator),
              )
              .semanticsLabel,
          'Sharing a window or screen to look at and control',
        );

        session.releaseShare();
        await done;
        await tester.pump();

        expect(find.byType(Card), findsNothing);
        expect(controller.offeringControl, isFalse);
      },
    );

    testWidgets('is not remembered by the offer that follows', (tester) async {
      final (session, controller) = await _show(tester);
      session.holdShare();
      final first = controller.share(_ipad, control: true);
      session.releaseShare();
      await first;
      session.holdShare();

      final second = controller.share(_ipad);
      await tester.pump();

      expect(controller.offeringControl, isFalse);
      expect(
        find.text('Sharing a window or screen with Kitchen iPad'),
        findsOneWidget,
      );
      session.releaseShare();
      await second;
    });
  });

  group('a screen that this device asks for', () {
    testWidgets('shows who it asks until they have answered', (tester) async {
      final (session, controller) = await _show(tester);
      session.holdStart();

      final done = controller.startScreen(_mac);
      await tester.pump();

      expect(find.text('Asking Anna’s Mac for its screen'), findsOneWidget);
      expect(
        find.text(
          'It starts by itself if the person at Anna’s Mac allowed this '
          'device to ask.',
        ),
        findsOneWidget,
      );
      expect(
        tester
            .widget<LinearProgressIndicator>(
              find.byType(LinearProgressIndicator),
            )
            .semanticsLabel,
        'Asking for a screen',
      );

      session.releaseStart();
      await done;
      await tester.pump();

      expect(find.byType(Card), findsNothing);
    });

    testWidgets('says that the person wishes to control it', (tester) async {
      final (session, controller) = await _show(tester);
      session.holdStart();

      final done = controller.startScreen(_mac, control: true);
      await tester.pump();

      expect(
        find.text('Asking Anna’s Mac for its screen, to look at and control'),
        findsOneWidget,
      );
      expect(find.text('Asking Anna’s Mac for its screen'), findsNothing);
      expect(
        tester
            .widget<LinearProgressIndicator>(
              find.byType(LinearProgressIndicator),
            )
            .semanticsLabel,
        'Asking for a screen to look at and control',
      );

      session.releaseStart();
      await done;
      await tester.pump();

      expect(find.byType(Card), findsNothing);
    });

    testWidgets('names the device as fit to show', (tester) async {
      final (session, controller) = await _show(tester);
      session.holdStart();

      final done = controller.startScreen(_oddPad);
      await tester.pump();

      expect(find.text('Asking Pad\uFFFDexe for its screen'), findsOneWidget);

      session.releaseStart();
      await done;
      await tester.pump();
    });

    testWidgets('that the device did not start tells the person why', (
      tester,
    ) async {
      final (session, controller) = await _show(tester);
      session.screenStart = const ScreenStarted.notStarted(
        message:
            'Anna’s Mac has not allowed this device to ask for its screen.',
      );

      await controller.startScreen(_mac);
      await tester.pump();

      expect(
        find.text(
          'Anna’s Mac has not allowed this device to ask for its screen.',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('Asking'), findsNothing);
    });
  });

  group('a window that runs', () {
    testWidgets('says who can control it when someone can', (tester) async {
      final (session, _) = await _show(tester);

      await _begin(tester, session, sampleWindowSession(id: 1, control: true));
      await _begin(
        tester,
        session,
        sampleWindowSession(
          id: 2,
          role: WindowRole.sharing,
          device: 'Kitchen iPad',
          control: true,
        ),
      );

      expect(find.text('You can control it.'), findsOneWidget);
      expect(find.text('Kitchen iPad can control it.'), findsOneWidget);
    });

    testWidgets('says nothing of control when it is only looked at', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      await _begin(tester, session, sampleWindowSession(id: 1));
      await _begin(
        tester,
        session,
        sampleWindowSession(id: 2, role: WindowRole.sharing),
      );

      expect(find.textContaining('control it'), findsNothing);
    });

    testWidgets('has the name of the other device made fit to show', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      await _begin(
        tester,
        session,
        sampleWindowSession(id: 1, device: 'Mac\u202Eexe'),
      );
      await _begin(
        tester,
        session,
        sampleWindowSession(
          id: 2,
          role: WindowRole.sharing,
          device: 'Pad\u202Eexe',
        ),
      );

      expect(
        find.text('Showing Budget.numbers from Mac\uFFFDexe'),
        findsOneWidget,
      );
      expect(
        find.text('Sharing Budget.numbers with Pad\uFFFDexe'),
        findsOneWidget,
      );
    });

    testWidgets('and is shown here can be gone back to or stopped', (
      tester,
    ) async {
      final opened = <WindowSession>[];
      final (session, _) = await _show(tester, opened: opened);
      final window = sampleWindowSession();
      await _begin(tester, session, window);

      expect(
        find.text('Showing Budget.numbers from Anna’s Mac'),
        findsOneWidget,
      );
      expect(find.text('Stop sharing'), findsNothing);

      await tester.tap(find.text('Open'));
      await tester.pump();

      expect(opened, [window]);
      expect(session.windowsEnded, isEmpty);

      await tester.tap(find.text('Stop showing'));
      await tester.pump();

      expect(session.windowsEnded, [1]);
      expect(find.byType(Card), findsNothing);
    });

    testWidgets('and is shared from here can be stopped', (tester) async {
      final opened = <WindowSession>[];
      final (session, _) = await _show(tester, opened: opened);
      await _begin(
        tester,
        session,
        sampleWindowSession(
          id: 2,
          role: WindowRole.sharing,
          device: 'Kitchen iPad',
        ),
      );

      expect(
        find.text('Sharing Budget.numbers with Kitchen iPad'),
        findsOneWidget,
      );
      expect(find.text('Open'), findsNothing);
      expect(find.text('Stop showing'), findsNothing);

      await tester.tap(find.text('Stop sharing'));
      await tester.pump();

      expect(session.windowsEnded, [2]);
      expect(opened, isEmpty);
      expect(find.byType(Card), findsNothing);
    });

    testWidgets('is named after its application when it has no title', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      await _begin(tester, session, sampleWindowSession(title: null));

      expect(find.text('Showing Numbers from Anna’s Mac'), findsOneWidget);
    });

    testWidgets('is called a window when it has neither', (tester) async {
      final (session, _) = await _show(tester);

      await _begin(
        tester,
        session,
        sampleWindowSession(title: null, app: null),
      );

      expect(find.text('Showing a window from Anna’s Mac'), findsOneWidget);
    });
  });

  group('how a window ended', () {
    testWidgets('is shown until the person puts it away', (tester) async {
      final (session, _) = await _show(tester);
      await _begin(tester, session, sampleWindowSession());

      await _end(tester, session, 1, const WindowEnd.otherDevice());

      expect(
        find.text('Anna’s Mac stopped sharing Budget.numbers'),
        findsOneWidget,
      );
      expect(find.text('Showing Budget.numbers from Anna’s Mac'), findsNothing);

      await tester.tap(find.byTooltip('Dismiss'));
      await tester.pump();

      expect(find.byType(Card), findsNothing);
    });

    testWidgets('is not shown when the person ended the window', (
      tester,
    ) async {
      final (session, _) = await _show(tester);
      await _begin(tester, session, sampleWindowSession());

      await _end(tester, session, 1, const WindowEnd.thisDevice());

      expect(find.byType(Card), findsNothing);
    });

    testWidgets('is shown with the reason when the connection was lost', (
      tester,
    ) async {
      final (session, _) = await _show(tester);
      await _begin(tester, session, sampleWindowSession());

      await _end(tester, session, 1, const WindowEnd.lost());

      expect(
        find.text('The connection to Anna’s Mac was lost'),
        findsOneWidget,
      );
      expect(
        find.text('The window stopped. Ask Anna’s Mac to share it again.'),
        findsOneWidget,
      );
    });

    testWidgets('is shown with the sentence that Baton gave for a failure', (
      tester,
    ) async {
      final (session, _) = await _show(tester);
      await _begin(
        tester,
        session,
        sampleWindowSession(role: WindowRole.sharing, device: 'Kitchen iPad'),
      );

      await _end(
        tester,
        session,
        1,
        const WindowEnd.failed(message: 'screen recording was turned off'),
      );

      expect(find.text('Sharing with Kitchen iPad stopped'), findsOneWidget);
      expect(find.text('Screen recording was turned off.'), findsOneWidget);
    });

    testWidgets('of a window that was not opened tells the person', (
      tester,
    ) async {
      final (session, controller) = await _show(tester);
      session.windowRequestWaiting = false;
      await _offer(tester, session, sampleWindowRequest);

      await tester.tap(find.text('Show it'));
      await tester.pump();

      expect(controller.notes, hasLength(1));
      expect(
        find.text('Anna’s Mac is not offering that window any more'),
        findsOneWidget,
      );
    });

    testWidgets('of a window that a device did not take tells the person', (
      tester,
    ) async {
      final (session, controller) = await _show(tester);
      session.sharing = const WindowShared.declined(
        message: 'Kitchen iPad did not accept the window.',
      );

      await controller.share(_ipad);
      await tester.pump();

      expect(
        find.text('Kitchen iPad did not accept the window.'),
        findsOneWidget,
      );
    });

    testWidgets('shows a problem as one and the rest as plain news', (
      tester,
    ) async {
      final (session, controller) = await _show(tester);
      await _begin(tester, session, sampleWindowSession(id: 1));
      await _begin(tester, session, sampleWindowSession(id: 2));
      await _end(tester, session, 1, const WindowEnd.otherDevice());
      await _end(tester, session, 2, const WindowEnd.lost());
      expect(controller.notes, hasLength(2));

      // The newest first: the lost connection is a problem, the other device's stop is not.
      final problem = tester.widget<Icon>(find.byType(Icon).first);
      final news = tester.widget<Icon>(find.byType(Icon).at(2));
      expect(problem.icon, Icons.error_outline);
      expect(news.icon, Icons.info_outline);
    });
  });

  testWidgets('many cards scroll inside half of the screen', (tester) async {
    final (session, _) = await _show(tester, size: const Size(900, 700));
    for (var i = 1; i <= 6; i++) {
      await _offer(
        tester,
        session,
        WindowRequest(
          id: i,
          deviceId: 'd$i',
          deviceName: 'Device $i',
          display: false,
          width: 800,
          height: 600,
          control: false,
        ),
      );
    }

    final height = tester.getSize(find.byType(WindowsPanel)).height;

    expect(height, lessThanOrEqualTo(350));
    expect(height, greaterThan(300));
  });

  group('a whole screen', () {
    testWidgets(
      'is offered as the screen of the device, with nothing to name',
      (tester) async {
        final (session, _) = await _show(tester);

        await _offer(tester, session, _screenOffer);

        expect(
          find.text('Anna’s Mac wants to show you its screen'),
          findsOneWidget,
        );
        expect(find.textContaining('a window'), findsNothing);
        // The heading, the line about looking, and the two buttons: no line for what it is.
        expect(
          find.descendant(of: find.byType(Card), matching: find.byType(Text)),
          findsNWidgets(4),
        );
      },
    );

    testWidgets('is opened, shown and shared as a screen', (tester) async {
      final (session, _) = await _show(tester);
      await _offer(tester, session, _screenOffer);

      await tester.tap(find.text('Show it'));
      await tester.pump();

      expect(find.text('Opening the screen from Anna’s Mac…'), findsOneWidget);
      expect(find.text('Opening the window from Anna’s Mac…'), findsNothing);

      await _begin(
        tester,
        session,
        sampleWindowSession(id: 1, display: true, title: null, app: null),
      );
      await _begin(
        tester,
        session,
        sampleWindowSession(
          id: 2,
          role: WindowRole.sharing,
          device: 'Kitchen iPad',
          display: true,
          title: null,
          app: null,
        ),
      );

      expect(find.text('Showing a screen from Anna’s Mac'), findsOneWidget);
      expect(find.text('Sharing a screen with Kitchen iPad'), findsOneWidget);
    });

    testWidgets('is offered with control, and says that it can be controlled', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      await _offer(
        tester,
        session,
        const WindowRequest(
          id: 8,
          deviceId: 'd3',
          deviceName: 'Anna’s Mac',
          display: true,
          width: 3024,
          height: 1964,
          control: true,
        ),
      );

      expect(
        find.text('Anna’s Mac wants to show you its screen'),
        findsOneWidget,
      );
      expect(
        find.text(
          'You will see it live on this screen, and you can control it: what '
          'you touch and type here is done there.',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('only look'), findsNothing);
    });

    testWidgets('says who controls it when it runs', (tester) async {
      final (session, _) = await _show(tester);

      await _begin(
        tester,
        session,
        sampleWindowSession(
          id: 1,
          display: true,
          title: null,
          app: null,
          control: true,
        ),
      );
      await _begin(
        tester,
        session,
        sampleWindowSession(
          id: 2,
          role: WindowRole.sharing,
          device: 'Kitchen iPad',
          display: true,
          title: null,
          app: null,
          control: true,
        ),
      );

      expect(find.text('You can control it.'), findsOneWidget);
      expect(find.text('Kitchen iPad can control it.'), findsOneWidget);
    });

    testWidgets('says nothing of control when it is only looked at', (
      tester,
    ) async {
      final (session, _) = await _show(tester);

      await _begin(
        tester,
        session,
        sampleWindowSession(id: 1, display: true, title: null, app: null),
      );
      await _begin(
        tester,
        session,
        sampleWindowSession(
          id: 2,
          role: WindowRole.sharing,
          device: 'Kitchen iPad',
          display: true,
          title: null,
          app: null,
        ),
      );

      expect(find.textContaining('control it'), findsNothing);
    });

    test('is a screen in what a note says', () {
      final shown = sampleWindowSession(display: true, title: null, app: null);
      final shared = sampleWindowSession(
        role: WindowRole.sharing,
        device: 'Kitchen iPad',
        display: true,
        title: null,
        app: null,
      );

      expect(
        describeWindowNote(WindowOver(shown, const WindowEnd.thisDevice())),
        (title: 'You stopped showing a screen', detail: null, problem: false),
      );
      expect(
        describeWindowNote(WindowOver(shown, const WindowEnd.otherDevice())),
        (
          title: 'Anna’s Mac stopped sharing a screen',
          detail: null,
          problem: false,
        ),
      );
      expect(
        describeWindowNote(WindowOver(shown, const WindowEnd.lost())).detail,
        'The screen stopped. Ask Anna’s Mac to share it again.',
      );
      expect(
        describeWindowNote(WindowOver(shared, const WindowEnd.lost())).detail,
        'Sharing stopped. Share the screen again when Kitchen iPad is back.',
      );
      expect(
        describeWindowNote(
          WindowOver(shown, const WindowEnd.failed(message: 'it broke')),
        ).title,
        'The screen from Anna’s Mac stopped',
      );
      expect(
        describeWindowNote(const WindowWithdrawn(_screenOffer)).title,
        ('Anna’s Mac is not offering that screen any more'),
      );
      expect(
        describeWindowNote(const WindowNotOpened(_screenOffer)).title,
        'The screen from Anna’s Mac did not open',
      );
    });
  });

  group('the name of a window', () {
    test('is its title, else its application, else "a window"', () {
      expect(windowName('Plan', 'Pages'), 'Plan');
      expect(windowName(null, 'Pages'), 'Pages');
      expect(windowName('', 'Pages'), 'Pages');
      expect(windowName('   ', 'Pages'), 'Pages');
      expect(windowName(null, null), 'a window');
      expect(windowName('', ''), 'a window');
      expect(windowName(' ', '\t'), 'a window');
    });

    test('is "a screen" for a display that has no name, or its name', () {
      expect(windowName(null, null, display: true), 'a screen');
      expect(windowName('  ', '', display: true), 'a screen');
      expect(
        windowName('Studio Display', null, display: true),
        'Studio Display',
      );
      expect(windowName(null, null), 'a window');
      expect(windowNoun(display: true), 'screen');
      expect(windowNoun(display: false), 'window');
    });

    test('is made fit to show, and a title that is only marks falls back', () {
      expect(windowName('Plan\u202E', null), 'Plan\uFFFD');
      expect(windowName('Plan\r\n  B\tC', null), 'Plan B C');
      expect(windowName(null, 'Pages\u200B'), 'Pages\uFFFD');
    });
  });

  group('what a note says', () {
    final shown = sampleWindowSession();
    final shared = sampleWindowSession(
      role: WindowRole.sharing,
      device: 'Kitchen iPad',
    );

    test('of a window shown here that ended', () {
      WindowNote over(WindowEnd end) => WindowOver(shown, end);

      expect(describeWindowNote(over(const WindowEnd.thisDevice())), (
        title: 'You stopped showing Budget.numbers',
        detail: null,
        problem: false,
      ));
      expect(describeWindowNote(over(const WindowEnd.otherDevice())), (
        title: 'Anna’s Mac stopped sharing Budget.numbers',
        detail: null,
        problem: false,
      ));
      expect(describeWindowNote(over(const WindowEnd.lost())), (
        title: 'The connection to Anna’s Mac was lost',
        detail: 'The window stopped. Ask Anna’s Mac to share it again.',
        problem: true,
      ));
      expect(
        describeWindowNote(over(const WindowEnd.failed(message: 'it broke'))),
        (
          title: 'The window from Anna’s Mac stopped',
          detail: 'It broke.',
          problem: true,
        ),
      );
    });

    test('of a window shared from here that ended', () {
      WindowNote over(WindowEnd end) => WindowOver(shared, end);

      expect(describeWindowNote(over(const WindowEnd.thisDevice())), (
        title: 'You stopped sharing Budget.numbers',
        detail: null,
        problem: false,
      ));
      expect(describeWindowNote(over(const WindowEnd.otherDevice())), (
        title: 'Kitchen iPad stopped showing Budget.numbers',
        detail: null,
        problem: false,
      ));
      expect(describeWindowNote(over(const WindowEnd.lost())), (
        title: 'The connection to Kitchen iPad was lost',
        detail:
            'Sharing stopped. Share the window again when Kitchen iPad is '
            'back.',
        problem: true,
      ));
      expect(
        describeWindowNote(over(const WindowEnd.failed(message: 'it broke'))),
        (
          title: 'Sharing with Kitchen iPad stopped',
          detail: 'It broke.',
          problem: true,
        ),
      );
    });

    test('of a window that did not begin', () {
      expect(describeWindowNote(WindowWithdrawn(sampleWindowRequest)), (
        title: 'Anna’s Mac is not offering that window any more',
        detail: null,
        problem: false,
      ));
      expect(describeWindowNote(WindowNotOpened(sampleWindowRequest)), (
        title: 'The window from Anna’s Mac did not open',
        detail: 'Ask Anna’s Mac to share it again.',
        problem: true,
      ));
      expect(
        describeWindowNote(
          const WindowFailed('this device cannot show windows'),
        ),
        (
          title: 'This device cannot show windows.',
          detail: null,
          problem: true,
        ),
      );
    });

    test('has the names and the words of other devices made fit to show', () {
      final odd = sampleWindowSession(
        device: 'Mac\u202Eexe',
        title: 'A\u202Eb',
      );

      expect(
        describeWindowNote(WindowOver(odd, const WindowEnd.lost())).title,
        'The connection to Mac\uFFFDexe was lost',
      );
      expect(
        describeWindowNote(WindowOver(odd, const WindowEnd.otherDevice()))
            .title,
        'Mac\uFFFDexe stopped sharing A\uFFFDb',
      );
      expect(
        describeWindowNote(
          WindowOver(odd, const WindowEnd.failed(message: 'bad\u202E')),
        ).detail,
        'Bad\uFFFD.',
      );
      expect(
        describeWindowNote(const WindowFailed('no\u202E')).title,
        'No\uFFFD.',
      );
      expect(
        describeWindowNote(const WindowWithdrawn(_oddOffer)).title,
        'Mac\uFFFDexe is not offering that window any more',
      );
      final notOpened = describeWindowNote(const WindowNotOpened(_oddOffer));
      expect(notOpened.title, 'The window from Mac\uFFFDexe did not open');
      expect(notOpened.detail, 'Ask Mac\uFFFDexe to share it again.');
    });
  });
}
