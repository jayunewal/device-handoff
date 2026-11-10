// SPDX-License-Identifier: Apache-2.0

import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'backend.dart';

/// A browser that Baton reads the page in front from, when a paired device that the person
/// allowed asks what they are working on (docs/adr/0015-continue-from-another-device.md).
enum Browser {
  safari('Safari'),
  chrome('Chrome');

  const Browser(this.title);

  /// The name of the browser as a person says it.
  final String title;
}

/// Whether Baton may read the page in front from a [Browser]. The system asks the person once, and
/// they can change their answer in System Settings, under Privacy & Security, Automation.
enum BrowserAccess {
  /// The person allowed it.
  allowed,

  /// The system has not asked yet. [ContinueReader.browserAccess] with `ask` has it ask, which it
  /// can do only while the browser is open.
  needed,

  /// The person said no. Only the settings of the system change that, and the system asks nothing
  /// again.
  denied,

  /// The browser is not open, so the system can neither say nor ask.
  notOpen,

  /// This device has no such browser, or is not a Mac.
  unavailable,
}

/// What to tell the person about the browsers once the system was asked about each of them: where
/// the answer was no, or that nothing was asked yet, which is so while no browser is open. `null`
/// where there is nothing to add.
String? describeBrowserAccess(Map<Browser, BrowserAccess> access) {
  List<String> titles(BrowserAccess of) => [
    for (final MapEntry(:key, :value) in access.entries)
      if (value == of) key.title,
  ];
  String list(List<String> names) =>
      names.length == 2 ? '${names[0]} and ${names[1]}' : names.join(', ');
  final denied = titles(BrowserAccess.denied);
  if (denied.isNotEmpty) {
    return 'macOS says that Baton may not control ${list(denied)}, so no page '
        'can be read from ${denied.length == 1 ? 'it' : 'them'}. To allow it, '
        'open System Settings, then Privacy & Security, then Automation.';
  }
  if (access.containsValue(BrowserAccess.allowed)) return null;
  final closed = titles(BrowserAccess.notOpen);
  if (closed.isEmpty) return null;
  final one = closed.length == 1;
  return '${list(closed)} ${one ? 'is' : 'are'} not open, so macOS has not '
      'asked about ${one ? 'it' : 'them'} yet. It asks when Baton first reads '
      'a page from ${one ? 'it' : 'one of them'}. To be asked now, open '
      '${one ? 'it' : 'one of them'} and save this again.';
}

/// What the person at this device is working on, read when a paired device that they allowed asks.
/// The screens are written against this, so that tests can say what is open and see what was asked
/// of the system.
abstract class ContinueReader {
  const ContinueReader();

  /// What the person is working on at this moment: the page that is in front in a browser, and the
  /// text on the clipboard, each `null` when there is none. A device that is locked, or whose
  /// display is off, gives nothing at all rather than what it last held, which is an empty
  /// [ContinueItems]. Returns `null` when it could not be read, which is not the same as nothing:
  /// the browser may not have been allowed to answer, or did not answer in time.
  Future<ContinueItems?> read();

  /// Where the permission to read the page in front from [browser] stands. With [ask], the system
  /// asks the person now if it has not yet, and the call returns once they have answered.
  Future<BrowserAccess> browserAccess(Browser browser, {bool ask = false});
}

/// Asks the operating system, through the code of the Mac app in
/// macos/Runner/MainFlutterWindow.swift. Only a Mac has the code, so the other systems read
/// nothing.
class PlatformContinueReader extends ContinueReader {
  const PlatformContinueReader();

  /// The channel that the Mac app answers on. The name, the questions and the words of the
  /// answers must be those in macos/Runner/MainFlutterWindow.swift.
  static const channelName = 'io.github.jayunewal.baton/continuing';
  static const readQuestion = 'read';
  static const browserQuestion = 'browser';

  /// The most that the protocol carries of the title of a page
  /// (crates/baton-proto/src/limits.rs). A page whose title is longer is not carried at all, so a
  /// title that is longer is cut short here, and the page goes with the start of it.
  static const maxTitleBytes = 1024;

  static const _channel = MethodChannel(channelName);

  @override
  Future<ContinueItems?> read() async {
    final answer = await _ask(readQuestion);
    if (answer is! Map) return null;
    final url = _text(answer['pageUrl']);
    final title = _text(answer['pageTitle']);
    return ContinueItems(
      page: url == null
          ? null
          : ContinuePage(
              url: url,
              title: title == null ? null : cutToBytes(title, maxTitleBytes),
            ),
      clipboard: _text(answer['clipboard']),
    );
  }

  @override
  Future<BrowserAccess> browserAccess(
    Browser browser, {
    bool ask = false,
  }) async {
    final answer = await _ask(browserQuestion, {
      'browser': browser.name,
      'ask': ask,
    });
    return switch (answer) {
      'allowed' => BrowserAccess.allowed,
      'needed' => BrowserAccess.needed,
      'denied' => BrowserAccess.denied,
      'notOpen' => BrowserAccess.notOpen,
      _ => BrowserAccess.unavailable,
    };
  }

  /// What the Mac gave for a part: the text, or `null` where there is none to give.
  static String? _text(Object? value) =>
      value is String && value.trim().isNotEmpty ? value : null;

  /// Puts [question] to the Mac app and gives its answer, or `null` where there is none: this is
  /// not a Mac, or the app has no code for the question, or it could not read. What came of
  /// reading is not written down: it can be a page, and a page can hold a sign-in token.
  Future<Object?> _ask(
    String question, [
    Map<String, Object>? arguments,
  ]) async {
    if (defaultTargetPlatform != TargetPlatform.macOS) return null;
    try {
      return await _channel.invokeMethod<Object>(question, arguments);
    } on PlatformException catch (e) {
      debugPrint('Baton: continuing: $question: ${e.code}');
    } on MissingPluginException {
      debugPrint('Baton: continuing: $question: no code for it');
    }
    return null;
  }
}

/// [text] cut short to at most [limit] bytes of UTF-8, at the end of a character, with "…" in
/// place of what was cut. Text that fits is returned as it is.
@visibleForTesting
String cutToBytes(String text, int limit) {
  if (utf8.encode(text).length <= limit) return text;
  const ellipsis = '…';
  final room = limit - utf8.encode(ellipsis).length;
  final kept = StringBuffer();
  var used = 0;
  for (final rune in text.runes) {
    final size = utf8.encode(String.fromCharCode(rune)).length;
    if (used + size > room) break;
    kept.writeCharCode(rune);
    used += size;
  }
  return '$kept$ellipsis';
}
