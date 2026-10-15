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

  /// Makes `devices` wait until [releaseDevices]. What it then gives is the list as it was when
  /// it was asked, as a read that takes a while does.
  void holdDevices() => _devicesReady = Completer<void>();

  /// Lets the held reads of `devices` answer.
  void releaseDevices() => _devicesReady?.complete();

  Completer<void>? _devicesReady;

  @override
  Future<List<Device>> devices() async {
    final asked = pairedDevices;
    await _devicesReady?.future;
    return asked;
  }

  @override
  Future<Network> network() async => reach;

  /// What `unpair` throws, if it should fail.
  Object? unpairFailure;

  /// The ids that `unpair` was given and accepted, in order.
  final List<String> unpaired = [];

  @override
  Future<void> unpair(String id) async {
    final failure = unpairFailure;
    if (failure != null) throw failure;
    unpaired.add(id);
    pairedDevices = [
      for (final device in pairedDevices)
        if (device.id != id) device,
    ];
  }

  /// The invitation that `offerPairing` hands out.
  Offer offer = const Offer(ticket: sampleTicket, expiresAt: 1760529600 + 300);

  /// What `offerPairing` throws, if it should fail.
  Object? offerFailure;

  /// Makes `offerPairing` wait until [releaseOffer], as a real one can take seconds.
  void holdOffer() => _offerReady = Completer<void>();

  /// Lets a held `offerPairing` answer.
  void releaseOffer() => _offerReady?.complete();

  Completer<void>? _offerReady;

  /// How many invitations were made.
  int offersMade = 0;

  /// How many times `cancelOffer` was called.
  int offersCancelled = 0;

  @override
  Future<Offer> offerPairing() async {
    offersMade++;
    await _offerReady?.future;
    final failure = offerFailure;
    if (failure != null) throw failure;
    return offer;
  }

  @override
  void cancelOffer() => offersCancelled++;

  /// What `answerPairing` was asked, in order.
  final List<({int request, bool accept})> answers = [];

  /// What `answerPairing` says: whether the request was still waiting.
  bool requestWaiting = true;

  @override
  bool answerPairing(int request, {required bool accept}) {
    answers.add((request: request, accept: accept));
    return requestWaiting;
  }

  /// The text that `joinPairing` was given, in order.
  final List<String> joined = [];

  /// What the latest `joinPairing` reports. Add to it as Baton would; closing it ends the
  /// stream.
  StreamController<JoinProgress> get joining => _joining!;

  StreamController<JoinProgress>? _joining;

  /// How many times `cancelJoin` was called.
  int joinsCancelled = 0;

  @override
  Stream<JoinProgress> joinPairing(String ticket) {
    joined.add(ticket);
    return (_joining = StreamController<JoinProgress>()).stream;
  }

  @override
  void cancelJoin() => joinsCancelled++;

  @override
  Future<void> stop() async {
    stopped = true;
    await _joining?.close();
    await _events.close();
  }
}

/// A ticket as the library writes it: the prefix, then capital letters and digits. This one is
/// made up and leads nowhere.
const sampleTicket =
    'BATON1:AEBAGBAFAYDQQCIKBMGA2DQPCAIREEYUCULBOGA3DSNZVHI4DCODEMBYGQ4TKNJYGU5DEOBZG44TO'
    'NRRGQ2TIMBTHA3DOOJZGM3DCNZSGE4TCOBXGQ3TMNJVHA2DKNZUGM2TQMBQGEZDQMJRHE3TMMZRGU2DOOJX';

/// A request to pair that a test can show.
const sampleRequest = PairingRequest(
  id: 7,
  deviceId: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  name: 'Anna’s iPhone',
  platform: Platform.ios,
  fingerprint: 'iphn-abcd-efgh-ijkl-mnop',
  code: '514 643',
);
