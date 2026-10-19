// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';
import '../transfers_controller.dart';

/// The files that other devices send, at the bottom of the home screen where both places can
/// show it: the requests for the person to answer, the transfers that they said yes to, and how
/// the last ones ended. It takes no room when there is nothing to show.
class TransfersPanel extends StatelessWidget {
  const TransfersPanel({required this.controller, this.platform, super.key});

  final TransfersController controller;

  /// The system to say where files are put for. Without one, it is the system that runs the app.
  final TargetPlatform? platform;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) {
        final taking = controller.taking;
        final where = _where(controller.folder);
        final cards = <Widget>[
          if (taking is Receiving_Unusable) _Unusable(message: taking.message),
          for (final request in controller.requests)
            _RequestCard(
              request: request,
              where: where,
              onAnswer: (accept) => controller.answer(request, accept: accept),
            ),
          for (final request in controller.coming)
            _ComingCard(request: request),
          for (final end in controller.ended)
            _EndedCard(
              end: end,
              where: where,
              showLabel: describeShowFiles(platform ?? defaultTargetPlatform),
              onShow: controller.canShow(end)
                  ? () => unawaited(_show(context, end, where))
                  : null,
              onDismiss: () => controller.dismiss(end),
            ),
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

  String? _where(String? folder) => folder == null
      ? null
      : describeReceiveFolder(folder, platform ?? defaultTargetPlatform);

  /// Takes the person to what arrived with [end], and says so when that did not work: the files
  /// are the person's, so they can have been moved or deleted since.
  Future<void> _show(
    BuildContext context,
    TransferEnd end,
    String? where,
  ) async {
    final messenger = ScaffoldMessenger.of(context);
    final one = end.saved.length == 1;
    final problem = switch (await controller.show(end)) {
      ShowResult.shown => null,
      ShowResult.gone =>
        one
            ? 'Baton cannot find it any more. It may have been moved or deleted.'
            : 'Baton cannot find them any more. They may have been moved or deleted.',
      ShowResult.failed =>
        'Baton could not show ${one ? 'it' : 'them'}. '
            '${one ? 'It is' : 'They are'} in ${where ?? 'the folder for received files'}.',
    };
    if (problem != null) {
      messenger
        ..hideCurrentSnackBar()
        ..showSnackBar(SnackBar(content: Text(problem)));
    }
  }
}

/// A device asks to send files. The person has two minutes to say yes or no.
class _RequestCard extends StatelessWidget {
  const _RequestCard({
    required this.request,
    required this.where,
    required this.onAnswer,
  });

  final TransferRequest request;
  final String? where;
  final void Function(bool accept) onAnswer;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final more = request.items - request.preview.length;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Semantics(
              liveRegion: true,
              child: Text(
                '${visibleText(request.deviceName)} wants to send you '
                '${_what(request)}',
                style: theme.textTheme.titleMedium,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              describeContents(
                files: request.files,
                folders: request.folders,
                bytes: request.bytes.toInt(),
              ),
              style: theme.textTheme.bodyMedium,
            ),
            const SizedBox(height: 8),
            for (final item in request.preview) _Item(item: item),
            if (more > 0)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Text('and $more more', style: theme.textTheme.bodySmall),
              ),
            if (where != null) ...[
              const SizedBox(height: 8),
              Text('They go in $where.', style: theme.textTheme.bodySmall),
            ],
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
                  child: const Text('Accept'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  static String _what(TransferRequest request) {
    if (request.folders == 0) {
      return request.files == 1 ? 'a file' : fileCount(request.files);
    }
    return '${fileCount(request.files)} in ${folderCount(request.folders)}';
  }
}

/// One of the things that a device offers: a file, or a folder with files in it.
class _Item extends StatelessWidget {
  const _Item({required this.item});

  final PreviewName item;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final size = item.folder
        ? '${fileCount(item.files)}, ${fileSize(item.bytes.toInt())}'
        : fileSize(item.bytes.toInt());
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 2),
      child: Row(
        children: [
          Icon(
            item.folder
                ? Icons.folder_outlined
                : Icons.insert_drive_file_outlined,
            size: 18,
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              visibleText(item.name),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const SizedBox(width: 8),
          Text(size, style: theme.textTheme.bodySmall),
        ],
      ),
    );
  }
}

/// A transfer that the person said yes to and that is on its way. Nothing says how far it has
/// got, so the bar does not fill.
class _ComingCard extends StatelessWidget {
  const _ComingCard({required this.request});

  final TransferRequest request;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Semantics(
              liveRegion: true,
              child: Text(
                'Receiving from ${visibleText(request.deviceName)}…',
                style: theme.textTheme.titleSmall,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              '${describeContents(files: request.files, folders: request.folders, bytes: request.bytes.toInt())}. '
              'Keep Baton open until it is done.',
              style: theme.textTheme.bodySmall,
            ),
            const SizedBox(height: 12),
            const LinearProgressIndicator(semanticsLabel: 'Receiving files'),
          ],
        ),
      ),
    );
  }
}

/// How a transfer that the person said yes to ended.
class _EndedCard extends StatelessWidget {
  const _EndedCard({
    required this.end,
    required this.where,
    required this.showLabel,
    required this.onShow,
    required this.onDismiss,
  });

  final TransferEnd end;
  final String? where;

  /// What the button that takes the person to the files says.
  final String showLabel;

  /// Takes the person to the files that arrived, or `null` when none did.
  final VoidCallback? onShow;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final device = visibleText(end.deviceName);
    final place = where ?? 'the folder for received files';
    final (ok, title, detail) = switch (end.result) {
      TransferResult_Received(:final failed) when failed == 0 => (
        true,
        'Received ${fileCount(end.files)} from $device',
        'They are in $place.',
      ),
      TransferResult_Received(:final failed) when failed < end.files => (
        false,
        'Received ${end.files - failed} of ${fileCount(end.files)} from $device',
        '${fileCount(failed)} could not be saved. The others are in $place.',
      ),
      TransferResult_Received() => (
        false,
        'Nothing from $device could be saved',
        'Baton could not save ${end.files == 1 ? 'the file' : 'the files'}. '
            'Ask $device to send ${end.files == 1 ? 'it' : 'them'} again.',
      ),
      TransferResult_Dropped() => (
        false,
        'The transfer from $device stopped',
        'Nothing was kept. $device stopped sending, or the connection was '
            'lost.',
      ),
    };
    return Card(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: Icon(
              ok ? Icons.check_circle_outline : Icons.error_outline,
              color: ok ? scheme.primary : scheme.error,
            ),
            title: Text(title),
            subtitle: Text(detail),
            isThreeLine: true,
            trailing: IconButton(
              icon: const Icon(Icons.close),
              tooltip: 'Dismiss',
              onPressed: onDismiss,
            ),
          ),
          if (onShow != null)
            Padding(
              padding: const EdgeInsets.only(right: 16, bottom: 8),
              child: Align(
                alignment: Alignment.centerRight,
                child: TextButton.icon(
                  onPressed: onShow,
                  icon: const Icon(Icons.folder_open_outlined),
                  label: Text(showLabel),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// This device cannot take files, and why.
class _Unusable extends StatelessWidget {
  const _Unusable({required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Card(
      child: ListTile(
        leading: Icon(Icons.error_outline, color: scheme.error),
        title: const Text('This device cannot take files'),
        subtitle: Text(message),
      ),
    );
  }
}
