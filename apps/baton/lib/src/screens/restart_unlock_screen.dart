// SPDX-License-Identifier: Apache-2.0

import 'dart:async';
import 'dart:math';

import 'package:flutter/material.dart';

import '../format.dart';
import '../mac_passwords.dart';
import '../rust/api/mac_unlock.dart';
import 'content_box.dart';

/// The command that shows the fingerprint of a Mac's own key, for the person to compare with
/// what the phone heard.
const macKeyCommand = 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub';

/// What may be behind an answer with another key than the one that was saved, for the person to
/// weigh. The unlock and the check of the way give it alike.
const _otherKeyReasons =
    'Something else on the network may be answering for your Mac, or a Mac that is still '
    'locked after a restart may answer with a key of its own.';

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
    this.now = DateTime.now,
    super.key,
  });

  final MacPasswordKeeper keeper;
  final SshUnlocker unlocker;

  /// Makes the id of a Mac that is being saved. Tests give one that they know.
  final String Function() newId;

  /// The time, to say how long a Mac took to answer. Tests give a clock that they set.
  final DateTime Function() now;

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

  /// The Mac that is being unlocked, or whose way is being checked, while it is.
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
        await _unknownKey(mac, fingerprint);
      case SshUnlock_NoPassword():
        // The Mac answered with the key that was saved, so Remote Login is on. It takes keys only,
        // as a Mac that runs may be set up to on purpose, and the Add screen saves one.
        await _tell(
          mac,
          'The Mac answered, but it takes keys only, so it did not take the password. That is '
          'expected while the Mac runs. If it is waiting for its disk password after a restart, '
          'this way cannot unlock it, and it has to be unlocked at the Mac.',
        );
      case SshUnlock_Unreachable():
        await _tell(
          mac,
          'The Mac did not answer. Check that this device is on the same network as the Mac, '
          'or on the private network that leads to it, and that Remote Login is on. A Mac that '
          'has just restarted may take a minute to join the network. "Check the way" in the '
          'menu of the Mac says whether it can be reached from here.',
        );
      case SshUnlock_Failed(:final detail):
        await _tell(
          mac,
          'Something went wrong while talking to the Mac (${visibleText(detail)}).',
        );
    }
  }

  /// The Mac answered with a key that is not saved, and was given nothing. Nothing here lets the
  /// person trust it: a Mac that is locked cannot show its key to compare, so whoever answered at
  /// its address, which any device on the network can do, would get the password.
  Future<void> _unknownKey(SavedMac mac, String fingerprint) async {
    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('The Mac answered with another key'),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'This is not the key you saved for ${mac.host}, so nothing was sent to it. '
                'It answered with:',
              ),
              const SizedBox(height: 12),
              SelectableText(
                fingerprint,
                style: const TextStyle(fontFamily: 'monospace'),
              ),
              const SizedBox(height: 12),
              const Text(_otherKeyReasons),
            ],
          ),
        ),
        actions: [
          FilledButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('OK'),
          ),
        ],
      ),
    );
  }

  /// Asks the Mac for its key at the saved address and says what came of it. The password is not
  /// sent, and the face is not asked for: the check can be made on any network, as often as the
  /// person likes, with a Mac that runs and with one that waits for its disk password. It shows
  /// from where the person is whether the way to the Mac works, and whether the Mac is the one that
  /// was saved. The key it hears is the key of the Mac that runs: a Mac that waits for its disk
  /// password may answer with a key of its own, so "the way is open" says nothing yet of the unlock
  /// after a restart, and the dialog says so.
  Future<void> _checkWay(SavedMac mac) async {
    setState(() => _busy = mac.id);
    final started = widget.now();
    final SshKeyCheck heard;
    final Duration took;
    try {
      heard = await widget.unlocker.key(mac.host, mac.port);
      took = widget.now().difference(started);
    } finally {
      if (mounted) setState(() => _busy = null);
    }
    if (!mounted) return;
    final host = visibleText(mac.host);
    final (title, message, shown, reasons) = switch (heard) {
      SshKeyCheck_Heard(:final fingerprint) when _isSaved(mac, fingerprint) => (
        'The way is open',
        'Reached $host in ${_took(took)}. It answered with the key that you saved, so the way '
            'from here works. Nothing was sent to it, not even the password. A Mac that waits '
            'for its disk password after a restart may answer with a key of its own.',
        null,
        null,
      ),
      SshKeyCheck_Heard(:final fingerprint) => (
        'The Mac answered with another key',
        'Reached $host in ${_took(took)}, but with another key than the one you saved, so an '
            'unlock from here would send it nothing. It answered with:',
        fingerprint,
        _otherKeyReasons,
      ),
      SshKeyCheck_Unreachable() => (
        'No answer',
        'Nothing answered at $host. At home, check that Remote Login is on, that the Mac is '
            'awake and that its address has not changed. Away from home, check that your private '
            'network is on, and that the device at home that carries it is on too. A Mac that '
            'has just restarted may take a minute to join the network.',
        null,
        null,
      ),
      SshKeyCheck_Failed(:final detail) => (
        'Not a Mac with Remote Login',
        'Something answered at $host, but not as a Mac with Remote Login does '
            '(${visibleText(detail)}).',
        null,
        null,
      ),
    };
    await showDialog<void>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(message),
              if (shown != null) ...[
                const SizedBox(height: 12),
                SelectableText(
                  shown,
                  style: const TextStyle(fontFamily: 'monospace'),
                ),
              ],
              if (reasons != null) ...[
                const SizedBox(height: 12),
                Text(reasons),
              ],
            ],
          ),
        ),
        actions: [
          FilledButton(
            onPressed: () => Navigator.of(context).pop(),
            child: const Text('OK'),
          ),
        ],
      ),
    );
  }

  /// Whether `fingerprint` is one of the keys that `mac` was saved with.
  bool _isSaved(SavedMac mac, String fingerprint) =>
      mac.keys.any((key) => key.trim() == fingerprint);

  /// How long a Mac took, for a sentence: "340 ms", or "1.4 s" from a second.
  String _took(Duration time) {
    final milliseconds = time.inMilliseconds;
    return milliseconds < 1000
        ? '$milliseconds ms'
        : '${(milliseconds / 1000).toStringAsFixed(1)} s';
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
                          enabled: _busy == null,
                          onSelected: (action) => switch (action) {
                            _MacAction.check => _checkWay(mac),
                            _MacAction.password => _changePassword(mac),
                            _MacAction.forget => _forget(mac),
                          },
                          itemBuilder: (context) => const [
                            PopupMenuItem(
                              value: _MacAction.check,
                              child: Text('Check the way'),
                            ),
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

enum _MacAction { check, password, forget }

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

  /// Whether the password is being tried and saved.
  bool _saving = false;

  /// Whether the password could not be tried, because the Mac did not answer or does not take a
  /// password while it runs, so that it may be saved without the try.
  bool _unchecked = false;

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
      _unchecked = false;
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
              'The Mac did not answer. Check the address, that Remote Login is on, and that '
              'this device is on the same network as the Mac or on the private network that '
              'leads to it.';
        case SshKeyCheck_Failed(:final detail):
          _problem =
              'Something answered, but not as a Mac with Remote Login does '
              '(${visibleText(detail)}).';
      }
    });
  }

  /// Saves the Mac. The password is tried first, while the Mac is running and its key has just
  /// been compared, so that a typo shows now and not at a Mac that is locked after a restart, where
  /// it would cost a try. A Mac that does not answer the try can be saved without it.
  Future<void> _save({bool check = true}) async {
    final heard = _heard;
    if (heard == null) return;
    // Read once: the password that was tried is the one that is saved.
    final password = _password.text;
    final mac = SavedMac(
      id: widget.id,
      host: _address,
      account: _account.text.trim(),
      keys: [heard],
    );
    setState(() {
      _saving = true;
      _problem = null;
      _unchecked = false;
    });
    if (check) {
      final outcome = await widget.unlocker.unlock(mac, password);
      if (!mounted) return;
      final String? problem;
      switch (outcome) {
        case SshUnlock_Accepted():
          problem = null;
        case SshUnlock_Rejected():
          problem = 'The Mac said the account name or the password is wrong. Nothing was saved.';
        case SshUnlock_UnknownKey():
          problem =
              'The Mac now answers with another key than the one you compared, so it was '
              'given nothing and nothing was saved. Check the Mac again.';
          _heard = null;
        case SshUnlock_NoPassword():
          // The Mac that runs takes keys only. The SSH server that answers before the disk is
          // unlocked is another one, and it is not known to read that setting, so the Mac may
          // still take a password after a restart. The person decides.
          problem =
              'This Mac does not take a password over SSH while it runs, so the password could '
              'not be tried. A Mac that waits for its disk password after a restart may still '
              'take one. You can save it without the try.';
          _unchecked = true;
        case SshUnlock_Unreachable():
        case SshUnlock_Failed():
          problem =
              'The Mac did not say whether the password is right. You can save it '
              'without the try.';
          _unchecked = true;
      }
      if (problem != null) {
        setState(() {
          _saving = false;
          _problem = problem;
        });
        return;
      }
    }
    try {
      await widget.keeper.save(mac, password: password);
    } on MacPasswordProblem catch (e) {
      if (!mounted) return;
      setState(() {
        _saving = false;
        _problem = e.message;
      });
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
                'it is the account. If you can, use the Mac\'s number address, such as '
                '192.168.73.20, and not its name: a name may not work from outside your home '
                'network, and a Mac that waits for its disk password may not announce one. The '
                'guide "Reach your Mac from away" says how to give the Mac a fixed address.',
              ),
              const SizedBox(height: 16),
              TextField(
                controller: _host,
                readOnly: _saving,
                decoration: const InputDecoration(
                  labelText: 'Address of the Mac',
                ),
                autocorrect: false,
                keyboardType: TextInputType.url,
                onChanged: (_) => setState(() {}),
              ),
              TextField(
                controller: _account,
                readOnly: _saving,
                decoration: const InputDecoration(labelText: 'Account name'),
                autocorrect: false,
                onChanged: (_) => setState(() {}),
              ),
              TextField(
                controller: _password,
                readOnly: _saving,
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
                  onPressed: _filled && !_saving ? _save : null,
                  child: Text(
                    _saving ? 'Trying the password…' : 'They match. Save',
                  ),
                ),
                if (_unchecked)
                  TextButton(
                    onPressed: _filled && !_saving
                        ? () => _save(check: false)
                        : null,
                    child: const Text('Save without trying'),
                  ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
