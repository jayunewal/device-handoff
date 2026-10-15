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
        addresses: [
          '192.168.1.5:4000',
          '[fe80::1]:4000',
          '8.8.8.8:41641',
          '[2606:4700:4700::1111]:443',
        ],
        lanDiscovery: true,
      ),
    );
    final controller = _controller(
      session,
      log: [
        '2026-10-15T08:40:00.000001Z  INFO baton_app: Baton started device=f0f0f0f0f0',
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
This device: Desk Mac (Mac), id f0f0f0f0f0
Paired devices: 2
Relay: https://relay.example./
Addresses other devices can try: 192.168.1.5:4000, [fe80::1]:4000, public IPv4 (hidden):41641, public IPv6 (hidden):443
Finding devices on the local network: on

Log
2 lines, oldest first.
2026-10-15T08:40:00.000001Z  INFO baton_app: Baton started device=f0f0f0f0f0
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
        addresses: ['[fe80::1%2\u2066]:4000'],
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
    expect(report, contains('[fe80::1%2\uFFFD]:4000'));
    expect(report, contains('\na \uFFFDline\uFFFD b\n'));
  });

  test(
    'does not copy the fingerprint, only the short id of the device',
    () async {
      final session = FakeSession(me: _me());
      final controller = _controller(session);
      addTearDown(controller.dispose);

      await controller.refresh();

      final report = controller.diagnostics!.report;
      expect(report, isNot(contains('wxyz')));
      expect(report, isNot(contains('fingerprint')));
      expect(report, contains('This device: Desk Mac (Mac), id f0f0f0f0f0\n'));
    },
  );

  test('writes the account name in a path as a tilde', () async {
    final session = FakeSession(me: _me())
      ..networkFailure = const BatonError(
        kind: ErrorKind.stopped,
        message: 'Could not read /home/anna/baton.',
      );
    final controller = _controller(
      session,
      problem: 'Could not create /Users/anna/Library/Containers/io.github.jayunewal.baton/Data.',
    );
    addTearDown(controller.dispose);

    await controller.refresh();

    final report = controller.diagnostics!.report;
    expect(
      report,
      contains(
        'Problem: Could not create '
        '/Users/~/Library/Containers/io.github.jayunewal.baton/Data.\n',
      ),
    );
    expect(
      report,
      contains('Connection: could not be read (Could not read /home/~/baton.)'),
    );
    expect(report, isNot(contains('anna')));
  });

  test(
    'hides what is not an address, and a public one wherever it is listed',
    () async {
      final session = FakeSession(
        me: _me(),
        reach: const Network(
          addresses: ['not an address, 8.8.8.8', '9.9.9.9:1', '10.0.0.7:2'],
          lanDiscovery: false,
        ),
      );
      final controller = _controller(session);
      addTearDown(controller.dispose);

      await controller.refresh();

      final report = controller.diagnostics!.report;
      expect(
        report,
        contains(
          'Addresses other devices can try: unrecognised address (hidden), '
          'public IPv4 (hidden):1, 10.0.0.7:2\n',
        ),
      );
      expect(report, isNot(contains('8.8.8.8')));
      expect(report, isNot(contains('9.9.9.9')));
    },
  );

  group('describeAddress', () {
    test('shows a public address by its kind and keeps the port', () {
      for (final (address, shown) in [
        ('8.8.8.8:53', 'public IPv4 (hidden):53'),
        ('1.1.1.1', 'public IPv4 (hidden)'),
        ('8.8.8.8:65535', 'public IPv4 (hidden):65535'),
        ('[2606:4700:4700::1111]:443', 'public IPv6 (hidden):443'),
        ('2606:4700:4700::1111', 'public IPv6 (hidden)'),
        ('[2606:4700:4700::1111]', 'public IPv6 (hidden)'),
        ('::ffff:8.8.8.8', 'public IPv6 (hidden)'),
        ('[::ffff:8.8.8.8]:99', 'public IPv6 (hidden):99'),
      ]) {
        expect(describeAddress(address), shown, reason: address);
      }
    });

    test('keeps the addresses that belong to no one network', () {
      for (final address in [
        '127.0.0.1:41641',
        '[::1]:80',
        '0.0.0.0:0',
        '[::]:0',
        '10.1.2.3',
        '172.16.0.1:9',
        '172.31.255.254',
        '192.168.1.20:41641',
        '100.64.0.1',
        '100.127.255.254:1',
        '169.254.10.1',
        '[fe80::1%2]:41641',
        'fe80::abcd:1',
        'fc00::1',
        '[fd12:3456:789a::1]:5',
        '224.0.0.251:5353',
        '[ff02::fb]:5353',
        '255.255.255.255',
        '192.0.2.7',
        '198.51.100.7:3',
        '203.0.113.200',
        '2001:db8::1',
        '::ffff:192.168.1.5',
      ]) {
        expect(describeAddress(address), address, reason: address);
      }
    });

    test('shows the first address outside each private range as public', () {
      for (final address in [
        '172.15.255.255',
        '172.32.0.1',
        '100.63.255.255',
        '100.128.0.1',
        '11.0.0.1',
        '9.255.255.255',
        '169.253.1.1',
        '192.169.0.1',
        '223.255.255.255',
        '240.0.0.1',
      ]) {
        expect(
          describeAddress(address),
          'public IPv4 (hidden)',
          reason: address,
        );
      }
    });

    test(
      'does not show text that is not an address, nor a port that is not one',
      () {
        for (final text in [
          '',
          'relay.example:443',
          'localhost:80',
          '8.8.8.8:',
          '8.8.8.8:65536',
          '8.8.8.8:99999',
          '8.8.8.8:-1',
          '8.8.8.8:+5',
          '8.8.8.8:80 and more',
          '[8.8.8.8',
          '[8.8.8.8]x',
          '999.1.1.1:5',
          ' 8.8.8.8',
          'secret 8.8.8.8',
          '8.8.8.8\n',
        ]) {
          expect(
            describeAddress(text),
            'unrecognised address (hidden)',
            reason: text,
          );
        }
      },
    );

    test('shows what it keeps the way the app shows received text', () {
      expect(
        describeAddress('[fe80::1%2\u2066]:4000'),
        '[fe80::1%2\uFFFD]:4000',
      );
    });
  });

  group('hideAccountNames', () {
    test('writes the account name in a path as a tilde', () {
      expect(
        hideAccountNames(
          'could not create /Users/anna/Library/Containers/io.github.jayunewal.baton/Data: denied',
        ),
        'could not create /Users/~/Library/Containers/io.github.jayunewal.baton/Data: denied',
      );
      expect(
        hideAccountNames(
          'error="/home/anna.b/baton-app/identity" and /home/zoe, ok',
        ),
        'error="/home/~/baton-app/identity" and /home/~, ok',
      );
    });

    test('takes a Windows name with its spaces', () {
      expect(
        hideAccountNames(r'C:\Users\Anna Berg\AppData\Local\baton'),
        r'C:\Users\~\AppData\Local\baton',
      );
      // As a path looks when it is printed with its backslashes doubled.
      expect(
        hideAccountNames(r'path: "C:\\Users\\Anna Berg\\AppData"'),
        r'path: "C:\\Users\\~\\AppData"',
      );
      expect(hideAccountNames('C:/Users/anna/AppData'), 'C:/Users/~/AppData');
    });

    test('leaves a path that names no account', () {
      for (final text in [
        '/private/var/mobile/Containers/Data/Application/1234/Library',
        '/Users/',
        '/Users/~/Library',
        'the Users folder and /home alone',
        r'C:\Users\',
      ]) {
        expect(hideAccountNames(text), text, reason: text);
      }
    });

    test('takes a name in other alphabets whole', () {
      expect(hideAccountNames('Zoë’s /home/é/x'), 'Zoë’s /home/~/x');
      expect(hideAccountNames('/Users/Zoë'), '/Users/~');
    });
  });

  test('gives the first ten digits of an id as its short form', () {
    expect(shortId('f0' * 32), 'f0f0f0f0f0');
    expect(shortId('abc'), 'abc');
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
