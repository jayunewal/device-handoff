// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

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

  /// The addresses that were asked for their key, with the port.
  final asked = <(String, int)>[];

  /// When set, [key] answers only when this does.
  Completer<SshKeyCheck>? hold;

  @override
  Future<SshKeyCheck> key(String host, int port) async {
    asked.add((host, port));
    return hold?.future ?? heard;
  }

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

/// A clock that is read twice by a check of the way, and that says the second time that [took]
/// has gone by.
class _Clock {
  _Clock(this.took);

  final Duration took;
  var _reads = 0;

  DateTime call() =>
      DateTime.utc(2026, 11, 10).add(_reads++ == 0 ? Duration.zero : took);
}

Future<void> _show(
  WidgetTester tester,
  _FakeKeeper keeper,
  _FakeUnlocker unlocker, {
  DateTime Function()? now,
}) async {
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
        now: now ?? DateTime.now,
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

  testWidgets(
    'a refusal after an earlier no-answer is not saved without trying',
    (tester) async {
      final keeper = _FakeKeeper();
      final unlocker = _FakeUnlocker()
        ..outcomes.add(const SshUnlock.unreachable(detail: 'gone'))
        ..outcomes.add(const SshUnlock.rejected());
      await _show(tester, keeper, unlocker);
      await fillAndCheck(tester);
      await tester.tap(find.text('They match. Save'));
      await tester.pumpAndSettle();
      expect(find.text('Save without trying'), findsOneWidget);
      await tester.tap(find.text('They match. Save'));
      await tester.pumpAndSettle();
      expect(find.textContaining('wrong. Nothing was saved.'), findsOneWidget);
      expect(find.text('Save without trying'), findsNothing);
      expect(keeper.macs, isEmpty);
    },
  );

  testWidgets('a new check takes away the offer to save without trying', (
    tester,
  ) async {
    final keeper = _FakeKeeper();
    final unlocker = _FakeUnlocker()
      ..outcomes.add(const SshUnlock.unreachable(detail: 'gone'));
    await _show(tester, keeper, unlocker);
    await fillAndCheck(tester);
    await tester.tap(find.text('They match. Save'));
    await tester.pumpAndSettle();
    expect(find.text('Save without trying'), findsOneWidget);
    await tester.enterText(
      find.widgetWithText(TextField, 'Address of the Mac'),
      'other.local',
    );
    await tester.pump();
    await tester.tap(find.text('Check the Mac'));
    await tester.pumpAndSettle();
    expect(find.text('Save without trying'), findsNothing);
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

  testWidgets('what a new Mac answered with is shown plainly', (tester) async {
    final keeper = _FakeKeeper();
    final unlocker = _FakeUnlocker()
      ..heard = const SshKeyCheck.failed(detail: 'not ssh\u202E');
    await _show(tester, keeper, unlocker);
    await fillAndCheck(tester);

    expect(
      find.textContaining(
        'Something answered, but not as a Mac with Remote Login does '
        '(not ssh\u{FFFD}).',
      ),
      findsOneWidget,
    );
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
      expect(
        find.text(
          'Something else on the network may be answering for your Mac, or a Mac that is '
          'still locked after a restart may answer with a key of its own.',
        ),
        findsOneWidget,
      );
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

  testWidgets(
    'a running Mac that takes keys only can be saved without the try',
    (tester) async {
      final keeper = _FakeKeeper();
      final unlocker = _FakeUnlocker()
        ..outcomes.add(const SshUnlock.noPassword());
      await _show(tester, keeper, unlocker);
      await fillAndCheck(tester);
      await tester.tap(find.text('They match. Save'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('does not take a password over SSH while it runs'),
        findsOneWidget,
      );
      expect(keeper.macs, isEmpty);

      await tester.tap(find.text('Save without trying'));
      await tester.pumpAndSettle();
      expect(
        keeper.macs['new'],
        const SavedMac(
          id: 'new',
          host: 'mac.local',
          account: 'jay',
          keys: [_key],
        ),
      );
      expect(keeper.passwords['new'], 'typo');
      expect(unlocker.tries, hasLength(1));
    },
  );

  testWidgets('a Mac that does not answer an unlock is pointed to the check', (
    tester,
  ) async {
    final keeper = _FakeKeeper()..macs['m1'] = _mac;
    keeper.opens = (_) => 'secret';
    final unlocker = _FakeUnlocker()
      ..outcomes.add(const SshUnlock.unreachable(detail: 'gone'));
    await _show(tester, keeper, unlocker);
    await tester.tap(find.text('Unlock'));
    await tester.pumpAndSettle();
    expect(find.text('Could not unlock Jays-MBP.local'), findsOneWidget);
    expect(
      find.textContaining(
        '"Check the way" in the menu of the Mac says whether it can be '
        'reached from here.',
      ),
      findsOneWidget,
    );
  });

  testWidgets(
    'a Mac that takes keys only is not told to turn Remote Login on',
    (tester) async {
      final keeper = _FakeKeeper()..macs['m1'] = _mac;
      keeper.opens = (_) => 'secret';
      final unlocker = _FakeUnlocker()
        ..outcomes.add(const SshUnlock.noPassword());
      await _show(tester, keeper, unlocker);
      await tester.tap(find.text('Unlock'));
      await tester.pumpAndSettle();

      expect(find.text('Could not unlock Jays-MBP.local'), findsOneWidget);
      expect(
        find.textContaining(
          'it takes keys only, so it did not take the password.',
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining('That is expected while the Mac runs.'),
        findsOneWidget,
      );
      expect(
        find.textContaining(
          'If it is waiting for its disk password after a restart, this way '
          'cannot unlock it, and it has to be unlocked at the Mac.',
        ),
        findsOneWidget,
      );
      // The Mac answered with the key that was saved, so Remote Login is on and the person has
      // nothing to turn on.
      expect(
        find.descendant(
          of: find.byType(AlertDialog),
          matching: find.textContaining('Remote Login'),
        ),
        findsNothing,
      );
      expect(find.text('Enter the password again'), findsNothing);
    },
  );

  testWidgets('shows what went wrong with a Mac plainly', (tester) async {
    final keeper = _FakeKeeper()..macs['m1'] = _mac;
    keeper.opens = (_) => 'secret';
    final unlocker = _FakeUnlocker()
      ..outcomes.add(const SshUnlock.failed(detail: 'not ssh\u202E'));
    await _show(tester, keeper, unlocker);
    await tester.tap(find.text('Unlock'));
    await tester.pumpAndSettle();

    expect(
      find.text(
        'Something went wrong while talking to the Mac (not ssh\u{FFFD}).',
      ),
      findsOneWidget,
    );
  });

  group('the check of the way', () {
    Future<void> checkWay(WidgetTester tester) async {
      await tester.tap(find.byTooltip('More'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Check the way'));
      await tester.pumpAndSettle();
    }

    testWidgets('says that the way is open, and sends nothing', (tester) async {
      final keeper = _FakeKeeper()..macs['m1'] = _mac;
      final unlocker = _FakeUnlocker();
      await _show(
        tester,
        keeper,
        unlocker,
        now: _Clock(const Duration(milliseconds: 340)).call,
      );
      await checkWay(tester);

      expect(find.text('The way is open'), findsOneWidget);
      expect(
        find.textContaining('Reached Jays-MBP.local in 340 ms.'),
        findsOneWidget,
      );
      expect(unlocker.asked, [('Jays-MBP.local', 22)]);
      // The face was not asked for, and no password was given.
      expect(keeper.reasons, isEmpty);
      expect(unlocker.tries, isEmpty);
    });

    testWidgets('promises the way and no more than the way', (tester) async {
      final keeper = _FakeKeeper()..macs['m1'] = _mac;
      await _show(tester, keeper, _FakeUnlocker());
      await checkWay(tester);

      expect(
        find.textContaining('so the way from here works.'),
        findsOneWidget,
      );
      // After a restart a Mac may answer with a key of its own, which this check cannot know.
      expect(
        find.textContaining(
          'A Mac that waits for its disk password after a restart may answer '
          'with a key of its own.',
        ),
        findsOneWidget,
      );
      expect(find.textContaining('would reach your Mac'), findsNothing);
    });

    for (final (milliseconds, said) in [
      (0, '0 ms'),
      (999, '999 ms'),
      (1000, '1.0 s'),
      (1432, '1.4 s'),
      (12040, '12.0 s'),
    ]) {
      testWidgets('tells $milliseconds ms as $said', (tester) async {
        final keeper = _FakeKeeper()..macs['m1'] = _mac;
        await _show(
          tester,
          keeper,
          _FakeUnlocker(),
          now: _Clock(Duration(milliseconds: milliseconds)).call,
        );
        await checkWay(tester);
        expect(
          find.textContaining('Reached Jays-MBP.local in $said.'),
          findsOneWidget,
        );
      });
    }

    testWidgets(
      'shows a key that was not saved, and offers no way to trust it',
      (tester) async {
        final keeper = _FakeKeeper()..macs['m1'] = _mac;
        final unlocker = _FakeUnlocker()
          ..heard = const SshKeyCheck.heard(fingerprint: _other);
        await _show(tester, keeper, unlocker);
        await checkWay(tester);

        expect(find.text('The Mac answered with another key'), findsOneWidget);
        expect(find.text(_other), findsOneWidget);
        expect(find.text('The way is open'), findsNothing);
        expect(find.textContaining('Trust'), findsNothing);
        // The reasons are the ones that the unlock gives for the same answer.
        expect(
          find.text(
            'Something else on the network may be answering for your Mac, or a Mac that is '
            'still locked after a restart may answer with a key of its own.',
          ),
          findsOneWidget,
        );
        await tester.tap(find.text('OK'));
        await tester.pumpAndSettle();
        expect(keeper.macs['m1']!.keys, [_key]);
        expect(unlocker.tries, isEmpty);
      },
    );

    testWidgets('knows a saved key by its text, whatever surrounds it', (
      tester,
    ) async {
      final keeper = _FakeKeeper()
        ..macs['m1'] = _mac.copyWith(keys: [_other, ' $_key\n']);
      await _show(tester, keeper, _FakeUnlocker());
      await checkWay(tester);
      expect(find.text('The way is open'), findsOneWidget);
    });

    testWidgets('names what to look at when nothing answers', (tester) async {
      final keeper = _FakeKeeper()..macs['m1'] = _mac;
      final unlocker = _FakeUnlocker()
        ..heard = const SshKeyCheck.unreachable(detail: 'gone');
      await _show(tester, keeper, unlocker);
      await checkWay(tester);

      expect(find.text('No answer'), findsOneWidget);
      expect(
        find.textContaining('Nothing answered at Jays-MBP.local.'),
        findsOneWidget,
      );
      expect(
        find.textContaining('Away from home, check that your private network'),
        findsOneWidget,
      );
    });

    testWidgets('shows what another device said, and the address, plainly', (
      tester,
    ) async {
      final keeper = _FakeKeeper()
        ..macs['m1'] = _mac.copyWith(host: 'evil\u202Ehost');
      final unlocker = _FakeUnlocker()
        ..heard = const SshKeyCheck.failed(detail: 'not ssh\u202E');
      await _show(tester, keeper, unlocker);
      await checkWay(tester);

      expect(find.text('Not a Mac with Remote Login'), findsOneWidget);
      expect(
        find.textContaining(
          'Something answered at evil\u{FFFD}host, but not as a Mac with '
          'Remote Login does (not ssh\u{FFFD}).',
        ),
        findsOneWidget,
      );
    });

    testWidgets('shows nothing when the screen was closed meanwhile', (
      tester,
    ) async {
      final keeper = _FakeKeeper()..macs['m1'] = _mac;
      final unlocker = _FakeUnlocker()..hold = Completer<SshKeyCheck>();
      await _show(tester, keeper, unlocker);
      await tester.tap(find.byTooltip('More'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Check the way'));
      await tester.pump(const Duration(milliseconds: 500));

      await tester.pumpWidget(const MaterialApp(home: SizedBox()));
      unlocker.hold!.complete(const SshKeyCheck.heard(fingerprint: _key));
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsNothing);
      expect(tester.takeException(), isNull);
    });

    testWidgets('holds the Mac while it waits, and lets it go after', (
      tester,
    ) async {
      final keeper = _FakeKeeper()..macs['m1'] = _mac;
      final unlocker = _FakeUnlocker()..hold = Completer<SshKeyCheck>();
      await _show(tester, keeper, unlocker);

      await tester.tap(find.byTooltip('More'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Check the way'));
      // Long enough for the menu that was tapped to be gone.
      await tester.pump(const Duration(milliseconds: 500));
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      expect(find.text('Unlock'), findsNothing);
      // The menu does not open while the Mac is being asked.
      await tester.tap(find.byTooltip('More'));
      await tester.pump(const Duration(milliseconds: 500));
      expect(find.text('Check the way'), findsNothing);
      expect(unlocker.asked, hasLength(1));

      unlocker.hold!.complete(const SshKeyCheck.heard(fingerprint: _key));
      await tester.pumpAndSettle();
      expect(find.text('The way is open'), findsOneWidget);
      await tester.tap(find.text('OK'));
      await tester.pumpAndSettle();
      expect(find.byType(CircularProgressIndicator), findsNothing);
      expect(find.text('Unlock'), findsOneWidget);
    });
  });
}
