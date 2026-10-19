// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';
import '../windows_controller.dart';

/// The windows, at the bottom of the home screen where both places can show it: the offers for
/// the person to answer, the window that is being opened, the offer that this device is making,
/// the windows that run, and how the last ones ended. It takes no room when there is nothing to
/// show.
class WindowsPanel extends StatelessWidget {
  const WindowsPanel({
    required this.controller,
    required this.onOpen,
    super.key,
  });

  final WindowsController controller;

  /// Takes the person to a window that is shown on this device.
  final void Function(WindowSession session) onOpen;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final opening = controller.opening;
        final offering = controller.offering;
        final cards = <Widget>[
          for (final request in controller.requests)
            _RequestCard(
              request: request,
              onAnswer: (accept) => controller.answer(request, accept: accept),
            ),
          if (opening != null) _OpeningCard(request: opening),
          if (offering != null) _OfferingCard(device: offering),
          for (final session in controller.sessions)
            _SessionCard(
              session: session,
              onOpen: () => onOpen(session),
              onStop: () => controller.end(session),
            ),
          for (final note in controller.notes)
            _NoteCard(note: note, onDismiss: () => controller.dismiss(note)),
        ];
        if (cards.isEmpty) return const SizedBox.shrink();
        return Material(
          color: Theme.of(context).colorScheme.surfaceContainerHigh,
          elevation: 3,
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxHeight: MediaQuery.sizeOf(context).height * 0.5,
            ),
            child: ListView(
              shrinkWrap: true,
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              children: cards,
            ),
          ),
        );
      },
    );
  }
}

/// A label that another device wrote, such as the title of a window or the name of an
/// application, made fit to show on one line: the marks that cannot be seen or that turn the text
/// round are shown as replacement marks, and each run of spaces and line ends is one space.
String windowLabel(String? text) =>
    visibleText(text ?? '').replaceAll(RegExp(r'\s+'), ' ').trim();

/// What a window is called: its title, else the application it belongs to, else "a window".
String windowName(String? title, String? app) {
  for (final label in [windowLabel(title), windowLabel(app)]) {
    if (label.isNotEmpty) return label;
  }
  return 'a window';
}

/// A device offers to show a window on this one. The person says yes or no.
class _RequestCard extends StatelessWidget {
  const _RequestCard({required this.request, required this.onAnswer});

  final WindowRequest request;
  final void Function(bool accept) onAnswer;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final title = windowLabel(request.title);
    final app = windowLabel(request.app);
    final what = [
      if (title.isNotEmpty) title,
      if (app.isNotEmpty) app,
    ].join(' · ');
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Semantics(
              liveRegion: true,
              child: Text(
                '${visibleText(request.deviceName)} wants to show you a window',
                style: theme.textTheme.titleMedium,
              ),
            ),
            if (what.isNotEmpty) ...[
              const SizedBox(height: 4),
              Text(what, style: theme.textTheme.bodyMedium),
            ],
            const SizedBox(height: 4),
            Text(
              'You will see it live on this screen. You can only look at it.',
              style: theme.textTheme.bodySmall,
            ),
            const SizedBox(height: 8),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                TextButton(
                  onPressed: () => onAnswer(false),
                  child: const Text('Decline'),
                ),
                const SizedBox(width: 8),
                FilledButton(
                  onPressed: () => onAnswer(true),
                  child: const Text('Show it'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// The person said yes, and the window has not begun.
class _OpeningCard extends StatelessWidget {
  const _OpeningCard({required this.request});

  final WindowRequest request;

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Semantics(
              liveRegion: true,
              child: Text(
                'Opening the window from ${visibleText(request.deviceName)}…',
                style: Theme.of(context).textTheme.titleSmall,
              ),
            ),
            const SizedBox(height: 12),
            const LinearProgressIndicator(semanticsLabel: 'Opening the window'),
          ],
        ),
      ),
    );
  }
}

/// This device is offering a window to [device]: the person chooses it, and the device is asked.
class _OfferingCard extends StatelessWidget {
  const _OfferingCard({required this.device});

  final Device device;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final name = visibleText(device.name);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Semantics(
              liveRegion: true,
              child: Text(
                'Sharing a window with $name',
                style: theme.textTheme.titleSmall,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              'Choose the window. $name is then asked whether to show it.',
              style: theme.textTheme.bodySmall,
            ),
            const SizedBox(height: 12),
            const LinearProgressIndicator(semanticsLabel: 'Sharing a window'),
          ],
        ),
      ),
    );
  }
}

/// A window that runs: one that another device shows on this one, or one of this device that
/// another shows.
class _SessionCard extends StatelessWidget {
  const _SessionCard({
    required this.session,
    required this.onOpen,
    required this.onStop,
  });

  final WindowSession session;
  final VoidCallback onOpen;
  final VoidCallback onStop;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final viewing = session.role == WindowRole.viewing;
    final name = windowName(session.title, session.app);
    final device = visibleText(session.deviceName);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Semantics(
              liveRegion: true,
              child: Text(
                viewing
                    ? 'Showing $name from $device'
                    : 'Sharing $name with $device',
                style: theme.textTheme.titleSmall,
              ),
            ),
            const SizedBox(height: 8),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                if (viewing) ...[
                  TextButton(onPressed: onOpen, child: const Text('Open')),
                  const SizedBox(width: 8),
                ],
                TextButton(
                  onPressed: onStop,
                  child: Text(viewing ? 'Stop showing' : 'Stop sharing'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// What the person is told of a window that is over, or that did not begin.
class _NoteCard extends StatelessWidget {
  const _NoteCard({required this.note, required this.onDismiss});

  final WindowNote note;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = describeWindowNote(note);
    return Card(
      child: ListTile(
        leading: Icon(
          text.problem ? Icons.error_outline : Icons.info_outline,
          color: text.problem ? scheme.error : scheme.primary,
        ),
        title: Text(text.title),
        subtitle: text.detail == null ? null : Text(text.detail!),
        isThreeLine: text.detail != null,
        trailing: IconButton(
          icon: const Icon(Icons.close),
          tooltip: 'Dismiss',
          onPressed: onDismiss,
        ),
      ),
    );
  }
}

/// What a note says: a line for the person, a sentence after it if there is more to say, and
/// whether it is a problem to be shown as one. The names and the words came from other devices,
/// so they are made fit to show.
({String title, String? detail, bool problem}) describeWindowNote(
  WindowNote note,
) {
  switch (note) {
    case WindowOver(:final session, :final end):
      final viewing = session.role == WindowRole.viewing;
      final device = visibleText(session.deviceName);
      final name = windowName(session.title, session.app);
      return switch (end) {
        WindowEnd_ThisDevice() => (
          title: viewing
              ? 'You stopped showing $name'
              : 'You stopped sharing $name',
          detail: null,
          problem: false,
        ),
        WindowEnd_OtherDevice() => (
          title: viewing
              ? '$device stopped sharing $name'
              : '$device stopped showing $name',
          detail: null,
          problem: false,
        ),
        WindowEnd_Lost() => (
          title: 'The connection to $device was lost',
          detail: viewing
              ? 'The window stopped. Ask $device to share it again.'
              : 'Sharing stopped. Share the window again when $device is back.',
          problem: true,
        ),
        WindowEnd_Failed(:final message) => (
          title: viewing
              ? 'The window from $device stopped'
              : 'Sharing with $device stopped',
          detail: sentence(visibleText(message)),
          problem: true,
        ),
      };
    case WindowWithdrawn(:final request):
      return (
        title:
            '${visibleText(request.deviceName)} is not offering that window any more',
        detail: null,
        problem: false,
      );
    case WindowNotOpened(:final request):
      final device = visibleText(request.deviceName);
      return (
        title: 'The window from $device did not open',
        detail: 'Ask $device to share it again.',
        problem: true,
      );
    case WindowFailed(:final message):
      return (
        title: sentence(visibleText(message)),
        detail: null,
        problem: true,
      );
  }
}
