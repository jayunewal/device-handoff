// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/mac_passwords.dart';
import 'package:baton/src/rust/api/mac_unlock.dart';
import 'package:baton/src/screens/restart_unlock_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const _key = 'SHA256:9mX2bq0Zt1sYtWw3m4nU5oP6qR7sT8uV9wX0yZ1aB2c';
const _other = 'SHA256:AAAAbq0Zt1sYtWw3m4nU5oP6qR7sT8uV9wX0yZ1aB2c';

class _FakeKeeper extends MacPasswordKeeper {
  final macs = <String, SavedMac>{};
  final passwords = <String, String>{};

  /// What [open] does: the password, `null` for a person who stopped, or a problem to throw.
  Object? Function(String id) opens = (_) => null;
  final reasons = <String>[];

  @override
  Future<List<SavedMac>> list() async => macs.values.toList();

  @override
  Future<void> save(SavedMac mac, {String? password}) async {
    macs[mac.id] = mac;
    if (password != null) passwords[mac.id] = password;
  }

  @override
  Future<String?> open(String id, {required String reason}) async {
    reasons.add(reason);
    final opened = opens(id);
    if (opened is MacPasswordProblem) throw opened;
    return opened as String?;
  }

  @override
  Future<void> forget(String id) async {
    macs.remove(id);
    passwords.remove(id);
  }
}

class _FakeUnlocker extends SshUnlocker {
  SshKeyCheck heard = const SshKeyCheck.heard(fingerprint: _key);
  final outcomes = <SshUnlock>[];
  final tries = <(SavedMac, String)>[];

  @override
  Future<SshKeyCheck> key(String host, int port) async => heard;

  @override
  Future<SshUnlock> unlock(SavedMac mac, String password) async {
    tries.add((mac, password));
    return outcomes.isEmpty ? const SshUnlock.accepted() : outcomes.removeAt(0);
  }
}

const _mac = SavedMac(
  id: 'm1',
  host: 'Jays-MBP.local',
  account: 'jay',
  keys: [_key],
);

Future<void> _show(
  WidgetTester tester,
  _FakeKeeper keeper,
  _FakeUnlocker unlocker,
) async {
  // Tall enough that the whole form of a new Mac is on the screen.
  tester.view.physicalSize = const Size(1200, 2400);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      home: RestartUnlockScreen(
        keeper: keeper,
        unlocker: unlocker,
        newId: () => 'new',
      ),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  test(
    'a saved Mac reads back as it was written, and junk reads as nothing',
    () {
      expect(SavedMac.fromJson(_mac.toJson()), _mac);
      expect(SavedMac.fromJson('not json'), isNull);
      expect(SavedMac.fromJson('{"id": 1}'), isNull);
    },
  );

  testWidgets('a Mac is saved only after its key has been heard and compared', (
    tester,
  ) async {
    final keeper = _FakeKeeper();
    final unlocker = _FakeUnlocker();
    await _show(tester, keeper, unlocker);
    expect(find.text('No Mac saved yet.'), findsOneWidget);

    await tester.tap(find.text('Add a Mac'));
    await tester.pumpAndSettle();
    expect(find.text('They match. Save'), findsNothing);
    await tester.enterText(
      find.widgetWithText(TextField, 'Address of the Mac'),
      ' Jays-MBP.local ',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Account name'),
      'jay',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Password of the account'),
      'secret',
    );
    await tester.pump();
    await tester.tap(find.text('Check the Mac'));
    await tester.pumpAndSettle();

    expect(find.text(_key), findsOneWidget);
    expect(find.text(macKeyCommand), findsOneWidget);
    await tester.tap(find.text('They match. Save'));
    await tester.pumpAndSettle();

    expect(
      keeper.macs['new'],
      const SavedMac(
        id: 'new',
        host: 'Jays-MBP.local',
        account: 'jay',
        keys: [_key],
      ),
    );
    expect(keeper.passwords['new'], 'secret');
    expect(unlocker.tries.single.$2, 'secret');
    expect(find.text('Jays-MBP.local'), findsOneWidget);
  });

  Future<void> fillAndCheck(WidgetTester tester) async {
    await tester.tap(find.text('Add a Mac'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'Address of the Mac'),
      'mac.local',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Account name'),
      'jay',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Password of the account'),
      'typo',
    );
    await tester.pump();
    await tester.tap(find.text('Check the Mac'));
    await tester.pumpAndSettle();
  }

  testWidgets('a password the running Mac turns down is not saved', (
    tester,
  ) async {
    final keeper = _FakeKeeper();
    final unlocker = _FakeUnlocker()..outcomes.add(const SshUnlock.rejected());
    await _show(tester, keeper, unlocker);
    await fillAndCheck(tester);
    await tester.tap(find.text('They match. Save'));
    await tester.pumpAndSettle();
    expect(find.textContaining('wrong. Nothing was saved.'), findsOneWidget);
    expect(find.text('Save without trying'), findsNothing);
    expect(keeper.macs, isEmpty);
  });

  testWidgets('a Mac that does not answer the try can be saved without it', (
    tester,
  ) async {
    final keeper = _FakeKeeper();
    final unlocker = _FakeUnlocker()
      ..outcomes.add(const SshUnlock.unreachable(detail: 'gone'));
    await _show(tester, keeper, unlocker);
    await fillAndCheck(tester);
    await tester.tap(find.text('They match. Save'));
    await tester.pumpAndSettle();
    expect(keeper.macs, isEmpty);
    await tester.tap(find.text('Save without trying'));
    await tester.pumpAndSettle();
    expect(keeper.passwords['new'], 'typo');
    expect(unlocker.tries, hasLength(1));
  });

  testWidgets('a Mac whose key changed since the check is not saved', (
    tester,
  ) async {
    final keeper = _FakeKeeper();
    final unlocker = _FakeUnlocker()
      ..outcomes.add(const SshUnlock.unknownKey(fingerprint: _other));
    await _show(tester, keeper, unlocker);
    await fillAndCheck(tester);
    await tester.tap(find.text('They match. Save'));
    await tester.pumpAndSettle();
    expect(keeper.macs, isEmpty);
    expect(find.text('Check the Mac'), findsOneWidget);
  });

  testWidgets('a Mac that does not answer is not saved', (tester) async {
    final keeper = _FakeKeeper();
    final unlocker = _FakeUnlocker()
      ..heard = const SshKeyCheck.unreachable(detail: 'refused');
    await _show(tester, keeper, unlocker);
    await tester.tap(find.text('Add a Mac'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'Address of the Mac'),
      'mac.local',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Account name'),
      'jay',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Password of the account'),
      'secret',
    );
    await tester.pump();
    await tester.tap(find.text('Check the Mac'));
    await tester.pumpAndSettle();
    expect(find.textContaining('The Mac did not answer.'), findsOneWidget);
    expect(find.text('They match. Save'), findsNothing);
    expect(keeper.macs, isEmpty);
  });

  testWidgets('unlocking asks for the face and gives the Mac its password', (
    tester,
  ) async {
    final keeper = _FakeKeeper()..macs['m1'] = _mac;
    keeper.opens = (_) => 'secret';
    final unlocker = _FakeUnlocker();
    await _show(tester, keeper, unlocker);

    await tester.tap(find.text('Unlock'));
    await tester.pumpAndSettle();

    expect(keeper.reasons, ['Unlock Jays-MBP.local']);
    expect(unlocker.tries, [(_mac, 'secret')]);
    expect(find.text('Unlocked'), findsOneWidget);
  });

  testWidgets('a person who stops at the face gives the Mac nothing', (
    tester,
  ) async {
    final keeper = _FakeKeeper()..macs['m1'] = _mac;
    final unlocker = _FakeUnlocker();
    await _show(tester, keeper, unlocker);
    await tester.tap(find.text('Unlock'));
    await tester.pumpAndSettle();
    expect(unlocker.tries, isEmpty);
    expect(find.byType(AlertDialog), findsNothing);
  });

  testWidgets('a password the Mac turns down can be entered again', (
    tester,
  ) async {
    final keeper = _FakeKeeper()..macs['m1'] = _mac;
    keeper.opens = (_) => 'old';
    final unlocker = _FakeUnlocker()..outcomes.add(const SshUnlock.rejected());
    await _show(tester, keeper, unlocker);
    await tester.tap(find.text('Unlock'));
    await tester.pumpAndSettle();
    expect(find.textContaining('wrong'), findsOneWidget);

    await tester.tap(find.text('Enter the password again'));
    await tester.pumpAndSettle();
    await tester.enterText(find.widgetWithText(TextField, 'Password'), 'new');
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(keeper.passwords['m1'], 'new');
  });

  testWidgets(
    'a Mac with another key is given nothing and cannot be trusted from here',
    (tester) async {
      final keeper = _FakeKeeper()..macs['m1'] = _mac;
      keeper.opens = (_) => 'secret';
      final unlocker = _FakeUnlocker()
        ..outcomes.add(const SshUnlock.unknownKey(fingerprint: _other));
      await _show(tester, keeper, unlocker);

      await tester.tap(find.text('Unlock'));
      await tester.pumpAndSettle();
      expect(find.text(_other), findsOneWidget);
      expect(find.textContaining('nothing was sent'), findsOneWidget);
      expect(find.textContaining('Trust'), findsNothing);
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();
      expect(keeper.macs['m1']!.keys, [_key]);
      expect(unlocker.tries, hasLength(1));
    },
  );

  testWidgets('a password that the system took away can be entered again', (
    tester,
  ) async {
    final keeper = _FakeKeeper()..macs['m1'] = _mac;
    keeper.opens = (_) =>
        const MacPasswordProblem('The saved password is gone.');
    final unlocker = _FakeUnlocker();
    await _show(tester, keeper, unlocker);
    await tester.tap(find.text('Unlock'));
    await tester.pumpAndSettle();
    expect(find.text('The saved password is gone.'), findsOneWidget);
    expect(find.text('Enter the password again'), findsOneWidget);
    expect(unlocker.tries, isEmpty);
  });

  testWidgets('a forgotten Mac is gone with its password', (tester) async {
    final keeper = _FakeKeeper()
      ..macs['m1'] = _mac
      ..passwords['m1'] = 'secret';
    await _show(tester, keeper, _FakeUnlocker());
    await tester.tap(find.byTooltip('More'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Forget'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Forget'));
    await tester.pumpAndSettle();
    expect(keeper.macs, isEmpty);
    expect(keeper.passwords, isEmpty);
    expect(find.text('No Mac saved yet.'), findsOneWidget);
  });
}
