// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../backend.dart';
import '../control_access.dart';
import '../controller.dart';
import '../file_chooser.dart';
import '../format.dart';
import '../scanner.dart';
import 'add_device_screen.dart';
import 'content_box.dart';
import 'diagnostics_screen.dart';
import 'send_screen.dart';
import 'start_access_dialog.dart';

/// The home screen: this device, and the devices it is paired with.
class DevicesScreen extends StatefulWidget {
  const DevicesScreen({
    required this.session,
    this.now = DateTime.now,
    this.scanner = cameraScanner,
    this.chooser = const SystemFileChooser(),
    this.onShareWindow,
    this.onStartScreen,
    this.canGiveScreen = false,
    this.controlAccess = const PlatformControlAccess(),
    this.screenAccess = const PlatformScreenAccess(),
    super.key,
  });

  final Session session;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// Reads QR codes with the camera, for pairing. Tests give it one of their own.
  final ScannerBuilder scanner;

  /// Lets the person choose files and folders to send. Tests give it one of their own.
  final FileChooser chooser;

  /// Starts sharing a window or a screen with a device, to be looked at or, with [control], to be
  /// controlled as well, or `null` on a device that does not share windows, which then offers no
  /// button for it.
  final void Function(Device device, {required bool control})? onShareWindow;

  /// Asks a device to start sharing its screen with this one, to be looked at or, with [control],
  /// to be controlled as well, or `null` on a device that shows no screens, which then offers no
  /// button for it. Only a device that gives its screen when it is asked has the button.
  final void Function(Device device, {required bool control})? onStartScreen;

  /// Whether this device can give its screen to a device that asks for it, so that the menu of
  /// each device lets the person say which may.
  final bool canGiveScreen;

  /// What the system says about letting another device control a window or a screen of this one.
  /// Tests give it one of their own.
  final ControlAccessKeeper controlAccess;

  /// What the system says about letting Baton record the screen of this device, which a device
  /// that asks for the screen needs. Tests give it one of their own.
  final ScreenAccessKeeper screenAccess;

  @override
  State<DevicesScreen> createState() => _DevicesScreenState();
}

/// What the menu at the top of the screen offers.
enum _ScreenAction { diagnostics }

class _DevicesScreenState extends State<DevicesScreen>
    with WidgetsBindingObserver {
  late final DevicesController _controller = DevicesController(widget.session);

  /// What the system says about controlling windows of this device. A device that shares no
  /// window is not asked.
  ControlAccess _access = ControlAccess.unavailable;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(_controller.start());
    unawaited(_askAccess());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _controller.dispose();
    super.dispose();
  }

  /// The person may have said yes in the settings of the system, which they leave the app for.
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) unawaited(_askAccess());
  }

  Future<void> _askAccess() async {
    if (widget.onShareWindow == null && !widget.canGiveScreen) return;
    final access = await widget.controlAccess.state();
    if (mounted && access != _access) setState(() => _access = access);
  }

  /// Shares a window or a screen with [device]. What is to be controlled needs the consent of the
  /// system, which is asked about now: it may have been given or taken back since the screen was
  /// shown.
  Future<void> _share(Device device, {required bool control}) async {
    final share = widget.onShareWindow;
    if (share == null) return;
    if (!control) {
      share(device, control: false);
      return;
    }
    final access = await widget.controlAccess.state();
    if (!mounted) return;
    switch (access) {
      case ControlAccess.allowed:
        share(device, control: true);
      case ControlAccess.needed:
        await widget.controlAccess.request();
        if (!mounted) return;
        _tell('${_allowControl(device)} Then share it again.');
      case ControlAccess.unavailable:
        _tell('Controlling a window or a screen needs macOS 15.2 or later.');
    }
  }

  /// What the person does in the settings of the system to let [device] control a window or a
  /// screen of this device.
  String _allowControl(Device device) =>
      'To let ${visibleText(device.name)} control a window or a screen, allow '
      'Baton to control this computer in System Settings, under Privacy & '
      'Security, Accessibility. If Baton is in the list already, take it out '
      'and add it again.';

  /// What the person does in the settings of the system to let [device] ask this device for its
  /// screen. The system takes the answer into effect only when Baton is opened again.
  String _allowRecording(Device device) =>
      'To let ${visibleText(device.name)} ask for the screen, allow Baton to '
      'record it in System Settings, under Privacy & Security, Screen & System '
      'Audio Recording. Then quit Baton and open it again.';

  /// Lets the person say whether [device] may ask this device for its screen, with no one here
  /// asked each time. To show the screen needs the consent of the system to record it, and to let
  /// the device control it needs the consent to control. Both are asked about now, once the
  /// setting is kept, since nobody is here to say yes when the device asks.
  Future<void> _allowScreen(Device device) async {
    final current = _controller.startAccess(device);
    if (current == null) return;
    final chosen = await showDialog<StartAccess>(
      context: context,
      builder: (context) => StartAccessDialog(
        device: device,
        thisDevice: _controller.me.platform,
        current: current,
        canControl: _access != ControlAccess.unavailable,
      ),
    );
    if (chosen == null || !mounted) return;
    final problem = await _controller.setStartAccess(device, chosen);
    if (!mounted) return;
    if (problem != null) {
      _tell(problem);
      return;
    }
    if (chosen == StartAccess.off) return;
    final sentences = <String>[];
    if (await widget.screenAccess.state() == ScreenAccess.needed) {
      await widget.screenAccess.request();
      sentences.add(_allowRecording(device));
    }
    if (chosen == StartAccess.control &&
        await widget.controlAccess.state() == ControlAccess.needed) {
      await widget.controlAccess.request();
      sentences.add(_allowControl(device));
    }
    if (mounted && sentences.isNotEmpty) _tell(sentences.join(' '));
  }

  void _tell(String sentence) {
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(sentence)));
  }

  void _add() {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => AddDeviceScreen(
          session: widget.session,
          now: widget.now,
          scanner: widget.scanner,
        ),
      ),
    );
  }

  void _diagnostics() {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => DiagnosticsScreen(
          recentLog: widget.session.recentLog,
          session: widget.session,
          now: widget.now,
        ),
      ),
    );
  }

  void _send(Device device) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => SendScreen(
          session: widget.session,
          device: device,
          now: widget.now,
          chooser: widget.chooser,
        ),
      ),
    );
  }

  Future<void> _forget(Device device) async {
    final messenger = ScaffoldMessenger.of(context);
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Forget ${device.name}?'),
        content: const Text(
          'The two devices will no longer be able to send each other anything. '
          'You can pair them again later.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Forget'),
          ),
        ],
      ),
    );
    if (sure != true) return;
    final problem = await _controller.forget(device);
    if (problem != null) {
      messenger.showSnackBar(SnackBar(content: Text(problem)));
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text('Baton'),
        actions: [
          PopupMenuButton<_ScreenAction>(
            tooltip: 'More',
            onSelected: (_) => _diagnostics(),
            itemBuilder: (context) => const [
              PopupMenuItem(
                value: _ScreenAction.diagnostics,
                child: Text('Diagnostics'),
              ),
            ],
          ),
        ],
      ),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: _add,
        icon: const Icon(Icons.add),
        label: const Text('Add a device'),
      ),
      body: SafeArea(
        child: ContentBox(
          child: ListenableBuilder(
            listenable: _controller,
            builder: (context, _) => _Devices(
              controller: _controller,
              now: widget.now(),
              onSend: _send,
              onShareWindow: widget.onShareWindow == null ? null : _share,
              onStartScreen: widget.onStartScreen,
              canControl: _access != ControlAccess.unavailable,
              onAllowScreen: widget.canGiveScreen ? _allowScreen : null,
              onForget: _forget,
            ),
          ),
        ),
      ),
    );
  }
}

class _Devices extends StatelessWidget {
  const _Devices({
    required this.controller,
    required this.now,
    required this.onSend,
    required this.onShareWindow,
    required this.onStartScreen,
    required this.canControl,
    required this.onAllowScreen,
    required this.onForget,
  });

  final DevicesController controller;
  final DateTime now;
  final void Function(Device device) onSend;
  final void Function(Device device, {required bool control})? onShareWindow;
  final void Function(Device device, {required bool control})? onStartScreen;

  /// Whether a window can be shared to be controlled, now or after the person has said yes to
  /// the system.
  final bool canControl;

  /// Lets the person say whether a device may ask for the screen of this one, or `null` on a
  /// device that cannot give its screen.
  final void Function(Device device)? onAllowScreen;
  final void Function(Device device) onForget;

  @override
  Widget build(BuildContext context) {
    final devices = controller.devices;
    return ListView(
      // Room under the last device for the button that floats above it.
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 96),
      children: [
        _ThisDevice(me: controller.me, network: controller.network),
        const SizedBox(height: 24),
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: Text(
            'Your devices',
            style: Theme.of(context).textTheme.titleMedium,
          ),
        ),
        const SizedBox(height: 8),
        if (devices == null)
          const Padding(
            padding: EdgeInsets.all(32),
            child: Center(child: CircularProgressIndicator()),
          )
        else if (devices.isEmpty)
          const _NoDevices()
        else ...[
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 0, 8, 12),
            child: Text(
              'Tap a device to send it a link or some text.',
              style: Theme.of(context).textTheme.bodyMedium,
            ),
          ),
          for (final device in devices)
            _DeviceTile(
              device: device,
              now: now,
              onSend: onSend,
              onShareWindow: onShareWindow,
              onStartScreen: onStartScreen,
              canControl: canControl,
              onAllowScreen: onAllowScreen,
              onForget: onForget,
            ),
        ],
      ],
    );
  }
}

/// The card at the top: what this device is called, and whether others can reach it.
class _ThisDevice extends StatelessWidget {
  const _ThisDevice({required this.me, required this.network});

  final Me me;
  final Network? network;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final online = network?.relay != null;
    return Card(
      margin: EdgeInsets.zero,
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(platformIcon(me.platform), size: 40),
                const SizedBox(width: 16),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(me.name, style: theme.textTheme.titleLarge),
                      Text(
                        platformName(me.platform),
                        style: theme.textTheme.bodyMedium,
                      ),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            Row(
              children: [
                Icon(
                  online
                      ? Icons.cloud_done_outlined
                      : Icons.cloud_sync_outlined,
                  size: 18,
                  color: online
                      ? theme.colorScheme.primary
                      : theme.colorScheme.outline,
                ),
                const SizedBox(width: 8),
                Text(online ? 'Online' : 'Connecting…'),
              ],
            ),
            const SizedBox(height: 12),
            Text('Fingerprint', style: theme.textTheme.labelMedium),
            SelectableText(
              me.fingerprint,
              style: theme.textTheme.bodyMedium?.copyWith(
                fontFamily: 'Menlo',
                fontFamilyFallback: const ['Courier', 'monospace'],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _NoDevices extends StatelessWidget {
  const _NoDevices();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 32, horizontal: 16),
      child: Column(
        children: [
          Icon(
            Icons.devices_outlined,
            size: 48,
            color: theme.colorScheme.outline,
          ),
          const SizedBox(height: 12),
          Text('No other devices yet', style: theme.textTheme.titleMedium),
          const SizedBox(height: 4),
          Text(
            'Tap Add a device to pair this one with another. They will show '
            'up here.',
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyMedium,
          ),
        ],
      ),
    );
  }
}

/// What a device's menu offers.
enum _DeviceAction { allowScreen, forget }

class _DeviceTile extends StatelessWidget {
  const _DeviceTile({
    required this.device,
    required this.now,
    required this.onSend,
    required this.onShareWindow,
    required this.onStartScreen,
    required this.canControl,
    required this.onAllowScreen,
    required this.onForget,
  });

  final Device device;
  final DateTime now;
  final void Function(Device device) onSend;
  final void Function(Device device, {required bool control})? onShareWindow;
  final void Function(Device device, {required bool control})? onStartScreen;
  final bool canControl;
  final void Function(Device device)? onAllowScreen;
  final void Function(Device device) onForget;

  @override
  Widget build(BuildContext context) {
    final seen = lastSeen(device.lastSeenAt?.toInt(), now);
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: ListTile(
        leading: Icon(platformIcon(device.platform), size: 32),
        title: Text(device.name),
        subtitle: Text('${platformName(device.platform)} · $seen'),
        onTap: () => onSend(device),
        trailing: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            IconButton(
              tooltip: 'Send to ${device.name}',
              icon: const Icon(Icons.send_outlined),
              onPressed: () => onSend(device),
            ),
            if (onShareWindow != null && canControl)
              PopupMenuButton<bool>(
                tooltip: 'Share a window or screen with ${device.name}',
                icon: const Icon(Icons.screen_share_outlined),
                onSelected: (control) =>
                    onShareWindow!(device, control: control),
                itemBuilder: (context) => const [
                  PopupMenuItem(value: false, child: Text('Share to look at')),
                  PopupMenuItem(
                    value: true,
                    child: Text(
                      'Share a window or screen to look at and control',
                    ),
                  ),
                ],
              )
            else if (onShareWindow != null)
              IconButton(
                tooltip: 'Share a window or screen with ${device.name}',
                icon: const Icon(Icons.screen_share_outlined),
                onPressed: () => onShareWindow!(device, control: false),
              ),
            if (onStartScreen != null && givesScreenWhenAsked(device.platform))
              PopupMenuButton<bool>(
                tooltip: 'Look at the screen of ${device.name}',
                icon: const Icon(Icons.visibility_outlined),
                onSelected: (control) =>
                    onStartScreen!(device, control: control),
                itemBuilder: (context) => const [
                  PopupMenuItem(
                    value: false,
                    child: Text('Look at its screen'),
                  ),
                  PopupMenuItem(
                    value: true,
                    child: Text('Look at its screen and control it'),
                  ),
                ],
              ),
            PopupMenuButton<_DeviceAction>(
              tooltip: 'More about ${device.name}',
              onSelected: (action) {
                switch (action) {
                  case _DeviceAction.allowScreen:
                    onAllowScreen?.call(device);
                  case _DeviceAction.forget:
                    onForget(device);
                }
              },
              itemBuilder: (context) => [
                if (onAllowScreen != null)
                  const PopupMenuItem(
                    value: _DeviceAction.allowScreen,
                    child: Text('Let this device ask for the screen…'),
                  ),
                const PopupMenuItem(
                  value: _DeviceAction.forget,
                  child: Text('Forget this device'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
