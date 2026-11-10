// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../backend.dart';
import '../continue_controller.dart';
import '../format.dart';
import '../links.dart';

/// Asks [device] what its person is working on and shows it in a sheet over the screen, to open or
/// copy with one tap each.
Future<void> showContinueSheet(
  BuildContext context, {
  required Session session,
  required Device device,
  OpenLink openLink = openInBrowser,
}) => showModalBottomSheet<void>(
  context: context,
  isScrollControlled: true,
  showDragHandle: true,
  builder: (context) =>
      ContinueSheet(session: session, device: device, openLink: openLink),
);

/// What a paired device has open: the page in front in its browser, and the text on its
/// clipboard. It asks when it is opened, and again when the person says so; nothing of what comes
/// is kept after the sheet is closed.
class ContinueSheet extends StatefulWidget {
  const ContinueSheet({
    required this.session,
    required this.device,
    this.openLink = openInBrowser,
    super.key,
  });

  final Session session;

  /// The device that is asked.
  final Device device;

  /// Opens a web address in the web browser. Tests give it one of their own.
  final OpenLink openLink;

  @override
  State<ContinueSheet> createState() => _ContinueSheetState();
}

class _ContinueSheetState extends State<ContinueSheet> {
  late final ContinueController _controller = ContinueController(
    widget.session,
    widget.device,
  );

  @override
  void initState() {
    super.initState();
    unawaited(_controller.ask());
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final name = visibleText(widget.device.name);
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Continue from $name', style: theme.textTheme.titleLarge),
            const SizedBox(height: 12),
            ListenableBuilder(
              listenable: _controller,
              builder: (context, _) => switch (_controller.view) {
                ContinueAsking() => _Asking(name: name),
                ContinueFound(:final page, :final address, :final text) =>
                  _Found(
                    page: page,
                    address: address,
                    text: text,
                    openLink: widget.openLink,
                  ),
                ContinueNothing() => Text(
                  'Nothing to continue. $name is locked or asleep, or has no '
                  'page open and nothing copied.',
                  style: theme.textTheme.bodyLarge,
                ),
                ContinueStuck(:final sentence, :final mayRetry) => _Stuck(
                  sentence: sentence,
                  onTryAgain: mayRetry
                      ? () => unawaited(_controller.ask())
                      : null,
                ),
              },
            ),
          ],
        ),
      ),
    );
  }
}

class _Asking extends StatelessWidget {
  const _Asking({required this.name});

  final String name;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(
        children: [
          const SizedBox(
            width: 20,
            height: 20,
            child: CircularProgressIndicator(strokeWidth: 2),
          ),
          const SizedBox(width: 12),
          Expanded(child: Text('Asking $name…')),
        ],
      ),
    );
  }
}

class _Stuck extends StatelessWidget {
  const _Stuck({required this.sentence, required this.onTryAgain});

  final String sentence;
  final VoidCallback? onTryAgain;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(sentence, style: Theme.of(context).textTheme.bodyLarge),
        if (onTryAgain != null)
          Align(
            alignment: Alignment.centerRight,
            child: TextButton(
              onPressed: onTryAgain,
              child: const Text('Try again'),
            ),
          ),
      ],
    );
  }
}

class _Found extends StatelessWidget {
  const _Found({
    required this.page,
    required this.address,
    required this.text,
    required this.openLink,
  });

  final ContinuePage? page;
  final String? address;
  final String? text;
  final OpenLink openLink;

  @override
  Widget build(BuildContext context) {
    final page = this.page;
    final text = this.text;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (page != null)
          _PageCard(page: page, address: address, openLink: openLink),
        if (page != null && text != null) const SizedBox(height: 8),
        if (text != null) _TextCard(text: text),
      ],
    );
  }
}

/// The page that is in front in the browser of the device, with the buttons to open it here and to
/// copy its address.
class _PageCard extends StatefulWidget {
  const _PageCard({
    required this.page,
    required this.address,
    required this.openLink,
  });

  final ContinuePage page;
  final String? address;
  final OpenLink openLink;

  @override
  State<_PageCard> createState() => _PageCardState();
}

class _PageCardState extends State<_PageCard> {
  /// What the person is told of the last tap, in the card: a sheet hides the messages that a
  /// screen would show at the bottom.
  String? _note;

  Future<void> _open(String address) async {
    final opened = await widget.openLink(address);
    if (!mounted || opened) return;
    setState(
      () => _note =
          'Baton could not open that page. You can copy its address '
          'instead.',
    );
  }

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.page.url));
    if (mounted) setState(() => _note = 'Copied the address.');
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final title = widget.page.title;
    final address = widget.address;
    return Card(
      margin: EdgeInsets.zero,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(
                  Icons.language,
                  size: 20,
                  color: theme.colorScheme.outline,
                ),
                const SizedBox(width: 8),
                Text('Page in front', style: theme.textTheme.labelLarge),
              ],
            ),
            const SizedBox(height: 8),
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  if (title != null && title.trim().isNotEmpty) ...[
                    Text(
                      visibleText(title),
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleMedium,
                    ),
                    const SizedBox(height: 4),
                  ],
                  Text(
                    visibleText(widget.page.url),
                    maxLines: 3,
                    overflow: TextOverflow.ellipsis,
                    style: theme.textTheme.bodyMedium,
                  ),
                  if (address == null) ...[
                    const SizedBox(height: 8),
                    Text(
                      'This is not a web address, so Baton will not open it.',
                      style: theme.textTheme.bodySmall,
                    ),
                  ],
                  if (_note != null) ...[
                    const SizedBox(height: 8),
                    Text(_note!, style: theme.textTheme.bodySmall),
                  ],
                ],
              ),
            ),
            const SizedBox(height: 4),
            OverflowBar(
              alignment: MainAxisAlignment.end,
              children: [
                if (address != null)
                  FilledButton.tonalIcon(
                    onPressed: () => unawaited(_open(address)),
                    icon: const Icon(Icons.open_in_new),
                    label: const Text('Open'),
                  ),
                TextButton.icon(
                  onPressed: () => unawaited(_copy()),
                  icon: const Icon(Icons.copy),
                  label: const Text('Copy address'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// The text on the clipboard of the device, with the button to copy it here.
class _TextCard extends StatefulWidget {
  const _TextCard({required this.text});

  final String text;

  @override
  State<_TextCard> createState() => _TextCardState();
}

class _TextCardState extends State<_TextCard> {
  bool _copied = false;

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.text));
    if (mounted) setState(() => _copied = true);
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      margin: EdgeInsets.zero,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.notes, size: 20, color: theme.colorScheme.outline),
                const SizedBox(width: 8),
                Text('On the clipboard', style: theme.textTheme.labelLarge),
              ],
            ),
            const SizedBox(height: 8),
            Padding(
              padding: const EdgeInsets.only(right: 8),
              // A long text is cut short here, and copied whole.
              child: Text(
                visibleText(widget.text),
                maxLines: 6,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.bodyLarge,
              ),
            ),
            const SizedBox(height: 4),
            OverflowBar(
              alignment: MainAxisAlignment.end,
              children: [
                FilledButton.tonalIcon(
                  onPressed: () => unawaited(_copy()),
                  icon: Icon(_copied ? Icons.check : Icons.copy),
                  label: Text(_copied ? 'Copied' : 'Copy'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
