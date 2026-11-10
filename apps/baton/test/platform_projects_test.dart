// SPDX-License-Identifier: Apache-2.0

// The iOS and macOS projects cannot be built on the computers that most tests run on, so this
// checks the few settings in them that the app depends on and that a change could lose without
// anything failing until someone tried the app on a phone or a Mac.

import 'dart:io';

import 'package:baton/src/always_on.dart';
import 'package:baton/src/awake.dart';
import 'package:baton/src/continue_reader.dart';
import 'package:baton/src/file_shower.dart';
import 'package:baton/src/notice_access.dart';
import 'package:baton/src/rust_backend.dart';
import 'package:baton/src/support_folder.dart';
import 'package:flutter_test/flutter_test.dart';

String _read(String path) => File(path).readAsStringSync();

/// The text of the Swift function that begins with [signature], up to the brace that closes it at
/// the start of a line.
String _swiftFunction(String swift, String signature) {
  final start = swift.indexOf(signature);
  expect(start, isNonNegative, reason: signature);
  final end = swift.indexOf('\n}\n', start);
  expect(end, isNonNegative, reason: 'the end of $signature');
  return swift.substring(start, end + 2);
}

/// What a Swift function does, a line at a time: without its comments, its blank lines and the
/// indentation. The Swift of the apps runs only on an Apple computer, so what a line of it does
/// with what it reads is held by the text of the whole line, and not by the names that it uses.
List<String> _swiftCode(String function) => [
  for (final line in function.split('\n'))
    if (line.trim().isNotEmpty && !line.trim().startsWith('//')) line.trim(),
];

/// Lines of Swift, one after the other, as [_swiftCode] gives them.
String _inOrder(List<String> lines) => lines.join('\n');

void main() {
  test('both apps are called Baton and have the same identifier', () {
    final ios = _read('ios/Runner.xcodeproj/project.pbxproj');
    expect(
      RegExp(r'PRODUCT_BUNDLE_IDENTIFIER = io\.github\.jayunewal\.baton;')
          .allMatches(ios),
      hasLength(3),
      reason: 'debug, profile and release',
    );
    final mac = _read('macos/Runner/Configs/AppInfo.xcconfig');
    expect(mac, contains('PRODUCT_NAME = Baton\n'));
    expect(
      mac,
      contains('PRODUCT_BUNDLE_IDENTIFIER = io.github.jayunewal.baton\n'),
    );
    expect(_read('ios/Runner/Info.plist'), contains('<string>Baton</string>'));
  });

  test('both apps say why they talk to devices on the local network', () {
    for (final plist in ['ios/Runner/Info.plist', 'macos/Runner/Info.plist']) {
      expect(_read(plist), contains('<key>NSLocalNetworkUsageDescription'));
    }
  });

  test('both apps say why they use the camera, which pairing needs', () {
    for (final plist in ['ios/Runner/Info.plist', 'macos/Runner/Info.plist']) {
      expect(_read(plist), contains('<key>NSCameraUsageDescription'));
    }
  });

  test('the iOS app says why it asks for a face, which an unlock needs', () {
    // The system may fail a request for Face ID from a program that has no such text.
    expect(
      _read('ios/Runner/Info.plist'),
      contains('<key>NSFaceIDUsageDescription'),
    );
  });

  test('the Mac app says why it records the sound of the Mac', () {
    // The system gives silence to a program that has no such text, and asks nothing.
    expect(
      _read('macos/Runner/Info.plist'),
      contains('<key>NSAudioCaptureUsageDescription'),
    );
  });

  test('the Mac app says why it puts files in Downloads', () {
    expect(
      _read('macos/Runner/Info.plist'),
      contains('<key>NSDownloadsFolderUsageDescription'),
    );
  });

  test('the sandboxed Mac app may use the camera', () {
    for (final file in [
      'macos/Runner/DebugProfile.entitlements',
      'macos/Runner/Release.entitlements',
    ]) {
      expect(
        _read(file),
        contains('<key>com.apple.security.device.camera</key>\n\t<true/>'),
        reason: file,
      );
    }
  });

  test(
    'the sandboxed Mac app may put files in Downloads and read the ones chosen',
    () {
      // Without the first, received files go to a folder inside the sandbox that nobody looks in,
      // or are refused. Without the second, the open panel shows files that Baton cannot read.
      for (final file in [
        'macos/Runner/DebugProfile.entitlements',
        'macos/Runner/Release.entitlements',
      ]) {
        final entitlements = _read(file);
        for (final key in [
          'com.apple.security.files.downloads.read-write',
          'com.apple.security.files.user-selected.read-only',
        ]) {
          expect(
            entitlements,
            contains('<key>$key</key>\n\t<true/>'),
            reason: '$key in $file',
          );
        }
      }
    },
  );

  test('the iPhone app shows the files that arrive in the Files app', () {
    // The files are put in the Documents folder of the app, which the Files app lists only when
    // the app says so.
    final plist = _read('ios/Runner/Info.plist');
    for (final key in [
      'UIFileSharingEnabled',
      'LSSupportsOpeningDocumentsInPlace',
    ]) {
      expect(plist, contains('<key>$key</key>\n\t<true/>'), reason: key);
    }
  });

  test('the Mac app is sandboxed and may use the network both ways', () {
    for (final file in [
      'macos/Runner/DebugProfile.entitlements',
      'macos/Runner/Release.entitlements',
    ]) {
      final entitlements = _read(file);
      for (final key in [
        'com.apple.security.app-sandbox',
        'com.apple.security.network.client',
        'com.apple.security.network.server',
      ]) {
        expect(
          entitlements,
          contains('<key>$key</key>\n\t<true/>'),
          reason: '$key in $file',
        );
      }
    }
  });

  test('the apps keep the folder that Baton uses out of backups', () {
    // The name is in three places: where Baton makes the folder, and where each app excludes
    // it from backups. They must agree, or the key that makes the device what it is is backed up.
    for (final file in [
      'ios/Runner/AppDelegate.swift',
      'macos/Runner/MainFlutterWindow.swift',
    ]) {
      final swift = _read(file);
      expect(
        swift,
        contains('let batonFolderName = "${RustBackend.folderName}"'),
        reason: file,
      );
      expect(swift, contains('isExcludedFromBackup = true'), reason: file);
    }
  });

  test('the iPhone app answers where Baton asks to keep the screen on', () {
    // The channel and its methods are named in lib/src/awake.dart and in the app. If they
    // differ, nothing fails but pairing: the screen goes off while the person is on the other
    // device, and the call is only logged.
    final swift = _read('ios/Runner/AppDelegate.swift');
    expect(swift, contains('name: "${PlatformScreenKeeper.channelName}"'));
    for (final method in ['keepOn', 'letTurnOff', 'isKeptOn']) {
      expect(swift, contains('case "$method":'), reason: method);
    }
  });

  test(
    'the iPhone app answers where Baton asks for the folder of its files',
    () {
      // The channel and its question are named in lib/src/support_folder.dart and in the app. If
      // they differ, Baton does not start on a phone, and the error only says that the system
      // would not say.
      final swift = _read('ios/Runner/AppDelegate.swift');
      expect(swift, contains('name: "${SupportFolder.channelName}"'));
      expect(swift, contains('case "${SupportFolder.question}":'));
    },
  );

  test(
    'both apps answer where Baton asks for the folder of received files',
    () {
      // The channel and its question are named in lib/src/support_folder.dart and in each app. If
      // they differ, files are not taken, and Baton says only that the system would not say where.
      for (final file in [
        'ios/Runner/AppDelegate.swift',
        'macos/Runner/MainFlutterWindow.swift',
      ]) {
        final swift = _read(file);
        expect(
          swift,
          contains('name: "${SupportFolder.channelName}"'),
          reason: file,
        );
        expect(
          swift,
          contains('case "${SupportFolder.receiveQuestion}":'),
          reason: file,
        );
      }
    },
  );

  test('both apps answer where Baton asks to show the files that arrived', () {
    // The channel and its question are named in lib/src/file_shower.dart and in each app. If they
    // differ, the button that takes the person to the files can only say that it could not.
    for (final file in [
      'ios/Runner/AppDelegate.swift',
      'macos/Runner/MainFlutterWindow.swift',
    ]) {
      final swift = _read(file);
      expect(
        swift,
        contains('name: "${SystemFileShower.channelName}"'),
        reason: file,
      );
      expect(
        swift,
        contains('case "${SystemFileShower.question}":'),
        reason: file,
      );
    }
  });

  test('both apps register what answers the question about showing files', () {
    // An answer that nothing registered is the same to Baton as an app that does not know the
    // question: the button can only say that it could not, and nothing else here fails.
    expect(
      _read('ios/Runner/AppDelegate.swift'),
      contains(
        '    showReceivedFilesInTheFilesApp(messenger: engineBridge.applicationRegistrar.messenger())',
      ),
    );
    expect(
      _read('macos/Runner/MainFlutterWindow.swift'),
      contains(
        '    showReceivedFilesInFinder(messenger: flutterViewController.engine.binaryMessenger)',
      ),
    );
  });

  test('the Mac app answers what Baton asks about notifications', () {
    // The channel, its questions and the words of its answers are named in
    // lib/src/notice_access.dart and in the app. If they differ, Baton cannot tell the person
    // that a device unlocked the Mac, and nothing fails but that: the history has it all the
    // same.
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    expect(swift, contains('name: "${PlatformNoticeKeeper.channelName}"'));
    for (final question in [
      PlatformNoticeKeeper.stateQuestion,
      PlatformNoticeKeeper.requestQuestion,
      PlatformNoticeKeeper.postQuestion,
    ]) {
      expect(swift, contains('case "$question":'), reason: question);
    }
    for (final word in ['allowed', 'needed', 'denied', 'unavailable']) {
      expect(swift, contains('return "$word"'), reason: word);
    }
    // The question is registered, and notifications show while Baton is the app in front: the
    // person who comes back to the Mac and opens Baton is the one who most needs to see one.
    expect(
      swift,
      contains(
        '    answerAboutNotifications(messenger: flutterViewController.engine.binaryMessenger)',
      ),
    );
    expect(swift, contains('center.delegate = noticeDelegate'));
    expect(swift, contains('completionHandler([.banner, .list])'));
  });

  test('the Mac app shows a notification only where the system allows it', () {
    // The system takes a notification from an app that was told no and shows nothing, so the
    // app checks first and says that it was not shown.
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    expect(
      swift,
      contains('guard noticeAccess(settings.authorizationStatus) == "allowed"'),
    );
    // Only the title and the body that the app gave are shown: no sound, no badge, and no
    // action.
    expect(swift, contains('content.title = title'));
    expect(swift, contains('content.body = body'));
    expect(swift, isNot(contains('content.sound')));
    expect(swift, isNot(contains('content.badge')));
    expect(swift, contains('requestAuthorization(options: [.alert])'));
  });

  test('the Mac app keeps running when its window is closed, and hides it', () {
    // The devices that reach this Mac do so through Baton, so a Baton that ended with its window
    // could be reached only while somebody sat at the Mac. Closing the window hides it, and the
    // Dock, a second start of the app and the item in the menu bar bring it back.
    final delegate = _read('macos/Runner/AppDelegate.swift');
    final terminate = delegate.indexOf(
      'override func applicationShouldTerminateAfterLastWindowClosed(',
    );
    expect(terminate, isNonNegative);
    final terminateBody = delegate.substring(
      terminate,
      delegate.indexOf('\n  }\n', terminate),
    );
    expect(terminateBody, contains('return false'));
    expect(terminateBody, isNot(contains('return true')));

    final window = _read('macos/Runner/MainFlutterWindow.swift');
    final close = window.indexOf('override func performClose(');
    expect(close, isNonNegative);
    final closeBody = window.substring(close, window.indexOf('\n  }\n', close));
    expect(closeBody, contains('orderOut(sender)'));
    expect(closeBody, isNot(contains('super.performClose')));
    expect(closeBody, isNot(contains('.close()')));

    expect(delegate, contains('override func applicationShouldHandleReopen('));
    expect(delegate, contains('NSStatusBar.system.statusItem('));
    expect(
      delegate,
      contains('menuBarItem = item'),
      reason: 'the menu bar drops an item that nothing holds',
    );
    for (final entry in ['Open Baton', 'Quit Baton']) {
      expect(delegate, contains('withTitle: "$entry"'), reason: entry);
    }
    expect(delegate, contains('#selector(NSApplication.terminate(_:))'));
  });

  test(
    'the Mac app asks the system not to slow it down while it is hidden',
    () {
      // An app with no window on the screen is slowed by the system, and Baton would answer the
      // devices that reach it late. The claim leaves the Mac free to sleep when the person is
      // away: a Baton that kept it awake would drain a laptop in a bag.
      final delegate = _read('macos/Runner/AppDelegate.swift');
      expect(delegate, contains('ProcessInfo.processInfo.beginActivity('));
      expect(delegate, contains('.userInitiatedAllowingIdleSystemSleep'));
      expect(delegate, isNot(contains('idleSystemSleepDisabled')));
      expect(delegate, isNot(contains('IOPMAssertion')));
    },
  );

  test('a Baton that a login item opened hides its window only once the app '
      'is up', () {
    // Flutter builds no screen while no window is on the screen, so a window that hid at once
    // would leave the app on its first screen, with nothing listening for what other devices
    // send. The window is made invisible and stays, and it goes when the app says it is up.
    final delegate = _read('macos/Runner/AppDelegate.swift');
    expect(delegate, contains('keyAELaunchedAsLogInItem'));
    final start = delegate.indexOf('private func startHidden()');
    expect(start, isNonNegative);
    final startBody = delegate.substring(
      start,
      delegate.indexOf('\n  }\n', start),
    );
    expect(startBody, contains('window.alphaValue = 0'));
    expect(startBody, contains('window.ignoresMouseEvents = true'));
    expect(
      startBody,
      isNot(contains('orderOut')),
      reason: 'the window must be on the screen until the app says it is up',
    );
    expect(
      startBody,
      contains('self?.showBaton()'),
      reason: 'a start that goes wrong must show its window',
    );
    final up = delegate.indexOf('func interfaceIsUp()');
    expect(up, isNonNegative);
    final upBody = delegate.substring(up, delegate.indexOf('\n  }\n', up));
    expect(upBody, contains('window.orderOut(nil)'));
    // A window that comes back after that is whole again.
    expect(upBody, contains('window.alphaValue = 1'));
    expect(upBody, contains('window.ignoresMouseEvents = false'));
    expect(delegate, contains('if !visible || windowWaitsToHide {'));
  });

  test('a Baton that nobody opened at sign-in never hides its window', () {
    // A window that hid by mistake would leave the person with an app that does not open: no
    // window, and an item in the menu bar. Only the event that a login item sends hides it, and
    // the app's saying that it is up changes nothing for a window that was not waiting.
    final delegate = _read('macos/Runner/AppDelegate.swift');
    final launch = delegate.indexOf(
      'override func applicationDidFinishLaunching(',
    );
    expect(launch, isNonNegative);
    final launchBody = delegate.substring(
      launch,
      delegate.indexOf('\n  }\n', launch),
    );
    expect(
      launchBody,
      contains('if wasOpenedAtSignIn() {\n      startHidden()'),
    );
    expect(
      'startHidden()'.allMatches(delegate),
      hasLength(2),
      reason: 'the call at launch, and the definition',
    );

    final opened = delegate.indexOf('private func wasOpenedAtSignIn()');
    expect(opened, isNonNegative);
    final openedBody = delegate.substring(
      opened,
      delegate.indexOf('\n  }\n', opened),
    );
    // Both: the event that opens the app, and the flag that says a login item sent it.
    expect(
      openedBody,
      contains('UInt32(event.eventID) == UInt32(kAEOpenApplication)'),
    );
    expect(
      openedBody,
      contains('launchedBy == OSType(keyAELaunchedAsLogInItem)'),
    );
    expect(openedBody, contains('return opensApp && launchedBy =='));
    expect(
      openedBody,
      contains('else { return false }'),
      reason: 'no event, no hiding',
    );

    final up = delegate.indexOf('func interfaceIsUp()');
    final upBody = delegate.substring(up, delegate.indexOf('\n  }\n', up));
    expect(upBody, contains('guard windowWaitsToHide'));
    expect(
      upBody.indexOf('guard windowWaitsToHide'),
      lessThan(upBody.indexOf('orderOut')),
      reason: 'a window that was not waiting is left alone',
    );

    // What the app's saying that it is up reaches.
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    expect(
      swift,
      contains(
        'case "ready":\n'
        '      (NSApp.delegate as? AppDelegate)?.interfaceIsUp()',
      ),
    );

    // And a window that is brought back is whole, whatever state it was left in.
    final show = delegate.indexOf('func showBaton()');
    expect(show, isNonNegative);
    final showBody = delegate.substring(
      show,
      delegate.indexOf('\n  }\n', show),
    );
    for (final line in [
      'windowWaitsToHide = false',
      'window.alphaValue = 1',
      'window.ignoresMouseEvents = false',
      'window.deminiaturize(nil)',
      'window.makeKeyAndOrderFront(nil)',
    ]) {
      expect(showBody, contains(line), reason: line);
    }
  });

  test('Command-W and the close button hide the window the same way', () {
    // The Close item of the Window menu sends performClose to the window in front, which the
    // window answers by hiding (MainFlutterWindow.performClose), so the claim that Command-W hides
    // Baton and leaves it running holds.
    final menu = _read('macos/Runner/Base.lproj/MainMenu.xib');
    final close = RegExp(
      r'<menuItem title="Close" keyEquivalent="w" id="[^"]+">\s*'
      r'<connections>\s*'
      r'<action selector="performClose:" target="-1" id="[^"]+"/>',
    );
    expect(close.hasMatch(menu), isTrue);
    expect(
      'keyEquivalent="w"'.allMatches(menu),
      hasLength(1),
      reason: 'one item answers Command-W',
    );
    // The ids of the items that Interface Builder reads are each used once.
    final ids = RegExp(r'\sid="([^"]+)"').allMatches(menu).map((m) => m[1]!);
    expect(ids.toSet(), hasLength(ids.length));
  });

  test('closing a window in full screen leaves full screen and hides it', () {
    // One click on the close button does both. Leaving full screen takes a moment, and the window
    // is hidden when the system says it has left, not before: hiding one in full screen would
    // leave its space empty.
    final window = _read('macos/Runner/MainFlutterWindow.swift');
    final close = window.indexOf('override func performClose(');
    final closeBody = window.substring(close, window.indexOf('\n  }\n', close));
    expect(
      closeBody.indexOf('hideWhenOutOfFullScreen()'),
      lessThan(closeBody.indexOf('toggleFullScreen(sender)')),
      reason: 'the wait is set before the window starts to leave',
    );
    final wait = window.indexOf('private func hideWhenOutOfFullScreen()');
    expect(wait, isNonNegative);
    final waitBody = window.substring(wait, window.indexOf('\n  }\n', wait));
    expect(waitBody, contains('NSWindow.didExitFullScreenNotification'));
    expect(waitBody, contains('object: self'));
    expect(waitBody, contains('self.orderOut(nil)'));
    expect(
      waitBody,
      contains('removeObserver(waiting)'),
      reason: 'a wait is over once the window has been hidden',
    );
    expect(waitBody, contains('guard fullScreenExit == nil else { return }'));
  });

  test('the Mac app tells whether the person can see the window, and opens it '
      'for a click on a notification', () {
    // What came for the person is told by a notification when they cannot see Baton, and not when
    // they can (lib/src/arrival_notices.dart). They cannot while the window is hidden,
    // minimized, waiting to hide, behind another window or while another app is in front.
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    final looking = swift.indexOf('private func personCanSeeBaton()');
    expect(looking, isNonNegative);
    final lookingBody = swift.substring(
      looking,
      swift.indexOf('\n}\n', looking),
    );
    for (final check in [
      'NSApp.isActive',
      'window.isVisible',
      '!window.isMiniaturized',
      'window.alphaValue > 0',
      'window.occlusionState.contains(.visible)',
    ]) {
      expect(lookingBody, contains(check), reason: check);
    }
    expect(
      lookingBody,
      contains('return false'),
      reason: 'no window, no seeing',
    );

    // A click on a notification, and only a click, brings the window forward.
    final click = swift.indexOf('didReceive response: UNNotificationResponse');
    expect(click, isNonNegative);
    final clickBody = swift.substring(click, swift.indexOf('\n  }\n', click));
    expect(
      clickBody,
      contains(
        'response.actionIdentifier == UNNotificationDefaultActionIdentifier',
      ),
    );
    expect(
      clickBody,
      contains('(NSApp.delegate as? AppDelegate)?.showBaton()'),
    );
    expect(clickBody, contains('completionHandler()'));
  });

  test('the Mac app answers what Baton asks about opening at sign-in', () {
    // The channel, its questions and the words of its answers are named in
    // lib/src/always_on.dart and in the app. If they differ, the switch in the settings cannot
    // be shown, or does nothing, and a Baton that a login item opened never hides its window.
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    expect(swift, contains('name: "${PlatformAlwaysOnKeeper.channelName}"'));
    for (final question in [
      PlatformAlwaysOnKeeper.signInQuestion,
      PlatformAlwaysOnKeeper.setSignInQuestion,
      PlatformAlwaysOnKeeper.openSignInSettingsQuestion,
      PlatformAlwaysOnKeeper.readyQuestion,
      PlatformAlwaysOnKeeper.lookingQuestion,
    ]) {
      expect(swift, contains('case "$question":'), reason: question);
    }
    for (final word in ['on', 'off', 'needsApproval', 'unavailable']) {
      expect(swift, contains('return "$word"'), reason: word);
    }
    expect(
      swift,
      contains(
        '    answerAboutStayingOn(messenger: flutterViewController.engine.binaryMessenger)',
      ),
    );
    // The system has no way for an app to open at sign-in before macOS 13, which Baton still
    // runs on, so each use of it sits behind a check, and an older Mac says that it cannot.
    expect(
      'guard #available(macOS 13.0, *) else { return "unavailable" }'
          .allMatches(swift),
      hasLength(2),
      reason: 'the state and the change',
    );
    expect(
      'guard #available(macOS 13.0, *) else { return false }'.allMatches(swift),
      hasLength(1),
      reason: 'the pane of Settings',
    );
    expect(swift, contains('@available(macOS 13.0, *)'));
    // It registers the app itself and nothing else: no helper program, no agent.
    expect(swift, contains('SMAppService.mainApp'));
    expect(swift, isNot(contains('SMAppService.agent')));
    expect(swift, isNot(contains('SMAppService.daemon')));
    expect(swift, isNot(contains('SMAppService.loginItem')));
  });

  test(
    'the Mac app says why it reads the page in front in Safari and Chrome',
    () {
      // The system gives no answer to a program that has no such text, and asks nothing.
      expect(
        _read('macos/Runner/Info.plist'),
        contains('<key>NSAppleEventsUsageDescription'),
      );
    },
  );

  test('the sandboxed Mac app may send Apple events to Safari and Chrome, and '
      'to no other app', () {
    // Without this the sandbox turns the question to the browser away, and the system never asks
    // the person. It is a list of two: another app in it is another place that Baton can read
    // from, which the person is told of in the settings for a device and in docs/security.md.
    for (final file in [
      'macos/Runner/DebugProfile.entitlements',
      'macos/Runner/Release.entitlements',
    ]) {
      expect(
        _read(file),
        contains(
          '<key>com.apple.security.temporary-exception.apple-events</key>\n'
          '\t<array>\n'
          '\t\t<string>com.apple.Safari</string>\n'
          '\t\t<string>com.google.Chrome</string>\n'
          '\t</array>',
        ),
        reason: file,
      );
    }
  });

  test('the Mac app answers what Baton asks about what the person is working '
      'on', () {
    // The channel, its questions, the words of its answers and the names of the parts of what it
    // reads are named in lib/src/continue_reader.dart and in the app. If they differ, a phone that
    // asks is given nothing, or is told that the Mac could not read, and nothing fails but that.
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    expect(swift, contains('name: "${PlatformContinueReader.channelName}"'));
    for (final question in [
      PlatformContinueReader.readQuestion,
      PlatformContinueReader.browserQuestion,
    ]) {
      expect(swift, contains('case "$question":'), reason: question);
    }
    for (final part in ['pageUrl', 'pageTitle', 'clipboard']) {
      expect(swift, contains('found["$part"] = '), reason: part);
    }
    for (final browser in Browser.values) {
      expect(swift, contains('name: "${browser.name}"'), reason: browser.name);
    }
    expect(
      swift,
      contains(
        '    answerAboutContinuing(messenger: flutterViewController.engine.binaryMessenger)',
      ),
    );
    // A Mac starts the library able to continue, and no other system does, because only a Mac has
    // this code. Without it every question from a phone is turned away as not supported.
    final backend = _read('lib/src/rust_backend.dart');
    expect(
      backend,
      contains('serveContinue: io.Platform.isMacOS ? true : null,'),
    );
    expect('serveContinue:'.allMatches(backend), hasLength(1));
  });

  test('the Mac app reads only the page in front in the two browsers, and '
      'never starts one', () {
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    // The scripts ask for an address and a title and do nothing else, in the two browsers that the
    // entitlements name, and each gives up after three seconds.
    final scripts = [
      for (final match in RegExp(
        r'script: """\n(.*?)"""',
        dotAll: true,
      ).allMatches(swift))
        match.group(1)!,
    ];
    expect(scripts, hasLength(2));
    final targets = <String>{};
    for (final script in scripts) {
      expect(script, contains('with timeout of 3 seconds'));
      targets.addAll(
        RegExp(r'tell application id "([^"]+)"')
            .allMatches(script)
            .map((match) => match.group(1)!),
      );
      for (final action in [
        'activate',
        'launch',
        'open',
        'make',
        'close',
        'delete',
        'quit',
        'reload',
        'save',
        'print',
        'execute',
        'do JavaScript',
      ]) {
        expect(script, isNot(contains(action)), reason: action);
      }
      // The one thing that is assigned is a name for what was found.
      for (final line in script.split('\n').where((l) => l.contains('set '))) {
        expect(line, matches(RegExp(r'^\s*set (thePage|theWindow) to ')));
      }
    }
    expect(targets, {'com.apple.Safari', 'com.google.Chrome'});
    // A browser that is not open is not asked, because asking one by name would open it.
    expect(
      'runningApplications(withBundleIdentifier: browser.bundleIdentifier)'
          .allMatches(swift),
      hasLength(2),
      reason: 'reading the page, and saying where the consent stands',
    );
    // A private window of Chrome gives nothing.
    expect(
      swift,
      contains('if (mode of theWindow) is "incognito" then return {}'),
    );
  });

  test('the Mac app gives nothing while nobody is using the Mac, and nothing '
      'that was marked as secret', () {
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    // Locked, with the screen off, or with another account in front: nothing is read at all, and
    // a session that cannot be read is taken as one that nobody uses. The keys alone would pass
    // with each answer turned round, so every line is held whole.
    expect(_swiftCode(_swiftFunction(swift, 'private func macIsNotInUse()')), [
      'private func macIsNotInUse() -> Bool {',
      'guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return true }',
      'if session["CGSSessionScreenIsLocked"] as? Bool == true { return true }',
      'if session["kCGSSessionOnConsoleKey"] as? Bool == false { return true }',
      'return CGDisplayIsAsleep(CGMainDisplayID()) != 0',
      '}',
    ]);
    final read = _swiftCode(
      _swiftFunction(swift, 'private func readWhatThePersonIsWorkingOn('),
    );
    expect(read.take(5), [
      'private func readWhatThePersonIsWorkingOn(_ result: @escaping FlutterResult) {',
      'if macIsNotInUse() {',
      'result([String: Any]())',
      'return',
      '}',
    ], reason: 'the check comes first, and gives an empty answer');
    // A browser that cannot be read is told as a failure only when there is nothing else to give,
    // and is never told as nothing.
    expect(
      read.join('\n'),
      contains(
        _inOrder([
          'case .failed:',
          'if found.isEmpty {',
          'result(',
          'FlutterError(',
          'code: "unreadable", message: "The browser did not answer.", details: nil))',
          '} else {',
          'result(found)',
          '}',
        ]),
      ),
    );
    // Text that the app that copied it marked as secret, or as one-time, is not read, and neither
    // is text that is blank or more than the protocol carries, which is a mebibyte
    // (crates/baton-proto/src/limits.rs).
    expect(_swiftCode(_swiftFunction(swift, 'private func clipboardText()')), [
      'private func clipboardText() -> String? {',
      'let pasteboard = NSPasteboard.general',
      'let marks = ["org.nspasteboard.ConcealedType", "org.nspasteboard.TransientType"]',
      'if (pasteboard.types ?? []).contains(where: { marks.contains(\$0.rawValue) }) {',
      'return nil',
      '}',
      'guard let text = pasteboard.string(forType: .string),',
      '!text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,',
      'text.utf8.count <= 1024 * 1024',
      'else {',
      'return nil',
      '}',
      'return text',
      '}',
    ]);
  });

  test('the Mac app says where the consent of a browser stands, and asks the '
      'person only when it is told to', () {
    final swift = _read('macos/Runner/MainFlutterWindow.swift');
    // The five words of the answer are also the words about notifications and about the camera,
    // so only the body of this function says that they are said of the browsers, and for which
    // answer of the system.
    final access = _swiftCode(
      _swiftFunction(swift, 'private func browserAccess('),
    ).join('\n');
    expect(
      access,
      contains(
        _inOrder([
          'switch status {',
          'case 0:',
          'return "allowed"',
          'case -1744:',
          'return "needed"',
          'case -1743:',
          'return "denied"',
          'case -600:',
          'return "notOpen"',
          'default:',
          'return "unavailable"',
          '}',
        ]),
      ),
    );
    // A browser that is not on this Mac, and one that is not open, are told before the system is
    // asked, because asking about one that is closed would start it.
    expect(
      access,
      contains(
        _inOrder([
          'guard',
          'NSWorkspace.shared.urlForApplication(withBundleIdentifier: browser.bundleIdentifier) != nil',
          'else {',
          'return "unavailable"',
          '}',
          'guard',
          '!NSRunningApplication.runningApplications(withBundleIdentifier: browser.bundleIdentifier)',
          '.isEmpty',
          'else {',
          'return "notOpen"',
          '}',
        ]),
      ),
    );
    // The system puts its question to the person only when the question from the app said to ask.
    expect(
      access,
      contains(
        _inOrder([
          'ask',
          '? AEDeterminePermissionToAutomateTarget(address, anyEvent, anyEvent, true)',
          ': AEDeterminePermissionToAutomateTarget(address, anyEvent, anyEvent, false)',
        ]),
      ),
    );
    final handler = _swiftCode(
      _swiftFunction(swift, 'private func answerAboutContinuing('),
    ).join('\n');
    expect(
      handler,
      contains(
        _inOrder([
          'let ask = arguments["ask"] as? Bool ?? false',
          'continueQueue.async {',
          'let answer = browserAccess(browser, ask: ask)',
          'DispatchQueue.main.async { result(answer) }',
          '}',
        ]),
      ),
    );
  });

  test('the apps show the files that arrived and never open them', () {
    // What a file does when it is opened is for the person to choose, and Baton opens nothing
    // that another device sent. Finder is asked to select the items, and the Files app is asked
    // to open a folder. Neither is asked to open a file.
    final mac = _read('macos/Runner/MainFlutterWindow.swift');
    expect(mac, contains('NSWorkspace.shared.activateFileViewerSelecting('));
    // The one thing it opens is a pane of System Settings, for a permission, and the one place
    // that opens it refuses any other address.
    expect('NSWorkspace.shared.open('.allMatches(mac), hasLength(1));
    final settings = mac.indexOf('func openSystemSettings(');
    expect(settings, isNonNegative);
    final scheme = mac.indexOf(
      'url.scheme == "x-apple.systempreferences"',
      settings,
    );
    expect(scheme, isNonNegative);
    expect(
      mac.indexOf('NSWorkspace.shared.open(', settings),
      greaterThan(scheme),
      reason: 'the address is checked before it is opened',
    );
    expect(
      mac.substring(
        mac.indexOf('NSWorkspace.shared.open(') - 400,
        mac.indexOf('NSWorkspace.shared.open('),
      ),
      contains('func openSystemSettings('),
      reason: 'it opens only in the place that checks the address',
    );
    for (final opening in [
      'NSWorkspace.shared.openFile',
      'NSWorkspace.shared.openApplication',
      'NSWorkspace.shared.launchApplication',
    ]) {
      expect(mac, isNot(contains(opening)), reason: opening);
    }
    final ios = _read('ios/Runner/AppDelegate.swift');
    expect(ios, contains('address.scheme = "shareddocuments"'));
    // Besides the Files app, the iPhone opens only Baton's own page in Settings, for a permission.
    expect(
      'URL(string: UIApplication.openSettingsURLString)'.allMatches(ios),
      hasLength(1),
    );
    expect(
      ios,
      contains('item.deletingLastPathComponent()'),
      reason: 'anything but a folder that arrived alone is shown by its folder',
    );
    // A folder that the system takes for a document, such as Report.pages, is not opened either:
    // it is shown by the folder that holds it, like a file.
    expect(ios, contains('forKeys: [.isPackageKey]'));
    expect(ios, contains('isFolder.boolValue && !isPackage'));
  });

  test('the Mac app builds its guard and signs it to inherit the sandbox', () {
    // The guard locks the Mac when the app goes away during a private session
    // (docs/adr/0011-privacy-mode.md). Only the build puts it into the app, and the app looks
    // for it by name, so a lost phase or a renamed program would show as a Mac that cannot start
    // a private session, on the first try with a phone.
    final project = _read('macos/Runner.xcodeproj/project.pbxproj');
    expect(project, contains('/* Build Guard */,'), reason: 'Runner runs it');
    expect(project, contains(r'sh \"$SRCROOT/Guard/build-guard.sh\"'));
    final script = _read('macos/Guard/build-guard.sh');
    expect(script, contains(r'"$programs/BatonGuard"'));
    expect(script, contains('BatonGuard.swift'));
    expect(script, contains('BatonLock.swift'));
    expect(script, contains('--identifier io.github.jayunewal.baton.guard'));
    expect(
      _read('rust_builder/macos/Classes/BatonPrivacyMac.swift'),
      contains('forAuxiliaryExecutable: "BatonGuard"'),
    );
    // A program that inherits the sandbox of its parent may hold no other entitlement.
    final keys = RegExp(r'<key>([^<]+)</key>')
        .allMatches(_read('macos/Guard/Guard.entitlements'))
        .map((match) => match.group(1))
        .toList();
    expect(keys, [
      'com.apple.security.app-sandbox',
      'com.apple.security.inherit',
    ]);
  });

  test('the guard is built from a checkout whose path has a space in it', () {
    // Xcode hands the script paths such as "/Volumes/Backup/Personal Projects/...", and a list of
    // programs kept in a string and left unquoted falls apart at the space: lipo is then given
    // "/Volumes/Backup/Personal" and "Projects/..." and the build of the whole app stops. The
    // tools are replaced by scripts that write down what they were given, and lipo, like the
    // real one, refuses a file that is not there.
    final root = Directory.systemTemp.createTempSync('baton guard ');
    addTearDown(() => root.deleteSync(recursive: true));
    final tools = Directory('${root.path}/tools')..createSync();
    final notes = File('${root.path}/notes');
    void tool(String name, String body) {
      final file = File('${tools.path}/$name')..writeAsStringSync(body);
      expect(Process.runSync('chmod', ['+x', file.path]).exitCode, 0);
    }

    // `xcrun swiftc ... -o program` makes the program.
    tool('xcrun', r'''#!/bin/sh
while [ "$#" -gt 0 ]; do
  [ "$1" = "-o" ] && : > "$2"
  shift
done
''');
    tool('lipo', '''#!/bin/sh
echo "lipo \$*" >> "${notes.path}"
output=""
while [ "\$#" -gt 0 ]; do
  case "\$1" in
    -create) ;;
    -output) output="\$2"; shift ;;
    *) [ -e "\$1" ] || { echo "lipo: file not found: \$1" >&2; exit 1; } ;;
  esac
  shift
done
: > "\$output"
''');
    tool('codesign', '''#!/bin/sh
echo "codesign \$*" >> "${notes.path}"
''');
    final checkout = Directory('${root.path}/Personal Projects/app/macos')
      ..createSync(recursive: true);
    Directory('${checkout.path}/../rust_builder/macos/Classes')
        .createSync(recursive: true);
    final build = Directory('${root.path}/Build Products/Baton.app')
      ..createSync(recursive: true);
    final derived = Directory('${root.path}/Derived Files')..createSync();
    final script = File('macos/Guard/build-guard.sh').absolute.path;
    final ran = Process.runSync(
      'sh',
      [script],
      environment: {
        'PATH': '${tools.path}:${Platform.environment['PATH']}',
        'SRCROOT': checkout.path,
        'SDKROOT': '${root.path}/SDK',
        'TARGET_BUILD_DIR': build.path,
        'CONTENTS_FOLDER_PATH': 'Contents',
        'DERIVED_FILE_DIR': derived.path,
        'ARCHS': 'arm64 x86_64',
      },
    );
    expect(ran.exitCode, 0, reason: '${ran.stdout}${ran.stderr}');
    final work = '${derived.path}/baton-guard';
    final program = '${build.path}/Contents/MacOS/BatonGuard';
    expect(
      notes.readAsLinesSync().first,
      'lipo -create $work/BatonGuard-arm64 $work/BatonGuard-x86_64 '
      '-output $program',
    );
    expect(File(program).existsSync(), isTrue);
    expect(notes.readAsStringSync(), contains('codesign '));
  }, skip: Platform.isWindows ? 'it runs sh' : false);

  test('the library is linked in a way that keeps its functions findable', () {
    for (final system in ['ios', 'macos']) {
      final podspec = _read('rust_builder/$system/baton_ffi.podspec');
      expect(podspec, contains('-force_load'));
      expect(podspec, contains("'STRIP_STYLE' => 'non-global'"));
    }
  });

  test('the pods name the system frameworks that the library needs', () {
    // The library is a static one, which does not bring them along. Without them the build
    // ends in "Undefined symbol" from the linker, after minutes of compiling Rust.
    expect(
      _read('rust_builder/ios/baton_ffi.podspec'),
      contains("s.frameworks = 'Network', 'SystemConfiguration'"),
    );
    expect(
      _read('rust_builder/macos/baton_ffi.podspec'),
      contains("s.frameworks = 'SystemConfiguration'"),
    );
  });
}
