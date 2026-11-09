// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../control_access.dart';
import '../notice_access.dart';
import '../permissions.dart';
import '../theme.dart';
import 'content_box.dart';

/// Shows each permission Baton can ask the system for, whether it is on, and a button that turns
/// it on: one tap asks, when the system has not asked yet, and otherwise it opens the screen of
/// Settings where it is turned on. It looks again when the person comes back from Settings.
class PermissionsScreen extends StatefulWidget {
  const PermissionsScreen({
    this.platform,
    this.controlAccess = const PlatformControlAccess(),
    this.screenAccess = const PlatformScreenAccess(),
    this.notices = const PlatformNoticeKeeper(),
    this.system = const PlatformSystemSettings(),
    super.key,
  });

  /// The system this runs on. Tests give it one of their own.
  final TargetPlatform? platform;

  final ControlAccessKeeper controlAccess;
  final ScreenAccessKeeper screenAccess;
  final NoticeKeeper notices;
  final SystemSettingsKeeper system;

  @override
  State<PermissionsScreen> createState() => _PermissionsScreenState();
}

class _PermissionsScreenState extends State<PermissionsScreen>
    with WidgetsBindingObserver {
  List<PermissionCheck>? _checks;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(_look());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) unawaited(_look());
  }

  Future<void> _look() async {
    final checks = await checkPermissions(
      platform: widget.platform ?? defaultTargetPlatform,
      controlAccess: widget.controlAccess,
      screenAccess: widget.screenAccess,
      notices: widget.notices,
      system: widget.system,
    );
    if (mounted) setState(() => _checks = checks);
  }

  Future<void> _turnOn(PermissionCheck check) async {
    final permission = check.permission;
    switch (permission) {
      // Asking for these opens Settings on its own when the system has said no before.
      case Permission.control:
        await widget.controlAccess.request();
      case Permission.screenRecording:
        await widget.screenAccess.request();
      case Permission.notifications
          when check.state == PermissionState.notAsked:
        await widget.notices.request();
      case Permission.camera when check.state == PermissionState.notAsked:
        await widget.system.requestCamera();
      case Permission.faceId when check.state == PermissionState.notAsked:
        await widget.system.requestFaceId();
      default:
        final opened = await widget.system.open(permission);
        if (!opened && mounted) {
          ScaffoldMessenger.of(context)
            ..hideCurrentSnackBar()
            ..showSnackBar(
              SnackBar(content: Text(permissionText(permission).where)),
            );
        }
    }
    await _look();
  }

  @override
  Widget build(BuildContext context) {
    final checks = _checks;
    return Scaffold(
      appBar: AppBar(title: const Text('Permissions')),
      body: SafeArea(
        child: ContentBox(
          child: checks == null
              ? const Center(child: CircularProgressIndicator())
              : ListView(
                  padding: const EdgeInsets.all(BatonTheme.gap),
                  children: [
                    _Summary(checks: checks),
                    const SizedBox(height: BatonTheme.gapLarge),
                    for (final check in checks)
                      _PermissionRow(
                        check: check,
                        onTurnOn: () => unawaited(_turnOn(check)),
                      ),
                  ],
                ),
        ),
      ),
    );
  }
}

/// The words for a permission: its name as Settings shows it, what Baton uses it for, and where in
/// Settings it is, for when Settings cannot be opened.
({String title, String why, String where, String? whenOff}) permissionText(
  Permission permission,
) => switch (permission) {
  Permission.control => (
    title: 'Accessibility',
    why:
        'Lets a device you allow control this Mac with its touch, keyboard '
        'and trackpad.',
    where: 'Open System Settings, Privacy & Security, Accessibility.',
    whenOff: 'If Baton is in the list already, take it out and add it again.',
  ),
  Permission.screenRecording => (
    title: 'Screen & System Audio Recording',
    why:
        'Lets a device you allow see this Mac’s screen and hear its sound. '
        'After you turn it on, quit Baton and open it again.',
    where:
        'Open System Settings, Privacy & Security, Screen & System Audio '
        'Recording.',
    whenOff: null,
  ),
  Permission.notifications => (
    title: 'Notifications',
    why: 'Tells you when one of your devices unlocked this Mac.',
    where: 'Open System Settings, Notifications, Baton.',
    whenOff: null,
  ),
  Permission.camera => (
    title: 'Camera',
    why: 'Reads the code another device shows, so the two can pair.',
    where: defaultTargetPlatform == TargetPlatform.macOS
        ? 'Open System Settings, Privacy & Security, Camera.'
        : 'Open Settings, Apps, Baton.',
    whenOff: null,
  ),
  Permission.faceId => (
    title: 'Face ID',
    why: 'Checks that it’s you before this device unlocks your Mac.',
    where: 'Open Settings, Apps, Baton.',
    whenOff: null,
  ),
  Permission.localNetwork => (
    title: 'Local Network',
    why:
        'Connects straight to your devices on the same network, so things '
        'arrive faster. The system doesn’t tell Baton whether it’s on, so '
        'check it in Settings.',
    where: defaultTargetPlatform == TargetPlatform.macOS
        ? 'Open System Settings, Privacy & Security, Local Network.'
        : 'Open Settings, Apps, Baton.',
    whenOff: null,
  ),
};

/// How many permissions need the person, or that all is well.
class _Summary extends StatelessWidget {
  const _Summary({required this.checks});

  final List<PermissionCheck> checks;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final off = checks.where((check) => check.needsAttention).length;
    final allOn = off == 0;
    final color = allOn
        ? BatonTheme.success(theme.brightness)
        : theme.colorScheme.error;
    return Semantics(
      liveRegion: true,
      child: Row(
        children: [
          Icon(
            allOn ? Icons.verified_user_outlined : Icons.shield_outlined,
            size: 40,
            color: color,
          ),
          const SizedBox(width: BatonTheme.gap),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  allOn
                      ? 'Everything Baton can check is on'
                      : off == 1
                      ? '1 permission needs you'
                      : '$off permissions need you',
                  style: theme.textTheme.titleLarge,
                ),
                const SizedBox(height: 2),
                Text(
                  allOn
                      ? 'Baton can do all it does on this device.'
                      : 'Turn them on so Baton can do all it does on this '
                            'device.',
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _PermissionRow extends StatelessWidget {
  const _PermissionRow({required this.check, required this.onTurnOn});

  final PermissionCheck check;
  final VoidCallback onTurnOn;

  static IconData _icon(Permission permission) => switch (permission) {
    Permission.control => Icons.mouse_outlined,
    Permission.screenRecording => Icons.screen_share_outlined,
    Permission.notifications => Icons.notifications_outlined,
    Permission.camera => Icons.photo_camera_outlined,
    Permission.faceId => Icons.face_outlined,
    Permission.localNetwork => Icons.wifi,
  };

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final text = permissionText(check.permission);
    final (status, statusColor) = switch (check.state) {
      PermissionState.on => ('On', BatonTheme.success(theme.brightness)),
      PermissionState.off => ('Off', scheme.error),
      PermissionState.notAsked => ('Not turned on yet', scheme.error),
      PermissionState.notSetUp => ('Not set up', scheme.error),
      PermissionState.cannotCheck => (
        'Check in Settings',
        scheme.onSurfaceVariant,
      ),
    };
    final button = switch (check.state) {
      PermissionState.on => null,
      PermissionState.notAsked
          when check.permission != Permission.control &&
              check.permission != Permission.screenRecording =>
        'Turn On',
      _ => 'Open Settings',
    };
    return Card(
      margin: const EdgeInsets.only(bottom: BatonTheme.gapSmall + 4),
      child: Padding(
        padding: const EdgeInsets.all(BatonTheme.gap),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Container(
                  width: 40,
                  height: 40,
                  decoration: BoxDecoration(
                    color: scheme.primary.withValues(alpha: 0.12),
                    borderRadius: BorderRadius.circular(BatonTheme.radius - 4),
                  ),
                  child: Icon(_icon(check.permission), color: scheme.primary),
                ),
                const SizedBox(width: BatonTheme.gap - 4),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(text.title, style: theme.textTheme.titleMedium),
                      Row(
                        children: [
                          Icon(
                            check.state == PermissionState.on
                                ? Icons.check_circle
                                : check.state == PermissionState.cannotCheck
                                ? Icons.help_outline
                                : Icons.error_outline,
                            size: 16,
                            color: statusColor,
                          ),
                          const SizedBox(width: 4),
                          Text(
                            status,
                            style: theme.textTheme.labelLarge?.copyWith(
                              color: statusColor,
                            ),
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: BatonTheme.gapSmall),
            Text(
              [
                text.why,
                if (check.state != PermissionState.on) ?text.whenOff,
              ].join(' '),
              style: theme.textTheme.bodyMedium?.copyWith(
                color: scheme.onSurfaceVariant,
              ),
            ),
            if (button != null) ...[
              const SizedBox(height: BatonTheme.gap - 4),
              Align(
                alignment: Alignment.centerRight,
                child: check.state == PermissionState.cannotCheck
                    ? OutlinedButton(
                        onPressed: onTurnOn,
                        child: Text(
                          button,
                          semanticsLabel: '$button, ${text.title}',
                        ),
                      )
                    : FilledButton(
                        onPressed: onTurnOn,
                        child: Text(
                          button,
                          semanticsLabel: '$button, ${text.title}',
                        ),
                      ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
