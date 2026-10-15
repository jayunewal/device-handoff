// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

/// A screen's worth of one thing to say: an icon (or a spinner), a headline, some words, and
/// the buttons that follow from them.
class StatusView extends StatelessWidget {
  const StatusView({
    required this.title,
    this.icon,
    this.color,
    this.body,
    this.children = const [],
    this.actions = const [],
    super.key,
  }) : busy = icon == null;

  /// What the screen says, in a few words.
  final String title;

  /// A picture of it. Without one, a spinner shows that something is going on.
  final IconData? icon;

  /// The colour of [icon]; the theme's main colour if there is none.
  final Color? color;

  /// A sentence or two that say more.
  final String? body;

  /// Anything else to show between the words and the buttons.
  final List<Widget> children;

  /// The buttons, in the order they are to be read.
  final List<Widget> actions;

  final bool busy;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return ListView(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 32),
      children: [
        Center(
          child: busy
              ? const Padding(
                  padding: EdgeInsets.all(8),
                  child: CircularProgressIndicator(),
                )
              : Icon(icon, size: 64, color: color ?? theme.colorScheme.primary),
        ),
        const SizedBox(height: 16),
        Text(
          title,
          textAlign: TextAlign.center,
          style: theme.textTheme.headlineSmall,
        ),
        if (body != null) ...[
          const SizedBox(height: 12),
          Text(
            body!,
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyLarge,
          ),
        ],
        for (final child in children) ...[const SizedBox(height: 16), child],
        for (final action in actions) ...[const SizedBox(height: 12), action],
      ],
    );
  }
}

/// Six digits that two screens show and a person compares, large enough to read across a desk.
class CompareCode extends StatelessWidget {
  const CompareCode({required this.code, super.key});

  /// As `514 643`.
  final String code;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Semantics(
      label: 'Code to compare: ${code.replaceAll(' ', '').split('').join(' ')}',
      excludeSemantics: true,
      child: Center(
        child: FittedBox(
          fit: BoxFit.scaleDown,
          child: Text(
            code,
            style: theme.textTheme.displayMedium?.copyWith(
              fontWeight: FontWeight.w600,
              letterSpacing: 4,
              fontFeatures: const [FontFeature.tabularFigures()],
            ),
          ),
        ),
      ),
    );
  }
}
