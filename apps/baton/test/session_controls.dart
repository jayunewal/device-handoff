// SPDX-License-Identifier: Apache-2.0

import 'package:flutter_test/flutter_test.dart';

/// Opens what the ball of a window that runs holds (its sound, its settings, control and Stop), if
/// it is not open already. A window that does not run has no ball, and nothing is done.
Future<void> openSessionControls(WidgetTester tester) async {
  final ball = find.byTooltip('Session controls');
  if (ball.evaluate().isEmpty) return;
  if (find.byTooltip('Stop showing').evaluate().isNotEmpty) return;
  await tester.tap(ball);
  await tester.pump();
}

/// Closes what the ball holds, if it is open, so that the picture and what is over it take
/// touches again.
Future<void> closeSessionControls(WidgetTester tester) async {
  if (find.byTooltip('Stop showing').evaluate().isEmpty) return;
  await tester.tap(find.byTooltip('Session controls'));
  await tester.pump();
}
