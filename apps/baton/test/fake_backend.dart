// SPDX-License-Identifier: Apache-2.0

// A Baton that does nothing but what a test tells it to, so that the screens can be tried on
// any computer without the Rust library.

import 'dart:async';

import 'package:baton/src/backend.dart';

const noVisibleName = BatonError(
  kind: ErrorKind.badName,
  message: 'The name needs at least one visible character.',
);

class FakeBackend implements Backend {
  FakeBackend({this.savedName, List<Object> startFailures = const []})
    : _startFailures = [...startFailures];

  /// The name that is saved, if any.
  String? savedName;

  /// What `start` throws, one for each call, before it starts to succeed.
  final List<Object> _startFailures;

  /// What `setDeviceName` throws, if it should fail for a reason other than the name.
  Object? setNameFailure;

  /// The names that were saved, in order.
  final List<String> namesSaved = [];

  /// How many times `start` was called.
  int starts = 0;

  /// The Baton that `start` hands out.
  late FakeSession session;

  @override
  Future<String?> deviceName() async => savedName;

  @override
  String suggestedName() => 'Test Mac';

  @override
  Future<void> setDeviceName(String name) async {
    final failure = setNameFailure;
    if (failure != null) throw failure;
    if (name.trim().isEmpty) throw noVisibleName;
    namesSaved.add(name);
    savedName = name.trim();
  }

  @override
  Future<Session> start() async {
    starts++;
    if (_startFailures.isNotEmpty) throw _startFailures.removeAt(0);
    return session = FakeSession(
      me: Me(
        id: 'a' * 64,
        name: savedName!,
        platform: Platform.macOs,
        fingerprint: 'abcd-efgh-ijkl-mnop-qrst',
      ),
    );
  }
}

class FakeSession implements Session {
  FakeSession({
    required this.me,
    this.pairedDevices = const [],
    this.reach = const Network(addresses: [], lanDiscovery: true),
  });

  @override
  final Me me;

  /// What `devices` gives.
  List<Device> pairedDevices;

  /// What `network` gives.
  Network reach;

  /// Whether `stop` was called.
  bool stopped = false;

  late final _events = StreamController<Event>.broadcast(
    onListen: () => listens++,
  );

  /// Makes `events` wait until [releaseEvents], as the bridge takes a moment to hand the stream
  /// over.
  void holdEvents() => _eventsReady = Completer<void>();

  /// Lets a held `events` hand over its stream.
  void releaseEvents() => _eventsReady?.complete();

  Completer<void>? _eventsReady;

  /// Whether anything is listening to the events.
  bool get listening => _events.hasListener;

  /// How many times anything has started to listen to the events.
  int listens = 0;

  /// Hands an event to the screen, as Baton would.
  void emit(Event event) => _events.add(event);

  @override
  Future<Stream<Event>> events() async {
    await _eventsReady?.future;
    return _events.stream;
  }

  @override
  Future<List<Device>> devices() async => pairedDevices;

  @override
  Future<Network> network() async => reach;

  @override
  Future<void> stop() async {
    stopped = true;
    await _events.close();
  }
}
