// SPDX-License-Identifier: Apache-2.0

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../backend.dart';
import '../format.dart';
import '../inbox_controller.dart';
import '../links.dart';
import 'content_box.dart';
import 'status_view.dart';

/// What other devices have sent to this one, newest first, with what to do with each: open a
/// link, copy it or some text, remove it.
class InboxScreen extends StatelessWidget {
  const InboxScreen({
    required this.controller,
    this.now = DateTime.now,
    this.openLink = openInBrowser,
    super.key,
  });

  final InboxController controller;

  /// The time to measure "5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// Opens a link in the web browser. Tests give it one of their own.
  final OpenLink openLink;

  Future<void> _clear(BuildContext context) async {
    final messenger = ScaffoldMessenger.of(context);
    final count = controller.items?.length ?? 0;
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Clear the inbox?'),
        content: Text(
          count == 1
              ? 'The one item in it will be deleted from this device.'
              : 'All $count items in it will be deleted from this device.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Clear'),
          ),
        ],
      ),
    );
    if (sure != true) return;
    final problem = await controller.clear();
    if (problem != null) {
      messenger.showSnackBar(SnackBar(content: Text(problem)));
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final items = controller.items;
        return Scaffold(
          appBar: AppBar(
            title: const Text('Inbox'),
            actions: [
              IconButton(
                tooltip: 'Clear the inbox',
                icon: const Icon(Icons.delete_sweep_outlined),
                onPressed: items == null || items.isEmpty
                    ? null
                    : () => unawaited(_clear(context)),
              ),
            ],
          ),
          body: SafeArea(
            child: ContentBox(
              child: switch ((controller.problem, items)) {
                (final String problem, _) => StatusView(
                  icon: Icons.error_outline,
                  color: Theme.of(context).colorScheme.error,
                  title: 'Could not read the inbox',
                  body: problem,
                  actions: [
                    FilledButton(
                      onPressed: () => unawaited(controller.reload()),
                      child: const Text('Try again'),
                    ),
                  ],
                ),
                (_, null) => const Center(child: CircularProgressIndicator()),
                (_, final List<Received> list) when list.isEmpty =>
                  const _Empty(),
                (_, final List<Received> list) => ListView.builder(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
                  itemCount: list.length,
                  itemBuilder: (context, index) => _ItemCard(
                    // The number is never used twice, so a card keeps its place in the list
                    // (and whether it is open) as others come and go.
                    key: ValueKey(list[index].number),
                    received: list[index],
                    controller: controller,
                    now: now(),
                    openLink: openLink,
                  ),
                ),
              },
            ),
          ),
        );
      },
    );
  }
}

class _Empty extends StatelessWidget {
  const _Empty();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 48, horizontal: 24),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.start,
        children: [
          Icon(
            Icons.inbox_outlined,
            size: 48,
            color: theme.colorScheme.outline,
          ),
          const SizedBox(height: 12),
          Text('Nothing here yet', style: theme.textTheme.titleMedium),
          const SizedBox(height: 4),
          Text(
            'Links and text that your other devices send to this one show up '
            'here. Baton has to be open on this device to receive them.',
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyMedium,
          ),
        ],
      ),
    );
  }
}

/// How long text can be before a card shows only the start of it.
const _collapsedLines = 6;

/// The space to the right of what an item holds, between it and the edge of the card.
const _bodyInset = 8.0;

/// Whether [text] takes more than [_collapsedLines] lines when it is set the way a card sets
/// it, in [width]. How lines wrap depends on the width and on the size of the letters, which
/// the person can change for the whole system, so this asks the layout and does not count
/// characters or line ends.
bool _isCut(BuildContext context, String text, TextStyle? style, double width) {
  final painter = TextPainter(
    text: TextSpan(
      text: text,
      style: DefaultTextStyle.of(context).style.merge(style),
    ),
    maxLines: _collapsedLines,
    textDirection: Directionality.of(context),
    textScaler: MediaQuery.textScalerOf(context),
    locale: Localizations.maybeLocaleOf(context),
  )..layout(maxWidth: math.max(0, width));
  final cut = painter.didExceedMaxLines;
  painter.dispose();
  return cut;
}

class _ItemCard extends StatefulWidget {
  const _ItemCard({
    required this.received,
    required this.controller,
    required this.now,
    required this.openLink,
    super.key,
  });

  final Received received;
  final InboxController controller;
  final DateTime now;
  final OpenLink openLink;

  @override
  State<_ItemCard> createState() => _ItemCardState();
}

class _ItemCardState extends State<_ItemCard> {
  bool _expanded = false;

  Received get _received => widget.received;

  Future<void> _copy(String text) async {
    final messenger = ScaffoldMessenger.of(context);
    await Clipboard.setData(ClipboardData(text: text));
    messenger.showSnackBar(const SnackBar(content: Text('Copied.')));
  }

  Future<void> _open(String address) async {
    final messenger = ScaffoldMessenger.of(context);
    if (await widget.openLink(address)) return;
    messenger.showSnackBar(
      const SnackBar(
        content: Text(
          'Baton could not open that link. You can copy it instead.',
        ),
      ),
    );
  }

  Future<void> _remove() async {
    final messenger = ScaffoldMessenger.of(context);
    final problem = await widget.controller.remove(_received);
    if (problem != null) {
      messenger.showSnackBar(SnackBar(content: Text(problem)));
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final item = _received.item;
    final when = since(_received.receivedAt.toInt(), widget.now);
    final address = widget.controller.addressToOpen(_received);
    final (
      IconData icon,
      String copyThis,
      List<Widget> content,
    ) = switch (item) {
      ReceivedItem_Link(:final url, :final title) => (
        Icons.link,
        url,
        [
          if (title != null && title.trim().isNotEmpty) ...[
            Text(visibleText(title), style: theme.textTheme.titleMedium),
            const SizedBox(height: 4),
          ],
          SelectableText(visibleText(url), style: theme.textTheme.bodyMedium),
          if (address == null) ...[
            const SizedBox(height: 8),
            Text(
              'This is not a web address, so Baton will not open it.',
              style: theme.textTheme.bodySmall,
            ),
          ],
        ],
      ),
      ReceivedItem_Text(:final text) => (
        Icons.notes,
        text,
        [
          _TextBody(
            text: text,
            style: theme.textTheme.bodyLarge,
            expanded: _expanded,
          ),
        ],
      ),
    };
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
        child: LayoutBuilder(
          builder: (context, constraints) {
            final long = switch (item) {
              ReceivedItem_Text(:final text) => _isCut(
                context,
                visibleText(text),
                theme.textTheme.bodyLarge,
                constraints.maxWidth - _bodyInset,
              ),
              ReceivedItem_Link() => false,
            };
            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Icon(icon, size: 20, color: theme.colorScheme.outline),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        '${_received.fromName} · $when',
                        style: theme.textTheme.labelLarge,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Padding(
                  padding: const EdgeInsets.only(right: _bodyInset),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: content,
                  ),
                ),
                const SizedBox(height: 4),
                OverflowBar(
                  alignment: MainAxisAlignment.end,
                  children: [
                    if (long)
                      TextButton(
                        onPressed: () => setState(() => _expanded = !_expanded),
                        child: Text(_expanded ? 'Show less' : 'Show more'),
                      ),
                    if (address != null)
                      FilledButton.tonalIcon(
                        onPressed: () => unawaited(_open(address)),
                        icon: const Icon(Icons.open_in_new),
                        label: const Text('Open'),
                      ),
                    TextButton.icon(
                      onPressed: () => unawaited(_copy(copyThis)),
                      icon: const Icon(Icons.copy),
                      label: const Text('Copy'),
                    ),
                    IconButton(
                      tooltip: 'Remove',
                      onPressed: () => unawaited(_remove()),
                      icon: const Icon(Icons.delete_outline),
                    ),
                  ],
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}

class _TextBody extends StatelessWidget {
  const _TextBody({
    required this.text,
    required this.style,
    required this.expanded,
  });

  final String text;
  final TextStyle? style;
  final bool expanded;

  @override
  Widget build(BuildContext context) {
    final shown = visibleText(text);
    // Text that can be selected has no way to cut itself short, so a long one is cut short
    // here and shown whole, and selectable, once the person asks for it.
    return expanded
        ? SelectableText(shown, style: style)
        : Text(
            shown,
            style: style,
            maxLines: _collapsedLines,
            overflow: TextOverflow.ellipsis,
          );
  }
}
