// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/screens/transfers_panel.dart';
import 'package:baton/src/transfers_controller.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';
import 'fake_screen.dart';

/// Shows the panel at the bottom of a screen of [size], as the home screen does, and returns the
/// session behind it and the controller.
Future<(FakeSession, TransfersController)> _show(
  WidgetTester tester, {
  TargetPlatform platform = TargetPlatform.macOS,
  Size size = const Size(900, 1600),
  FakeSession? session,
}) async {
  final shown =
      session ??
      FakeSession(
        me: const Me(
          id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
          name: 'Desk Mac',
          platform: Platform.macOs,
          fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
        ),
      );
  final controller = TransfersController(shown, screen: FakeScreen());
  await controller.start();
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: _Host(controller: controller, platform: platform),
    ),
  );
  return (shown, controller);
}

/// The screen that owns the controller and shows the panel at its bottom, as the home screen
/// does: the controller is let go of when the screen is, which is when a hold on the screen of
/// the device ends.
class _Host extends StatefulWidget {
  const _Host({required this.controller, required this.platform});

  final TransfersController controller;
  final TargetPlatform platform;

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
    bottomNavigationBar: TransfersPanel(
      controller: widget.controller,
      platform: widget.platform,
    ),
  );
}

/// Hands [request] to the panel, as Baton does when a device asks to send files.
Future<void> _ask(
  WidgetTester tester,
  FakeSession session,
  TransferRequest request,
) async {
  session.emit(Event.transferRequested(request: request));
  await tester.pump();
}

void main() {
  testWidgets('takes no room when there is nothing to show', (tester) async {
    await _show(tester);

    expect(find.byType(Card), findsNothing);
    expect(tester.getSize(find.byType(TransfersPanel)).height, 0);
  });

  testWidgets(
    'a request shows who asks, what is offered and where it will go',
    (tester) async {
      final (session, _) = await _show(tester);

      await _ask(tester, session, sampleTransfer());

      expect(
        find.text('Anna’s iPhone wants to send you 2 files'),
        findsOneWidget,
      );
      expect(find.text('2 files, 3.2 MB'), findsOneWidget);
      expect(find.text('holiday.jpg'), findsOneWidget);
      expect(find.text('2.0 MB'), findsOneWidget);
      expect(find.text('notes.txt'), findsOneWidget);
      expect(find.text('1.2 MB'), findsOneWidget);
      expect(find.text('They go in your Downloads folder.'), findsOneWidget);
      expect(find.text('Accept'), findsOneWidget);
      expect(find.text('Decline'), findsOneWidget);
      expect(
        session.transferAnswers,
        isEmpty,
        reason: 'nothing is answered for the person',
      );
    },
  );

  testWidgets(
    'on an iPhone the files go to the folder that the Files app shows',
    (tester) async {
      final (session, _) = await _show(tester, platform: TargetPlatform.iOS);
      session.receiveFolder =
          '/var/mobile/Containers/Data/Application/ID/Documents';

      await _ask(tester, session, sampleTransfer());

      expect(
        find.text('They go in the Baton folder in the Files app.'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'one file is a file, and a folder says how many files are in it',
    (tester) async {
      final (session, _) = await _show(tester);

      await _ask(
        tester,
        session,
        sampleTransfer(
          id: 1,
          files: 1,
          bytes: 5,
          preview: const [
            PreviewName(name: 'a.txt', folder: false, files: 0, bytes: 5),
          ],
        ),
      );
      expect(
        find.text('Anna’s iPhone wants to send you a file'),
        findsOneWidget,
      );
      expect(find.text('1 file, 5 bytes'), findsOneWidget);

      await _ask(
        tester,
        session,
        sampleTransfer(
          id: 2,
          from: 'Desk Mac',
          files: 12,
          folders: 2,
          bytes: 31000000,
          preview: const [
            PreviewName(
              name: 'Photos',
              folder: true,
              files: 12,
              bytes: 31000000,
            ),
          ],
        ),
      );
      expect(
        find.text('Desk Mac wants to send you 12 files in 2 folders'),
        findsOneWidget,
      );
      expect(find.text('12 files in 2 folders, 31 MB'), findsOneWidget);
      expect(find.text('Photos'), findsOneWidget);
      expect(
        find.text('12 files, 31 MB'),
        findsOneWidget,
        reason: 'the folder',
      );
    },
  );

  testWidgets('what is not shown is counted', (tester) async {
    final (session, _) = await _show(tester);

    await _ask(
      tester,
      session,
      TransferRequest(
        id: 3,
        deviceId: 'b0' * 32,
        deviceName: 'Anna’s iPhone',
        items: 7,
        files: 7,
        folders: 0,
        bytes: 7000,
        preview: const [
          PreviewName(name: 'one', folder: false, files: 0, bytes: 1000),
          PreviewName(name: 'two', folder: false, files: 0, bytes: 1000),
        ],
      ),
    );

    expect(find.text('and 5 more'), findsOneWidget);
  });

  testWidgets('names that another device wrote cannot turn the text round', (
    tester,
  ) async {
    final (session, _) = await _show(tester);

    await _ask(
      tester,
      session,
      sampleTransfer(
        from: 'Anna\u202E’s iPhone',
        preview: const [
          PreviewName(
            name: 'invoice\u202Etxt.exe',
            folder: false,
            files: 0,
            bytes: 10,
          ),
        ],
      ),
    );

    expect(find.text('invoice�txt.exe'), findsOneWidget);
    expect(find.textContaining('\u202E'), findsNothing);
    expect(find.textContaining('Anna�’s iPhone'), findsWidgets);
  });

  testWidgets(
    'yes is told to Baton, and the card becomes a transfer on its way',
    (tester) async {
      final (session, _) = await _show(tester);
      await _ask(tester, session, sampleTransfer(id: 9));

      await tester.tap(find.text('Accept'));
      await tester.pump();

      expect(session.transferAnswers, [(request: 9, accept: true)]);
      expect(find.text('Accept'), findsNothing);
      expect(find.text('Receiving from Anna’s iPhone…'), findsOneWidget);
      expect(
        find.textContaining('Keep Baton open until it is done.'),
        findsOneWidget,
      );
      expect(find.byType(LinearProgressIndicator), findsOneWidget);
    },
  );

  testWidgets('no is told to Baton, and the card goes', (tester) async {
    final (session, _) = await _show(tester);
    await _ask(tester, session, sampleTransfer(id: 9));

    await tester.tap(find.text('Decline'));
    await tester.pump();

    expect(session.transferAnswers, [(request: 9, accept: false)]);
    expect(find.byType(Card), findsNothing);
  });

  testWidgets('a request that the other device gave up goes by itself', (
    tester,
  ) async {
    final (session, _) = await _show(tester);
    await _ask(tester, session, sampleTransfer(id: 9));

    session.emit(const Event.transferRequestEnded(id: 9));
    await tester.pump();

    expect(find.byType(Card), findsNothing);
    expect(session.transferAnswers, isEmpty);
  });

  testWidgets(
    'files that arrived are said to be where they are, until they are put away',
    (tester) async {
      final (session, _) = await _show(tester);
      final request = sampleTransfer(id: 9);
      await _ask(tester, session, request);
      await tester.tap(find.text('Accept'));
      await tester.pump();

      session.emit(Event.transferEnded(end: sampleTransferEnd(request)));
      await tester.pump();

      expect(find.text('Received 2 files from Anna’s iPhone'), findsOneWidget);
      expect(find.text('They are in your Downloads folder.'), findsOneWidget);
      expect(find.byIcon(Icons.check_circle_outline), findsOneWidget);
      expect(find.byType(LinearProgressIndicator), findsNothing);

      await tester.tap(find.byTooltip('Dismiss'));
      await tester.pump();
      expect(find.byType(Card), findsNothing);
    },
  );

  testWidgets('files that could not all be saved are told as that', (
    tester,
  ) async {
    final (session, controller) = await _show(tester);
    final request = sampleTransfer(files: 3);

    session.emit(
      Event.transferEnded(
        end: sampleTransferEnd(
          request,
          result: const TransferResult.received(failed: 1),
        ),
      ),
    );
    await tester.pump();
    expect(
      find.text('Received 2 of 3 files from Anna’s iPhone'),
      findsOneWidget,
    );
    expect(
      find.text(
        '1 file could not be saved. The others are in your Downloads folder.',
      ),
      findsOneWidget,
    );
    expect(find.byIcon(Icons.error_outline), findsOneWidget);

    for (final end in controller.ended) {
      controller.dismiss(end);
    }
    session.emit(
      Event.transferEnded(
        end: sampleTransferEnd(
          sampleTransfer(files: 1),
          result: const TransferResult.received(failed: 1),
        ),
      ),
    );
    await tester.pump();
    expect(
      find.text('Nothing from Anna’s iPhone could be saved'),
      findsOneWidget,
    );
    expect(
      find.text(
        'Baton could not save the file. Ask Anna’s iPhone to send it again.',
      ),
      findsOneWidget,
    );
  });

  testWidgets('a transfer that was given up says that nothing was kept', (
    tester,
  ) async {
    final (session, _) = await _show(tester);

    session.emit(
      Event.transferEnded(
        end: sampleTransferEnd(
          sampleTransfer(),
          result: const TransferResult.dropped(),
        ),
      ),
    );
    await tester.pump();

    expect(
      find.text('The transfer from Anna’s iPhone stopped'),
      findsOneWidget,
    );
    expect(
      find.text(
        'Nothing was kept. Anna’s iPhone stopped sending, or the connection was lost.',
      ),
      findsOneWidget,
    );
  });

  testWidgets('a device that cannot take files says why', (tester) async {
    final session =
        FakeSession(
            me: const Me(
              id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
              name: 'Desk Mac',
              platform: Platform.macOs,
              fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
            ),
          )
          ..taking = const Receiving.unusable(
            message: 'The folder for received files is not there.',
          );
    await _show(tester, session: session);

    expect(find.text('This device cannot take files'), findsOneWidget);
    expect(
      find.text('The folder for received files is not there.'),
      findsOneWidget,
    );
  });

  testWidgets('a device that does not take files, by choice, shows nothing', (
    tester,
  ) async {
    final session = FakeSession(
      me: const Me(
        id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
        name: 'Desk Mac',
        platform: Platform.macOs,
        fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
      ),
    )..taking = const Receiving.off();
    await _show(tester, session: session);

    expect(find.byType(Card), findsNothing);
  });

  testWidgets('many cards scroll inside half of the screen', (tester) async {
    final (session, _) = await _show(tester, size: const Size(400, 600));

    for (var id = 1; id <= 6; id++) {
      await _ask(tester, session, sampleTransfer(id: id));
    }

    expect(
      tester.getSize(find.byType(TransfersPanel)).height,
      lessThanOrEqualTo(300),
    );
    expect(find.text('Accept'), findsWidgets);
  });
}
