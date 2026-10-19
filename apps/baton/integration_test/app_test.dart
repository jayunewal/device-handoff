// SPDX-License-Identifier: Apache-2.0

// The app as it runs on a Mac or in an iPhone simulator: the real Rust library, found the way an
// installed app finds it, and the real screens. CI runs it on both. It is the first place where
// the build settings of the iOS and macOS projects, which nothing else here can try, meet the
// library.
//
//     flutter test integration_test -d macos
//
// Nothing here uses the internet: the settings it writes keep Baton to this computer.

import 'dart:async';
import 'dart:io' as io show Platform;
import 'dart:io' show Directory, File;

import 'package:baton/src/app.dart';
import 'package:baton/src/awake.dart';
import 'package:baton/src/backend.dart';
import 'package:baton/src/file_shower.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:baton/src/support_folder.dart';
import 'package:flutter/foundation.dart' show defaultTargetPlatform;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show MethodChannel;
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  // The system takes the screen away from the app for a moment now and then, and Baton lets go
  // of the screen of the device then. If a check of it fails, the log shows when that was.
  AppLifecycleListener(
    onStateChange: (state) => debugPrint(
      '${DateTime.now().toIso8601String()} the app is ${state.name}',
    ),
  );

  testWidgets('names the device, starts Baton and shows the device', (
    tester,
  ) async {
    final folder = await _folder();

    await tester.pumpWidget(
      BatonApp(backend: RustBackend(dataDir: folder.path)),
    );
    await _see(tester, find.text('Name this device'));

    await _type(tester, 'Test device');
    await _tap(tester, find.text('Continue'));
    await _see(tester, find.text('No other devices yet'));
    await _expectScreenKeptOn(
      tester,
      kept: false,
      when: 'when no one is pairing',
    );

    // The name went through the library and came back, and so did the fingerprint.
    expect(find.text('Test device'), findsOneWidget);
    expect(
      find.textContaining(RegExp(r'^(\w{4}-){4}\w{4}$')),
      findsOneWidget,
      reason: 'the fingerprint of this device',
    );

    // Closing the app stops Baton, which lets go of the folder.
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets(
    'finds the folder for its files from the system, and the library opens it',
    (tester) async {
      // An iPhone with iOS 27 starts an app without HOME in its environment, and so does the
      // simulator that CI starts, and a Baton that looked for HOME did not start. Every other
      // test in this file is given a folder, so none of them would have noticed.
      final path = await const SupportFolder().path();

      expect(path, startsWith('/'));
      expect(path, endsWith('/Library/Application Support'));
      expect(
        Directory(path).existsSync(),
        isTrue,
        reason: 'the system made it',
      );
      // The iOS app makes Baton's folder there when it opens, and leaves it out of backups.
      expect(
        Directory('$path/${RustBackend.folderName}').existsSync(),
        isTrue,
        reason: 'the folder that the app made for Baton',
      );
      // The app looks for the folder the way it does when a person opens it, and the library
      // opens it. Asking for the name reads it and changes nothing.
      await RustBackend().deviceName();
    },
    skip: defaultTargetPlatform != TargetPlatform.iOS,
  );

  testWidgets(
    'finds the folder for received files from the system, and takes files into it',
    (tester) async {
      // The person finds what arrives in the Files app on a phone and in Downloads on a Mac, and
      // the apps answer where that is (ios/Runner/AppDelegate.swift and
      // macos/Runner/MainFlutterWindow.swift). Nothing else here can try those answers.
      final path = await const SupportFolder().receiveFolder();

      expect(path, startsWith('/'));
      expect(
        path,
        endsWith(
          defaultTargetPlatform == TargetPlatform.iOS
              ? '/Documents'
              : '/Downloads',
        ),
      );
      expect(
        Directory(path).existsSync(),
        isTrue,
        reason: 'the system made it, or it was there',
      );
      if (defaultTargetPlatform == TargetPlatform.macOS) {
        // The sandbox gives the app a Downloads folder of its own inside its container, which the
        // person does not look in, and the app falls back to it when it cannot write to the real
        // one. The entitlement is what lets it use the real one, and nothing else shows it.
        expect(
          path,
          isNot(contains('/Library/Containers/')),
          reason:
              'the Downloads folder of the person, not the one in the sandbox',
        );
      }
      // A file can be made in it, which is what taking files needs.
      final probe = File('$path/.baton-integration-probe');
      await probe.writeAsString('x');
      await probe.delete();

      // The library takes files into it.
      final backend = RustBackend(dataDir: (await _folder()).path);
      await backend.setDeviceName('Test device');
      final session = await backend.start();
      addTearDown(session.stop);
      expect(session.receiving(), isA<Receiving_On>());
      expect(session.receiveFolder, path);
    },
    skip:
        defaultTargetPlatform != TargetPlatform.iOS &&
        defaultTargetPlatform != TargetPlatform.macOS,
  );

  testWidgets(
    'asks the system to show files, and is told when they are not there',
    (tester) async {
      // Finder on a Mac and the Files app on a phone show what arrived, and the apps ask them
      // (macos/Runner/MainFlutterWindow.swift and ios/Runner/AppDelegate.swift). Showing a file
      // takes the person out of the app, which a test cannot have, so this asks for one that is
      // not there. The answer says that the question reached the code and was understood, and
      // that nothing opens for a path that leads nowhere.
      final folder = await _folder();

      expect(
        await const SystemFileShower().show(['${folder.path}/not-there.txt']),
        isFalse,
      );
    },
    skip:
        defaultTargetPlatform != TargetPlatform.iOS &&
        defaultTargetPlatform != TargetPlatform.macOS,
  );

  testWidgets(
    'shows windows with the viewer that the Swift code gave the library',
    (tester) async {
      // Flutter registers the plugin of the app (rust_builder/macos/Classes/BatonStreamPlugin.swift)
      // before any Dart runs, and the plugin gives the library the viewer with which this device
      // shows the windows of other devices. Nothing else here can try that chain: a plugin that was
      // not registered, or a table that the library did not take, would leave a device that says no
      // to every window that another one offers.
      final backend = RustBackend(dataDir: (await _folder()).path);
      await backend.setDeviceName('Test device');
      final session = await backend.start();
      addTearDown(session.stop);

      final status = await _streamStatus();
      expect(
        session.windowAbilities().canShow,
        isTrue,
        reason: 'the plugin says: $status',
      );
      expect(status['viewer'], 0, reason: 'the plugin says: $status');
    },
    skip:
        defaultTargetPlatform != TargetPlatform.iOS &&
        defaultTargetPlatform != TargetPlatform.macOS,
  );

  testWidgets(
    'shares windows with the sharer that the Swift code gave the library',
    (tester) async {
      // The plugin gives the library the sharer as well, with which this Mac shares a window that
      // the person picks in the picker of the system. That picker is in macOS 14 and later; an
      // older Mac, an iPhone and an iPad cannot share a window, register none, and say 1. What the
      // person does at the picker cannot be tried here.
      final backend = RustBackend(dataDir: (await _folder()).path);
      await backend.setDeviceName('Test device');
      final session = await backend.start();
      addTearDown(session.stop);

      final status = await _streamStatus();
      final abilities = session.windowAbilities();
      final canShare =
          defaultTargetPlatform == TargetPlatform.macOS &&
          _macOsMajorVersion() >= 14;
      expect(
        status['sharer'],
        canShare ? 0 : 1,
        reason: 'the plugin says: $status',
      );
      expect(abilities.canShare, canShare, reason: 'the plugin says: $status');
      // A window is looked at in this version, and not steered.
      expect(abilities.canControl, isFalse);
    },
    skip:
        defaultTargetPlatform != TargetPlatform.iOS &&
        defaultTargetPlatform != TargetPlatform.macOS,
  );

  testWidgets(
    'draws windows from textures that Flutter gave the plugin',
    (tester) async {
      // The textures that windows are drawn from are taken from Flutter by the plugin, on the
      // platform thread. Flutter on an iPhone gives them when the plugin is registered. On a Mac
      // it gives none until its engine runs, which is after that, and the plugin asks again until
      // it has them. A window that is offered to a device without them is refused, so this is
      // what makes sure a Mac has them, and nothing else here sees it.
      final deadline = DateTime.now().add(const Duration(seconds: 30));
      var status = await _streamStatus();
      while (status['textures'] != status['wanted'] &&
          DateTime.now().isBefore(deadline)) {
        await tester.pump(const Duration(milliseconds: 200));
        status = await _streamStatus();
      }

      expect(
        status['wanted'],
        greaterThan(0),
        reason: 'the plugin says: $status',
      );
      expect(
        status['textures'],
        status['wanted'],
        reason: 'the plugin says: $status',
      );
    },
    skip:
        defaultTargetPlatform != TargetPlatform.iOS &&
        defaultTargetPlatform != TargetPlatform.macOS,
  );

  testWidgets('pairs with another Baton, by the code pasted in', (
    tester,
  ) async {
    await _pairThroughTheScreens(tester);

    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('shows what another Baton sends, and sends to it', (
    tester,
  ) async {
    final second = await _pairThroughTheScreens(tester);
    final firstId = (await second.devices()).single.id;

    // The second device sends some text. The person is told who sent what kind of thing, but
    // not what it says, and the inbox has it.
    await second.sendText(firstId, 'The door code is 4711');
    await _see(tester, find.text('Second device sent you some text.'));
    expect(find.textContaining('4711'), findsNothing);
    await _tap(tester, find.text('View'));
    await _see(tester, find.text('The door code is 4711'));
    expect(find.text('Second device · just now'), findsOneWidget);

    // Back on the devices, this device sends the second one a link, through the send screen.
    await _tap(tester, find.text('Devices'));
    await _see(tester, find.text('Second device'));
    await _tap(tester, find.text('Second device'));
    await _see(tester, find.text('Send to Second device'));
    // Files can be chosen here, and a folder only where the system can choose one.
    expect(find.text('Choose files'), findsOneWidget);
    expect(
      find.text('Choose a folder'),
      defaultTargetPlatform == TargetPlatform.macOS
          ? findsOneWidget
          : findsNothing,
    );
    await _type(tester, ' HTTPS://Example.com/plan ');
    await tester.scrollUntilVisible(
      find.text('Send link'),
      200,
      scrollable: find.byType(Scrollable).first,
    );
    await _tap(tester, find.text('Send link'));
    await _see(tester, find.text('Sent the link to Second device.'));

    await _until(
      tester,
      () async => (await second.inbox()).isNotEmpty,
      'the link to reach the second device',
    );
    final got = (await second.inbox()).single;
    expect(got.fromName, 'Test device');
    expect(got.item, isA<ReceivedItem_Link>());
    expect((got.item as ReceivedItem_Link).url, 'https://example.com/plan');

    await tester.pumpWidget(const SizedBox());
  });

  // Last, because the question that the system asks about the camera cannot be answered here. It
  // stays in front of the app, which stays inactive for whatever runs after.
  testWidgets('starts the camera view without taking the app down', (
    tester,
  ) async {
    final folder = await _folder();
    await RustBackend(dataDir: folder.path).setDeviceName('Test device');

    await tester.pumpWidget(
      BatonApp(backend: RustBackend(dataDir: folder.path)),
    );
    await _see(tester, find.text('No other devices yet'));
    await _tap(tester, find.text('Add a device'));
    await _see(tester, find.text('Scan or enter a code'));
    await _tap(tester, find.text('Scan or enter a code'));
    await _see(tester, find.text('Or paste the code', skipOffstage: false));

    // The camera starts, and on a phone the system asks about it. Nothing here depends on what
    // the camera sees, except that it must not take the app down, so give it time to do that.
    final until = DateTime.now().add(const Duration(seconds: 3));
    while (DateTime.now().isBefore(until)) {
      await tester.pump(const Duration(milliseconds: 200));
    }
    expect(find.text('Or paste the code', skipOffstage: false), findsOneWidget);

    await tester.pumpWidget(const SizedBox());
  });
}

/// Starts the app with a new device and pairs it with a second Baton in this process, through
/// the screens: the second Baton shows a code, the app has it pasted in, both show six digits
/// and the second says yes. Returns the second Baton, which stops when the test ends. The app
/// is left on its devices, where the second one is listed.
///
/// The camera is the one thing left out: CI has none, so the code is pasted (see [_noCamera]).
Future<Session> _pairThroughTheScreens(WidgetTester tester) async {
  // This device, and a second Baton in this process that plays the device with the code.
  final mine = await _folder();
  await RustBackend(dataDir: mine.path).setDeviceName('Test device');
  final theirs = RustBackend(dataDir: (await _folder()).path);
  await theirs.setDeviceName('Second device');
  final second = await theirs.start();
  addTearDown(second.stop);
  final asked = Completer<PairingRequest>();
  final events = (await second.events()).listen((event) {
    if (event is Event_PairingRequested && !asked.isCompleted) {
      asked.complete(event.request);
    }
  });
  addTearDown(() => unawaited(events.cancel()));
  final offer = await second.offerPairing();

  await tester.pumpWidget(
    BatonApp(
      backend: RustBackend(dataDir: mine.path),
      scanner: _noCamera,
    ),
  );
  await _see(tester, find.text('No other devices yet'));
  await _tap(tester, find.text('Add a device'));
  await _see(tester, find.text('Scan or enter a code'));
  await _tap(tester, find.text('Scan or enter a code'));
  await _see(tester, find.text('Or paste the code', skipOffstage: false));
  await _expectScreenKeptOn(
    tester,
    kept: true,
    when: 'while the screen waits for a code',
  );

  // The screen can be longer than a small window, so scroll to what is wanted.
  final list = find.byType(Scrollable).first;
  await tester.scrollUntilVisible(
    find.byType(TextField),
    200,
    scrollable: list,
  );
  // The field grows to fit the code, so Connect is further down once the code is in.
  await _type(tester, offer.ticket);
  await tester.scrollUntilVisible(find.text('Connect'), 200, scrollable: list);
  await _tap(tester, find.text('Connect'));

  // This device shows six digits, and the second one is asked about the same ones.
  await _see(tester, find.text('Check the other device'));
  final request = await asked.future.timeout(const Duration(seconds: 60));
  expect(request.name, 'Test device');
  expect(find.text(request.code), findsOneWidget);

  expect(second.answerPairing(request.id, accept: true), isTrue);
  await _see(tester, find.text('Paired with Second device'));
  await _expectScreenKeptOn(
    tester,
    kept: false,
    when: 'once the pairing is over',
  );
  await _tap(tester, find.text('Done'));
  await _see(tester, find.text('Second device'));
  expect(find.text('No other devices yet'), findsNothing);
  await _until(
    tester,
    () async => (await second.devices()).isNotEmpty,
    'the second device to keep this one',
  );
  expect((await second.devices()).single.name, 'Test device');

  return second;
}

/// What the pairing tests have in place of the camera view. The real one asks the system for
/// permission the first time it starts, and on a phone the question stays in front of the app
/// until someone answers it, which no one does here. The app is inactive for as long as that
/// lasts, and Baton lets go of the screen of the device (see [_expectScreenKeptOn]), so a check
/// of the screen would pass or fail on whether it came before the question did. The test that
/// starts the real camera view is the last one.
Widget _noCamera(BuildContext context, ValueChanged<String> onCode) =>
    const SizedBox(height: 200, child: Center(child: Text('(camera)')));

/// Checks what an iPhone or iPad itself says about whether its screen is being kept on: that
/// Baton's request went through the code in the iOS app and reached the system. Elsewhere there
/// is no such screen, and this checks nothing.
///
/// Baton lets go of the screen whenever its app is not the active one, such as while the system
/// asks a question in front of it, and asks again once the app is back (see
/// ios/Runner/AppDelegate.swift). So a screen that should be [kept] on is given a while to be,
/// and a failure says what state the app was in. A screen that should not be kept on is checked
/// at once, because nothing but Baton turns it on.
Future<void> _expectScreenKeptOn(
  WidgetTester tester, {
  required bool kept,
  required String when,
}) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  const screen = PlatformScreenKeeper();
  final deadline = DateTime.now().add(const Duration(seconds: 30));
  var isOn = await screen.isKeptOn();
  while (kept && !isOn && DateTime.now().isBefore(deadline)) {
    await tester.pump(const Duration(milliseconds: 200));
    isOn = await screen.isKeptOn();
  }
  final state = WidgetsBinding.instance.lifecycleState?.name ?? 'unknown';
  expect(
    isOn,
    kept,
    reason: 'the screen of the device $when (the state of the app: $state)',
  );
}

/// A folder for one device, with settings that keep it to this computer: no relay, no
/// discovery and no port mapping.
Future<Directory> _folder() async {
  final folder = await Directory.systemTemp.createTemp('baton-app-test-');
  addTearDown(() => folder.delete(recursive: true));
  await File('${folder.path}/config.toml').writeAsString(
    '[network]\nrelay = "disabled"\nlan_discovery = false\nport_mapping = false\n',
  );
  return folder;
}

/// Waits, in real time, until [done] says yes. Starting Baton takes a moment, and so does a
/// pairing. Then it lets the screen settle, because a page that has just appeared is still
/// sliding in.
Future<void> _until(
  WidgetTester tester,
  FutureOr<bool> Function() done,
  String what,
) async {
  final deadline = DateTime.now().add(const Duration(seconds: 60));
  while (!await done()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Never saw $what (${_describeScreen(tester)})');
    }
    await tester.pump(const Duration(milliseconds: 200));
  }
  await tester.pump(const Duration(milliseconds: 500));
}

/// The number of the version of macOS that the app runs on ("Version 14.5 (Build 23F79)"), or 0 if
/// the system does not say it that way.
int _macOsMajorVersion() {
  final match = RegExp(r'Version (\d+)')
      .firstMatch(io.Platform.operatingSystemVersion);
  return int.tryParse(match?.group(1) ?? '') ?? 0;
}

/// What the plugin that shows and shares windows says it did (BatonStreamPlugin.swift): the numbers
/// that the registration of the viewer and of the sharer gave back, how many textures the plugin
/// has and wants, and how many times it asked Flutter for them. The app does not use the channel;
/// it is there for this.
Future<Map<String, int>> _streamStatus() async {
  const channel = MethodChannel('io.github.jayunewal.baton/stream');
  return await channel.invokeMapMethod<String, int>('status') ?? {};
}

/// What there is to say about the app and its screen when a wait runs out, for the log: a
/// failure that shows only what was missing does not show why.
String _describeScreen(WidgetTester tester) {
  final state = WidgetsBinding.instance.lifecycleState?.name ?? 'unknown';
  final view = tester.view;
  final fields = find.byType(TextField, skipOffstage: false).evaluate().length;
  return 'the state of the app: $state, the view: ${view.physicalSize} at '
      '${view.devicePixelRatio}x, covered at the bottom: ${view.viewInsets.bottom}, '
      'text fields: $fields';
}

/// Waits until [finder] finds something.
Future<void> _see(WidgetTester tester, Finder finder) =>
    _until(tester, () => finder.evaluate().isNotEmpty, '$finder');

/// Taps what [finder] finds, once a tap can reach it. A widget that is on the screen can still
/// have something in front of it for a moment, a page that is sliding in for one, and a tap that
/// does not reach it goes to whatever is in front while the test goes on as if it had arrived.
///
/// The tap follows the check in the same frame. A frame that comes between them can move the
/// widget, and the layout that the check saw is no longer the one that the tap lands on.
Future<void> _tap(WidgetTester tester, Finder finder) async {
  final target = finder.hitTestable();
  final deadline = DateTime.now().add(const Duration(seconds: 60));
  do {
    if (DateTime.now().isAfter(deadline)) {
      fail('Never saw a place to tap on $finder (${_describeScreen(tester)})');
    }
    await tester.pump(const Duration(milliseconds: 200));
  } while (target.evaluate().isEmpty);
  await tester.tap(target);
}

/// Types [text] into the text field on the screen, once the field is there, and then puts the
/// keyboard away and lets the screen settle. The title above the field can be seen before the
/// field is, and typing into a field that is not there throws.
///
/// What the text changes on the screen changes after the typing: a field grows to fit what is in
/// it, a line of help appears under it, and on a phone the keyboard comes up and covers the
/// bottom of the screen. A button below the field is not where it was, and a scroll that aims at
/// it before the screen has settled aims at where it was. So look for the button after this.
Future<void> _type(WidgetTester tester, String text) async {
  await _see(tester, find.byType(EditableText));
  await tester.enterText(find.byType(TextField), text);
  FocusManager.instance.primaryFocus?.unfocus();
  await _until(
    tester,
    () => tester.view.viewInsets.bottom == 0,
    'the keyboard to go away',
  );
}
