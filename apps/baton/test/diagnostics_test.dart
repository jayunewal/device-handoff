// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/backend.dart';
import 'package:baton/src/diagnostics.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fake_backend.dart';

final _taken = DateTime.utc(2026, 10, 15, 8, 44, 54, 321);

Me _me([String name = 'Desk Mac']) => Me(
  id: 'f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0',
  name: name,
  platform: Platform.macOs,
  fingerprint: 'wxyz-abcd-efgh-ijkl-mnop',
);

Device _device(String name) => Device(
  id: name.padRight(64, '0'),
  name: name,
  platform: Platform.ios,
  fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
  pairedAt: 0,
);

DiagnosticsController _controller(
  FakeSession? session, {
  List<String> log = const [],
  String? problem,
}) => DiagnosticsController(
  session: session,
  recentLog: () => log,
  system: 'macos Version 14.5 (Build 23F79)',
  problem: problem,
  now: () => _taken,
);

void main() {
  test('gives the facts and then the log, as one text to copy', () async {
    final session = FakeSession(
      me: _me(),
      pairedDevices: [_device('Anna'), _device('Ben')],
      reach: const Network(
        relay: 'https://relay.example./',
        addresses: ['192.168.1.5:4000', '[fe80::1]:4000'],
        lanDiscovery: true,
      ),
    );
    final controller = _controller(
      session,
      log: [
        '2026-10-15T08:40:00.000001Z  INFO baton_app: Baton started device=f0f0f0f0',
        '2026-10-15T08:40:01.000002Z  INFO baton_core: Baton is listening',
      ],
    );
    addTearDown(controller.dispose);
    expect(controller.diagnostics, isNull, reason: 'nothing is read yet');

    await controller.refresh();

    expect(controller.diagnostics!.report, '''
Baton diagnostics

Taken: 2026-10-15T08:44:54Z
System: macos Version 14.5 (Build 23F79)
This device: Desk Mac (Mac), fingerprint wxyz-abcd-efgh-ijkl-mnop
Paired devices: 2
Relay: https://relay.example./
Addresses other devices can try: 192.168.1.5:4000, [fe80::1]:4000
Finding devices on the local network: on

Log
2 lines, oldest first.
2026-10-15T08:40:00.000001Z  INFO baton_app: Baton started device=f0f0f0f0
2026-10-15T08:40:01.000002Z  INFO baton_core: Baton is listening
''');
  });

  test(
    'says so when there is no relay, no address and nothing logged',
    () async {
      final session = FakeSession(
        me: _me(),
        reach: const Network(addresses: [], lanDiscovery: false),
      );
      final controller = _controller(session);
      addTearDown(controller.dispose);

      await controller.refresh();

      final report = controller.diagnostics!.report;
      expect(report, contains('Paired devices: 0\n'));
      expect(report, contains('Relay: not connected\n'));
      expect(report, contains('Addresses other devices can try: none found\n'));
      expect(report, contains('Finding devices on the local network: off\n'));
      expect(report, endsWith('\nLog\nNothing has been logged.\n'));
    },
  );

  test('counts one line as one line', () async {
    final controller = _controller(null, log: ['only']);
    addTearDown(controller.dispose);

    await controller.refresh();

    expect(controller.diagnostics!.logSummary, '1 line, oldest first.');
  });

  test(
    'can be read when Baton could not start, and says what went wrong',
    () async {
      final controller = _controller(
        null,
        problem: 'Another Baton already has this device’s folder open.',
        log: ['2026-10-15T08:40:00Z ERROR baton_app: could not start'],
      );
      addTearDown(controller.dispose);

      await controller.refresh();

      expect(controller.diagnostics!.report, '''
Baton diagnostics

Taken: 2026-10-15T08:44:54Z
System: macos Version 14.5 (Build 23F79)
Problem: Another Baton already has this device’s folder open.
Baton is not running.

Log
1 line, oldest first.
2026-10-15T08:40:00Z ERROR baton_app: could not start
''');
    },
  );

  test(
    'says what the library said when the connection cannot be read',
    () async {
      final session = FakeSession(me: _me())
        ..networkFailure = const BatonError(
          kind: ErrorKind.stopped,
          message: 'Baton has stopped.',
        );
      final controller = _controller(session);
      addTearDown(controller.dispose);

      await controller.refresh();

      final facts = controller.diagnostics!.facts;
      expect(
        facts,
        contains('Connection: could not be read (Baton has stopped.)'),
      );
      expect(
        facts.where((fact) => fact.startsWith('Relay')),
        isEmpty,
        reason: 'a half-read connection would mislead',
      );
    },
  );

  test(
    'names a failure that is not the library’s by its kind and no more',
    () async {
      final session = FakeSession(me: _me())
        ..networkFailure = StateError('secret from 192.168.0.7');
      final controller = _controller(session);
      addTearDown(controller.dispose);

      await controller.refresh();

      final report = controller.diagnostics!.report;
      expect(report, contains('Connection: could not be read (StateError)'));
      expect(report, isNot(contains('secret')));
      expect(report, contains('Log\nNothing has been logged.'));
    },
  );

  test('shows characters that act on a screen as replacement marks', () async {
    final session = FakeSession(
      me: _me('Desk\u202E Mac'),
      reach: const Network(
        relay: 'https://relay\u0007.example./',
        addresses: ['192.168.1.5\u2066:4000'],
        lanDiscovery: true,
      ),
    );
    final controller = DiagnosticsController(
      session: session,
      recentLog: () => ['a \u202Eline\u0085 b'],
      system: 'odd\u001B[31m system',
      problem: 'it\u202D broke',
      now: () => _taken,
    );
    addTearDown(controller.dispose);

    await controller.refresh();

    final report = controller.diagnostics!.report;
    for (final hidden in [
      '\u0007',
      '\u001B',
      '\u0085',
      '\u202D',
      '\u202E',
      '\u2066',
    ]) {
      expect(
        report,
        isNot(contains(hidden)),
        reason: 'U+${hidden.codeUnitAt(0).toRadixString(16)}',
      );
    }
    expect(report, contains('System: odd\uFFFD[31m system'));
    expect(report, contains('Problem: it\uFFFD broke'));
    expect(report, contains('This device: Desk\uFFFD Mac (Mac)'));
    expect(report, contains('Relay: https://relay\uFFFD.example./'));
    expect(report, contains('192.168.1.5\uFFFD:4000'));
    expect(report, contains('\na \uFFFDline\uFFFD b\n'));
  });

  test('a read that a newer one overtook does not tell anyone', () async {
    final session = FakeSession(me: _me())..holdDevices();
    final controller = _controller(session);
    addTearDown(controller.dispose);
    var told = 0;
    controller.addListener(() => told++);

    final first = controller.refresh();
    final second = controller.refresh();
    session.releaseDevices();
    await Future.wait([first, second]);

    expect(told, 1);
    expect(controller.diagnostics, isNotNull);
  });

  test('a read that ends after the screen is gone does not tell it', () async {
    final session = FakeSession(me: _me())..holdDevices();
    final controller = _controller(session);

    final read = controller.refresh();
    controller.dispose();
    session.releaseDevices();
    await read;

    expect(controller.diagnostics, isNull);
  });

  test('describes the system on one line', () {
    final system = describeSystem();

    expect(system, isNotEmpty);
    expect(system, isNot(contains('\n')));
    expect(system, isNot(contains('  ')));
  });
}
