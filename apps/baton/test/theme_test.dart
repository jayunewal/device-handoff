// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

double _contrast(Color a, Color b) {
  final la = a.computeLuminance();
  final lb = b.computeLuminance();
  final hi = la > lb ? la : lb;
  final lo = la > lb ? lb : la;
  return (hi + 0.05) / (lo + 0.05);
}

void main() {
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

    test('$name: the action on a note reads on the note', () {
      expect(
        _contrast(scheme.inversePrimary, scheme.inverseSurface),
        greaterThan(4.5),
      );
    });
  }
}
