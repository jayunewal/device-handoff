// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../awake.dart';
import '../backend.dart';
import '../file_chooser.dart';
import '../files_controller.dart';
import '../format.dart';
import '../send_controller.dart';
import 'content_box.dart';

/// "Send to" one paired device: files and folders to choose, or a link or some text to type or
/// paste, and the buttons to send them.
class SendScreen extends StatefulWidget {
  const SendScreen({
    required this.session,
    required this.device,
    this.now = DateTime.now,
    this.chooser = const SystemFileChooser(),
    this.screen = const PlatformScreenKeeper(),
    super.key,
  });

  final Session session;

  /// The device that things are sent to.
  final Device device;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// Lets the person choose files and folders. Tests give it one of their own.
  final FileChooser chooser;

  /// Keeps the screen on while files are going. Tests give it one of their own.
  final ScreenKeeper screen;

  @override
  State<SendScreen> createState() => _SendScreenState();
}

class _SendScreenState extends State<SendScreen> {
  late final SendController _controller = SendController(
    widget.session,
    widget.device,
  );
  late final FilesController _files = FilesController(
    widget.session,
    widget.device,
    screen: widget.screen,
  );
  final TextEditingController _field = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    _files.dispose();
    _field.dispose();
    super.dispose();
  }

  /// Asks the person for what to send with [choose], and sends it.
  Future<void> _choose(Future<List<String>> Function() choose) async {
    final List<String> paths;
    try {
      paths = await choose();
    } on Object catch (e) {
      debugPrint('Baton: choosing files: $e');
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('Baton could not open the file chooser.')),
      );
      return;
    }
    if (!mounted || paths.isEmpty) return;
    await _files.send(paths);
  }

  Future<void> _chooseFolder() => _choose(() async {
    final folder = await widget.chooser.folder();
    return folder == null ? const [] : [folder];
  });

  /// Asks before leaving a screen that is sending, because leaving gives the transfer up.
  Future<void> _askBeforeLeaving() async {
    final stop = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Stop sending to ${widget.device.name}?'),
        content: const Text('What has not arrived yet will not be sent.'),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Keep sending'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Stop'),
          ),
        ],
      ),
    );
    if (stop != true || !mounted) return;
    _files.cancel();
    Navigator.of(context).pop();
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
    return ListenableBuilder(
      listenable: _files,
      builder: (context, child) => PopScope(
        canPop: !_files.busy,
        onPopInvokedWithResult: (didPop, _) {
          if (!didPop) unawaited(_askBeforeLeaving());
        },
        child: child!,
      ),
      child: Scaffold(
        appBar: AppBar(title: Text('Send to ${device.name}')),
        body: SafeArea(
          child: ContentBox(
            child: ListenableBuilder(
              listenable: Listenable.merge([_controller, _files, _field]),
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
                    _FilesSection(
                      files: _files,
                      chooser: widget.chooser,
                      device: device,
                      onChooseFiles: () =>
                          unawaited(_choose(widget.chooser.files)),
                      onChooseFolder: () => unawaited(_chooseFolder()),
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
                            label: Text(
                              link != null ? 'Send link' : 'Send text',
                            ),
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
                                child: CircularProgressIndicator(
                                  strokeWidth: 2,
                                ),
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

/// Choosing files and folders to send, and how the sending is going.
class _FilesSection extends StatelessWidget {
  const _FilesSection({
    required this.files,
    required this.chooser,
    required this.device,
    required this.onChooseFiles,
    required this.onChooseFolder,
  });

  final FilesController files;
  final FileChooser chooser;
  final Device device;
  final VoidCallback onChooseFiles;
  final VoidCallback onChooseFolder;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final outcome = files.outcome;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Files', style: theme.textTheme.titleMedium),
        const SizedBox(height: 4),
        Text(
          '${device.name} is asked before anything is kept. Baton has to be '
          'open there to be asked.',
          style: theme.textTheme.bodySmall,
        ),
        const SizedBox(height: 12),
        Wrap(
          spacing: 12,
          runSpacing: 8,
          children: [
            FilledButton.tonalIcon(
              onPressed: files.busy ? null : onChooseFiles,
              icon: const Icon(Icons.attach_file),
              label: const Text('Choose files'),
            ),
            if (chooser.canChooseFolders)
              OutlinedButton.icon(
                onPressed: files.busy ? null : onChooseFolder,
                icon: const Icon(Icons.folder_open),
                label: const Text('Choose a folder'),
              ),
          ],
        ),
        if (files.busy) ...[
          const SizedBox(height: 16),
          _FilesProgress(files: files, device: device),
        ],
        if (outcome != null) ...[
          const SizedBox(height: 16),
          _OutcomeLine(outcome: outcome),
        ],
      ],
    );
  }
}

/// What is being sent, a bar that fills as it goes, and a button to stop.
class _FilesProgress extends StatelessWidget {
  const _FilesProgress({required this.files, required this.device});

  final FilesController files;
  final Device device;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final progress = files.progress;
    final fraction = files.fraction;
    final stage = progress == null
        ? 'Getting ready…'
        : describeStage(progress.stage, device.name);
    return Card(
      margin: EdgeInsets.zero,
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              files.what,
              style: theme.textTheme.titleSmall,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: 12),
            LinearProgressIndicator(
              value: fraction,
              semanticsLabel: 'Sending to ${device.name}',
            ),
            const SizedBox(height: 12),
            Semantics(
              liveRegion: true,
              child: Text(stage, style: theme.textTheme.bodyMedium),
            ),
            if (progress != null && progress.bytes.toInt() > 0)
              Text(
                '${fileSize(progress.sent.toInt())} of '
                '${fileSize(progress.bytes.toInt())}'
                ' · ${fileCount(progress.files)}',
                style: theme.textTheme.bodySmall,
              ),
            Align(
              alignment: Alignment.centerRight,
              child: TextButton(
                onPressed: files.cancel,
                child: const Text('Stop sending'),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
