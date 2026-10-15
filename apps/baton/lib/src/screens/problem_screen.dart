// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import 'content_box.dart';

/// Shown when Baton could not start: what went wrong, and a way to try again.
class ProblemScreen extends StatelessWidget {
  const ProblemScreen({
    required this.problem,
    required this.onTryAgain,
    super.key,
  });

  /// One sentence that says what went wrong.
  final String problem;

  final Future<void> Function() onTryAgain;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      body: SafeArea(
        child: ContentBox(
          child: ListView(
            padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 32),
            children: [
              Icon(
                Icons.error_outline,
                size: 48,
                color: theme.colorScheme.error,
              ),
              const SizedBox(height: 16),
              Text(
                'Baton could not start',
                style: theme.textTheme.headlineMedium,
              ),
              const SizedBox(height: 12),
              SelectableText(problem, style: theme.textTheme.bodyLarge),
              const SizedBox(height: 24),
              FilledButton(
                onPressed: () => unawaited(onTryAgain()),
                child: const Text('Try again'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Shown while the saved name is looked up and while Baton starts.
class WaitingScreen extends StatelessWidget {
  const WaitingScreen({super.key});

  @override
  Widget build(BuildContext context) {
    return const Scaffold(body: Center(child: CircularProgressIndicator()));
  }
}
