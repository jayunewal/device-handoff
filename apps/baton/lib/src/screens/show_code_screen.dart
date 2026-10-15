// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:qr/qr.dart';

import '../backend.dart';
import '../format.dart';
import '../pairing_controller.dart';
import '../qr.dart';
import 'content_box.dart';
import 'status_view.dart';

/// "Show a code": this device invites another to pair. The other device scans the QR code or
/// is given the code as text, and then both show the same six digits for the person to
/// compare.
class ShowCodeScreen extends StatefulWidget {
  const ShowCodeScreen({
    required this.session,
    this.now = DateTime.now,
    super.key,
  });

  final Session session;

  /// The time to count the minutes left from. Tests give it a clock of their own.
  final DateTime Function() now;

  @override
  State<ShowCodeScreen> createState() => _ShowCodeScreenState();
}

class _ShowCodeScreenState extends State<ShowCodeScreen> {
  late final OfferController _controller = OfferController(widget.session);
  QrImage? _code;
  String? _codeFor;

  @override
  void initState() {
    super.initState();
    unawaited(_controller.start());
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  /// The QR code for the ticket, made once for each.
  QrImage _qr(String ticket) {
    if (_codeFor != ticket) {
      _code = ticketCode(ticket);
      _codeFor = ticket;
    }
    return _code!;
  }

  Future<void> _copy(String ticket) async {
    final messenger = ScaffoldMessenger.of(context);
    await Clipboard.setData(ClipboardData(text: ticket));
    messenger.showSnackBar(
      const SnackBar(
        content: Text('Copied. Paste it in Baton on your other device.'),
      ),
    );
  }

  void _done() => Navigator.of(context).popUntil((route) => route.isFirst);

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Show a code')),
      body: SafeArea(
        child: ContentBox(
          child: ListenableBuilder(
            listenable: _controller,
            builder: (context, _) => switch (_controller.phase) {
              OfferPhase.starting => const StatusView(
                title: 'Getting a code ready…',
              ),
              OfferPhase.showing => _showing(context, _controller.offer!),
              OfferPhase.asking => _asking(context, _controller.request!),
              OfferPhase.answering => const StatusView(title: 'Pairing…'),
              OfferPhase.paired => _paired(_controller.device!),
              OfferPhase.over => _over(_controller.outcome!),
              OfferPhase.failed => StatusView(
                icon: Icons.error_outline,
                color: Theme.of(context).colorScheme.error,
                title: 'Could not make a code',
                body: sentence(_controller.problem),
                actions: [
                  FilledButton(
                    onPressed: () => unawaited(_controller.again()),
                    child: const Text('Try again'),
                  ),
                ],
              ),
            },
          ),
        ),
      ),
    );
  }

  Widget _showing(BuildContext context, Offer offer) {
    final theme = Theme.of(context);
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 24),
      children: [
        Text(
          'Scan this on your other device',
          textAlign: TextAlign.center,
          style: theme.textTheme.headlineSmall,
        ),
        const SizedBox(height: 8),
        Text(
          'Open Baton there, tap Add a device, then Scan or enter a code.',
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyLarge,
        ),
        const SizedBox(height: 24),
        Center(
          child: LayoutBuilder(
            builder: (context, box) => QrView(
              code: _qr(offer.ticket),
              size: box.maxWidth.clamp(200.0, 320.0).toDouble(),
            ),
          ),
        ),
        const SizedBox(height: 16),
        _TimeLeft(expiresAt: offer.expiresAt.toInt(), now: widget.now),
        const SizedBox(height: 24),
        OutlinedButton.icon(
          onPressed: () => unawaited(_copy(offer.ticket)),
          icon: const Icon(Icons.copy),
          label: const Text('Copy the code as text'),
        ),
        const SizedBox(height: 8),
        Text(
          'No camera on the other device? Copy the code, paste it there, and '
          'Baton will ask you to compare six digits.',
          textAlign: TextAlign.center,
          style: theme.textTheme.bodySmall,
        ),
      ],
    );
  }

  Widget _asking(BuildContext context, PairingRequest request) {
    final theme = Theme.of(context);
    final waiting = _controller.waiting;
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 24),
      children: [
        Icon(
          platformIcon(request.platform),
          size: 48,
          color: theme.colorScheme.primary,
        ),
        const SizedBox(height: 12),
        Text(
          '${request.name} wants to pair',
          textAlign: TextAlign.center,
          style: theme.textTheme.headlineSmall,
        ),
        Text(
          platformName(request.platform),
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyMedium,
        ),
        const SizedBox(height: 24),
        Text(
          'Does the other device show this code?',
          textAlign: TextAlign.center,
          style: theme.textTheme.titleMedium,
        ),
        const SizedBox(height: 8),
        CompareCode(code: request.code),
        const SizedBox(height: 8),
        Text(
          'Say yes only if the six digits are the same on both screens. If '
          'they are not, someone else may be trying to pair.',
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyMedium,
        ),
        const SizedBox(height: 16),
        Text(
          'Its fingerprint',
          textAlign: TextAlign.center,
          style: theme.textTheme.labelMedium,
        ),
        SelectableText(
          request.fingerprint,
          textAlign: TextAlign.center,
          style: theme.textTheme.bodyMedium?.copyWith(
            fontFamily: 'Menlo',
            fontFamilyFallback: const ['Courier', 'monospace'],
          ),
        ),
        const SizedBox(height: 24),
        FilledButton(
          onPressed: () => _controller.answer(accept: true),
          child: const Text('Yes, they match'),
        ),
        const SizedBox(height: 12),
        OutlinedButton(
          onPressed: () => _controller.answer(accept: false),
          child: const Text('No, cancel'),
        ),
        if (waiting > 0) ...[
          const SizedBox(height: 16),
          Text(
            waiting == 1
                ? 'Another device is also asking.'
                : '$waiting other devices are also asking.',
            textAlign: TextAlign.center,
            style: theme.textTheme.bodySmall,
          ),
        ],
      ],
    );
  }

  Widget _paired(Device device) {
    return StatusView(
      icon: Icons.check_circle_outline,
      title: 'Paired with ${device.name}',
      body: platformName(device.platform),
      actions: [FilledButton(onPressed: _done, child: const Text('Done'))],
    );
  }

  Widget _over(OfferOutcome outcome) {
    final (title, body) = switch (outcome) {
      OfferOutcome_Expired() => (
        'The code ran out',
        'A code works for a few minutes. Nothing was paired.',
      ),
      OfferOutcome_Cancelled() => (
        'The code was withdrawn',
        'Nothing was paired.',
      ),
      OfferOutcome_Declined() => (
        'You said no',
        'Nothing was paired, and nothing was shared.',
      ),
      OfferOutcome_Abandoned() => (
        'The other device gave up',
        'It stopped asking before you answered. Nothing was paired.',
      ),
      OfferOutcome_Failed(:final message) => ('Pairing did not work', message),
      // A device was paired: the screen for that is not this one.
      OfferOutcome_Paired() => ('Paired', ''),
    };
    return StatusView(
      icon: Icons.info_outline,
      title: title,
      body: body,
      actions: [
        FilledButton(
          onPressed: () => unawaited(_controller.again()),
          child: const Text('Show a new code'),
        ),
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Close'),
        ),
      ],
    );
  }
}

/// How long the code works for, counted down as it shows.
class _TimeLeft extends StatefulWidget {
  const _TimeLeft({required this.expiresAt, required this.now});

  final int expiresAt;
  final DateTime Function() now;

  @override
  State<_TimeLeft> createState() => _TimeLeftState();
}

class _TimeLeftState extends State<_TimeLeft> {
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    _timer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted) setState(() {});
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        Icon(Icons.timer_outlined, size: 18, color: theme.colorScheme.outline),
        const SizedBox(width: 8),
        Text(
          'Works for ${timeLeft(widget.expiresAt, widget.now())} more',
          style: theme.textTheme.bodyMedium,
        ),
      ],
    );
  }
}
