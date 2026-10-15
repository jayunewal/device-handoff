// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/enter_code_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

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
  _Camera camera,
) async {
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
                builder: (_) =>
                    EnterCodeScreen(session: session, scanner: camera.build),
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
}
