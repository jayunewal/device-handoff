// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

double _contrast(Color a, Color b) {
  final la = a.computeLuminance();
  final lb = b.computeLuminance();
  final hi = la > lb ? la : lb;
  final lo = la > lb ? lb : la;
  return (hi + 0.05) / (lo + 0.05);
}

/// The size that [finder]'s text is drawn at.
double _drawnSize(WidgetTester tester, Finder finder) {
  final paragraph = tester.renderObject<RenderParagraph>(
    find.descendant(of: finder, matching: find.byType(RichText)).first,
  );
  return paragraph.text.style!.fontSize!;
}

void main() {
  testWidgets('titles, list rows and tab labels keep their sizes', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: BatonTheme.light(),
        home: Scaffold(
          appBar: AppBar(title: const Text('Bar title')),
          body: const Column(
            children: [
              ListTile(title: Text('Row title'), subtitle: Text('Row detail')),
              Text('Plain text'),
              TextField(decoration: InputDecoration(labelText: 'Label')),
            ],
          ),
          bottomNavigationBar: NavigationBar(
            destinations: const [
              NavigationDestination(icon: Icon(Icons.devices), label: 'One'),
              NavigationDestination(icon: Icon(Icons.inbox), label: 'Two'),
            ],
          ),
        ),
      ),
    );
    final scheme = BatonTheme.light().colorScheme;
    expect(_drawnSize(tester, find.text('Bar title')), 22);
    expect(_drawnSize(tester, find.text('Row title')), 17);
    expect(_drawnSize(tester, find.text('Row detail')), 14);
    expect(_drawnSize(tester, find.text('One')), 12);
    // Plain text is the main text colour, not the quieter one kept for details.
    final plain = tester.renderObject<RenderParagraph>(find.text('Plain text'));
    expect(plain.text.style!.color, scheme.onSurface);
  });

  testWidgets('a filled field keeps its floating label inside the fill', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: BatonTheme.light(),
        home: const Scaffold(
          body: TextField(decoration: InputDecoration(labelText: 'Label')),
        ),
      ),
    );
    final decoration = tester
        .widget<InputDecorator>(find.byType(InputDecorator))
        .decoration
        .applyDefaults(BatonTheme.light().inputDecorationTheme);
    expect(decoration.border, isA<UnderlineInputBorder>());
    expect(decoration.enabledBorder, isA<UnderlineInputBorder>());
    expect(decoration.focusedBorder, isA<UnderlineInputBorder>());
  });

  for (final (name, theme) in [
    ('light', BatonTheme.light()),
    ('dark', BatonTheme.dark()),
  ]) {
    final scheme = theme.colorScheme;

    test('$name: text on buttons reads at 4.5:1 or more', () {
      expect(_contrast(scheme.primary, scheme.onPrimary), greaterThan(4.5));
    });

    test('$name: the signature colour reads as text on every surface', () {
      for (final surface in [
        scheme.surface,
        scheme.surfaceContainer,
        scheme.surfaceContainerHigh,
      ]) {
        expect(_contrast(scheme.primary, surface), greaterThan(4.5));
      }
    });

    test('$name: quieter text still reads at 4.5:1 or more', () {
      for (final surface in [scheme.surface, scheme.surfaceContainer]) {
        expect(_contrast(scheme.onSurface, surface), greaterThan(4.5));
        expect(_contrast(scheme.onSurfaceVariant, surface), greaterThan(4.5));
      }
    });

    test('$name: the colour for what is on reads on every surface', () {
      final success = BatonTheme.success(theme.brightness);
      for (final surface in [scheme.surface, scheme.surfaceContainer]) {
        expect(_contrast(success, surface), greaterThan(4.5));
      }
    });

    test('$name: the action on a note reads on the note', () {
      expect(
        _contrast(scheme.inversePrimary, scheme.inverseSurface),
        greaterThan(4.5),
      );
    });
  }
}
