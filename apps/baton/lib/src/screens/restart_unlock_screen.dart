// SPDX-License-Identifier: Apache-2.0

import 'dart:async';
import 'dart:math';

import 'package:flutter/material.dart';

import '../mac_passwords.dart';
import '../rust/api/mac_unlock.dart';
import 'content_box.dart';

/// The command that shows the fingerprint of a Mac's own key, for the person to compare with
/// what the phone heard.
const macKeyCommand = 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub';

/// Unlocks a Mac that FileVault holds locked after a restart.
///
/// After a restart, a Mac with FileVault turned on runs nothing until someone types the password
/// of an account on it, Baton included. With Remote Login turned on, the Mac takes that password
/// over SSH, and this screen gives it from the phone. The password is kept in the keychain of the
/// phone behind the person's face, and the Mac is given it only when it proves who it is with a
/// key that the person compared with the Mac when it was saved.
class RestartUnlockScreen extends StatefulWidget {
  const RestartUnlockScreen({
    required this.keeper,
    this.unlocker = const RustSshUnlocker(),
    this.newId = _randomId,
    super.key,
  });

  final MacPasswordKeeper keeper;
  final SshUnlocker unlocker;

  /// Makes the id of a Mac that is being saved. Tests give one that they know.
  final String Function() newId;

  @override
  State<RestartUnlockScreen> createState() => _RestartUnlockScreenState();
}

String _randomId() {
  final random = Random.secure();
  return List.generate(
    16,
    (_) => random.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ).join();
}

class _RestartUnlockScreenState extends State<RestartUnlockScreen> {
  List<SavedMac>? _macs;
  String? _problem;

  /// The Mac that is being unlocked, while it is.
  String? _busy;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    try {
      final macs = await widget.keeper.list();
      if (!mounted) return;
      setState(() {
        _macs = macs;
        _problem = null;
      });
    } on Object catch (e) {
      if (!mounted) return;
      setState(() {
        _macs = const [];
        _problem = 'The saved Macs could not be read ($e).';
      });
    }
  }

  Future<void> _add() async {
    final saved = await Navigator.of(context).push<bool>(
      MaterialPageRoute(
        builder: (_) => AddMacScreen(
          keeper: widget.keeper,
          unlocker: widget.unlocker,
          id: widget.newId(),
        ),
      ),
    );
    if (saved ?? false) await _load();
  }

  Future<void> _unlock(SavedMac mac) async {
    final String? password;
    try {
      password = await widget.keeper.open(mac.id, reason: 'Unlock ${mac.host}');
    } on MacPasswordProblem catch (e) {
      await _tell(mac, e.message, offerPassword: true);
      return;
    }
    if (password == null || !mounted) return;
    setState(() => _busy = mac.id);
    final SshUnlock outcome;
    try {
      outcome = await widget.unlocker.unlock(mac, password);
    } finally {
      if (mounted) setState(() => _busy = null);
    }
    if (!mounted) return;
    switch (outcome) {
      case SshUnlock_Accepted():
        await _tell(
          mac,
          'The Mac took the password. If it was locked, it is unlocking its disk and starting '
          'up now, which takes a minute or two. If it stops at the login window, someone needs '
          'to log in there before Baton can connect.',
          title: 'Unlocked',
        );
      case SshUnlock_Rejected():
        await _tell(
          mac,
          'The Mac said the account name or the password is wrong.',
          offerPassword: true,
        );
      case SshUnlock_UnknownKey(:final fingerprint):
        await _unknownKey(mac, fingerprint, password);
      case SshUnlock_NoPassword():
        await _tell(
          mac,
          'The Mac does not take a password over SSH. Check that Remote Login is on in '
          'System Settings > General > Sharing.',
        );
      case SshUnlock_Unreachable():
        await _tell(
          mac,
          'The Mac did not answer. Check that this device is on the same network as the Mac, '
          'and that Remote Login is on. A Mac that has just restarted may take a minute to '
          'join the network.',
        );
      case SshUnlock_Failed(:final detail):
        await _tell(
          mac,
          'Something went wrong while talking to the Mac ($detail).',
        );
    }
  }

  /// The Mac answered with a key that is not saved. Before a Mac's disk is unlocked it may
  /// answer with another key than once it is running, so the person may trust this one too, but
  /// only after being told what it means.
  Future<void> _unknownKey(
    SavedMac mac,
    String fingerprint,
    String password,
  ) async {
    final trust = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('The Mac answered with another key'),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              const Text(
                'This is not the key you saved, so the Mac was not given the password. '
                'A Mac that is still locked after a restart can answer with a key of its own. '
                'Something else on the network pretending to be your Mac would also look like '
                'this.',
              ),
              const SizedBox(height: 12),
              SelectableText(
                fingerprint,
                style: const TextStyle(fontFamily: 'monospace'),
              ),
              const SizedBox(height: 12),
              const Text(
                'Trust it only if you are on your own network and expected this Mac to be '
                'locked.',
              ),
            ],
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(true),
            child: const Text('Trust and unlock'),
          ),
        ],
      ),
    );
    if (!(trust ?? false) || !mounted) return;
    final trusted = mac.copyWith(keys: [...mac.keys, fingerprint]);
    try {
      await widget.keeper.save(trusted);
    } on MacPasswordProblem catch (e) {
      await _tell(mac, e.message);
      return;
    }
    await _load();
    if (!mounted) return;
    setState(() => _busy = mac.id);
    final SshUnlock outcome;
    try {
      outcome = await widget.unlocker.unlock(trusted, password);
    } finally {
      if (mounted) setState(() => _busy = null);
    }
    if (!mounted) return;
    await _tell(
      trusted,
      outcome is SshUnlock_Accepted
          ? 'The Mac took the password. If it was locked, it is starting up now.'
          : 'The Mac still did not take the password. Try again in a minute.',
      title: outcome is SshUnlock_Accepted ? 'Unlocked' : null,
    );
  }

  Future<void> _tell(
    SavedMac mac,
    String message, {
    String? title,
    bool offerPassword = false,
  }) async {
    final again = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title ?? 'Could not unlock ${mac.host}'),
        content: Text(message),
        actions: [
          if (offerPassword)
            TextButton(
              onPressed: () => Navigator.of(context).pop(true),
              child: const Text('Enter the password again'),
            ),
          FilledButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('OK'),
          ),
        ],
      ),
    );
    if ((again ?? false) && mounted) await _changePassword(mac);
  }

  Future<void> _changePassword(SavedMac mac) async {
    final password = await showDialog<String>(
      context: context,
      builder: (context) => _PasswordDialog(account: mac.account),
    );
    if (password == null || password.isEmpty || !mounted) return;
    try {
      await widget.keeper.save(mac, password: password);
    } on MacPasswordProblem catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(e.message)));
    }
  }

  Future<void> _forget(SavedMac mac) async {
    final sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('Forget ${mac.host}?'),
        content: const Text('Its password is removed from this device.'),
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
    if (!(sure ?? false)) return;
    await widget.keeper.forget(mac.id);
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    final macs = _macs;
    return Scaffold(
      appBar: AppBar(title: const Text('Unlock after restart')),
      floatingActionButton: FloatingActionButton.extended(
        onPressed: _add,
        icon: const Icon(Icons.add),
        label: const Text('Add a Mac'),
      ),
      body: SafeArea(
        child: ContentBox(
          child: ListView(
            padding: const EdgeInsets.fromLTRB(16, 16, 16, 96),
            children: [
              const Text(
                'After a restart, a Mac with FileVault keeps its disk locked until someone types '
                'a password, and Baton cannot reach it. With Remote Login turned on, this device '
                'can give the Mac its password. The password stays on this device, behind your '
                'face.',
              ),
              if (_problem case final problem?) ...[
                const SizedBox(height: 16),
                Text(
                  problem,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ],
              const SizedBox(height: 16),
              if (macs == null)
                const Center(child: CircularProgressIndicator())
              else if (macs.isEmpty)
                const Text('No Mac saved yet.')
              else
                for (final mac in macs)
                  ListTile(
                    contentPadding: EdgeInsets.zero,
                    leading: const Icon(Icons.laptop_mac),
                    title: Text(mac.host),
                    subtitle: Text(mac.account),
                    trailing: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        if (_busy == mac.id)
                          const SizedBox.square(
                            dimension: 24,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        else
                          FilledButton(
                            onPressed: _busy == null
                                ? () => _unlock(mac)
                                : null,
                            child: const Text('Unlock'),
                          ),
                        PopupMenuButton<_MacAction>(
                          tooltip: 'More',
                          onSelected: (action) => switch (action) {
                            _MacAction.password => _changePassword(mac),
                            _MacAction.forget => _forget(mac),
                          },
                          itemBuilder: (context) => const [
                            PopupMenuItem(
                              value: _MacAction.password,
                              child: Text('Change password'),
                            ),
                            PopupMenuItem(
                              value: _MacAction.forget,
                              child: Text('Forget'),
                            ),
                          ],
                        ),
                      ],
                    ),
                  ),
            ],
          ),
        ),
      ),
    );
  }
}

enum _MacAction { password, forget }

class _PasswordDialog extends StatefulWidget {
  const _PasswordDialog({required this.account});

  final String account;

  @override
  State<_PasswordDialog> createState() => _PasswordDialogState();
}

class _PasswordDialogState extends State<_PasswordDialog> {
  final _password = TextEditingController();

  @override
  void dispose() {
    _password.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text('Password of ${widget.account}'),
      content: TextField(
        controller: _password,
        obscureText: true,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Password'),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_password.text),
          child: const Text('Save'),
        ),
      ],
    );
  }
}

/// Saves a Mac to unlock after a restart: its address, the account and its password, and the
/// fingerprint of its key once the person has compared it with the Mac.
class AddMacScreen extends StatefulWidget {
  const AddMacScreen({
    required this.keeper,
    required this.unlocker,
    required this.id,
    super.key,
  });

  final MacPasswordKeeper keeper;
  final SshUnlocker unlocker;
  final String id;

  @override
  State<AddMacScreen> createState() => _AddMacScreenState();
}

class _AddMacScreenState extends State<AddMacScreen> {
  final _host = TextEditingController();
  final _account = TextEditingController();
  final _password = TextEditingController();

  bool _checking = false;
  String? _problem;

  /// The fingerprint that the Mac answered with, once it has.
  String? _heard;

  /// The address that [_heard] came from. A change of address needs a new check.
  String? _heardFrom;

  @override
  void dispose() {
    _host.dispose();
    _account.dispose();
    _password.dispose();
    super.dispose();
  }

  String get _address => _host.text.trim();

  bool get _filled =>
      _address.isNotEmpty &&
      _account.text.trim().isNotEmpty &&
      _password.text.isNotEmpty;

  Future<void> _check() async {
    setState(() {
      _checking = true;
      _problem = null;
      _heard = null;
    });
    final address = _address;
    final check = await widget.unlocker.key(address, 22);
    if (!mounted) return;
    setState(() {
      _checking = false;
      switch (check) {
        case SshKeyCheck_Heard(:final fingerprint):
          _heard = fingerprint;
          _heardFrom = address;
        case SshKeyCheck_Unreachable():
          _problem =
              'The Mac did not answer. Check the address, that this device is on the same '
              'network, and that Remote Login is on.';
        case SshKeyCheck_Failed(:final detail):
          _problem =
              'Something answered, but not as a Mac with Remote Login does ($detail).';
      }
    });
  }

  Future<void> _save() async {
    final heard = _heard;
    if (heard == null) return;
    final mac = SavedMac(
      id: widget.id,
      host: _address,
      account: _account.text.trim(),
      keys: [heard],
    );
    try {
      await widget.keeper.save(mac, password: _password.text);
    } on MacPasswordProblem catch (e) {
      if (!mounted) return;
      setState(() => _problem = e.message);
      return;
    }
    if (mounted) Navigator.of(context).pop(true);
  }

  @override
  Widget build(BuildContext context) {
    final heard = _heardFrom == _address ? _heard : null;
    return Scaffold(
      appBar: AppBar(title: const Text('Add a Mac')),
      body: SafeArea(
        child: ContentBox(
          child: ListView(
            padding: const EdgeInsets.all(16),
            children: [
              const Text(
                'On the Mac, open System Settings > General > Sharing and turn on Remote Login. '
                'Click the info button next to it to see the address, such as '
                'ssh jay@Jays-MBP.local: the part after @ is the address, and the part before '
                'it is the account.',
              ),
              const SizedBox(height: 16),
              TextField(
                controller: _host,
                decoration: const InputDecoration(
                  labelText: 'Address of the Mac',
                ),
                autocorrect: false,
                keyboardType: TextInputType.url,
                onChanged: (_) => setState(() {}),
              ),
              TextField(
                controller: _account,
                decoration: const InputDecoration(labelText: 'Account name'),
                autocorrect: false,
                onChanged: (_) => setState(() {}),
              ),
              TextField(
                controller: _password,
                decoration: const InputDecoration(
                  labelText: 'Password of the account',
                ),
                obscureText: true,
                onChanged: (_) => setState(() {}),
              ),
              const SizedBox(height: 16),
              if (_problem case final problem?) ...[
                Text(
                  problem,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
                const SizedBox(height: 16),
              ],
              if (heard == null)
                FilledButton(
                  onPressed: _filled && !_checking ? _check : null,
                  child: Text(_checking ? 'Checking…' : 'Check the Mac'),
                )
              else ...[
                const Text(
                  'The Mac answered with this key. On the Mac, run this in Terminal and check '
                  'that the line it shows has the same text after SHA256:',
                ),
                const SizedBox(height: 8),
                const SelectableText(
                  macKeyCommand,
                  style: TextStyle(fontFamily: 'monospace'),
                ),
                const SizedBox(height: 8),
                SelectableText(
                  heard,
                  key: const Key('heard'),
                  style: const TextStyle(
                    fontFamily: 'monospace',
                    fontWeight: FontWeight.bold,
                  ),
                ),
                const SizedBox(height: 16),
                FilledButton(
                  onPressed: _filled ? _save : null,
                  child: const Text('They match. Save'),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
