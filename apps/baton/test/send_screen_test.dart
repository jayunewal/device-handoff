// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/awake.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/file_chooser.dart';
import 'package:baton/src/screens/send_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_clipboard.dart';
import 'fake_screen.dart';

// Five minutes after the iPad was last heard from.
final _now = DateTime.fromMillisecondsSinceEpoch((1760529600 + 300) * 1000);

final _kitchenIpad = Device(
  id: 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1',
  name: 'Kitchen iPad',
  platform: Platform.ipadOs,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 1760000000,
  lastSeenAt: 1760529600,
);

FakeSession _session() => FakeSession(
  me: const Me(
    id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
    name: 'Desk Mac',
    platform: Platform.macOs,
    fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
  ),
  pairedDevices: [_kitchenIpad],
);

/// What the person chooses, without a dialog of the system.
class _Chooser implements FileChooser {
  _Chooser({this.canChooseFolders = false});

  @override
  final bool canChooseFolders;

  /// What [files] gives: empty for a dialog that was closed.
  List<String> chosen = [];

  /// What [folder] gives: `null` for a dialog that was closed.
  String? chosenFolder;

  /// What the dialogs throw, if they should fail.
  Object? failure;

  /// How many dialogs were opened.
  int opened = 0;

  @override
  Future<List<String>> files() async {
    opened++;
    final failure = this.failure;
    if (failure != null) throw failure;
    return chosen;
  }

  @override
  Future<String?> folder() async {
    opened++;
    final failure = this.failure;
    if (failure != null) throw failure;
    return chosenFolder;
  }
}

const _sent = FilesSent(files: 2, bytes: 4000, failed: 0, leftOut: 0);

/// Shows the screen for sending to the iPad, on a surface of [size], and returns the session
/// behind it. The screen is never kept on for real, and [chooser] is asked instead of the system.
Future<FakeSession> _show(
  WidgetTester tester, {
  FakeSession? session,
  Size size = const Size(900, 1600),
  FileChooser? chooser,
  ScreenKeeper? screen,
}) async {
  final shown = session ?? _session();
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: SendScreen(
        session: shown,
        device: _kitchenIpad,
        now: () => _now,
        chooser: chooser ?? _Chooser(),
        screen: screen ?? FakeScreen(),
      ),
    ),
  );
  return shown;
}

/// Shows a first screen with a button that opens the screen for sending to the iPad on top of
/// it, so that there is a screen to go back to.
Future<FakeSession> _showOpened(
  WidgetTester tester, {
  required FileChooser chooser,
  FakeScreen? screen,
}) async {
  final shown = _session();
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
                builder: (_) => SendScreen(
                  session: shown,
                  device: _kitchenIpad,
                  now: () => _now,
                  chooser: chooser,
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
  return shown;
}

final _box = find.byType(TextField);

/// How full the bar of the files going is, or `null` for a bar that has nothing to measure.
double? _bar(WidgetTester tester) => tester
    .widget<LinearProgressIndicator>(find.byType(LinearProgressIndicator))
    .value;

String _typed(WidgetTester tester) =>
    tester.widget<TextField>(_box).controller!.text;

/// Whether the button with [label] can be pressed.
bool _enabled(WidgetTester tester, String label) =>
    tester
        .widget<ButtonStyleButton>(
          find.ancestor(
            of: find.text(label),
            matching: find.bySubtype<ButtonStyleButton>(),
          ),
        )
        .onPressed !=
    null;

Future<void> _type(WidgetTester tester, String text) async {
  await tester.enterText(_box, text);
  await tester.pump();
}

void main() {
  testWidgets('names the device, and has nothing to send until there is text', (
    tester,
  ) async {
    await _show(tester);

    expect(find.text('Send to Kitchen iPad'), findsOneWidget);
    expect(
      find.text('iPad · seen 5 minutes ago\nabcd-efgh-ijkl-mnop-qrst'),
      findsOneWidget,
    );
    expect(
      find.text('Baton has to be open on Kitchen iPad for it to arrive.'),
      findsOneWidget,
    );
    expect(find.textContaining('It will be sent as'), findsNothing);
    expect(_enabled(tester, 'Send text'), isFalse);
    expect(_enabled(tester, 'Paste'), isTrue);
  });

  testWidgets('says whether what is typed goes as a link or as text', (
    tester,
  ) async {
    await _show(tester);

    await _type(tester, 'https://example.com/plan');
    expect(
      find.text('It will be sent as a link, which Kitchen iPad can open.'),
      findsOneWidget,
    );
    expect(_enabled(tester, 'Send link'), isTrue);

    await _type(tester, 'Have a look at https://example.com/plan');
    expect(
      find.text('It will be sent as text, which Kitchen iPad can copy.'),
      findsOneWidget,
    );
    expect(_enabled(tester, 'Send text'), isTrue);

    await _type(tester, 'tel:+15551234');
    expect(find.text('Send text'), findsOneWidget);

    await _type(tester, '  \n ');
    expect(find.textContaining('It will be sent as'), findsNothing);
    expect(_enabled(tester, 'Send text'), isFalse);
  });

  testWidgets('sends a link, says so, and empties the box', (tester) async {
    final session = await _show(tester);

    await _type(tester, ' HTTPS://Example.com/plan ');
    await tester.tap(find.text('Send link'));
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => (sent.id, sent.url, sent.text)), [
      (_kitchenIpad.id, 'https://example.com/plan', null),
    ]);
    expect(find.text('Sent the link to Kitchen iPad.'), findsOneWidget);
    expect(_typed(tester), isEmpty);
    expect(_enabled(tester, 'Send text'), isFalse, reason: 'nothing to send');
  });

  testWidgets('sends text exactly as it was typed', (tester) async {
    final session = await _show(tester);
    const typed = '  The door code is\n4711  ';

    await _type(tester, typed);
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();

    expect(session.sent.single.id, _kitchenIpad.id);
    expect(session.sent.single.text, typed);
    expect(session.sent.single.url, isNull);
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);
    expect(_typed(tester), isEmpty);
  });

  testWidgets('says why a send failed, and what was typed stays', (
    tester,
  ) async {
    final session = _session()
      ..sendFailure = const BatonError(
        kind: ErrorKind.unreachable,
        message: 'could not reach Kitchen iPad. Baton has to be open on it',
      );
    await _show(tester, session: session);

    await _type(tester, 'hello');
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();

    expect(
      find.text('Could not reach Kitchen iPad. Baton has to be open on it.'),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.error_outline), findsOneWidget);
    expect(_typed(tester), 'hello');
    expect(_enabled(tester, 'Send text'), isTrue, reason: 'try again');

    // Once it can be reached, the same words go.
    session.sendFailure = null;
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['hello']);
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);
    expect(find.byIcon(Icons.error_outline), findsNothing);
  });

  testWidgets('shows that it is sending, and cannot send twice', (
    tester,
  ) async {
    final session = _session()..holdSend();
    await _show(tester, session: session);
    await _type(tester, 'hello');

    await tester.tap(find.text('Send text'));
    await tester.pump();

    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(
      find.text(
        'Sending to Kitchen iPad. If Baton is not open there, Baton gives up '
        'after a while.',
      ),
      findsOneWidget,
    );
    expect(_enabled(tester, 'Send text'), isFalse);
    expect(_enabled(tester, 'Paste'), isFalse);

    session.releaseSend();
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['hello']);
    expect(find.byType(CircularProgressIndicator), findsNothing);
    expect(find.textContaining('Sending to Kitchen iPad'), findsNothing);
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);
  });

  testWidgets('does not empty the box if something else was typed meanwhile', (
    tester,
  ) async {
    final session = _session()..holdSend();
    await _show(tester, session: session);
    await _type(tester, 'first');
    await tester.tap(find.text('Send text'));
    await tester.pump();

    await _type(tester, 'second');
    session.releaseSend();
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['first']);
    expect(_typed(tester), 'second');
  });

  testWidgets('forgets what happened once something else is typed', (
    tester,
  ) async {
    await _show(tester);
    await _type(tester, 'hello');
    await tester.tap(find.text('Send text'));
    await tester.pumpAndSettle();
    expect(find.text('Sent the text to Kitchen iPad.'), findsOneWidget);

    await _type(tester, 'and another thing');

    expect(find.text('Sent the text to Kitchen iPad.'), findsNothing);
  });

  testWidgets('pastes what is on the clipboard into the box', (tester) async {
    FakeClipboard(tester, holds: 'https://example.com/plan');
    await _show(tester);

    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(_typed(tester), 'https://example.com/plan');
    expect(_enabled(tester, 'Send link'), isTrue);
    expect(find.textContaining('no text on the clipboard'), findsNothing);
  });

  testWidgets('says when there is nothing on the clipboard to paste', (
    tester,
  ) async {
    final clipboard = FakeClipboard(tester);
    await _show(tester);
    await _type(tester, 'keep this');

    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(find.text('There is no text on the clipboard.'), findsOneWidget);
    expect(_typed(tester), 'keep this', reason: 'the box is left as it was');

    clipboard.holds = '  \n ';
    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(find.text('There is no text on the clipboard.'), findsOneWidget);
    expect(_typed(tester), 'keep this', reason: 'blank is nothing');
  });

  testWidgets('checks that the iPad answers, and how', (tester) async {
    final session = await _show(tester);

    await tester.tap(find.text('Check the connection'));
    await tester.pumpAndSettle();

    expect(session.pinged, [_kitchenIpad.id]);
    expect(
      find.text('Kitchen iPad answered in 24 ms, over your local network.'),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.check_circle_outline), findsOneWidget);
  });

  testWidgets('shows that it is waiting for the iPad to answer', (
    tester,
  ) async {
    final session = _session()..holdPing();
    await _show(tester, session: session);

    await tester.tap(find.text('Check the connection'));
    await tester.pump();

    expect(find.text('Waiting for Kitchen iPad to answer.'), findsOneWidget);
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(_enabled(tester, 'Check the connection'), isFalse);

    session.releasePing();
    await tester.pumpAndSettle();

    expect(find.textContaining('Waiting for'), findsNothing);
    expect(
      find.text('Kitchen iPad answered in 24 ms, over your local network.'),
      findsOneWidget,
    );
  });

  testWidgets('says when the iPad does not answer', (tester) async {
    final session = _session()
      ..pingFailure = const BatonError(
        kind: ErrorKind.unreachable,
        message: 'could not reach Kitchen iPad. Baton has to be open on it',
      );
    await _show(tester, session: session);

    await tester.tap(find.text('Check the connection'));
    await tester.pumpAndSettle();

    expect(
      find.text('Could not reach Kitchen iPad. Baton has to be open on it.'),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.error_outline), findsOneWidget);
  });

  testWidgets('a send that ends after the screen is closed does no harm', (
    tester,
  ) async {
    final session = _session()..holdSend();
    await _show(tester, session: session);
    await _type(tester, 'hello');
    await tester.tap(find.text('Send text'));
    await tester.pump();

    await tester.pumpWidget(const SizedBox());
    session.releaseSend();
    await tester.pumpAndSettle();

    expect(session.sent.map((sent) => sent.text), ['hello']);
    expect(tester.takeException(), isNull);
  });

  testWidgets('fits a small phone with the largest text', (tester) async {
    tester.platformDispatcher.textScaleFactorTestValue = 2.5;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    final session = await _show(tester, size: const Size(320, 568));
    // The text this large fills the first screen with the device, so the box is a scroll away.
    final list = find.byType(Scrollable).first;
    await tester.scrollUntilVisible(_box, 100, scrollable: list);
    await _type(tester, 'https://example.com/plan');
    await tester.scrollUntilVisible(
      find.text('Send link'),
      100,
      scrollable: list,
    );
    await tester.pump();
    await tester.tap(find.text('Send link'));
    await tester.pumpAndSettle();

    // An overflow would have been thrown while the tree was laid out.
    expect(tester.takeException(), isNull);
    expect(session.sent, hasLength(1));
  });

  testWidgets(
    'offers files, and no folder where the system cannot choose one',
    (tester) async {
      await _show(tester);

      expect(find.text('Choose files'), findsOneWidget);
      expect(_enabled(tester, 'Choose files'), isTrue);
      expect(find.text('Choose a folder'), findsNothing);
      expect(
        find.text(
          'Kitchen iPad is asked before anything is kept. Baton has to be open '
          'there to be asked.',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets('offers a folder where the system can choose one', (
    tester,
  ) async {
    await _show(tester, chooser: _Chooser(canChooseFolders: true));

    expect(find.text('Choose a folder'), findsOneWidget);
    expect(_enabled(tester, 'Choose a folder'), isTrue);
  });

  testWidgets(
    'sends the files that are chosen, and shows how far they have got',
    (tester) async {
      final chooser = _Chooser()
        ..chosen = ['/Users/me/holiday.jpg', '/Users/me/notes.txt'];
      final session = await _show(tester, chooser: chooser);

      await tester.tap(find.text('Choose files'));
      await tester.pump();

      expect(session.filesSent.map((sent) => sent.id), [_kitchenIpad.id]);
      expect(session.filesSent.map((sent) => sent.paths), [
        ['/Users/me/holiday.jpg', '/Users/me/notes.txt'],
      ]);
      expect(find.text('holiday.jpg and 1 more'), findsOneWidget);
      expect(find.text('Reaching Kitchen iPad…'), findsOneWidget);
      expect(_bar(tester), isNull);
      expect(_enabled(tester, 'Choose files'), isFalse);
      expect(find.text('Stop sending'), findsOneWidget);

      session.transfer.advance(
        stage: SendStage.sending,
        files: 2,
        bytes: 4000,
        sent: 1000,
      );
      await tester.pump(const Duration(milliseconds: 250));

      expect(find.text('Sending to Kitchen iPad…'), findsOneWidget);
      expect(find.text('1.0 KB of 4.0 KB · 2 files'), findsOneWidget);
      expect(_bar(tester), 0.25);

      session.transfer.finish(_sent);
      await tester.pump();

      expect(find.text('Stop sending'), findsNothing);
      expect(
        find.text('Sent 2 files (4.0 KB) to Kitchen iPad.'),
        findsOneWidget,
      );
      expect(find.byIcon(Icons.check_circle_outline), findsOneWidget);
      expect(_enabled(tester, 'Choose files'), isTrue);
    },
  );

  testWidgets('sends the folder that is chosen', (tester) async {
    final chooser = _Chooser(canChooseFolders: true)
      ..chosenFolder = '/Users/me/Photos';
    final session = await _show(tester, chooser: chooser);

    await tester.tap(find.text('Choose a folder'));
    await tester.pump();

    expect(session.filesSent.map((sent) => sent.id), [_kitchenIpad.id]);
    expect(session.filesSent.map((sent) => sent.paths), [
      ['/Users/me/Photos'],
    ]);
    expect(find.text('Photos'), findsOneWidget);
    expect(_enabled(tester, 'Choose a folder'), isFalse);

    session.transfer.finish(_sent);
    await tester.pump();
  });

  testWidgets('sends nothing when a dialog is closed without a choice', (
    tester,
  ) async {
    final chooser = _Chooser(canChooseFolders: true);
    final session = await _show(tester, chooser: chooser);

    await tester.tap(find.text('Choose files'));
    await tester.pump();
    await tester.tap(find.text('Choose a folder'));
    await tester.pump();

    expect(chooser.opened, 2);
    expect(session.filesSent, isEmpty);
    expect(find.text('Stop sending'), findsNothing);
    expect(_enabled(tester, 'Choose files'), isTrue);
  });

  testWidgets('says so when a dialog cannot be opened', (tester) async {
    final chooser = _Chooser()..failure = StateError('no window');
    final session = await _show(tester, chooser: chooser);

    await tester.tap(find.text('Choose files'));
    await tester.pump();

    expect(find.text('Baton could not open the file chooser.'), findsOneWidget);
    expect(session.filesSent, isEmpty);
    expect(_enabled(tester, 'Choose files'), isTrue);
  });

  testWidgets('gives the files up when the person stops, and says no more', (
    tester,
  ) async {
    final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
    final screen = FakeScreen();
    final session = await _show(tester, chooser: chooser, screen: screen);
    await tester.tap(find.text('Choose files'));
    await tester.pump();

    await tester.tap(find.text('Stop sending'));
    await tester.pump();

    expect(session.transfer.cancelled, isTrue);
    expect(find.text('Stop sending'), findsNothing);
    expect(find.byIcon(Icons.error_outline), findsNothing);
    expect(find.byIcon(Icons.check_circle_outline), findsNothing);
    expect(_enabled(tester, 'Choose files'), isTrue);
    expect(screen.on, isFalse);
  });

  testWidgets('says why the files could not be sent', (tester) async {
    final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
    final session = _session()
      ..sendFilesFailure = const BatonError(
        kind: ErrorKind.notPaired,
        message: 'Kitchen iPad is no longer paired with this device',
      );
    await _show(tester, session: session, chooser: chooser);

    await tester.tap(find.text('Choose files'));
    await tester.pump();

    expect(
      find.text('Kitchen iPad is no longer paired with this device.'),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.error_outline), findsOneWidget);
    expect(find.text('Stop sending'), findsNothing);
    expect(_enabled(tester, 'Choose files'), isTrue);
  });

  testWidgets('says so when the other device does not take the files', (
    tester,
  ) async {
    final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
    final session = await _show(tester, chooser: chooser);
    await tester.tap(find.text('Choose files'));
    await tester.pump();

    session.transfer.fail(
      const BatonError(
        kind: ErrorKind.declined,
        message: 'Kitchen iPad did not take the files.',
      ),
    );
    await tester.pump();

    expect(find.text('Kitchen iPad did not take the files.'), findsOneWidget);
    expect(find.byIcon(Icons.error_outline), findsOneWidget);
    expect(find.text('Stop sending'), findsNothing);
  });

  testWidgets(
    'says how many arrived when some did not, and what was left out',
    (tester) async {
      final chooser = _Chooser()..chosen = ['/Users/me/Photos'];
      final session = await _show(tester, chooser: chooser);
      await tester.tap(find.text('Choose files'));
      await tester.pump();

      session.transfer.finish(
        const FilesSent(files: 3, bytes: 6000, failed: 1, leftOut: 2),
      );
      await tester.pump();

      expect(
        find.textContaining(
          'Sent 2 of 3 files to Kitchen iPad. 1 file could not be saved there.',
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining('2 items in what you chose were left out'),
        findsOneWidget,
      );
      expect(find.byIcon(Icons.error_outline), findsOneWidget);
    },
  );

  testWidgets('keeps the screen on while files go, and lets it go after', (
    tester,
  ) async {
    final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
    final screen = FakeScreen();
    final session = await _show(tester, chooser: chooser, screen: screen);
    expect(screen.asked, isEmpty);

    await tester.tap(find.text('Choose files'));
    await tester.pump();
    expect(screen.on, isTrue);

    session.transfer.advance(stage: SendStage.sending, sent: 100);
    await tester.pump(const Duration(milliseconds: 250));
    expect(screen.on, isTrue);

    session.transfer.finish(_sent);
    await tester.pump();
    expect(screen.asked, ['keepOn', 'letTurnOff']);
  });

  testWidgets('lets the screen go when the files could not be sent', (
    tester,
  ) async {
    final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
    final screen = FakeScreen();
    final session = _session()
      ..sendFilesFailure = const BatonError(
        kind: ErrorKind.unreachable,
        message: 'Baton could not reach Kitchen iPad',
      );
    await _show(tester, session: session, chooser: chooser, screen: screen);

    await tester.tap(find.text('Choose files'));
    await tester.pump();

    expect(screen.asked, ['keepOn', 'letTurnOff']);
  });

  testWidgets('a link can be sent while files go', (tester) async {
    final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
    final session = await _show(tester, chooser: chooser);
    await tester.tap(find.text('Choose files'));
    await tester.pump();

    await _type(tester, 'https://example.com/plan');
    await tester.tap(find.text('Send link'));
    await tester.pump();

    expect(session.sent.map((sent) => sent.url), ['https://example.com/plan']);
    expect(_enabled(tester, 'Choose files'), isFalse);

    session.transfer.finish(_sent);
    await tester.pump();
  });

  testWidgets(
    'asks before leaving while files go, and stops them if asked to',
    (tester) async {
      final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
      final screen = FakeScreen();
      final session = await _showOpened(
        tester,
        chooser: chooser,
        screen: screen,
      );
      await tester.tap(find.text('Choose files'));
      await tester.pump();

      await tester.pageBack();
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));

      expect(find.text('Stop sending to Kitchen iPad?'), findsOneWidget);
      expect(
        find.text('What has not arrived yet will not be sent.'),
        findsOneWidget,
      );

      await tester.tap(find.text('Keep sending'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));

      expect(find.text('Stop sending to Kitchen iPad?'), findsNothing);
      expect(find.text('Send to Kitchen iPad'), findsOneWidget);
      expect(session.transfer.cancelled, isFalse);
      expect(screen.on, isTrue);

      await tester.pageBack();
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      await tester.tap(find.text('Stop'));
      await tester.pumpAndSettle();

      expect(session.transfer.cancelled, isTrue);
      expect(find.text('Send to Kitchen iPad'), findsNothing);
      expect(find.text('Open'), findsOneWidget);
      expect(screen.on, isFalse);
    },
  );

  testWidgets('leaves at once when no files are going', (tester) async {
    final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
    final session = await _showOpened(tester, chooser: chooser);

    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.text('Open'), findsOneWidget);

    await tester.tap(find.text('Open'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Choose files'));
    await tester.pump();
    session.transfer.finish(_sent);
    await tester.pump();

    await tester.pageBack();
    await tester.pumpAndSettle();

    expect(find.text('Stop sending to Kitchen iPad?'), findsNothing);
    expect(find.text('Open'), findsOneWidget);
  });

  testWidgets(
    'files that were still starting when the screen closed are not sent',
    (tester) async {
      final chooser = _Chooser()..chosen = ['/Users/me/holiday.jpg'];
      final session = _session()..holdSendFiles();
      await _show(tester, session: session, chooser: chooser);
      await tester.tap(find.text('Choose files'));
      await tester.pump();

      await tester.pumpWidget(const SizedBox());
      session.releaseSendFiles();
      await tester.pump();

      expect(session.transfer.cancelled, isTrue);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('fits a small phone with the largest text while files go', (
    tester,
  ) async {
    tester.platformDispatcher.textScaleFactorTestValue = 2.5;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    final chooser = _Chooser(canChooseFolders: true)
      ..chosen = ['/Users/me/a-long-name-for-a-photo-of-the-whole-holiday.jpg'];
    final session = await _show(
      tester,
      size: const Size(320, 568),
      chooser: chooser,
    );
    final list = find.byType(Scrollable).first;
    await tester.scrollUntilVisible(
      find.text('Choose files'),
      100,
      scrollable: list,
    );
    await tester.pump();
    await tester.tap(find.text('Choose files'));
    await tester.pump();
    session.transfer.advance(
      stage: SendStage.sending,
      files: 3,
      bytes: 4000000,
      sent: 1000000,
    );
    await tester.pump(const Duration(milliseconds: 250));
    await tester.scrollUntilVisible(
      find.text('Stop sending'),
      100,
      scrollable: list,
    );
    await tester.pump();
    expect(tester.takeException(), isNull);

    session.transfer.finish(_sent);
    await tester.pump();
    expect(tester.takeException(), isNull);
  });
}
