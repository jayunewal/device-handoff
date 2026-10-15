// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../backend.dart';
import '../diagnostics.dart';
import 'content_box.dart';

/// What a person copies into a bug report: facts about this device and how it is connected, and
/// the last lines Baton logged.
class DiagnosticsScreen extends StatefulWidget {
  const DiagnosticsScreen({
    required this.recentLog,
    this.session,
    this.problem,
    this.system,
    this.now = DateTime.now,
    super.key,
  });

  /// Where the lines of the log come from.
  final List<String> Function() recentLog;

  /// The Baton that is running, or `null` if the screen is shown because Baton could not start.
  final Session? session;

  /// What went wrong, if Baton could not start.
  final String? problem;

  /// What the operating system says it is, in a few words. By default, what this one says.
  final String? system;

  /// The time to put on the report. Tests give it a clock of their own.
  final DateTime Function() now;

  @override
  State<DiagnosticsScreen> createState() => _DiagnosticsScreenState();
}

class _DiagnosticsScreenState extends State<DiagnosticsScreen> {
  late final DiagnosticsController _controller = DiagnosticsController(
    recentLog: widget.recentLog,
    system: widget.system ?? describeSystem(),
    session: widget.session,
    problem: widget.problem,
    now: widget.now,
  );

  @override
  void initState() {
    super.initState();
    unawaited(_controller.refresh());
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _copy(Diagnostics diagnostics) async {
    final messenger = ScaffoldMessenger.of(context);
    await Clipboard.setData(ClipboardData(text: diagnostics.report));
    messenger.showSnackBar(const SnackBar(content: Text('Copied.')));
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Diagnostics')),
      body: SafeArea(
        child: ContentBox(
          child: ListenableBuilder(
            listenable: _controller,
            builder: (context, _) {
              final diagnostics = _controller.diagnostics;
              if (diagnostics == null) {
                return const Center(child: CircularProgressIndicator());
              }
              return _Report(
                diagnostics: diagnostics,
                onCopy: () => unawaited(_copy(diagnostics)),
                onRefresh: () => unawaited(_controller.refresh()),
              );
            },
          ),
        ),
      ),
    );
  }
}

class _Report extends StatelessWidget {
  const _Report({
    required this.diagnostics,
    required this.onCopy,
    required this.onRefresh,
  });

  final Diagnostics diagnostics;
  final VoidCallback onCopy;
  final VoidCallback onRefresh;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final heading = theme.textTheme.titleMedium;
    final log = theme.textTheme.bodySmall?.copyWith(
      fontFamily: 'Menlo',
      fontFamilyFallback: const ['Courier', 'monospace'],
    );
    return SelectionArea(
      child: ListView(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 32),
        children: [
          Text(
            'If something does not work, copy this and paste it where you report '
            'the problem. It names this device and the ones paired with it, and '
            'shows the addresses of your own network. Addresses on the internet '
            'are hidden. Read it before you paste it.',
            style: theme.textTheme.bodyMedium,
          ),
          const SizedBox(height: 16),
          Wrap(
            spacing: 12,
            runSpacing: 8,
            children: [
              FilledButton.icon(
                onPressed: onCopy,
                icon: const Icon(Icons.copy),
                label: const Text('Copy'),
              ),
              OutlinedButton.icon(
                onPressed: onRefresh,
                icon: const Icon(Icons.refresh),
                label: const Text('Refresh'),
              ),
            ],
          ),
          const SizedBox(height: 24),
          Text('Facts', style: heading),
          const SizedBox(height: 8),
          for (final fact in diagnostics.facts)
            Padding(
              padding: const EdgeInsets.only(bottom: 4),
              child: Text(fact),
            ),
          const SizedBox(height: 24),
          Text('Log', style: heading),
          const SizedBox(height: 4),
          Text(diagnostics.logSummary, style: theme.textTheme.bodyMedium),
          const SizedBox(height: 8),
          for (final line in diagnostics.log)
            Padding(
              padding: const EdgeInsets.only(bottom: 4),
              child: Text(line, style: log),
            ),
        ],
      ),
    );
  }
}
