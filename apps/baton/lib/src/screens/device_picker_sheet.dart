// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart';

import '../backend.dart';
import '../format.dart';

/// Asks the person which of their paired [devices] to send to, in a sheet over the screen, and
/// gives the one they tap, or `null` if they close it. [now] is what "seen 5 minutes ago" is
/// measured from.
Future<Device?> pickDevice(
  BuildContext context, {
  required List<Device> devices,
  required DateTime Function() now,
}) => showModalBottomSheet<Device>(
  context: context,
  isScrollControlled: true,
  showDragHandle: true,
  builder: (context) => DevicePickerSheet(devices: devices, now: now),
);

/// The paired devices, one to tap to send to.
class DevicePickerSheet extends StatelessWidget {
  const DevicePickerSheet({
    required this.devices,
    this.now = DateTime.now,
    super.key,
  });

  final List<Device> devices;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Send to which device?', style: theme.textTheme.titleLarge),
            const SizedBox(height: 12),
            for (final device in devices)
              Card(
                margin: const EdgeInsets.only(bottom: 8),
                child: ListTile(
                  leading: Icon(platformIcon(device.platform), size: 32),
                  title: Text(visibleText(device.name)),
                  subtitle: Text(
                    '${platformName(device.platform)} · '
                    '${lastSeen(device.lastSeenAt?.toInt(), now())}',
                  ),
                  onTap: () => Navigator.of(context).pop(device),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
