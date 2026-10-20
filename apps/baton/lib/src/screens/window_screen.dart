// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../awake.dart';
import '../backend.dart';
import '../control_keyboard.dart';
import '../format.dart';
import '../window_control.dart';
import '../windows_controller.dart';
import 'windows_panel.dart';

/// A window of another device, drawn live. The picture comes from the platform: the session gives
/// the number of a texture, and the platform draws into it as the pictures arrive.
///
/// The window is looked at, and zoomed with two fingers. If the other device let this one control
/// it, the person can turn control on: their fingers and keys then act on the window, and the
/// picture is not zoomed until they turn it off. Leaving the screen ends the window, after
/// asking: it would have to be shared again. A window that ends by itself leaves the screen in
/// place and says how, so that the person is not left wondering what happened.
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
  late final WindowControl _control = WindowControl(
    windows: widget.controller,
    session: widget.session,
    pictureSize: _pictureSize,
  );
  final FocusNode _keyboard = FocusNode(
    debugLabel: 'The keyboard for the window',
  );

  /// Names the picture, to ask how large it is.
  final GlobalKey _picture = GlobalKey();
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
    _control.dispose();
    _keyboard.dispose();
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

  /// The window ended, so the screen can turn off again, and nothing is controlled.
  void _changed() {
    if (_running) return;
    _setControl(false);
    _letGo();
  }

  /// How large the picture is on the screen, or `null` before it is laid out.
  Size? _pictureSize() {
    final box = _picture.currentContext?.findRenderObject();
    if (box is! RenderBox || !box.hasSize || box.size.isEmpty) return null;
    return box.size;
  }

  void _setControl(bool on) {
    if (on) {
      _control.turnOn();
    } else {
      _keyboard.unfocus();
      _control.turnOff();
    }
  }

  void _toggleKeyboard() {
    if (_keyboard.hasFocus) {
      _keyboard.unfocus();
    } else {
      _keyboard.requestFocus();
    }
  }

  /// Asks before the window is ended: the other device has to share it again to bring it back.
  Future<void> _confirmStop() async {
    final stop = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(
          'Stop showing this '
          '${windowNoun(display: widget.session.display)}?',
        ),
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
                    windowName(
                      session.title,
                      session.app,
                      display: session.display,
                    ),
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
              actions: [
                if (running && session.control)
                  ListenableBuilder(
                    listenable: _control,
                    builder: (context, _) => Padding(
                      padding: const EdgeInsets.only(right: 8),
                      child: FilterChip(
                        avatar: const Icon(Icons.touch_app_outlined),
                        label: const Text('Control'),
                        showCheckmark: false,
                        selected: _control.on,
                        tooltip: _control.on
                            ? 'Stop controlling the window'
                            : 'Control the window from this device',
                        onSelected: _setControl,
                      ),
                    ),
                  ),
              ],
            ),
            body: SafeArea(
              child: running
                  ? ListenableBuilder(
                      listenable: Listenable.merge([_control, _keyboard]),
                      builder: (context, _) => Column(
                        children: [
                          Expanded(
                            child: Stack(
                              fit: StackFit.expand,
                              children: [
                                _Picture(
                                  session: session,
                                  size: widget.controller.pictureOf(session),
                                  control: _control,
                                  pictureKey: _picture,
                                ),
                                if (_control.on)
                                  Positioned(
                                    left: 0,
                                    bottom: 0,
                                    child: SoftKeyboardField(
                                      focusNode: _keyboard,
                                      onTyped: _control.type,
                                    ),
                                  ),
                                if (_control.notice != null)
                                  Positioned(
                                    left: 16,
                                    right: 16,
                                    top: 8,
                                    child: _Notice(text: _control.notice!),
                                  ),
                              ],
                            ),
                          ),
                          if (_control.on)
                            ControlKeys(
                              chosen: _control.chosen,
                              onKey: _control.tapKey,
                              keyboardShown: _keyboard.hasFocus,
                              onKeyboard: _toggleKeyboard,
                            ),
                        ],
                      ),
                    )
                  : _Over(
                      session: session,
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
/// with two fingers so that small writing can be read. While the window is controlled the fingers
/// are the window's, and the picture stays as it is.
///
/// The shape is that of the picture, which the source says and can change: it scales the window to
/// what this device takes, and a window that is resized changes shape. The shape of the window as
/// it was offered is used until the source has said.
class _Picture extends StatelessWidget {
  const _Picture({
    required this.session,
    required this.size,
    required this.control,
    required this.pictureKey,
  });

  final WindowSession session;

  /// How large the picture is, or `null` before the source has said.
  final PictureSize? size;

  /// What the fingers do on the picture.
  final WindowControl control;

  /// Names the picture itself, to ask how large it is on the screen.
  final GlobalKey pictureKey;

  @override
  Widget build(BuildContext context) {
    final screen = session.screen;
    final width = size?.width ?? session.width;
    final height = size?.height ?? session.height;
    if (screen == null || width <= 0 || height <= 0) {
      return _Message(
        title:
            'This ${windowNoun(display: session.display)} cannot be shown here',
        detail: 'Baton did not get a screen to draw it on.',
      );
    }
    return InteractiveViewer(
      minScale: 1,
      maxScale: 6,
      panEnabled: !control.on,
      scaleEnabled: !control.on,
      child: SizedBox.expand(
        child: Center(
          child: AspectRatio(
            aspectRatio: width / height,
            // The fingers are told to the controller whether it is on or not, and it takes them
            // only while it is: the picture is the same in both, and is not made again when the
            // person turns control on.
            child: Listener(
              key: pictureKey,
              behavior: HitTestBehavior.opaque,
              onPointerDown: control.pointerDown,
              onPointerMove: control.pointerMove,
              onPointerUp: control.pointerUp,
              onPointerCancel: control.pointerCancel,
              child: Semantics(
                label: session.display
                    ? 'The screen from ${visibleText(session.deviceName)}'
                    : 'The window ${windowName(session.title, session.app)} '
                          'from ${visibleText(session.deviceName)}',
                image: true,
                child: Texture(
                  textureId: screen.toInt(),
                  filterQuality: FilterQuality.medium,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// What the other device did not do of what the person did, over the top of the picture. It lets
/// the touches through to the picture.
class _Notice extends StatelessWidget {
  const _Notice({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return IgnorePointer(
      child: Semantics(
        liveRegion: true,
        child: Material(
          color: scheme.inverseSurface,
          borderRadius: BorderRadius.circular(8),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            child: Text(
              text,
              textAlign: TextAlign.center,
              style: TextStyle(color: scheme.onInverseSurface),
            ),
          ),
        ),
      ),
    );
  }
}

/// A window that ended while the screen was open.
class _Over extends StatelessWidget {
  const _Over({
    required this.session,
    required this.over,
    required this.onClose,
  });

  final WindowSession session;
  final WindowOver? over;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final text = over == null ? null : describeWindowNote(over!);
    return _Message(
      title:
          text?.title ??
          'The ${windowNoun(display: session.display)} is closed',
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
