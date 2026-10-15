// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';
import 'package:qr/qr.dart';

/// The QR code that carries a pairing ticket.
///
/// A ticket is written in capital letters and digits so that a QR code can keep it in its
/// alphanumeric mode (`docs/protocol.md`, "The ticket"): a smaller code, with bigger squares at
/// the same size on the screen, which is what lets a camera read it from across a desk. The
/// `qr` package picks the mode from the characters it is given, so the ticket must reach this
/// as the library wrote it, and not in lower case.
QrImage ticketCode(String ticket) => QrImage(
  QrCode(
    payload: QrPayload.fromString(ticket),
    errorCorrectLevel: QrErrorCorrectLevel.medium,
  ),
);

/// A QR code, in black on white whatever the theme is, with the blank border that cameras need.
class QrView extends StatelessWidget {
  const QrView({required this.code, this.size = 280, super.key});

  final QrImage code;

  /// The width and height on the screen.
  final double size;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: 'QR code to scan with your other device',
      image: true,
      excludeSemantics: true,
      child: CustomPaint(size: Size.square(size), painter: _QrPainter(code)),
    );
  }
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.code);

  final QrImage code;

  /// How many squares wide the blank border is. The QR standard asks for four.
  static const _border = 4;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = Colors.white);
    final squares = code.moduleCount + 2 * _border;
    // Whole pixels, so that every square is as big as the others and none has a soft edge.
    final square = (size.shortestSide / squares).floorToDouble();
    if (square < 1) return;
    final left = (size.width - square * squares) / 2 + _border * square;
    final top = (size.height - square * squares) / 2 + _border * square;
    final dark = Path();
    for (var row = 0; row < code.moduleCount; row++) {
      for (var column = 0; column < code.moduleCount; column++) {
        if (code.isDark(row, column)) {
          dark.addRect(
            Rect.fromLTWH(
              left + column * square,
              top + row * square,
              square,
              square,
            ),
          );
        }
      }
    }
    canvas.drawPath(
      dark,
      Paint()
        ..color = Colors.black
        ..isAntiAlias = false,
    );
  }

  @override
  bool shouldRepaint(_QrPainter oldDelegate) => oldDelegate.code != code;
}
