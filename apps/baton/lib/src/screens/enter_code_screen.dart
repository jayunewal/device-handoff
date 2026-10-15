// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../backend.dart';
import '../format.dart';
import '../pairing_controller.dart';
import '../scanner.dart';
import 'content_box.dart';
import 'status_view.dart';

/// "Scan or enter a code": this device joins the invitation that another device shows, by
/// reading its QR code or by being given the code as text. Then both show the same six digits
/// for the person to compare, and the person says yes on the device that made the invitation.
class EnterCodeScreen extends StatefulWidget {
  const EnterCodeScreen({
    required this.session,
    this.scanner = cameraScanner,
    super.key,
  });

  final Session session;

  /// Reads QR codes with the camera. Tests give it one of their own.
  final ScannerBuilder scanner;

  @override
  State<EnterCodeScreen> createState() => _EnterCodeScreenState();
}

class _EnterCodeScreenState extends State<EnterCodeScreen> {
  late final JoinController _controller = JoinController(widget.session);
  final TextEditingController _text = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    _text.dispose();
    super.dispose();
  }

  Future<void> _paste() async {
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    final pasted = data?.text;
    if (pasted == null || pasted.isEmpty) return;
    _text.text = pasted;
  }

  void _done() => Navigator.of(context).popUntil((route) => route.isFirst);

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Scan or enter a code')),
      body: SafeArea(
        child: ContentBox(
          child: ListenableBuilder(
            listenable: _controller,
            builder: (context, _) => switch (_controller.phase) {
              JoinPhase.entering => _entering(context),
              JoinPhase.connecting => StatusView(
                title: 'Reaching the other device…',
                body: 'Keep Baton open on both devices.',
                actions: [_cancel()],
              ),
              JoinPhase.comparing => _comparing(context),
              JoinPhase.paired => StatusView(
                icon: Icons.check_circle_outline,
                title: 'Paired with ${_controller.device!.name}',
                body: platformName(_controller.device!.platform),
                actions: [
                  FilledButton(onPressed: _done, child: const Text('Done')),
                ],
              ),
              JoinPhase.failed => StatusView(
                icon: Icons.error_outline,
                color: Theme.of(context).colorScheme.error,
                title: 'Pairing did not work',
                body: sentence(_controller.problem),
                actions: [
                  FilledButton(
                    onPressed: _controller.tryAgain,
                    child: const Text('Try again'),
                  ),
                  TextButton(
                    onPressed: () => Navigator.of(context).pop(),
                    child: const Text('Close'),
                  ),
                ],
              ),
            },
          ),
        ),
      ),
    );
  }

  Widget _cancel() => OutlinedButton(
    onPressed: () => Navigator.of(context).pop(),
    child: const Text('Cancel'),
  );

  Widget _entering(BuildContext context) {
    final theme = Theme.of(context);
    final problem = _controller.problem;
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 24),
      children: [
        Text(
          'Point the camera at the code on your other device',
          textAlign: TextAlign.center,
          style: theme.textTheme.headlineSmall,
        ),
        const SizedBox(height: 4),
        Text(
          'There, open Baton, tap Add a device, then Show a code.',
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyLarge,
        ),
        const SizedBox(height: 16),
        Center(
          child: SizedBox.square(
            dimension: 280,
            child: widget.scanner(context, _controller.scanned),
          ),
        ),
        const SizedBox(height: 24),
        Text('Or paste the code', style: theme.textTheme.titleMedium),
        const SizedBox(height: 8),
        TextField(
          controller: _text,
          minLines: 1,
          maxLines: 4,
          autocorrect: false,
          enableSuggestions: false,
          textInputAction: TextInputAction.done,
          onSubmitted: _controller.join,
          decoration: InputDecoration(
            labelText: 'Code from the other device',
            border: const OutlineInputBorder(),
            suffixIcon: IconButton(
              icon: const Icon(Icons.content_paste),
              tooltip: 'Paste',
              onPressed: () => unawaited(_paste()),
            ),
          ),
        ),
        if (problem.isNotEmpty) ...[
          const SizedBox(height: 8),
          Text(
            sentence(problem),
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.error,
            ),
          ),
        ],
        const SizedBox(height: 16),
        FilledButton(
          onPressed: () => _controller.join(_text.text),
          child: const Text('Connect'),
        ),
      ],
    );
  }

  Widget _comparing(BuildContext context) {
    final theme = Theme.of(context);
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 32),
      children: [
        Text(
          'Check the other device',
          textAlign: TextAlign.center,
          style: theme.textTheme.headlineSmall,
        ),
        const SizedBox(height: 12),
        Text(
          'It should show this code. If the six digits are the same, tap Yes '
          'there.',
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyLarge,
        ),
        const SizedBox(height: 24),
        CompareCode(code: _controller.code),
        const SizedBox(height: 24),
        const Center(child: CircularProgressIndicator()),
        const SizedBox(height: 12),
        Text(
          'Waiting for the other device to say yes…',
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyMedium,
        ),
        const SizedBox(height: 24),
        _cancel(),
      ],
    );
  }
}
