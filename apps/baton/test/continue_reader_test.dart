// SPDX-License-Identifier: Apache-2.0

import 'dart:convert';

import 'package:baton/src/backend.dart';
import 'package:baton/src/continue_reader.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _channel = MethodChannel(PlatformContinueReader.channelName);

/// Answers the channel with [answer], and notes what it was asked, in order.
List<MethodCall> _listen(
  WidgetTester tester,
  Object? Function(MethodCall call) answer,
) {
  final calls = <MethodCall>[];
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(_channel, (
    call,
  ) async {
    calls.add(call);
    return answer(call);
  });
  addTearDown(
    () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      _channel,
      null,
    ),
  );
  return calls;
}

void main() {
  group('what the person is working on', () {
    const reader = PlatformContinueReader();

    test('is asked through the channel that the Mac app answers', () {
      expect(
        PlatformContinueReader.channelName,
        'io.github.jayunewal.baton/continuing',
      );
      expect(PlatformContinueReader.readQuestion, 'read');
      expect(PlatformContinueReader.browserQuestion, 'browser');
    });

    testWidgets('is the page, its title and the text, as the Mac gives them', (
      tester,
    ) async {
      final calls = _listen(
        tester,
        (_) => {
          'pageUrl': 'https://example.com/report',
          'pageTitle': 'The report',
          'clipboard': 'hello',
        },
      );

      expect(
        await reader.read(),
        const ContinueItems(
          page: ContinuePage(
            url: 'https://example.com/report',
            title: 'The report',
          ),
          clipboard: 'hello',
        ),
      );
      expect([for (final call in calls) call.method], ['read']);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('may be a page alone, with no title, or text alone', (
      tester,
    ) async {
      Object? answer = {'pageUrl': 'https://example.com/'};
      _listen(tester, (_) => answer);
      expect(
        await reader.read(),
        const ContinueItems(page: ContinuePage(url: 'https://example.com/')),
      );

      answer = {'clipboard': 'only text'};
      expect(await reader.read(), const ContinueItems(clipboard: 'only text'));
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('is nothing when the Mac gives nothing', (tester) async {
      _listen(tester, (_) => <String, Object>{});

      expect(await reader.read(), const ContinueItems());
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets(
      'leaves out a part with nothing in it, and keeps text as it is',
      (tester) async {
        Object? answer = {
          'pageUrl': '  ',
          'pageTitle': 'A title without a page',
          'clipboard': '',
        };
        _listen(tester, (_) => answer);
        expect(await reader.read(), const ContinueItems());

        // Text is the person's own, with the spaces and the line ends that it has.
        answer = {'clipboard': '  two\nlines  '};
        expect(
          await reader.read(),
          const ContinueItems(clipboard: '  two\nlines  '),
        );
      },
      variant: TargetPlatformVariant.only(TargetPlatform.macOS),
    );

    testWidgets('leaves out a part that is not text', (tester) async {
      _listen(tester, (_) => {'pageUrl': 42, 'clipboard': true});

      expect(await reader.read(), const ContinueItems());
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('keeps the page and cuts the title that is too long for it', (
      tester,
    ) async {
      final long = 'é' * 2000;
      _listen(
        tester,
        (_) => {'pageUrl': 'https://example.com/', 'pageTitle': long},
      );

      final title = (await reader.read())!.page!.title!;

      expect(utf8.encode(title).length, lessThanOrEqualTo(1024));
      expect(title, endsWith('…'));
      expect(title.startsWith('é' * 100), isTrue);
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('could not be read when the Mac says so, and when it is not '
        'a map', (tester) async {
      Object? answer;
      _listen(tester, (_) {
        if (answer is PlatformException) throw answer;
        return answer;
      });

      answer = PlatformException(code: 'unreadable');
      expect(await reader.read(), isNull);
      for (final other in [
        null,
        'text',
        42,
        true,
        ['pageUrl'],
      ]) {
        answer = other;
        expect(await reader.read(), isNull, reason: '$other');
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('could not be read where the app has no code for it', (
      tester,
    ) async {
      // A Mac app that is older than this code does not know the question.
      final calls = _listen(tester, (_) => throw MissingPluginException());

      expect(await reader.read(), isNull);
      expect(
        await reader.browserAccess(Browser.chrome),
        BrowserAccess.unavailable,
      );
      expect(calls, hasLength(2));
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('is not asked of a device that is not a Mac', (tester) async {
      final calls = _listen(tester, (_) => {'clipboard': 'hello'});

      expect(await reader.read(), isNull);
      expect(
        await reader.browserAccess(Browser.safari, ask: true),
        BrowserAccess.unavailable,
      );
      expect(calls, isEmpty);
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
  });

  group('the consent to read a browser', () {
    const reader = PlatformContinueReader();

    testWidgets('is asked for by browser, and for whether to ask the person', (
      tester,
    ) async {
      final calls = _listen(tester, (_) => 'allowed');

      await reader.browserAccess(Browser.safari);
      await reader.browserAccess(Browser.chrome, ask: true);

      expect(
        [for (final call in calls) '${call.method} ${call.arguments}'],
        [
          'browser {browser: safari, ask: false}',
          'browser {browser: chrome, ask: true}',
        ],
      );
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('is read as the app says it', (tester) async {
      var answer = 'allowed';
      _listen(tester, (_) => answer);

      for (final (word, access) in [
        ('allowed', BrowserAccess.allowed),
        ('needed', BrowserAccess.needed),
        ('denied', BrowserAccess.denied),
        ('notOpen', BrowserAccess.notOpen),
        ('unavailable', BrowserAccess.unavailable),
      ]) {
        answer = word;
        expect(
          await reader.browserAccess(Browser.safari),
          access,
          reason: word,
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    testWidgets('is not allowed by an answer that is not known', (
      tester,
    ) async {
      Object? answer;
      _listen(tester, (_) => answer);

      for (final unknown in [null, 42, true, '', 'yes', 'Allowed']) {
        answer = unknown;
        expect(
          await reader.browserAccess(Browser.chrome),
          BrowserAccess.unavailable,
          reason: '$unknown',
        );
      }
    }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

    test('is asked of the two browsers, by the words that the app knows them '
        'by', () {
      expect(Browser.values.map((b) => b.name), ['safari', 'chrome']);
      expect(Browser.values.map((b) => b.title), ['Safari', 'Chrome']);
    });
  });

  group('what the person is told of the browsers', () {
    test('says that Baton may not control one that was refused', () {
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.denied,
          Browser.chrome: BrowserAccess.allowed,
        }),
        'macOS says that Baton may not control Safari, so no page can be read '
        'from it. To allow it, open System Settings, then Privacy & Security, '
        'then Automation.',
      );
    });

    test('names both when both were refused', () {
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.denied,
          Browser.chrome: BrowserAccess.denied,
        }),
        contains(
          'may not control Safari and Chrome, so no page can be read '
          'from them',
        ),
      );
    });

    test('says nothing when one browser may be read', () {
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.allowed,
          Browser.chrome: BrowserAccess.notOpen,
        }),
        isNull,
      );
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.allowed,
          Browser.chrome: BrowserAccess.unavailable,
        }),
        isNull,
      );
    });

    test('says that the system has not asked yet while no browser is open', () {
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.notOpen,
          Browser.chrome: BrowserAccess.notOpen,
        }),
        'Safari and Chrome are not open, so macOS has not asked about them '
        'yet. It asks when Baton first reads a page from one of them. To be '
        'asked now, open one of them and save this again.',
      );
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.notOpen,
          Browser.chrome: BrowserAccess.unavailable,
        }),
        'Safari is not open, so macOS has not asked about it yet. It asks when '
        'Baton first reads a page from it. To be asked now, open it and save '
        'this again.',
      );
    });

    test('says nothing where there is no browser to ask about', () {
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.unavailable,
          Browser.chrome: BrowserAccess.unavailable,
        }),
        isNull,
      );
      expect(describeBrowserAccess({}), isNull);
    });

    test('puts a refusal before the browsers that are not open', () {
      expect(
        describeBrowserAccess({
          Browser.safari: BrowserAccess.notOpen,
          Browser.chrome: BrowserAccess.denied,
        }),
        startsWith('macOS says that Baton may not control Chrome'),
      );
    });
  });

  group('a title that is cut short', () {
    test('is left as it is where it fits', () {
      expect(cutToBytes('The report', 1024), 'The report');
      expect(cutToBytes('é' * 512, 1024), 'é' * 512);
    });

    test('ends at the end of a character, with a mark for what was cut', () {
      for (final text in ['a' * 50, 'é' * 50, '日本語' * 20, '😀' * 30]) {
        final cut = cutToBytes(text, 20);
        expect(utf8.encode(cut).length, lessThanOrEqualTo(20), reason: text);
        expect(cut, endsWith('…'), reason: text);
        // What is left is the start of the text, whole characters of it.
        expect(text.startsWith(cut.substring(0, cut.length - 1)), isTrue);
      }
    });

    test('fills what room there is', () {
      // 20 bytes less the 3 of the mark leave 17: 17 letters.
      expect(cutToBytes('a' * 50, 20), '${'a' * 17}…');
      // Two bytes each: eight letters take 16, a ninth would take 18.
      expect(cutToBytes('é' * 50, 20), '${'é' * 8}…');
    });
  });
}
