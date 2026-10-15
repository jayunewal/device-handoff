// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

/// Builds a view that reads QR codes with the camera and hands the text of each to `onCode`,
/// as often as the camera sees it. The screen that uses it gives its own way of making one to
/// tests, which have no camera.
typedef ScannerBuilder = Widget Function(
  BuildContext context,
  ValueChanged<String> onCode,
);

/// The camera of this device. It asks the person for permission the first time it starts, and
/// says what to do if there is none.
Widget cameraScanner(BuildContext context, ValueChanged<String> onCode) =>
    _CameraScanner(onCode: onCode);

class _CameraScanner extends StatefulWidget {
  const _CameraScanner({required this.onCode});

  final ValueChanged<String> onCode;

  @override
  State<_CameraScanner> createState() => _CameraScannerState();
}

class _CameraScannerState extends State<_CameraScanner> {
  // Codes of other kinds are none of Baton's business.
  final MobileScannerController _camera = MobileScannerController(
    formats: const [BarcodeFormat.qrCode],
  );

  @override
  void dispose() {
    unawaited(_camera.dispose());
    super.dispose();
  }

  void _onDetect(BarcodeCapture capture) {
    for (final barcode in capture.barcodes) {
      final text = barcode.rawValue;
      if (text != null && text.isNotEmpty) widget.onCode(text);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ClipRRect(
      borderRadius: BorderRadius.circular(16),
      child: AspectRatio(
        aspectRatio: 1,
        child: MobileScanner(
          controller: _camera,
          onDetect: _onDetect,
          errorBuilder: (context, error) => CameraProblem(error: error),
          placeholderBuilder: (context) => const ColoredBox(
            color: Colors.black,
            child: Center(child: CircularProgressIndicator()),
          ),
        ),
      ),
    );
  }
}

/// What to tell the person when the camera cannot be used. The code can be pasted instead.
@visibleForTesting
class CameraProblem extends StatelessWidget {
  const CameraProblem({required this.error, super.key});

  final MobileScannerException error;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final what = switch (error.errorCode) {
      MobileScannerErrorCode.permissionDenied =>
        'Baton may not use the camera. You can allow it in the Settings '
            'app, or in System Settings under Privacy & Security on a Mac.',
      MobileScannerErrorCode.unsupported =>
        'This device has no camera that Baton can use.',
      _ => 'The camera could not start.',
    };
    return ColoredBox(
      color: theme.colorScheme.surfaceContainerHighest,
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                Icons.no_photography_outlined,
                size: 40,
                color: theme.colorScheme.outline,
              ),
              const SizedBox(height: 12),
              Text(
                '$what You can paste the code instead.',
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyMedium,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
