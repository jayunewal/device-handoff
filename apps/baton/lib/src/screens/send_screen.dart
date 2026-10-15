// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../backend.dart';
import '../format.dart';
import '../send_controller.dart';
import 'content_box.dart';

/// "Send to" one paired device: a link or some text to type or paste, and a button.
class SendScreen extends StatefulWidget {
  const SendScreen({
    required this.session,
    required this.device,
    this.now = DateTime.now,
    super.key,
  });

  final Session session;

  /// The device that things are sent to.
  final Device device;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  @override
  State<SendScreen> createState() => _SendScreenState();
}

class _SendScreenState extends State<SendScreen> {
  late final SendController _controller = SendController(
    widget.session,
    widget.device,
  );
  final TextEditingController _field = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    _field.dispose();
    super.dispose();
  }

  Future<void> _paste() async {
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    if (!mounted) return;
    final text = data?.text;
    if (text == null || text.trim().isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('There is no text on the clipboard.')),
      );
      return;
    }
    _field.text = text;
    _controller.edited();
  }

  Future<void> _send() async {
    final text = _field.text;
    final sent = await _controller.send(text);
    // Empty the box, unless the person has started on something else in the meantime.
    if (sent && mounted && _field.text == text) _field.clear();
  }

  @override
  Widget build(BuildContext context) {
    final device = widget.device;
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(title: Text('Send to ${device.name}')),
      body: SafeArea(
        child: ContentBox(
          child: ListenableBuilder(
            listenable: Listenable.merge([_controller, _field]),
            builder: (context, _) {
              final text = _field.text;
              final link = _controller.linkIn(text);
              final empty = text.trim().isEmpty;
              final outcome = _controller.outcome;
              return ListView(
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 16,
                ),
                children: [
                  Card(
                    margin: EdgeInsets.zero,
                    child: ListTile(
                      leading: Icon(platformIcon(device.platform), size: 32),
                      title: Text(device.name),
                      subtitle: Text(
                        '${platformName(device.platform)} · '
                        '${lastSeen(device.lastSeenAt?.toInt(), widget.now())}'
                        '\n${device.fingerprint}',
                      ),
                      isThreeLine: true,
                    ),
                  ),
                  const SizedBox(height: 24),
                  TextField(
                    controller: _field,
                    minLines: 3,
                    maxLines: 8,
                    keyboardType: TextInputType.multiline,
                    textCapitalization: TextCapitalization.sentences,
                    onChanged: (_) => _controller.edited(),
                    decoration: const InputDecoration(
                      labelText: 'A link or some text',
                      alignLabelWithHint: true,
                      border: OutlineInputBorder(),
                    ),
                  ),
                  if (!empty) ...[
                    const SizedBox(height: 8),
                    Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 4),
                      child: Text(
                        link != null
                            ? 'It will be sent as a link, which ${device.name} '
                                  'can open.'
                            : 'It will be sent as text, which ${device.name} '
                                  'can copy.',
                        style: theme.textTheme.bodySmall,
                      ),
                    ),
                  ],
                  const SizedBox(height: 16),
                  Row(
                    children: [
                      OutlinedButton.icon(
                        onPressed: _controller.sending
                            ? null
                            : () => unawaited(_paste()),
                        icon: const Icon(Icons.content_paste),
                        label: const Text('Paste'),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: FilledButton.icon(
                          onPressed: empty || _controller.sending
                              ? null
                              : () => unawaited(_send()),
                          icon: _controller.sending
                              ? const SizedBox.square(
                                  dimension: 18,
                                  child: CircularProgressIndicator(
                                    strokeWidth: 2,
                                  ),
                                )
                              : const Icon(Icons.send),
                          label: Text(link != null ? 'Send link' : 'Send text'),
                        ),
                      ),
                    ],
                  ),
                  if (_controller.sending || _controller.checking) ...[
                    const SizedBox(height: 16),
                    Semantics(
                      liveRegion: true,
                      child: Text(
                        _controller.sending
                            ? 'Sending to ${device.name}. If Baton is not open '
                                  'there, Baton gives up after a while.'
                            : 'Waiting for ${device.name} to answer.',
                        style: theme.textTheme.bodyMedium,
                      ),
                    ),
                  ],
                  if (outcome != null) ...[
                    const SizedBox(height: 16),
                    _OutcomeLine(outcome: outcome),
                  ],
                  const SizedBox(height: 32),
                  Text(
                    'Baton has to be open on ${device.name} for it to arrive.',
                    style: theme.textTheme.bodyMedium,
                  ),
                  const SizedBox(height: 12),
                  Align(
                    alignment: Alignment.centerLeft,
                    child: TextButton.icon(
                      onPressed: _controller.checking
                          ? null
                          : () => unawaited(_controller.check()),
                      icon: _controller.checking
                          ? const SizedBox.square(
                              dimension: 18,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            )
                          : const Icon(Icons.network_check),
                      label: const Text('Check the connection'),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}

/// A sentence about what just happened, with a mark that says whether it went well.
class _OutcomeLine extends StatelessWidget {
  const _OutcomeLine({required this.outcome});

  final Outcome outcome;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final color = outcome.ok ? scheme.primary : scheme.error;
    return Semantics(
      liveRegion: true,
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Icon(
            outcome.ok ? Icons.check_circle_outline : Icons.error_outline,
            color: color,
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Text(
              outcome.message,
              style: Theme.of(context).textTheme.bodyLarge
                  ?.copyWith(color: color),
            ),
          ),
        ],
      ),
    );
  }
}
