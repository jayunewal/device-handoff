// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../format.dart';
import '../mac_passwords.dart';
import '../rust/api/mac_unlock.dart';
import '../rust/api/model.dart';
import '../theme.dart';
import 'content_box.dart';
import 'icon_tile.dart';

/// The command that shows the fingerprint of a Mac's own key, for the person to compare with
/// what this device heard.
const macKeyCommand = 'ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub';

/// The font of a key and of a command: the one that Settings shows a fingerprint in.
const _monospace = TextStyle(
  fontFamily: 'Menlo',
  fontFamilyFallback: ['Courier', 'monospace'],
);

/// Saves a Mac to unlock after a restart: its address, the account and its password, and the
/// fingerprint of its key once the person has compared it with the Mac.
///
/// Baton looks for the Mac where it can. A Mac that is paired with this device was last reached at
/// some addresses of a local network, and the screen asks each of them for the key of the Mac,
/// which only a Mac with Remote Login turned on gives. Where there is no such address, or the
/// Mac is not paired, the person types it. Either way the person compares the key with the Mac
/// before it is saved, because an answer from an address says nothing of who gave it.
class AddMacScreen extends StatefulWidget {
  const AddMacScreen({
    required this.keeper,
    required this.unlocker,
    required this.id,
    this.pairedMacs = const [],
    super.key,
  });

  final MacPasswordKeeper keeper;
  final SshUnlocker unlocker;

  /// Names the Mac among the saved ones.
  final String id;

  /// The devices paired with this one that are Macs, with the addresses they were last reached at.
  final List<Device> pairedMacs;

  @override
  State<AddMacScreen> createState() => _AddMacScreenState();
}

class _AddMacScreenState extends State<AddMacScreen> {
  final _host = TextEditingController();
  final _account = TextEditingController();
  final _password = TextEditingController();

  /// The paired Mac that the person picked, or that was picked for them because it is the only
  /// one.
  Device? _picked;

  /// Whether the person chose to type the address of a Mac that is not in the list.
  bool _other = false;

  /// Counts the looks for a Mac's key. The answer of one that is not the latest is let go: the
  /// person has picked another Mac since, or has typed another address.
  int _looks = 0;

  /// Whether the Mac is being asked for its key, and at which addresses.
  bool _checking = false;
  List<String> _asking = const [];

  /// What went wrong with the look or with the save, for the person.
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
  void initState() {
    super.initState();
    final macs = widget.pairedMacs;
    if (macs.isEmpty) {
      _other = true;
    } else if (macs.length == 1) {
      // The only Mac there is, so there is nothing to choose, and the look can begin as soon as
      // the screen is up.
      _choose(macs.single);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _lookForPicked();
      });
    }
  }

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

  /// Whether the person still has to say which Mac it is.
  bool get _choosing =>
      widget.pairedMacs.isNotEmpty && _picked == null && !_other;

  /// Takes `mac`, or another Mac that is typed when it is `null`, and forgets what was heard.
  /// Call it from `setState`.
  void _choose(Device? mac) {
    _looks++;
    _picked = mac;
    _other = mac == null;
    _host.text = mac?.localAddresses.firstOrNull ?? '';
    _checking = false;
    _problem = null;
    _heard = null;
    _heardFrom = null;
    _unchecked = false;
  }

  void _pick(Device? mac) {
    if (_saving) return;
    setState(() => _choose(mac));
    if (mac != null) _lookForPicked();
  }

  /// The person typed in the address box. A look that is still going would put the address that
  /// answered back over what they type, so it is let go, as a pick lets go of it.
  void _typedAddress() {
    setState(() {
      if (_checking) {
        _looks++;
        _checking = false;
      }
    });
  }

  /// Asks the addresses that the picked Mac was last reached at, if it has any.
  void _lookForPicked() {
    final addresses = _picked?.localAddresses ?? const <String>[];
    if (addresses.isNotEmpty) unawaited(_look(addresses));
  }

  Future<void> _check() => _look([_address]);

  /// Asks each of `addresses` for the key of the Mac at once, and goes with the first that answers
  /// with one: a Mac has its own address among those that it was reached at, and a stale one lets
  /// the person wait for as long as it takes to give up, which is longer than they should.
  Future<void> _look(List<String> addresses) async {
    final look = ++_looks;
    setState(() {
      _checking = true;
      _asking = addresses;
      _problem = null;
      _heard = null;
      _unchecked = false;
    });
    final answers = List<SshKeyCheck?>.filled(addresses.length, null);
    final finished = Completer<void>();
    Object? failure;
    var waiting = addresses.length;
    for (final (index, address) in addresses.indexed) {
      unawaited(
        Future.sync(() => widget.unlocker.key(address, 22))
            .then(
              (answer) {
                answers[index] = answer;
                if (answer is SshKeyCheck_Heard) {
                  if (!finished.isCompleted) finished.complete();
                }
              },
              onError: (Object e) {
                failure ??= e;
              },
            )
            .whenComplete(() {
              waiting--;
              if (waiting == 0 && !finished.isCompleted) finished.complete();
            }),
      );
    }
    await finished.future;
    if (!mounted || look != _looks) return;
    (int, String)? heard;
    (int, String)? odd;
    for (final (index, answer) in answers.indexed) {
      switch (answer) {
        case SshKeyCheck_Heard(:final fingerprint):
          heard ??= (index, fingerprint);
        case SshKeyCheck_Failed(:final detail):
          odd ??= (index, detail);
        case SshKeyCheck_Unreachable() || null:
          break;
      }
    }
    setState(() {
      _checking = false;
      if (heard case (final index, final fingerprint)) {
        if (_address != addresses[index]) _host.text = addresses[index];
        _heard = fingerprint;
        _heardFrom = addresses[index];
      } else if (odd case (final index, final detail)) {
        _problem =
            'Something answered at ${addresses[index]}, but not as a Mac with '
            'Remote Login does (${visibleText(detail)}).';
      } else if (failure != null) {
        _problem = 'The check could not be made (${visibleText('$failure')}).';
      } else {
        final where = addresses.length > 1
            ? ' at ${addresses.join(' or ')}'
            : '';
        final address = addresses.length > 1 ? '' : 'the address, ';
        _problem =
            'The Mac did not answer$where. Check $address'
            'that Remote Login is on, and that this device is on the same '
            'network as the Mac or on the private network that leads to it.';
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

  /// Puts the command on the clipboard. A Mac that uses the same Apple ID as this device gets it
  /// by itself, and the person pastes it into Terminal there.
  Future<void> _copyCommand() async {
    final messenger = ScaffoldMessenger.of(context);
    await Clipboard.setData(const ClipboardData(text: macKeyCommand));
    messenger
      ..hideCurrentSnackBar()
      ..showSnackBar(
        const SnackBar(
          content: Text(
            'Copied. If your Mac uses the same Apple ID, you can paste it there.',
          ),
        ),
      );
  }

  @override
  Widget build(BuildContext context) {
    final heard = _heardFrom == _address ? _heard : null;
    final choosing = _choosing;
    return Scaffold(
      appBar: AppBar(title: const Text('Add a Mac')),
      body: SafeArea(
        child: ContentBox(
          child: ListView(
            padding: const EdgeInsets.all(BatonTheme.gap),
            children: [
              if (widget.pairedMacs.isNotEmpty) ...[
                _WhichMac(
                  macs: widget.pairedMacs,
                  picked: _picked?.id,
                  other: _other,
                  enabled: !_saving,
                  onPick: _pick,
                ),
                const SizedBox(height: BatonTheme.gap),
              ],
              if (!choosing) ...[
                _AddressCard(
                  host: _host,
                  picked: _picked,
                  readOnly: _saving,
                  asking: _checking ? _asking : null,
                  found: heard != null,
                  problem: heard == null ? _problem : null,
                  onChanged: _typedAddress,
                  onCheck: _address.isEmpty ? null : _check,
                ),
                const SizedBox(height: BatonTheme.gap),
              ],
              if (heard != null) ...[
                _SignInCard(
                  account: _account,
                  password: _password,
                  readOnly: _saving,
                  onChanged: () => setState(() {}),
                ),
                const SizedBox(height: BatonTheme.gap),
                _KeyCard(fingerprint: heard, onCopy: _copyCommand),
                const SizedBox(height: BatonTheme.gap),
                if (_problem case final problem?) ...[
                  Text(
                    problem,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                  const SizedBox(height: BatonTheme.gap),
                ],
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
              ] else if (!choosing && !_checking)
                const _RemoteLoginHelp(),
            ],
          ),
        ),
      ),
    );
  }
}

/// A card that holds one step of adding a Mac, under a title.
class _Step extends StatelessWidget {
  const _Step({required this.title, required this.children});

  final String title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    return Card(
      margin: EdgeInsets.zero,
      child: Padding(
        padding: const EdgeInsets.all(BatonTheme.gap),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(title, style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: BatonTheme.gapSmall),
            ...children,
          ],
        ),
      ),
    );
  }
}

/// The paired Macs, to pick the one to add, and a way to type the address of another.
class _WhichMac extends StatelessWidget {
  const _WhichMac({
    required this.macs,
    required this.picked,
    required this.other,
    required this.enabled,
    required this.onPick,
  });

  final List<Device> macs;

  /// The id of the paired Mac that is picked, if one is.
  final String? picked;

  /// Whether the person chose to type the address of another Mac.
  final bool other;
  final bool enabled;

  /// Called with the Mac that was tapped, or with `null` for another Mac.
  final void Function(Device? mac) onPick;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    Widget row({
      required IconData icon,
      required String title,
      required String subtitle,
      required bool selected,
      required VoidCallback onTap,
    }) => ListTile(
      leading: IconTile(icon),
      title: Text(title),
      subtitle: Text(subtitle),
      selected: selected,
      selectedColor: scheme.onSurface,
      selectedTileColor: scheme.primary.withValues(alpha: 0.08),
      trailing: Icon(
        selected ? Icons.check_circle : Icons.circle_outlined,
        color: selected ? scheme.primary : scheme.outline,
      ),
      enabled: enabled,
      onTap: onTap,
    );
    return Card(
      margin: EdgeInsets.zero,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              BatonTheme.gap,
              BatonTheme.gap,
              BatonTheme.gap,
              BatonTheme.gapSmall,
            ),
            child: Text(
              'Which Mac?',
              style: Theme.of(context).textTheme.titleMedium,
            ),
          ),
          for (final mac in macs) ...[
            row(
              icon: Icons.laptop_mac,
              title: mac.name,
              subtitle: mac.localAddresses.isEmpty
                  ? 'Not seen on a local network yet'
                  : 'Seen at ${mac.localAddresses.join(', ')}',
              selected: picked == mac.id,
              onTap: () => onPick(mac),
            ),
            const Divider(indent: 64),
          ],
          row(
            icon: Icons.keyboard_outlined,
            title: 'Another Mac',
            subtitle: 'Type its address',
            selected: other,
            onTap: () => onPick(null),
          ),
        ],
      ),
    );
  }
}

/// Where the Mac is, and what came of asking it for its key.
class _AddressCard extends StatelessWidget {
  const _AddressCard({
    required this.host,
    required this.picked,
    required this.readOnly,
    required this.asking,
    required this.found,
    required this.problem,
    required this.onChanged,
    required this.onCheck,
  });

  final TextEditingController host;

  /// The paired Mac that the address came from, if it did.
  final Device? picked;
  final bool readOnly;

  /// The addresses that are being asked for the key of the Mac, or `null` while none is.
  final List<String>? asking;

  /// Whether the Mac answered with its key at the address that is shown.
  final bool found;

  /// What went wrong with the look, if something did.
  final String? problem;
  final VoidCallback onChanged;

  /// Asks the address for the key. `null` while there is no address to ask.
  final VoidCallback? onCheck;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final mac = picked;
    final asking = this.asking;
    return _Step(
      title: 'Where is it?',
      children: [
        TextField(
          controller: host,
          readOnly: readOnly,
          decoration: const InputDecoration(
            labelText: 'Address of the Mac',
            helperText: 'The number address works best, such as 192.168.73.20.',
            helperMaxLines: 2,
          ),
          autocorrect: false,
          enableSuggestions: false,
          keyboardType: TextInputType.url,
          onChanged: (_) => onChanged(),
        ),
        const SizedBox(height: BatonTheme.gap),
        if (asking != null)
          Semantics(
            liveRegion: true,
            child: Row(
              children: [
                const SizedBox.square(
                  dimension: 20,
                  child: CircularProgressIndicator(strokeWidth: 2),
                ),
                const SizedBox(width: BatonTheme.gapSmall + 4),
                Expanded(
                  child: Text('Looking for the Mac at ${asking.join(' or ')}…'),
                ),
              ],
            ),
          )
        else if (found)
          Semantics(
            liveRegion: true,
            child: Row(
              children: [
                Icon(
                  Icons.check_circle,
                  size: 20,
                  color: BatonTheme.success(theme.brightness),
                ),
                const SizedBox(width: BatonTheme.gapSmall + 4),
                const Expanded(
                  child: Text('The Mac answered, so Remote Login is on.'),
                ),
              ],
            ),
          )
        else ...[
          if (problem != null) ...[
            Semantics(
              liveRegion: true,
              child: Text(problem!, style: TextStyle(color: scheme.error)),
            ),
            const SizedBox(height: BatonTheme.gap),
          ] else if (mac != null && mac.localAddresses.isEmpty) ...[
            Text(
              'Baton has not seen ${mac.name} on a local network yet. Type its '
              'address, or come back when this device is on the same Wi-Fi as the Mac '
              'and Baton is open on both.',
              style: theme.textTheme.bodyMedium?.copyWith(
                color: scheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: BatonTheme.gap),
          ],
          FilledButton(onPressed: onCheck, child: const Text('Check the Mac')),
        ],
      ],
    );
  }
}

/// The account and its password.
class _SignInCard extends StatelessWidget {
  const _SignInCard({
    required this.account,
    required this.password,
    required this.readOnly,
    required this.onChanged,
  });

  final TextEditingController account;
  final TextEditingController password;
  final bool readOnly;
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return _Step(
      title: 'Sign in',
      children: [
        Text(
          'The account whose password unlocks the Mac. The password stays on this device, '
          'behind your face.',
          style: theme.textTheme.bodyMedium?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
        const SizedBox(height: BatonTheme.gap),
        TextField(
          controller: account,
          readOnly: readOnly,
          decoration: const InputDecoration(labelText: 'Account name'),
          autocorrect: false,
          enableSuggestions: false,
          textInputAction: TextInputAction.next,
          onChanged: (_) => onChanged(),
        ),
        const SizedBox(height: BatonTheme.gapSmall),
        TextField(
          controller: password,
          readOnly: readOnly,
          decoration: const InputDecoration(
            labelText: 'Password of the account',
          ),
          obscureText: true,
          onChanged: (_) => onChanged(),
        ),
      ],
    );
  }
}

/// The key that the Mac answered with, and the command that shows the key at the Mac.
class _KeyCard extends StatelessWidget {
  const _KeyCard({required this.fingerprint, required this.onCopy});

  final String fingerprint;
  final VoidCallback onCopy;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return _Step(
      title: 'Make sure it is your Mac',
      children: [
        const Text(
          'The Mac answered with the key below. On the Mac, run this in Terminal and check '
          'that the line it shows has the same text after SHA256:',
        ),
        const SizedBox(height: BatonTheme.gapSmall),
        DecoratedBox(
          decoration: BoxDecoration(
            color: scheme.surfaceContainerHigh,
            borderRadius: BorderRadius.circular(BatonTheme.radius),
          ),
          child: Row(
            children: [
              const Expanded(
                child: Padding(
                  padding: EdgeInsets.all(BatonTheme.gapSmall + 4),
                  child: SelectableText(macKeyCommand, style: _monospace),
                ),
              ),
              IconButton(
                tooltip: 'Copy the command',
                icon: const Icon(Icons.copy_outlined),
                onPressed: onCopy,
              ),
            ],
          ),
        ),
        const SizedBox(height: BatonTheme.gap),
        Text('The Mac answered with', style: theme.textTheme.labelMedium),
        const SizedBox(height: 2),
        SelectableText(
          fingerprint,
          key: const Key('heard'),
          style: _monospace.copyWith(fontWeight: FontWeight.bold),
        ),
      ],
    );
  }
}

/// How to turn on Remote Login at the Mac and where to find its address, for when Baton has not
/// found the Mac.
class _RemoteLoginHelp extends StatelessWidget {
  const _RemoteLoginHelp();

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    Widget step(int number, String text) => Padding(
      padding: const EdgeInsets.only(bottom: BatonTheme.gapSmall),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(width: 24, child: Text('$number.')),
          Expanded(child: Text(text)),
        ],
      ),
    );
    return _Step(
      title: 'Turn on Remote Login at the Mac',
      children: [
        step(
          1,
          'On the Mac, open System Settings, then General, then Sharing, and turn on Remote '
          'Login.',
        ),
        step(
          2,
          'Click the info button next to it. It shows a line such as '
          'ssh jay@192.168.73.20. The part before @ is the account name, and the part after '
          'it is the address.',
        ),
        const SizedBox(height: BatonTheme.gapSmall),
        Text(
          'Use the number address and not a name such as Jays-MBP.local. A name may not work '
          'from outside your home network, and a Mac that waits for its disk password may not '
          'announce one. The guide "Reach your Mac from away" says how to give the Mac a '
          'fixed address.',
          style: theme.textTheme.bodyMedium?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
      ],
    );
  }
}
