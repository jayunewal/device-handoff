// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../backend.dart';
import '../controller.dart';
import '../format.dart';
import '../scanner.dart';
import 'add_device_screen.dart';
import 'content_box.dart';
import 'send_screen.dart';

/// The home screen: this device, and the devices it is paired with.
class DevicesScreen extends StatefulWidget {
  const DevicesScreen({
    required this.session,
    this.now = DateTime.now,
    this.scanner = cameraScanner,
    super.key,
  });

  final Session session;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// Reads QR codes with the camera, for pairing. Tests give it one of their own.
  final ScannerBuilder scanner;

  @override
  State<DevicesScreen> createState() => _DevicesScreenState();
}

class _DevicesScreenState extends State<DevicesScreen> {
  late final DevicesController _controller = DevicesController(widget.session);

  @override
  void initState() {
    super.initState();
    unawaited(_controller.start());
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
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

  void _send(Device device) {
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => SendScreen(
          session: widget.session,
          device: device,
          now: widget.now,
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
      appBar: AppBar(title: const Text('Baton')),
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
    required this.onForget,
  });

  final DevicesController controller;
  final DateTime now;
  final void Function(Device device) onSend;
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
enum _DeviceAction { forget }

class _DeviceTile extends StatelessWidget {
  const _DeviceTile({
    required this.device,
    required this.now,
    required this.onSend,
    required this.onForget,
  });

  final Device device;
  final DateTime now;
  final void Function(Device device) onSend;
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
            PopupMenuButton<_DeviceAction>(
              tooltip: 'More about ${device.name}',
              onSelected: (_) => onForget(device),
              itemBuilder: (context) => const [
                PopupMenuItem(
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
