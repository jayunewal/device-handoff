// SPDX-License-Identifier: Apache-2.0

// The Rust library through the bridge, as the app uses it: two devices on this computer pair,
// hand each other things and stop. Nothing is faked, so these tests are what shows that the
// generated code, the library and the way the app calls it fit together.
//
// They need the library built for this computer:
//
//     cargo build -p baton-ffi
//
// and, if it is not in the usual place (`../../target/debug`), the path to it in the
// BATON_FFI_LIB environment variable.

import 'dart:async';
import 'dart:io';

import 'package:baton/src/rust/api/baton.dart';
import 'package:baton/src/rust/api/error.dart';
import 'package:baton/src/rust/api/model.dart' hide Platform;
import 'package:baton/src/rust/api/startup.dart';
import 'package:baton/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support.dart';

/// How long to wait for something that should happen at once.
const _wait = Duration(seconds: 30);

void main() {
  setUpAll(() async {
    await RustLib.init(externalLibrary: ExternalLibrary.open(libraryPath()));
  });

  test('the name is chosen before Baton starts', () async {
    final folder = await _folder(name: null);
    expect(await deviceName(dataDir: folder.path), isNull);

    // A name with nothing visible in it is the person's to fix, and the message can be shown
    // as it is.
    for (final name in ['', '  ', '\u200B']) {
      await expectLater(
        setDeviceName(dataDir: folder.path, name: name),
        throwsA(_batonError(ErrorKind.badName)),
      );
    }
    expect(await deviceName(dataDir: folder.path), isNull);

    await setDeviceName(dataDir: folder.path, name: 'Kitchen iPad');
    expect(await deviceName(dataDir: folder.path), 'Kitchen iPad');
    final baton = await _start(folder);
    expect(baton.me().name, 'Kitchen iPad');
    expect(baton.me().id, hasLength(64));
    expect(baton.me().fingerprint, matches(RegExp(r'^(\w{4}-){4}\w{4}$')));
    final network = await baton.network();
    expect(network.lanDiscovery, isFalse, reason: 'switched off for this run');
  });

  test('two devices pair and hand each other things', () async {
    final a = await _start(await _folder(name: 'Device A'));
    final b = await _start(await _folder(name: 'Device B'));
    expect(await a.devices(), isEmpty, reason: 'a new Baton has no devices');
    final aEvents = _Feed(await a.events());
    addTearDown(aEvents.close);

    final offer = await a.offerPairing();
    expect(offer.ticket, startsWith('BATON1:'));
    expect(
      offer.expiresAt,
      greaterThan(DateTime.now().millisecondsSinceEpoch ~/ 1000),
    );

    // The joiner pastes the whole command, as people do.
    final join = _Feed(
      b.joinPairing(ticket: 'baton pair join ${offer.ticket}\n'),
    );
    addTearDown(join.close);
    final shown = await join.next<JoinProgress_Code>();
    final asked = await aEvents.next<Event_PairingRequested>();
    expect(
      asked.request.code,
      shown.code,
      reason: 'both screens show the same',
    );
    expect(asked.request.deviceId, b.me().id);
    expect(asked.request.fingerprint, b.me().fingerprint);
    expect(asked.request.name, 'Device B');

    expect(a.answerPairing(request: asked.request.id, accept: true), isTrue);
    expect(
      a.answerPairing(request: asked.request.id, accept: true),
      isFalse,
      reason: 'an answer counts once',
    );
    final paired = await join.next<JoinProgress_Paired>();
    expect(paired.device.id, a.me().id);
    await join.ended;
    final ended = await aEvents.next<Event_OfferEnded>();
    expect(
      ended.outcome,
      isA<OfferOutcome_Paired>().having(
        (outcome) => outcome.device.id,
        'device id',
        b.me().id,
      ),
    );
    expect((await a.devices()).map((d) => d.id), [b.me().id]);
    expect((await b.devices()).map((d) => d.id), [a.me().id]);
    expect((await a.devices()).single.name, 'Device B');

    // B hands A a link and then some text, and A's screen hears of both.
    final reach = await b.ping(id: a.me().id);
    expect(reach.route, isNot(Route.relay));
    await b.sendLink(
      id: a.me().id,
      url: 'https://example.com/path?q=1',
      title: 'An example',
    );
    final link = (await aEvents.next<Event_Received>()).received;
    expect(link.fromName, 'Device B');
    expect(link.fromId, b.me().id);
    expect(
      link.item,
      isA<ReceivedItem_Link>()
          .having((item) => item.url, 'url', 'https://example.com/path?q=1')
          .having((item) => item.title, 'title', 'An example'),
    );
    await b.sendText(id: a.me().id, text: 'hello from B');
    final text = (await aEvents.next<Event_Received>()).received;
    expect(
      text.item,
      isA<ReceivedItem_Text>().having(
        (item) => item.text,
        'text',
        'hello from B',
      ),
    );

    // The inbox keeps both, newest first, until they are removed.
    expect((await a.inbox()).map((r) => r.number), [text.number, link.number]);
    expect(await a.removeReceived(number: text.number), isTrue);
    expect(await a.removeReceived(number: text.number), isFalse);
    expect((await a.inbox()).map((r) => r.number), [link.number]);
    expect(await a.clearInbox(), 1);
    expect(await a.inbox(), isEmpty);

    // Forgetting a device is heard of too.
    await a.unpair(id: b.me().id);
    await aEvents.next<Event_DevicesChanged>();
    expect(await a.devices(), isEmpty);
    await expectLater(
      a.unpair(id: b.me().id),
      throwsA(_batonError(ErrorKind.notPaired)),
    );
  });

  test(
    'a join that cannot work comes back as a value, not as a crash',
    () async {
      final a = await _start(await _folder(name: 'Device A'));
      final b = await _start(await _folder(name: 'Device B'));

      final notATicket = _Feed(a.joinPairing(ticket: 'hello'));
      addTearDown(notATicket.close);
      final bad = await notATicket.next<JoinProgress_Failed>();
      expect(bad.error.kind, ErrorKind.badTicket);
      expect(bad.error.message, isNot(contains('\n')));
      await notATicket.ended;

      // A device that is turned down is told so, and nothing is kept.
      final events = _Feed(await a.events());
      addTearDown(events.close);
      final offer = await a.offerPairing();
      final join = _Feed(b.joinPairing(ticket: offer.ticket));
      addTearDown(join.close);
      final asked = await events.next<Event_PairingRequested>();
      expect(a.answerPairing(request: asked.request.id, accept: false), isTrue);
      final refused = await join.next<JoinProgress_Failed>();
      expect(refused.error.kind, ErrorKind.pairingFailed);
      final ended = await events.next<Event_OfferEnded>();
      expect(ended.outcome, isA<OfferOutcome_Declined>());
      expect(await a.devices(), isEmpty);
      expect(await b.devices(), isEmpty);
    },
  );

  test('an offer can be withdrawn, and the screen hears that it was', () async {
    final a = await _start(await _folder(name: 'Device A'));
    final events = _Feed(await a.events());
    addTearDown(events.close);
    await a.offerPairing();
    a.cancelOffer();
    final ended = await events.next<Event_OfferEnded>();
    expect(ended.outcome, isA<OfferOutcome_Cancelled>());
  });

  test('what cannot be sent comes back as an error with a kind', () async {
    final a = await _start(await _folder(name: 'Device A'));
    final nobody = '0' * 64;
    await expectLater(
      a.sendText(id: nobody, text: 'hello'),
      throwsA(_batonError(ErrorKind.notPaired)),
    );
    await expectLater(
      a.sendText(id: nobody, text: '   '),
      throwsA(_batonError(ErrorKind.badItem)),
    );
    await expectLater(
      a.sendLink(id: nobody, url: 'example.com'),
      throwsA(_batonError(ErrorKind.badItem)),
    );
    await expectLater(
      a.ping(id: 'not an id'),
      throwsA(_batonError(ErrorKind.notPaired)),
    );
    // A negative number is no inbox item.
    expect(await a.removeReceived(number: -1), isFalse);
  });

  test('stopping ends the event stream and lets go of the folder', () async {
    final folder = await _folder(name: 'Device A');
    final first = await _start(folder);
    final id = first.me().id;
    final events = _Feed(await first.events());
    addTearDown(events.close);

    // One folder, one running Baton.
    await expectLater(
      Baton.start(dataDir: folder.path),
      throwsA(_batonError(ErrorKind.alreadyRunning)),
    );

    await first.stop();
    await events.ended;
    await expectLater(first.inbox(), throwsA(_batonError(ErrorKind.stopped)));
    await expectLater(
      first.offerPairing(),
      throwsA(_batonError(ErrorKind.stopped)),
    );
    // Stopping again is fine, from wherever it is asked.
    await Future.wait([first.stop(), first.stop()]);

    // The same device starts again with the same identity.
    final again = await _start(folder);
    expect(again.me().id, id);
  });

  test('a Baton that is let go of without being stopped frees its folder', () async {
    final folder = await _folder(name: 'Device A');
    // Not through _start, which would stop it again.
    final first = await Baton.start(
      dataDir: folder.path,
      lanDiscovery: false,
      portMapping: false,
    );
    final events = _Feed(await first.events());
    addTearDown(events.close);

    // This is what an app does when it is closed or restarted without calling stop. The
    // stream of events must not keep the Baton, and with it the folder, alive.
    first.dispose();

    // What runs in the background ends a moment after the last reference is dropped.
    final limit = DateTime.now().add(const Duration(seconds: 10));
    while (true) {
      try {
        await _start(folder);
        break;
      } on BatonError catch (error) {
        if (error.kind != ErrorKind.alreadyRunning ||
            DateTime.now().isAfter(limit)) {
          rethrow;
        }
        await Future<void>.delayed(const Duration(milliseconds: 20));
      }
    }
    await events.ended;
  });

  test(
    'what the library logged can be read, for the Diagnostics screen',
    () async {
      await _start(await _folder(name: 'Device A'));

      final log = recentLog();

      expect(
        log.where((line) => line.contains('Baton started')),
        isNotEmpty,
        reason: log.join('\n'),
      );
      for (final line in log) {
        expect(line, isNot(contains('\n')));
        expect(line.length, lessThanOrEqualTo(601));
      }
    },
  );

  test(
    'files are sent when the person says yes, and arrive as they were',
    () async {
      final received = await _tempDir('baton-bridge-received-');
      final a = await _start(
        await _folder(name: 'Device A'),
        receiveDir: received,
      );
      final b = await _start(await _folder(name: 'Device B'));
      expect(a.receiving(), isA<Receiving_On>());
      expect(b.receiving(), isA<Receiving_Off>());
      final events = _Feed(await a.events());
      addTearDown(events.close);
      await _pair(a, events, b);

      final sending = await b.sendFiles(
        id: a.me().id,
        paths: await _sampleFiles(),
      );

      // The person is asked who wants to send what, and nothing is kept before they answer.
      final request = (await events.next<Event_TransferRequested>()).request;
      expect(request.deviceName, 'Device B');
      expect(request.deviceId, b.me().id);
      expect(
        (request.items, request.files, request.folders, request.bytes),
        (2, 3, 2, 100017),
      );
      expect(
        request.preview.map(
          (item) => (item.name, item.folder, item.files, item.bytes),
        ),
        [('docs', true, 2, 100005), ('hello.txt', false, 0, 12)],
      );
      expect(_tree(received), isEmpty, reason: 'nothing is kept before yes');

      expect(a.answerTransfer(request: request.id, accept: true), isTrue);
      expect(
        a.answerTransfer(request: request.id, accept: true),
        isFalse,
        reason: 'an answer counts once',
      );

      final sent = await sending.outcome();
      expect(
        (sent.files, sent.bytes, sent.failed, sent.leftOut),
        (3, 100017, 0, 0),
      );
      final end = (await events.next<Event_TransferEnded>()).end;
      expect(end.request, request.id);
      expect(
        end.result,
        isA<TransferResult_Received>().having(
          (result) => result.failed,
          'failed',
          0,
        ),
      );
      expect((end.deviceName, end.files, end.bytes), ('Device B', 3, 100017));
      expect(_tree(received), _sampleTree());

      // The sender's screen has what it needs to show a finished bar.
      final progress = sending.progress();
      expect(progress.stage, SendStage.over);
      expect(
        (progress.files, progress.bytes, progress.sent),
        (3, 100017, 100017),
      );
    },
  );

  test(
    'files the person declines are not kept, and the sender is told',
    () async {
      final received = await _tempDir('baton-bridge-received-');
      final a = await _start(
        await _folder(name: 'Device A'),
        receiveDir: received,
      );
      final b = await _start(await _folder(name: 'Device B'));
      final events = _Feed(await a.events());
      addTearDown(events.close);
      await _pair(a, events, b);

      final sending = await b.sendFiles(
        id: a.me().id,
        paths: await _sampleFiles(),
      );
      final request = (await events.next<Event_TransferRequested>()).request;
      expect(a.answerTransfer(request: request.id, accept: false), isTrue);

      await expectLater(
        sending.outcome(),
        throwsA(
          _batonError(ErrorKind.declined).having(
            (error) => error.message,
            'message',
            contains('Device A did not take the files'),
          ),
        ),
      );
      expect(_tree(received), isEmpty, reason: 'a no keeps nothing');
    },
  );

  test('a sender that gives up takes the question off the screen', () async {
    final received = await _tempDir('baton-bridge-received-');
    final a = await _start(
      await _folder(name: 'Device A'),
      receiveDir: received,
    );
    final b = await _start(await _folder(name: 'Device B'));
    final events = _Feed(await a.events());
    addTearDown(events.close);
    await _pair(a, events, b);
    final sending = await b.sendFiles(
      id: a.me().id,
      paths: await _sampleFiles(),
    );
    final request = (await events.next<Event_TransferRequested>()).request;

    sending.cancel();

    final taken = await events.next<Event_TransferRequestEnded>();
    expect(taken.id, request.id);
    expect(
      a.answerTransfer(request: request.id, accept: true),
      isFalse,
      reason: 'a question that was taken back cannot be answered',
    );
    await expectLater(
      sending.outcome(),
      throwsA(_batonError(ErrorKind.cancelled)),
    );
    expect(_tree(received), isEmpty);
  });

  test('a send that is given up while it goes leaves no files', () async {
    final received = await _tempDir('baton-bridge-received-');
    final a = await _start(
      await _folder(name: 'Device A'),
      receiveDir: received,
    );
    final b = await _start(await _folder(name: 'Device B'));
    final events = _Feed(await a.events());
    addTearDown(events.close);
    await _pair(a, events, b);
    // Large enough that it cannot be over before the cancel is made. It is a hole in the file
    // system, so it costs nothing to make.
    final source = await _tempDir('baton-bridge-source-');
    final big = await File('${source.path}/big.bin').open(mode: FileMode.write);
    await big.truncate(64 << 20);
    await big.close();
    final sending = await b.sendFiles(
      id: a.me().id,
      paths: ['${source.path}/big.bin'],
    );
    final request = (await events.next<Event_TransferRequested>()).request;
    expect(a.answerTransfer(request: request.id, accept: true), isTrue);

    final limit = DateTime.now().add(_wait);
    while (sending.progress().sent == 0) {
      expect(
        DateTime.now().isBefore(limit),
        isTrue,
        reason: 'no byte was sent',
      );
      await Future<void>.delayed(const Duration(milliseconds: 2));
    }
    expect(sending.progress().stage, SendStage.sending);
    sending.cancel();

    await expectLater(
      sending.outcome(),
      throwsA(_batonError(ErrorKind.cancelled)),
    );
    final end = (await events.next<Event_TransferEnded>()).end;
    expect(end.request, request.id);
    expect(end.result, isA<TransferResult_Dropped>());
    expect(_tree(received), isEmpty, reason: 'half a file is never kept');
  });

  test('a device that takes no files says so to the sender', () async {
    final a = await _start(await _folder(name: 'Device A'));
    final b = await _start(await _folder(name: 'Device B'));
    final events = _Feed(await a.events());
    addTearDown(events.close);
    await _pair(a, events, b);
    expect(a.receiving(), isA<Receiving_Off>());

    final sending = await b.sendFiles(
      id: a.me().id,
      paths: await _sampleFiles(),
    );

    await expectLater(
      sending.outcome(),
      throwsA(
        _batonError(ErrorKind.declined).having(
          (error) => error.message,
          'message',
          contains('Device A did not take the files'),
        ),
      ),
    );
  });

  test(
    'a folder that cannot be used leaves the device taking no files',
    () async {
      final nowhere = await _tempDir('baton-bridge-received-');
      final a = await _start(
        await _folder(name: 'Device A'),
        receiveDir: Directory('${nowhere.path}/not-there'),
      );
      final b = await _start(await _folder(name: 'Device B'));
      final events = _Feed(await a.events());
      addTearDown(events.close);
      await _pair(a, events, b);
      expect(
        a.receiving(),
        isA<Receiving_Unusable>().having(
          (receiving) => receiving.message,
          'message',
          'The folder for received files is not there.',
        ),
      );

      final sending = await b.sendFiles(
        id: a.me().id,
        paths: await _sampleFiles(),
      );

      await expectLater(
        sending.outcome(),
        throwsA(_batonError(ErrorKind.declined)),
      );
      expect(
        nowhere.listSync(),
        isEmpty,
        reason: 'no folder was made for the files',
      );
    },
  );

  test(
    'files that cannot be sent are refused before anything is offered',
    () async {
      final received = await _tempDir('baton-bridge-received-');
      final a = await _start(
        await _folder(name: 'Device A'),
        receiveDir: received,
      );
      final b = await _start(await _folder(name: 'Device B'));
      final events = _Feed(await a.events());
      addTearDown(events.close);
      await _pair(a, events, b);
      final paths = await _sampleFiles();

      // A device that is not paired, nothing chosen, and something that is not there.
      await expectLater(
        b.sendFiles(id: '0' * 64, paths: paths),
        throwsA(_batonError(ErrorKind.notPaired)),
      );
      await expectLater(
        b.sendFiles(id: a.me().id, paths: const []),
        throwsA(_batonError(ErrorKind.badItem)),
      );
      await expectLater(
        b.sendFiles(id: a.me().id, paths: ['${received.path}/gone.txt']),
        throwsA(_batonError(ErrorKind.badItem)),
      );

      // Nothing was offered: the next thing A hears of is not a question.
      final sending = await b.sendFiles(id: a.me().id, paths: paths);
      final request = (await events.next<Event_TransferRequested>()).request;
      expect(
        request.files,
        3,
        reason: 'the first question is the one that was sent',
      );
      sending.cancel();
    },
  );

  test('only web links are opened, and as they were checked', () {
    // What comes back is the address to open: written out in full, not as it was sent.
    for (final (sent, opened) in [
      ('https://example.com/a?b=c', 'https://example.com/a?b=c'),
      ('http://example.com', 'http://example.com/'),
      ('HTTPS://EXAMPLE.COM', 'https://example.com/'),
      ('  https://example.com/a  ', 'https://example.com/a'),
      ('https://example.com/a b', 'https://example.com/a%20b'),
    ]) {
      expect(webAddress(url: sent), opened, reason: sent);
    }
    for (final url in [
      'file:///etc/passwd',
      'smb://server/share',
      'tel:+15555550100',
      'javascript:alert(1)',
      'obsidian://open?vault=x',
      'example.com',
      '',
    ]) {
      expect(webAddress(url: url), isNull, reason: url);
    }
  });
}

/// A folder for one device, with settings that keep it to this computer: no relay, no
/// discovery and no port mapping, so the devices find each other through the addresses in
/// the pairing ticket and the tests run the same everywhere.
Future<Directory> _folder({required String? name}) async {
  final folder = await Directory.systemTemp.createTemp('baton-bridge-test-');
  addTearDown(() => folder.delete(recursive: true));
  final named = name == null ? '' : 'device_name = "$name"\n';
  await File('${folder.path}/config.toml').writeAsString(
    '$named\n[network]\nrelay = "disabled"\nlan_discovery = false\nport_mapping = false\n',
  );
  return folder;
}

/// Starts Baton in [folder], and stops it when the test is over. It puts the files that other
/// devices send in [receiveDir], and takes none without one.
Future<Baton> _start(Directory folder, {Directory? receiveDir}) async {
  final baton = await Baton.start(
    dataDir: folder.path,
    lanDiscovery: false,
    portMapping: false,
    receiveDir: receiveDir?.path,
  );
  addTearDown(baton.stop);
  return baton;
}

/// An empty folder that is deleted when the test is over.
Future<Directory> _tempDir(String prefix) async {
  final folder = await Directory.systemTemp.createTemp(prefix);
  addTearDown(() => folder.delete(recursive: true));
  return folder;
}

/// Pairs [b] with [a], who makes the offer and hears of it on [aEvents].
Future<void> _pair(Baton a, _Feed<Event> aEvents, Baton b) async {
  final offer = await a.offerPairing();
  final join = _Feed(b.joinPairing(ticket: offer.ticket));
  addTearDown(join.close);
  final asked = await aEvents.next<Event_PairingRequested>();
  expect(a.answerPairing(request: asked.request.id, accept: true), isTrue);
  await join.next<JoinProgress_Paired>();
  await join.ended;
}

/// A file, and a folder with a file and a folder inside it, ready to be sent: 100 017 bytes in
/// three files. The folder they are in is deleted when the test is over.
Future<List<String>> _sampleFiles() async {
  final source = await _tempDir('baton-bridge-source-');
  final docs = Directory('${source.path}/docs/sub');
  await docs.create(recursive: true);
  await File('${source.path}/hello.txt').writeAsString('hello from B');
  await File('${source.path}/docs/a.txt').writeAsString('first');
  await File('${docs.path}/b.bin').writeAsBytes(List.filled(100000, 7));
  return ['${source.path}/hello.txt', '${source.path}/docs'];
}

/// What [_sampleFiles] comes to in the folder that receives it.
Map<String, List<int>?> _sampleTree() => {
  'docs': null,
  'docs/a.txt': 'first'.codeUnits,
  'docs/sub': null,
  'docs/sub/b.bin': List.filled(100000, 7),
  'hello.txt': 'hello from B'.codeUnits,
};

/// What is in [root], by path: the bytes of a file, and `null` for a folder. The folders that
/// Baton makes for the files that are on their way are left out.
Map<String, List<int>?> _tree(Directory root) {
  final found = <String, List<int>?>{};
  for (final entry in root.listSync(recursive: true)) {
    final path = entry.path.substring(root.path.length + 1);
    if (path.startsWith('.baton-incoming-')) continue;
    found[path] = entry is File ? entry.readAsBytesSync() : null;
  }
  return found;
}

/// A [BatonError] of [kind].
TypeMatcher<BatonError> _batonError(ErrorKind kind) =>
    isA<BatonError>().having((error) => error.kind, 'kind', kind);

/// What a stream has said so far, so that a test can ask for the next thing it cares about
/// without missing what came between two questions.
class _Feed<T extends Object> {
  _Feed(Stream<T> stream) {
    _subscription = stream.listen(
      (item) {
        _items.add(item);
        _wake();
      },
      onDone: () {
        _done = true;
        _wake();
      },
    );
  }

  final _items = <T>[];
  var _cursor = 0;
  var _done = false;
  var _changed = Completer<void>();
  late final StreamSubscription<T> _subscription;

  void _wake() {
    final old = _changed;
    _changed = Completer<void>();
    old.complete();
  }

  /// The next item that is an [E], skipping any others. Fails if none comes in time, or the
  /// stream ends first.
  Future<E> next<E extends T>() async {
    final limit = DateTime.now().add(_wait);
    while (true) {
      while (_cursor < _items.length) {
        final item = _items[_cursor++];
        if (item is E) {
          return item;
        }
      }
      if (_done) {
        throw StateError('The stream ended while waiting for a $E.');
      }
      final left = limit.difference(DateTime.now());
      if (left.isNegative) {
        throw TimeoutException('No $E came.', _wait);
      }
      await _changed.future.timeout(left, onTimeout: () {});
    }
  }

  /// Completes when the stream has ended, and fails if it does not in time.
  Future<void> get ended async {
    final limit = DateTime.now().add(_wait);
    while (!_done) {
      final left = limit.difference(DateTime.now());
      if (left.isNegative) {
        throw TimeoutException('The stream did not end.', _wait);
      }
      await _changed.future.timeout(left, onTimeout: () {});
    }
  }

  /// Stops listening. The cancel is not waited for: a stream from the bridge is an `async*`
  /// generator, and cancelling it completes only when it next yields, which for a stream that
  /// has gone quiet is when its next event arrives.
  void close() => unawaited(_subscription.cancel());
}
