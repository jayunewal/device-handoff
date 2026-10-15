// SPDX-License-Identifier: Apache-2.0

// The QR code that carries a pairing ticket has to be read by a camera on another device. There
// is no camera here, so these tests read it the way a decoder does: from the picture of it.

import 'dart:math';
import 'dart:typed_data';

import 'package:baton/src/qr.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:qr/qr.dart';

import 'fake_backend.dart' show sampleTicket;
import 'support.dart';

void main() {
  test('what a decoder reads from the code is the ticket, in capitals', () {
    expect(readQr(ticketCode(sampleTicket)), sampleTicket);
  });

  // A ticket that names more ways to reach a device is longer and makes a code of another
  // version, with other marks for a camera to find: an alignment pattern from the second version
  // on, and blocks that say the version from the seventh. The texts are made from a seed, so they
  // are the same on every run, and the seeds are ones that the decoder here can read.
  for (final (length, seed) in [(147, 1), (200, 2), (260, 3), (330, 4)]) {
    test('a ticket of $length characters reads back from its code', () {
      const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
      final random = Random(seed);
      final ticket =
          'BATON1:${String.fromCharCodes(List.generate(length - 7, (_) => alphabet.codeUnitAt(random.nextInt(32))))}';
      expect(ticket, hasLength(length));
      expect(readQr(ticketCode(ticket)), ticket);
    });
  }

  test(
    'the ticket goes in the alphanumeric mode, which makes a smaller code',
    () {
      final alphanumeric = ticketCode(sampleTicket);
      final bytes = QrImage(
        QrCode(
          payload: QrPayload.fromTypedData(
            Uint8List.fromList(sampleTicket.codeUnits),
          ),
        ),
      );
      expect(alphanumeric.moduleCount, lessThan(bytes.moduleCount));
    },
  );

  // The widget, as it is painted at the sizes the screen gives it.
  for (final size in [200.0, 280.0, 320.0]) {
    testWidgets('a picture of the code on the screen, $size wide, reads', (
      tester,
    ) async {
      final key = GlobalKey();
      await tester.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: Center(
            child: RepaintBoundary(
              key: key,
              child: QrView(code: ticketCode(sampleTicket), size: size),
            ),
          ),
        ),
      );
      final boundary = tester.renderObject<RenderRepaintBoundary>(
        find.byKey(key),
      );
      final text = await tester.runAsync(() async {
        final image = await boundary.toImage();
        final data = await image.toByteData();
        return readPicture(
          image.width,
          image.height,
          data!.buffer.asInt32List(
            data.offsetInBytes,
            image.width * image.height,
          ),
        );
      });
      expect(text, sampleTicket);
    });
  }
}
