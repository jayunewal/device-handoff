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

  /// What `windowAbilities` gives: a device that does nothing with windows, as a build without
  /// platform code is.
  WindowAbilities abilities = const WindowAbilities(
    canShow: false,
    canShare: false,
    canControl: false,
  );

  @override
  WindowAbilities windowAbilities() => abilities;

  /// What `answerWindow` was asked, in order.
  final List<({int request, bool accept})> windowAnswers = [];

  /// What `answerWindow` says: whether the request was still waiting.
  bool windowRequestWaiting = true;

  @override
  bool answerWindow(int request, {required bool accept}) {
    windowAnswers.add((request: request, accept: accept));
    return windowRequestWaiting;
  }

  /// What `shareWindow` was asked, in order.
  final List<({String id, bool control})> windowsShared = [];

  /// What `shareWindow` gives.
  WindowShared sharing = const WindowShared.nothingChosen();

  /// What `shareWindow` throws, if it should fail.
  Object? shareFailure;

  /// Makes `shareWindow` wait until [releaseShare], as it does while the person chooses a
  /// window and the person at the other device answers.
  void holdShare() => _shareReady = Completer<void>();

  /// Lets the held `shareWindow` answer.
  void releaseShare() => _shareReady?.complete();

  Completer<void>? _shareReady;

  @override
  Future<WindowShared> shareWindow(String id, {bool control = false}) async {
    windowsShared.add((id: id, control: control));
    await _shareReady?.future;
    final failure = shareFailure;
    if (failure != null) throw failure;
    return sharing;
  }

  /// The sessions that `endWindow` was asked to end, in order.
  final List<int> windowsEnded = [];

  /// What `endWindow` says: whether there was such a window.
  bool windowRuns = true;

  @override
  bool endWindow(int session) {
    windowsEnded.add(session);
    return windowRuns;
  }

  /// What `windowInput` was given, in order.
  final List<({int session, WindowInput input})> windowInputs = [];

  /// What `windowInput` says: whether the event was taken.
  bool windowInputTaken = true;

  @override
  bool windowInput(int session, WindowInput input) {
    windowInputs.add((session: session, input: input));
    return windowInputTaken;
  }

  /// The windows whose input `releaseWindowInput` was asked to let go of, in order.
  final List<int> windowInputsReleased = [];

  @override
  void releaseWindowInput(int session) => windowInputsReleased.add(session);

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

  /// What `receiving` gives.
  Receiving taking = const Receiving.on_();

  @override
  Receiving receiving() => taking;

  /// What `receiveFolder` gives.
  @override
  String? receiveFolder = '/Users/test/Downloads';

  /// What `answerTransfer` was asked, in order.
  final List<({int request, bool accept})> transferAnswers = [];

  /// What `answerTransfer` says: whether the request was still waiting.
  bool transferWaiting = true;

  @override
  bool answerTransfer(int request, {required bool accept}) {
    transferAnswers.add((request: request, accept: accept));
    return transferWaiting;
  }

  /// What `sendFiles` was given and accepted, in order.
  final List<({String id, List<String> paths})> filesSent = [];

  /// What `sendFiles` throws, if it should fail.
  Object? sendFilesFailure;

  /// Makes `sendFiles` wait until [releaseSendFiles], as a start that takes a while does.
  void holdSendFiles() => _sendFilesReady = Completer<void>();

  /// Lets the held `sendFiles` answer.
  void releaseSendFiles() => _sendFilesReady?.complete();

  Completer<void>? _sendFilesReady;

  /// The transfer that the latest `sendFiles` started.
  late FakeFileTransfer transfer;

  @override
  Future<FileTransfer> sendFiles(String id, List<String> paths) async {
    await _sendFilesReady?.future;
    final failure = sendFilesFailure;
    if (failure != null) throw failure;
    filesSent.add((id: id, paths: [...paths]));
    return transfer = FakeFileTransfer();
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

/// A transfer of files that a test moves on by hand.
class FakeFileTransfer implements FileTransfer {
  FakeFileTransfer() {
    // A transfer that is given up before anyone waits for its end has no one to hear the error.
    _outcome.future.ignore();
  }

  FileProgress _progress = const FileProgress(
    stage: SendStage.connecting,
    attempt: 1,
    files: 1,
    bytes: 1000,
    sent: 0,
  );

  final Completer<FilesSent> _outcome = Completer<FilesSent>();

  /// Whether `cancel` was called.
  bool cancelled = false;

  @override
  FileProgress progress() => _progress;

  @override
  void cancel() {
    cancelled = true;
    fail(
      const BatonError(
        kind: ErrorKind.cancelled,
        message: 'You stopped sending.',
      ),
    );
  }

  @override
  Future<FilesSent> outcome() => _outcome.future;

  /// Moves the transfer on, as Baton would.
  void advance({
    SendStage? stage,
    int? files,
    int? bytes,
    int? sent,
    int? attempt,
  }) {
    _progress = FileProgress(
      stage: stage ?? _progress.stage,
      attempt: attempt ?? _progress.attempt,
      files: files ?? _progress.files,
      bytes: bytes ?? _progress.bytes.toInt(),
      sent: sent ?? _progress.sent.toInt(),
    );
  }

  /// Ends the transfer with [sent].
  void finish(FilesSent sent) {
    advance(stage: SendStage.over);
    if (!_outcome.isCompleted) _outcome.complete(sent);
  }

  /// Ends the transfer with [error].
  void fail(BatonError error) {
    advance(stage: SendStage.over);
    if (!_outcome.isCompleted) _outcome.completeError(error);
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

/// A window that a paired device offers, for a test to show.
const sampleWindowRequest = WindowRequest(
  id: 3,
  deviceId: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  deviceName: 'Anna’s Mac',
  display: false,
  title: 'Budget.numbers',
  app: 'Numbers',
  width: 1440,
  height: 900,
  control: false,
);

/// A window that is shown on this device, or shared from it, for a test to show. A window that
/// is shown has a screen to draw it with, and one that is shared has none.
WindowSession sampleWindowSession({
  int id = 1,
  WindowRole role = WindowRole.viewing,
  String device = 'Anna’s Mac',
  String? title = 'Budget.numbers',
  String? app = 'Numbers',
  int width = 1440,
  int height = 900,
  bool control = false,
  bool display = false,
  int? screen,
}) => WindowSession(
  id: id,
  role: role,
  deviceId: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  deviceName: device,
  display: display,
  title: title,
  app: app,
  width: width,
  height: height,
  control: control,
  screen: screen ?? (role == WindowRole.viewing ? 7 : null),
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

/// A request for files that a test can show.
TransferRequest sampleTransfer({
  int id = 3,
  String from = 'Anna’s iPhone',
  int files = 2,
  int folders = 0,
  int bytes = 3200000,
  List<PreviewName>? preview,
}) => TransferRequest(
  id: id,
  deviceId: 'b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0',
  deviceName: from,
  items: preview?.length ?? 2,
  files: files,
  folders: folders,
  bytes: bytes,
  preview:
      preview ??
      const [
        PreviewName(
          name: 'holiday.jpg',
          folder: false,
          files: 0,
          bytes: 2000000,
        ),
        PreviewName(name: 'notes.txt', folder: false, files: 0, bytes: 1200000),
      ],
);

/// How a transfer that [request] asked for ended. [saved] is where the items that arrived are.
/// Without it they are in the Downloads folder under the names they were offered with, when
/// [result] says that something arrived, and there are none when it does not.
TransferEnd sampleTransferEnd(
  TransferRequest request, {
  TransferResult result = const TransferResult.received(failed: 0),
  List<String>? saved,
}) => TransferEnd(
  request: request.id,
  deviceId: request.deviceId,
  deviceName: request.deviceName,
  files: request.files,
  bytes: request.bytes,
  preview: request.preview,
  saved:
      saved ??
      switch (result) {
        TransferResult_Received(:final failed) when failed < request.files => [
          for (final item in request.preview)
            '/Users/test/Downloads/${item.name}',
        ],
        TransferResult_Received() || TransferResult_Dropped() => const [],
      },
  result: result,
);
