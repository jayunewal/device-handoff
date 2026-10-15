// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../backend.dart';
import '../controller.dart';
import '../format.dart';
import 'content_box.dart';

/// The home screen: this device, and the devices it is paired with.
class DevicesScreen extends StatefulWidget {
  const DevicesScreen({
    required this.session,
    this.now = DateTime.now,
    super.key,
  });

  final Session session;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

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

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Baton')),
      body: SafeArea(
        child: ContentBox(
          child: ListenableBuilder(
            listenable: _controller,
            builder: (context, _) =>
                _Devices(controller: _controller, now: widget.now()),
          ),
        ),
      ),
    );
  }
}

class _Devices extends StatelessWidget {
  const _Devices({required this.controller, required this.now});

  final DevicesController controller;
  final DateTime now;

  @override
  Widget build(BuildContext context) {
    final devices = controller.devices;
    return ListView(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 32),
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
        else
          for (final device in devices) _DeviceTile(device: device, now: now),
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
            'Devices you pair with this one will show up here.',
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyMedium,
          ),
        ],
      ),
    );
  }
}

class _DeviceTile extends StatelessWidget {
  const _DeviceTile({required this.device, required this.now});

  final Device device;
  final DateTime now;

  @override
  Widget build(BuildContext context) {
    final seen = lastSeen(device.lastSeenAt?.toInt(), now);
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: ListTile(
        leading: Icon(platformIcon(device.platform), size: 32),
        title: Text(device.name),
        subtitle: Text('${platformName(device.platform)} · $seen'),
      ),
    );
  }
}
