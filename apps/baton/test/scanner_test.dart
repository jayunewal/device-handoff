// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/scanner.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

Future<void> _show(WidgetTester tester, MobileScannerErrorCode code) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: CameraProblem(error: MobileScannerException(errorCode: code)),
      ),
    ),
  );
}

void main() {
  // There is no camera to try, here or on a simulator. What can be held to is what the person
  // is told when there is none, which is the way to the paste box.
  testWidgets('says that the camera is not allowed, and where to allow it', (
    tester,
  ) async {
    await _show(tester, MobileScannerErrorCode.permissionDenied);

    expect(find.textContaining('may not use the camera'), findsOneWidget);
    expect(find.textContaining('Settings'), findsOneWidget);
    expect(find.textContaining('paste the code instead'), findsOneWidget);
  });

  testWidgets('says that there is no camera to use', (tester) async {
    await _show(tester, MobileScannerErrorCode.unsupported);

    expect(find.textContaining('has no camera'), findsOneWidget);
    expect(find.textContaining('paste the code instead'), findsOneWidget);
  });

  testWidgets('says that the camera did not start, whatever the reason', (
    tester,
  ) async {
    await _show(tester, MobileScannerErrorCode.genericError);

    expect(find.textContaining('could not start'), findsOneWidget);
    expect(find.textContaining('paste the code instead'), findsOneWidget);
  });
}
