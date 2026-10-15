// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../scanner.dart';
import 'content_box.dart';
import 'enter_code_screen.dart';
import 'show_code_screen.dart';

/// "Add a device": one of the two devices shows a code, and the other reads it.
class AddDeviceScreen extends StatelessWidget {
  const AddDeviceScreen({
    required this.session,
    this.now = DateTime.now,
    this.scanner = cameraScanner,
    super.key,
  });

  final Session session;

  /// The time to count the minutes left from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// Reads QR codes with the camera. Tests give it one of their own.
  final ScannerBuilder scanner;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Scaffold(
      appBar: AppBar(title: const Text('Add a device')),
      body: SafeArea(
        child: ContentBox(
          child: ListView(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 24),
            children: [
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8),
                child: Text(
                  'Open Baton on the other device too. One of you shows a code, '
                  'and the other reads it.',
                  style: text.bodyLarge,
                ),
              ),
              const SizedBox(height: 24),
              Card(
                margin: EdgeInsets.zero,
                child: ListTile(
                  leading: const Icon(Icons.qr_code_2, size: 32),
                  title: const Text('Show a code'),
                  subtitle: const Text(
                    'The other device reads it with its camera, or you paste it '
                    'there.',
                  ),
                  trailing: const Icon(Icons.chevron_right),
                  onTap: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) =>
                          ShowCodeScreen(session: session, now: now),
                    ),
                  ),
                ),
              ),
              const SizedBox(height: 12),
              Card(
                margin: EdgeInsets.zero,
                child: ListTile(
                  leading: const Icon(Icons.qr_code_scanner, size: 32),
                  title: const Text('Scan or enter a code'),
                  subtitle: const Text(
                    'Read the code that the other device shows.',
                  ),
                  trailing: const Icon(Icons.chevron_right),
                  onTap: () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      builder: (_) =>
                          EnterCodeScreen(session: session, scanner: scanner),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
