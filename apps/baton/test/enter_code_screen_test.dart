// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:baton/src/awake.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/enter_code_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

const _anna = Device(
  id: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  name: 'Anna’s iPhone',
  platform: Platform.ios,
  fingerprint: 'iphn-abcd-efgh-ijkl-mnop',
  pairedAt: 1760529700,
);

const _notATicket = BatonError(
  kind: ErrorKind.badTicket,
  message: 'That is not a Baton code. It should start with BATON1:.',
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
);

/// A camera that sees what a test shows it.
class _Camera {
  ValueChanged<String>? _onCode;

  /// Whether the camera is on, that is, whether the screen has one in it.
  bool get on => _onCode != null;

  Widget build(BuildContext context, ValueChanged<String> onCode) {
    _onCode = onCode;
    return _Switch(onDispose: () => _onCode = null);
  }

  /// Holds a QR code up to the camera.
  void see(String text) => _onCode!(text);
}

class _Switch extends StatefulWidget {
  const _Switch({required this.onDispose});

  final VoidCallback onDispose;

  @override
  State<_Switch> createState() => _SwitchState();
}

class _SwitchState extends State<_Switch> {
  @override
  void dispose() {
    widget.onDispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => const Text('(camera)');
}

/// Opens the screen from a first screen with an Open button, so that leaving it can be seen.
Future<void> _open(
  WidgetTester tester,
  FakeSession session,
  _Camera camera, {
  ScreenKeeper? screen,
}) async {
  tester.view
    ..physicalSize = const Size(900, 1600)
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: Builder(
        builder: (context) => Scaffold(
          body: TextButton(
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute<void>(
                builder: (_) => EnterCodeScreen(
                  session: session,
                  scanner: camera.build,
                  screen: screen ?? FakeScreen(),
                ),
              ),
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open'));
  await tester.pumpAndSettle();
}

void _mockClipboard(WidgetTester tester, {String? holds}) {
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
    SystemChannels.platform,
    (call) async => call.method == 'Clipboard.getData' && holds != null
        ? <String, Object?>{'text': holds}
        : null,
  );
  addTearDown(
    () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      null,
    ),
  );
}

/// Something that is not a code is tried and turned down while its stream stays open, and then
/// a code is tried. Gives the stream of the first try.
Future<StreamController<JoinProgress>> _turnedDownThenTried(
  WidgetTester tester,
  FakeSession session,
  _Camera camera,
) async {
  camera.see('https://example.com/menu');
  await tester.pump();
  final earlier = session.joining;
  earlier.add(const JoinProgress.failed(error: _notATicket));
  await tester.pumpAndSettle();

  camera.see(sampleTicket);
  await tester.pump();
  expect(session.joined, ['https://example.com/menu', sampleTicket]);
  expect(find.text('Reaching the other device…'), findsOneWidget);
  return earlier;
}

void main() {
  testWidgets('pairs with the code that the camera reads', (tester) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    expect(find.text('(camera)'), findsOneWidget);

    camera.see(sampleTicket);
    await tester.pump();
    expect(session.joined, [sampleTicket]);
    expect(find.text('Reaching the other device…'), findsOneWidget);
    expect(camera.on, isFalse, reason: 'the camera is off once it has a code');

    session.joining.add(const JoinProgress.code(code: '514 643'));
    await tester.pump();
    expect(find.text('Check the other device'), findsOneWidget);
    expect(find.text('514 643'), findsOneWidget);

    session.joining.add(const JoinProgress.paired(device: _anna));
    await tester.pump();
    expect(find.text('Paired with Anna’s iPhone'), findsOneWidget);

    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();
    expect(
      find.text('Open'),
      findsOneWidget,
      reason: 'back on the first screen',
    );
    expect(session.joinsCancelled, 0);
  });

  testWidgets('starts one pairing, however often the camera sees the code', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);

    // A camera sees the same code again and again, before the screen has moved on.
    camera.see(sampleTicket);
    camera.see(sampleTicket);
    camera.see(sampleTicket);
    await tester.pump();

    expect(session.joined, [sampleTicket]);
  });

  testWidgets('pairs with a code that was pasted in', (tester) async {
    final session = _session();
    await _open(tester, session, _Camera());

    await tester.enterText(
      find.byType(TextField),
      'baton pair join $sampleTicket\n',
    );
    await tester.tap(find.text('Connect'));
    await tester.pump();

    expect(session.joined, ['baton pair join $sampleTicket\n']);
    expect(find.text('Reaching the other device…'), findsOneWidget);
  });

  testWidgets('pastes what is on the clipboard', (tester) async {
    _mockClipboard(tester, holds: sampleTicket);
    final session = _session();
    await _open(tester, session, _Camera());

    await tester.tap(find.byTooltip('Paste'));
    await tester.pumpAndSettle();

    expect(find.text(sampleTicket), findsOneWidget);
    expect(session.joined, isEmpty, reason: 'the person connects');
  });

  testWidgets('has nothing to paste when the clipboard is empty', (
    tester,
  ) async {
    _mockClipboard(tester);
    await _open(tester, _session(), _Camera());

    await tester.tap(find.byTooltip('Paste'));
    await tester.pumpAndSettle();

    final field = tester.widget<TextField>(find.byType(TextField));
    expect(field.controller!.text, isEmpty);
  });

  testWidgets('does not connect without a code', (tester) async {
    final session = _session();
    await _open(tester, session, _Camera());

    await tester.tap(find.text('Connect'));
    await tester.enterText(find.byType(TextField), '   ');
    await tester.tap(find.text('Connect'));
    await tester.pump();

    expect(session.joined, isEmpty);
  });

  testWidgets('says what is wrong with text that is not a code', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);

    await tester.enterText(find.byType(TextField), 'hello');
    await tester.tap(find.text('Connect'));
    await tester.pump();
    session.joining.add(const JoinProgress.failed(error: _notATicket));
    await tester.pumpAndSettle();

    expect(find.text(_notATicket.message), findsOneWidget);
    expect(find.text('(camera)'), findsOneWidget, reason: 'try again at once');
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      'hello',
      reason: 'what was typed is still there to fix',
    );

    // The same text, sent again, is looked at again and gets the same answer.
    await tester.tap(find.text('Connect'));
    await tester.pump();
    expect(session.joined, ['hello', 'hello']);
  });

  testWidgets('does not start over for a picture that is not a code', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);

    camera.see('https://example.com/menu');
    await tester.pump();
    session.joining.add(const JoinProgress.failed(error: _notATicket));
    await tester.pumpAndSettle();
    expect(find.text(_notATicket.message), findsOneWidget);

    // The camera is still pointed at it, and sees it again and again.
    camera.see('https://example.com/menu');
    camera.see('https://example.com/menu');
    await tester.pump();
    expect(session.joined, ['https://example.com/menu']);

    camera.see(sampleTicket);
    await tester.pump();
    expect(session.joined, ['https://example.com/menu', sampleTicket]);
    expect(find.text('Reaching the other device…'), findsOneWidget);
  });

  testWidgets('hears nothing more from a try once a new one has begun', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    final earlier = await _turnedDownThenTried(tester, session, camera);

    // The earlier stream has not ended, and it says one more thing.
    earlier.add(const JoinProgress.paired(device: _anna));
    await tester.pump();

    expect(find.text('Paired with Anna’s iPhone'), findsNothing);
    expect(find.text('Reaching the other device…'), findsOneWidget);
    unawaited(earlier.close());
  });

  testWidgets('an earlier try that ends late does not end the new one', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    final earlier = await _turnedDownThenTried(tester, session, camera);

    unawaited(earlier.close());
    await tester.pump();

    expect(find.text('Pairing stopped before it was finished.'), findsNothing);
    expect(find.text('Reaching the other device…'), findsOneWidget);
  });

  testWidgets('says why a pairing failed, and goes back to the start', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    camera.see(sampleTicket);
    await tester.pump();
    session.joining.add(const JoinProgress.code(code: '514 643'));
    await tester.pump();

    session.joining.add(
      const JoinProgress.failed(
        error: BatonError(
          kind: ErrorKind.pairingFailed,
          message: 'The other device said no.',
        ),
      ),
    );
    await tester.pump();
    expect(find.text('Pairing did not work'), findsOneWidget);
    expect(find.text('The other device said no.'), findsOneWidget);
    expect(find.text('514 643'), findsNothing);

    await tester.tap(find.text('Try again'));
    await tester.pumpAndSettle();
    expect(find.text('(camera)'), findsOneWidget);
    expect(find.text('The other device said no.'), findsNothing);
  });

  testWidgets('does not leave the person waiting if the pairing stops', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    camera.see(sampleTicket);
    await tester.pump();

    await session.joining.close();
    await tester.pump();

    expect(find.text('Pairing did not work'), findsOneWidget);
    expect(
      find.text('Pairing stopped before it was finished.'),
      findsOneWidget,
    );
  });

  testWidgets('does not leave the person waiting if the pairing breaks', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    camera.see(sampleTicket);
    await tester.pump();

    session.joining.addError(StateError('the bridge broke'));
    await tester.pump();

    expect(find.text('Pairing did not work'), findsOneWidget);
    expect(find.textContaining('the bridge broke'), findsOneWidget);
  });

  testWidgets('gives up on a pairing that Cancel is pressed in', (
    tester,
  ) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    camera.see(sampleTicket);
    await tester.pump();

    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    expect(session.joinsCancelled, 1);
    expect(find.text('Open'), findsOneWidget);
  });

  testWidgets('gives up on a pairing when the screen is left', (tester) async {
    final session = _session();
    final camera = _Camera();
    await _open(tester, session, camera);
    camera.see(sampleTicket);
    await tester.pump();
    session.joining.add(const JoinProgress.code(code: '514 643'));
    await tester.pump();

    await tester.pageBack();
    await tester.pumpAndSettle();

    expect(session.joinsCancelled, 1);
  });

  testWidgets('has nothing to give up when the screen is left early', (
    tester,
  ) async {
    final session = _session();
    await _open(tester, session, _Camera());

    await tester.pageBack();
    await tester.pumpAndSettle();

    expect(session.joinsCancelled, 0);
  });

  group('the screen of the device', () {
    testWidgets('stays on while the camera waits and the pairing happens', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      final camera = _Camera();
      await _open(tester, session, camera, screen: screen);
      expect(screen.asked, ['keepOn'], reason: 'on as soon as the camera is');

      camera.see(sampleTicket);
      await tester.pump();
      session.joining.add(const JoinProgress.code(code: '514 643'));
      await tester.pump();
      expect(find.text('Check the other device'), findsOneWidget);
      expect(screen.asked, ['keepOn'], reason: 'asked once, and still on');

      session.joining.add(const JoinProgress.paired(device: _anna));
      await tester.pump();
      expect(find.text('Paired with Anna’s iPhone'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('stays on after a picture that is not a code', (tester) async {
      final screen = FakeScreen();
      final session = _session();
      final camera = _Camera();
      await _open(tester, session, camera, screen: screen);

      camera.see('https://example.com/menu');
      await tester.pump();
      session.joining.add(const JoinProgress.failed(error: _notATicket));
      await tester.pumpAndSettle();

      expect(find.text(_notATicket.message), findsOneWidget);
      expect(screen.asked, ['keepOn'], reason: 'the camera is still waiting');
    });

    testWidgets('is let go when pairing fails, and kept on to try again', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      final camera = _Camera();
      await _open(tester, session, camera, screen: screen);
      camera.see(sampleTicket);
      await tester.pump();

      session.joining.add(
        const JoinProgress.failed(
          error: BatonError(
            kind: ErrorKind.pairingFailed,
            message: 'The other device said no.',
          ),
        ),
      );
      await tester.pump();
      expect(find.text('Pairing did not work'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      await tester.tap(find.text('Try again'));
      await tester.pumpAndSettle();
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
    });

    testWidgets('is let go if the pairing stops without a word', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      final camera = _Camera();
      await _open(tester, session, camera, screen: screen);
      camera.see(sampleTicket);
      await tester.pump();

      await session.joining.close();
      await tester.pump();

      expect(find.text('Pairing did not work'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('is let go when the screen is left', (tester) async {
      final screen = FakeScreen();
      await _open(tester, _session(), _Camera(), screen: screen);
      expect(screen.on, isTrue);

      await tester.pageBack();
      await tester.pumpAndSettle();

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('is let go when a pairing is cancelled', (tester) async {
      final screen = FakeScreen();
      final session = _session();
      final camera = _Camera();
      await _open(tester, session, camera, screen: screen);
      camera.see(sampleTicket);
      await tester.pump();

      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });

    testWidgets('is let go after the limit, if the screen is left open', (
      tester,
    ) async {
      final screen = FakeScreen();
      final session = _session();
      final camera = _Camera();
      await _open(tester, session, camera, screen: screen);

      await tester.pump(awakeLimit + const Duration(seconds: 1));
      expect(screen.asked, ['keepOn', 'letTurnOff']);

      // The person came back and held the camera up. Going on to pair keeps the screen on, so
      // that it does not lock while the other device says yes.
      camera.see(sampleTicket);
      await tester.pump();
      expect(find.text('Reaching the other device…'), findsOneWidget);
      expect(screen.asked, ['keepOn', 'letTurnOff', 'keepOn']);
    });

    testWidgets('counts the limit again from each step', (tester) async {
      final screen = FakeScreen();
      final session = _session();
      final camera = _Camera();
      await _open(tester, session, camera, screen: screen);

      // Each step comes just before the limit would have been reached, and the pairing is
      // still going on after more than the limit has passed.
      await tester.pump(awakeLimit - const Duration(minutes: 1));
      camera.see(sampleTicket);
      await tester.pump();
      await tester.pump(awakeLimit - const Duration(minutes: 1));
      session.joining.add(const JoinProgress.code(code: '514 643'));
      await tester.pump();
      await tester.pump(awakeLimit - const Duration(minutes: 1));
      expect(find.text('Check the other device'), findsOneWidget);
      expect(screen.asked, ['keepOn'], reason: 'asked once, and still on');

      // And with no step after it the screen is let go.
      await tester.pump(const Duration(minutes: 2));
      expect(screen.asked, ['keepOn', 'letTurnOff']);
    });
  });
}
