// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../awake.dart';
import '../backend.dart';
import '../control_keyboard.dart';
import '../format.dart';
import '../window_control.dart';
import '../window_sound.dart';
import '../windows_controller.dart';
import 'windows_panel.dart';

/// A window of another device, drawn live. The picture comes from the platform: the session gives
/// the number of a texture, and the platform draws into it as the pictures arrive.
///
/// The window is looked at, and zoomed with two fingers. If the other device let this one control
/// it, the person can turn control on: their fingers and keys then act on the window, and the
/// picture is not zoomed until they turn it off. The keys of an attached keyboard are taken by the
/// focus scope of the screen, so that a key which the window takes does not also move the focus,
/// press a button or type into a field of the app. Leaving the screen ends the window, after
/// asking: it would have to be shared again. A window that ends by itself leaves the screen in
/// place and says how, so that the person is not left wondering what happened.
///
/// A screen that this device controls and that the other device keeps private (dark, with its
/// keyboard and trackpad out of use) has settings for the session, which the person opens from the
/// bar: two switches, to show the screen at the other device and to let its keyboard and trackpad
/// be used, each on and off as often as they like. Both are off when the session starts.
///
/// A whole screen whose device can send its sound has a speaker in the bar, which turns the sound
/// on and off and shows how it stands. When the other device says that it sends none, a line over
/// the picture says why, until the person tries again or turns the sound off.
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

  /// The scope of the whole screen, which takes the keys of a keyboard that is attached to this
  /// device while the window is controlled.
  late final FocusScopeNode _scope = FocusScopeNode(
    debugLabel: 'The window that is controlled',
    onKeyEvent: _key,
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
    _scope.dispose();
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

  /// A key of a keyboard that is attached to this device. The focus tree gives it to every
  /// widget between the focus and the root until one takes it, and the shortcuts of the app, which
  /// move the focus with Tab and press the button that has it with Return and Space, sit above this
  /// scope: a key that the window takes stops here, and so does not also act on the app.
  KeyEventResult _key(FocusNode node, KeyEvent event) =>
      _control.key(event) ? KeyEventResult.handled : KeyEventResult.ignored;

  void _setControl(bool on) {
    if (on) {
      _control.turnOn();
      // A key goes through the scope only while the focus is in it, and the focus lies on the
      // route around the screen, above the scope, until it is given to the scope.
      _scope.requestFocus();
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

  /// Opens the settings for the session, over the picture: the touches of the person then go to
  /// the switches and not to the other device.
  Future<void> _showSettings() => showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    isScrollControlled: true,
    builder: (context) => _SessionSettings(
      controller: widget.controller,
      session: widget.session,
    ),
  );

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
    return FocusScope.withExternalFocusNode(
      focusScopeNode: _scope,
      child: _screen(context),
    );
  }

  Widget _screen(BuildContext context) {
    final session = widget.session;
    return ListenableBuilder(
      listenable: widget.controller,
      builder: (context, _) {
        final running = _running;
        // Why the other device sends no sound, while the person wants it and it says so.
        final sound = widget.controller.soundOf(session);
        final silence = running && sound.stage == SoundStage.silent
            ? describeSilence(
                sound.why ?? WindowSound.other,
                session.deviceName,
              )
            : null;
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
                if (running && session.sound)
                  _SoundButton(controller: widget.controller, session: session),
                if (running && session.private)
                  Builder(
                    builder: (context) {
                      final lifted = widget.controller.liftedOf(session);
                      final anything = lifted.screen || lifted.input;
                      return IconButton(
                        icon: Badge(
                          isLabelVisible: anything,
                          smallSize: 8,
                          child: const Icon(Icons.settings_outlined),
                        ),
                        tooltip: anything
                            ? 'Settings for this session, '
                                  'something is switched on'
                            : 'Settings for this session',
                        onPressed: () => unawaited(_showSettings()),
                      );
                    },
                  ),
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
                            ? 'Stop controlling the '
                                  '${windowNoun(display: session.display)}'
                            : 'Control the '
                                  '${windowNoun(display: session.display)} '
                                  'from this device',
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
                                if (_control.notice != null || silence != null)
                                  Positioned(
                                    left: 16,
                                    right: 16,
                                    top: 8,
                                    child: Column(
                                      mainAxisSize: MainAxisSize.min,
                                      spacing: 8,
                                      children: [
                                        if (_control.notice != null)
                                          _Notice(text: _control.notice!),
                                        if (silence != null)
                                          _Notice(text: silence),
                                      ],
                                    ),
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

/// The speaker in the bar of a screen that can be heard: it turns the sound on and off, and shows
/// how the sound stands. When the other device sends none, it asks again.
class _SoundButton extends StatelessWidget {
  const _SoundButton({required this.controller, required this.session});

  final WindowsController controller;
  final WindowSession session;

  @override
  Widget build(BuildContext context) {
    final stage = controller.soundOf(session).stage;
    final name = visibleText(session.deviceName);
    final wanted = stage == SoundStage.asking || stage == SoundStage.playing;
    return IconButton(
      isSelected: stage == SoundStage.playing,
      icon: Icon(switch (stage) {
        SoundStage.off => Icons.volume_off_outlined,
        SoundStage.asking => Icons.volume_up_outlined,
        SoundStage.playing => Icons.volume_up,
        SoundStage.silent => Icons.volume_off,
      }),
      selectedIcon: const Icon(Icons.volume_up),
      color: stage == SoundStage.silent
          ? Theme.of(context).colorScheme.error
          : null,
      tooltip: switch (stage) {
        SoundStage.off => 'Hear the sound of $name',
        SoundStage.asking => 'Waiting for the sound of $name, tap to stop',
        SoundStage.playing => 'Turn off the sound of $name',
        SoundStage.silent => 'No sound from $name, tap to try again',
      },
      onPressed: () {
        if (controller.hear(session, listen: !wanted)) return;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(
              'Baton could not ask $name for its sound. Try again.',
            ),
          ),
        );
      },
    );
  }
}

/// The settings for a session of a screen that the other device keeps private: what the people at
/// that device may do while this one controls it. Only a Mac keeps its screen private for now
/// (docs/adr/0011-privacy-mode.md), so the switches name it.
///
/// Each switch tells the other device at once, and shows how things stand only once it was told:
/// when the other device could not be told, the switch stays as it was and the sheet says so.
class _SessionSettings extends StatefulWidget {
  const _SessionSettings({required this.controller, required this.session});

  final WindowsController controller;
  final WindowSession session;

  @override
  State<_SessionSettings> createState() => _SessionSettingsState();
}

class _SessionSettingsState extends State<_SessionSettings> {
  /// What to tell the person after a switch that was not taken, or `null`.
  String? _problem;

  void _switch({bool? screen, bool? input}) {
    final told = widget.controller.lift(
      widget.session,
      screen: screen,
      input: input,
    );
    setState(() {
      _problem = told
          ? null
          : 'The Mac did not get that, so it is as it was. Try again.';
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SafeArea(
      child: ListenableBuilder(
        listenable: widget.controller,
        builder: (context, _) {
          final lifted = widget.controller.liftedOf(widget.session);
          final running = widget.controller.sessions.any(
            (s) => s.id == widget.session.id,
          );
          return SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 24),
                  child: Text(
                    'Settings for this session',
                    style: theme.textTheme.titleMedium,
                  ),
                ),
                SwitchListTile(
                  title: const Text('Show screen on the Mac'),
                  subtitle: Text(
                    lifted.screen
                        ? 'Anyone near the Mac can see what you do on it.'
                        : 'The Mac’s screen is dark, so nobody near it sees '
                              'what you do.',
                  ),
                  value: lifted.screen,
                  onChanged: running ? (on) => _switch(screen: on) : null,
                ),
                SwitchListTile(
                  title: const Text('Allow the Mac’s keyboard and trackpad'),
                  subtitle: Text(
                    lifted.input
                        ? 'Anyone at the Mac can use it.'
                        : 'Nobody at the Mac can use it.',
                  ),
                  value: lifted.input,
                  onChanged: running ? (on) => _switch(input: on) : null,
                ),
                Padding(
                  padding: const EdgeInsets.fromLTRB(24, 8, 24, 16),
                  child: Text(
                    running
                        ? 'Both switch off again when this session ends, '
                              'and the Mac locks.'
                        : 'This session is over.',
                    style: theme.textTheme.bodySmall,
                  ),
                ),
                if (_problem != null)
                  Padding(
                    padding: const EdgeInsets.fromLTRB(24, 0, 24, 16),
                    child: Semantics(
                      liveRegion: true,
                      child: Text(
                        _problem!,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: theme.colorScheme.error,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          );
        },
      ),
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
    if (screen != null && width <= 0 && height <= 0 && session.display) {
      // A screen that this device asked for has no size until the first picture comes.
      return _Message(
        title: 'Waiting for the screen of ${visibleText(session.deviceName)}',
        detail:
            'A device that is locked or asleep shows nothing. If this goes on, '
            'wake and unlock it.',
        busy: true,
      );
    }
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
  const _Message({
    required this.title,
    this.detail,
    this.onClose,
    this.busy = false,
  });

  final String title;
  final String? detail;
  final VoidCallback? onClose;

  /// Whether the message is about something that is going on, and so shows a bar that moves.
  final bool busy;

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
            if (busy) ...[
              const SizedBox(height: 16),
              SizedBox(
                width: 200,
                child: LinearProgressIndicator(semanticsLabel: title),
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
