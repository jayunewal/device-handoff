// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../awake.dart';
import '../backend.dart';
import '../format.dart';
import '../windows_controller.dart';
import 'windows_panel.dart';

/// A window of another device, drawn live. The picture comes from the platform: the session gives
/// the number of a texture, and the platform draws into it as the pictures arrive.
///
/// The window is only looked at, and leaving the screen ends it, after asking: it would have to be
/// shared again. A window that ends by itself leaves the screen in place and says how, so that the
/// person is not left wondering what happened.
///
/// The screen of a phone stays on for as long as the window shows. Nobody touches it while a
/// window is watched, so the hold is renewed every [renewEvery] and not left to run out.
class WindowScreen extends StatefulWidget {
  const WindowScreen({
    required this.controller,
    required this.session,
    this.screen = const PlatformScreenKeeper(),
    this.renewEvery = const Duration(minutes: 1),
    super.key,
  });

  final WindowsController controller;

  /// The window to show. It has to be one that this device shows, which has a screen.
  final WindowSession session;

  /// Keeps the screen of the device on. Tests give it one of their own.
  final ScreenKeeper screen;

  /// How often the hold on the screen is renewed.
  final Duration renewEvery;

  @override
  State<WindowScreen> createState() => _WindowScreenState();
}

class _WindowScreenState extends State<WindowScreen> {
  late final AwakeHold _awake = AwakeHold(widget.screen);
  Timer? _renew;

  bool get _running =>
      widget.controller.sessions.any((s) => s.id == widget.session.id);

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_changed);
    if (_running) _keepAwake();
  }

  @override
  void dispose() {
    widget.controller.removeListener(_changed);
    _letGo();
    super.dispose();
  }

  void _keepAwake() {
    _awake.hold();
    _renew = Timer.periodic(widget.renewEvery, (_) => _awake.renew());
  }

  void _letGo() {
    _renew?.cancel();
    _renew = null;
    _awake.release();
  }

  /// The window ended, so the screen can turn off again.
  void _changed() {
    if (!_running) _letGo();
  }

  /// Asks before the window is ended: the other device has to share it again to bring it back.
  Future<void> _confirmStop() async {
    final stop = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Stop showing this window?'),
        content: Text(
          '${visibleText(widget.session.deviceName)} has to share it again '
          'to show it here.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Keep showing'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Stop'),
          ),
        ],
      ),
    );
    if (stop != true || !mounted) return;
    widget.controller.end(widget.session);
    Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final session = widget.session;
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) {
        final running = _running;
        return PopScope(
          canPop: !running,
          onPopInvokedWithResult: (didPop, _) {
            if (!didPop) {
              unawaited(_confirmStop());
              return;
            }
            // The person has seen how it ended.
            final over = widget.controller.over(session);
            if (over != null) widget.controller.dismiss(over);
          },
          child: Scaffold(
            backgroundColor: Colors.black,
            appBar: AppBar(
              leading: IconButton(
                icon: const Icon(Icons.close),
                tooltip: running ? 'Stop showing' : 'Close',
                onPressed: () => unawaited(Navigator.of(context).maybePop()),
              ),
              title: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    windowName(session.title, session.app),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                  Text(
                    'from ${visibleText(session.deviceName)}',
                    style: Theme.of(context).textTheme.bodySmall,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                  ),
                ],
              ),
            ),
            body: SafeArea(
              child: running
                  ? _Picture(
                      session: session,
                      size: widget.controller.pictureOf(session),
                    )
                  : _Over(
                      over: widget.controller.over(session),
                      onClose: () =>
                          unawaited(Navigator.of(context).maybePop()),
                    ),
            ),
          ),
        );
      },
    );
  }
}

/// The picture of the window, as large as the screen allows with its shape kept, and zoomed
/// with two fingers so that small writing can be read.
///
/// The shape is that of the picture, which the source says and can change: it scales the window to
/// what this device takes, and a window that is resized changes shape. The shape of the window as
/// it was offered is used until the source has said.
class _Picture extends StatelessWidget {
  const _Picture({required this.session, required this.size});

  final WindowSession session;

  /// How large the picture is, or `null` before the source has said.
  final PictureSize? size;

  @override
  Widget build(BuildContext context) {
    final screen = session.screen;
    final width = size?.width ?? session.width;
    final height = size?.height ?? session.height;
    if (screen == null || width <= 0 || height <= 0) {
      return const _Message(
        title: 'This window cannot be shown here',
        detail: 'Baton did not get a screen to draw it on.',
      );
    }
    return InteractiveViewer(
      minScale: 1,
      maxScale: 6,
      child: SizedBox.expand(
        child: Center(
          child: AspectRatio(
            aspectRatio: width / height,
            child: Semantics(
              label:
                  'The window ${windowName(session.title, session.app)} from '
                  '${visibleText(session.deviceName)}',
              image: true,
              child: Texture(
                textureId: screen.toInt(),
                filterQuality: FilterQuality.medium,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A window that ended while the screen was open.
class _Over extends StatelessWidget {
  const _Over({required this.over, required this.onClose});

  final WindowOver? over;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final text = over == null ? null : describeWindowNote(over!);
    return _Message(
      title: text?.title ?? 'The window is closed',
      detail: text?.detail,
      onClose: onClose,
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.title, this.detail, this.onClose});

  final String title;
  final String? detail;
  final VoidCallback? onClose;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Semantics(
              liveRegion: true,
              child: Text(
                title,
                textAlign: TextAlign.center,
                style: theme.textTheme.titleMedium?.copyWith(
                  color: Colors.white,
                ),
              ),
            ),
            if (detail != null) ...[
              const SizedBox(height: 8),
              Text(
                detail!,
                textAlign: TextAlign.center,
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: Colors.white70,
                ),
              ),
            ],
            if (onClose != null) ...[
              const SizedBox(height: 16),
              FilledButton(onPressed: onClose, child: const Text('Close')),
            ],
          ],
        ),
      ),
    );
  }
}
