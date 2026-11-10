// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../always_on.dart';
import '../backend.dart';
import '../control_access.dart';
import '../controller.dart';
import '../format.dart';
import '../mac_passwords.dart';
import '../notice_access.dart';
import '../permissions.dart';
import '../theme.dart';
import 'content_box.dart';
import 'diagnostics_screen.dart';
import 'permissions_screen.dart';
import 'restart_unlock_screen.dart';

/// What is about this device rather than another one: its name, whether others can reach it and
/// its fingerprint, what the system lets Baton do, whether a Mac opens Baton when its person signs
/// in, unlocking a Mac after it restarts (on an iPhone or iPad), and the diagnostics.
class SettingsScreen extends StatefulWidget {
  const SettingsScreen({
    required this.session,
    this.now = DateTime.now,
    this.platform,
    this.screenAccess = const PlatformScreenAccess(),
    this.notices = const PlatformNoticeKeeper(),
    this.system = const PlatformSystemSettings(),
    this.alwaysOn = const PlatformAlwaysOnKeeper(),
    this.macPasswords,
    super.key,
  });

  final Session session;

  /// The time the diagnostics measure from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// The system this runs on, which says what permissions there are to check. Tests give it one
  /// of their own.
  final TargetPlatform? platform;

  final ScreenAccessKeeper screenAccess;
  final NoticeKeeper notices;
  final SystemSettingsKeeper system;

  /// Says whether a Mac opens Baton when its person signs in, and changes it. Tests give it one of
  /// their own.
  final AlwaysOnKeeper alwaysOn;

  /// Keeps the Macs to unlock after a restart, and their passwords. Only an iPhone or iPad has one,
  /// and only then is there a way in to it.
  final MacPasswordKeeper? macPasswords;

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen>
    with WidgetsBindingObserver {
  late final DevicesController _controller = DevicesController(widget.session);

  /// How many permissions need the person, or null before they were looked at.
  int? _permissionsOff;

  /// Whether this Mac opens Baton when the person signs in, or null before it was looked at.
  SignInOpening? _signIn;

  /// Whether a change of that is on its way to the system.
  bool _changingSignIn = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(_controller.start());
    unawaited(_checkPermissions());
    unawaited(_checkSignIn());
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _controller.dispose();
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(_checkPermissions());
      unawaited(_checkSignIn());
    }
  }

  Future<void> _checkSignIn() async {
    final opening = await widget.alwaysOn.signIn();
    if (mounted) setState(() => _signIn = opening);
  }

  /// Turns opening at sign-in on or off. Where the system waits for the person's yes, which only
  /// System Settings takes, the switch stays off and Settings opens at the place for it.
  Future<void> _setSignIn(bool on) async {
    if (_changingSignIn) return;
    setState(() => _changingSignIn = true);
    try {
      final opening = await widget.alwaysOn.setSignIn(on);
      if (!mounted) return;
      setState(() => _signIn = opening);
      if (on && opening == SignInOpening.needsApproval) {
        await widget.alwaysOn.openSignInSettings();
      }
    } on AlwaysOnFailure catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context)
        ..hideCurrentSnackBar()
        ..showSnackBar(
          SnackBar(
            content: Text(
              'Baton could not change this. The system said: ${e.message}',
            ),
          ),
        );
    } finally {
      if (mounted) setState(() => _changingSignIn = false);
    }
  }

  Future<void> _checkPermissions() async {
    final checks = await checkPermissions(
      platform: widget.platform ?? defaultTargetPlatform,
      notices: widget.notices,
      system: widget.system,
    );
    final off = checks.where((check) => check.needsAttention).length;
    if (mounted) setState(() => _permissionsOff = off);
  }

  Future<void> _open(Widget screen) async {
    await Navigator.of(context)
        .push(MaterialPageRoute<void>(builder: (_) => screen));
    await _checkPermissions();
  }

  @override
  Widget build(BuildContext context) {
    final keeper = widget.macPasswords;
    final off = _permissionsOff;
    final signIn = _signIn;
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: SafeArea(
        child: ContentBox(
          child: ListView(
            padding: const EdgeInsets.all(BatonTheme.gap),
            children: [
              ListenableBuilder(
                listenable: _controller,
                builder: (context, _) => _ThisDevice(
                  me: _controller.me,
                  network: _controller.network,
                ),
              ),
              const SizedBox(height: BatonTheme.gapLarge),
              _Group(
                children: [
                  _Row(
                    icon: Icons.verified_user_outlined,
                    title: 'Permissions',
                    detail: switch (off) {
                      null => null,
                      0 => 'All on',
                      1 => '1 needs you',
                      _ => '$off need you',
                    },
                    attention: (off ?? 0) > 0,
                    onTap: () => unawaited(
                      _open(
                        PermissionsScreen(
                          platform: widget.platform,
                          screenAccess: widget.screenAccess,
                          notices: widget.notices,
                          system: widget.system,
                        ),
                      ),
                    ),
                  ),
                  if (signIn != null && signIn != SignInOpening.unavailable)
                    _SwitchRow(
                      icon: Icons.login_outlined,
                      title: 'Open Baton when you sign in',
                      subtitle: signIn == SignInOpening.needsApproval
                          ? 'Turned off in System Settings. Tap to turn it on there.'
                          : 'Baton stays in the menu bar, so that your other '
                                'devices can reach this Mac.',
                      value: signIn == SignInOpening.on,
                      onChanged: _changingSignIn ? null : _setSignIn,
                    ),
                  if (keeper != null)
                    _Row(
                      icon: Icons.lock_open_outlined,
                      title: 'Unlock a Mac after restart',
                      onTap: () =>
                          unawaited(_open(RestartUnlockScreen(keeper: keeper))),
                    ),
                ],
              ),
              const SizedBox(height: BatonTheme.gapLarge),
              _Group(
                children: [
                  _Row(
                    icon: Icons.monitor_heart_outlined,
                    title: 'Diagnostics',
                    onTap: () => unawaited(
                      _open(
                        DiagnosticsScreen(
                          recentLog: widget.session.recentLog,
                          session: widget.session,
                          now: widget.now,
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// This device: its name and kind, whether other devices can reach it, and its fingerprint, which
/// the person compares with what another device shows when the two pair.
class _ThisDevice extends StatelessWidget {
  const _ThisDevice({required this.me, required this.network});

  final Me me;
  final Network? network;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final online = network?.relay != null;
    return Card(
      margin: EdgeInsets.zero,
      child: Padding(
        padding: const EdgeInsets.all(BatonTheme.gap),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Container(
                  width: 56,
                  height: 56,
                  decoration: BoxDecoration(
                    color: scheme.primary.withValues(alpha: 0.12),
                    borderRadius: BorderRadius.circular(BatonTheme.radius),
                  ),
                  child: Icon(
                    platformIcon(me.platform),
                    size: 30,
                    color: scheme.primary,
                  ),
                ),
                const SizedBox(width: BatonTheme.gap),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(me.name, style: theme.textTheme.titleLarge),
                      Text(
                        platformName(me.platform),
                        style: theme.textTheme.bodyMedium?.copyWith(
                          color: scheme.onSurfaceVariant,
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: BatonTheme.gap),
            Row(
              children: [
                Icon(
                  Icons.circle,
                  size: 10,
                  color: online
                      ? BatonTheme.success(theme.brightness)
                      : scheme.outline,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    online
                        ? 'Online: your devices can reach this one'
                        : 'Connecting…',
                  ),
                ),
              ],
            ),
            const SizedBox(height: BatonTheme.gap - 4),
            Text('Fingerprint', style: theme.textTheme.labelMedium),
            const SizedBox(height: 2),
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

/// Rows that belong together, on one card with a line between each, as the system's own settings
/// show them.
class _Group extends StatelessWidget {
  const _Group({required this.children});

  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: EdgeInsets.zero,
      child: Column(
        children: [
          for (final (index, child) in children.indexed) ...[
            if (index > 0) const Divider(indent: 64),
            child,
          ],
        ],
      ),
    );
  }
}

class _Row extends StatelessWidget {
  const _Row({
    required this.icon,
    required this.title,
    required this.onTap,
    this.detail,
    this.attention = false,
  });

  final IconData icon;
  final String title;
  final String? detail;

  /// Whether [detail] says that something needs the person.
  final bool attention;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final detail = this.detail;
    return ListTile(
      onTap: onTap,
      leading: _RowIcon(icon),
      title: Text(title),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (detail != null)
            Text(
              detail,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: attention ? scheme.error : scheme.onSurfaceVariant,
              ),
            ),
          const SizedBox(width: 4),
          Icon(Icons.chevron_right, color: scheme.onSurfaceVariant),
        ],
      ),
    );
  }
}

/// A row with a switch, which a tap on the row turns as well.
class _SwitchRow extends StatelessWidget {
  const _SwitchRow({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.value,
    required this.onChanged,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final bool value;

  /// Null while a change is on its way, which leaves the switch as it is.
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final onChanged = this.onChanged;
    return ListTile(
      onTap: onChanged == null ? null : () => onChanged(!value),
      leading: _RowIcon(icon),
      title: Text(title),
      subtitle: Text(subtitle),
      trailing: Switch(value: value, onChanged: onChanged),
    );
  }
}

/// The small colored square that leads each row.
class _RowIcon extends StatelessWidget {
  const _RowIcon(this.icon);

  final IconData icon;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      width: 32,
      height: 32,
      decoration: BoxDecoration(
        color: scheme.primary.withValues(alpha: 0.12),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Icon(icon, size: 20, color: scheme.primary),
    );
  }
}
