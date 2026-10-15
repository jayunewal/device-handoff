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

  /// What `recentLog` gives, and what the Baton that `start` hands out gives too.
  final List<String> log = [];

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
      log: log,
    );
  }

  @override
  List<String> recentLog() => [...log];
}

class FakeSession implements Session {
  FakeSession({
    required this.me,
    this.pairedDevices = const [],
    this.reach = const Network(addresses: [], lanDiscovery: true),
    List<String>? log,
  }) : log = log ?? [];

  @override
  final Me me;

  /// What `recentLog` gives. Add to it as the library would.
  final List<String> log;

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

  /// What `network` throws, if it should fail.
  Object? networkFailure;

  @override
  Future<Network> network() async {
    final failure = networkFailure;
    if (failure != null) throw failure;
    return reach;
  }

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

  /// What `sendLink` and `sendText` were given and accepted, in order: a link as its address and
  /// title, text as the text.
  final List<({String id, String? url, String? title, String? text})> sent = [];

  /// What `sendLink` and `sendText` throw, if they should fail.
  Object? sendFailure;

  /// Makes `sendLink` and `sendText` wait until [releaseSend], as a send to a device that is
  /// slow to answer does.
  void holdSend() => _sendReady = Completer<void>();

  /// Lets the held sends answer.
  void releaseSend() => _sendReady?.complete();

  Completer<void>? _sendReady;

  Future<void> _send(
    String id, {
    String? url,
    String? title,
    String? text,
  }) async {
    await _sendReady?.future;
    final failure = sendFailure;
    if (failure != null) throw failure;
    sent.add((id: id, url: url, title: title, text: text));
  }

  @override
  Future<void> sendLink(String id, String url, {String? title}) =>
      _send(id, url: url, title: title);

  @override
  Future<void> sendText(String id, String text) => _send(id, text: text);

  /// The ids that `ping` was given, in order.
  final List<String> pinged = [];

  /// What `ping` gives.
  Reach pingResult = const Reach(roundTripMs: 24, route: Route.lan);

  /// What `ping` throws, if it should fail.
  Object? pingFailure;

  /// Makes `ping` wait until [releasePing], as a device that is slow to answer does.
  void holdPing() => _pingReady = Completer<void>();

  /// Lets the held pings answer.
  void releasePing() => _pingReady?.complete();

  Completer<void>? _pingReady;

  @override
  Future<Reach> ping(String id) async {
    pinged.add(id);
    await _pingReady?.future;
    final failure = pingFailure;
    if (failure != null) throw failure;
    return pingResult;
  }

  /// What `inbox` gives, newest first.
  List<Received> received = [];

  /// What `inbox`, `removeReceived` and `clearInbox` throw, if they should fail.
  Object? inboxFailure;

  /// How many times `inbox` was called.
  int inboxReads = 0;

  /// Makes `inbox` wait until [releaseInbox]. What it then gives is the list as it was when
  /// it was asked.
  void holdInbox() => _inboxReady = Completer<void>();

  /// Lets the held reads of `inbox` answer.
  void releaseInbox() => _inboxReady?.complete();

  Completer<void>? _inboxReady;

  /// Puts [item] at the top of the inbox and tells the screens, as Baton does when a device
  /// hands it something.
  void arrive(Received item) {
    received = [item, ...received];
    emit(Event_Received(received: item));
  }

  @override
  Future<List<Received>> inbox() async {
    inboxReads++;
    final asked = received;
    await _inboxReady?.future;
    final failure = inboxFailure;
    if (failure != null) throw failure;
    return asked;
  }

  /// The numbers that `removeReceived` was given, in order.
  final List<int> removed = [];

  @override
  Future<bool> removeReceived(int number) async {
    final failure = inboxFailure;
    if (failure != null) throw failure;
    removed.add(number);
    final before = received.length;
    received = [
      for (final item in received)
        if (item.number != number) item,
    ];
    return received.length < before;
  }

  /// How many times `clearInbox` emptied the inbox.
  int clears = 0;

  @override
  Future<int> clearInbox() async {
    final failure = inboxFailure;
    if (failure != null) throw failure;
    clears++;
    final count = received.length;
    received = [];
    return count;
  }

  // The rule that the library keeps, closely enough for a screen to be tried against it: only
  // http and https, written out as the library writes them.
  @override
  String? webAddress(String url) {
    final uri = Uri.tryParse(url.trim());
    if (uri == null || !uri.hasAuthority || uri.host.isEmpty) return null;
    return switch (uri.scheme.toLowerCase()) {
      'http' || 'https' => uri.toString(),
      _ => null,
    };
  }

  @override
  List<String> recentLog() => [...log];

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

/// An item in the inbox that a test can show.
Received sampleReceived(
  int number, {
  String from = 'Anna’s iPhone',
  String fromId =
      'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  int at = 1760529600,
  ReceivedItem item = const ReceivedItem.text(text: 'Hello from the phone'),
}) => Received(
  number: number,
  fromId: fromId,
  fromName: from,
  receivedAt: at,
  item: item,
);
