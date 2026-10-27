// SPDX-License-Identifier: Apache-2.0

// The one place that calls the Rust library. Everything else in the app goes through the
// interfaces in backend.dart.

import 'dart:io' as io;

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import 'backend.dart';
import 'rust/api/baton.dart' as api;
import 'rust/api/startup.dart' as startup;
import 'rust/frb_generated.dart';
import 'support_folder.dart';

/// Baton as the Rust library runs it.
class RustBackend implements Backend {
  /// [dataDir] is the folder that belongs to Baton alone. Without one, the folder for this
  /// system is used, made inside [supportFolder]. [receiveDir] is the folder that the files other
  /// devices send are put in; without one, [supportFolder] says which. [library] says where to
  /// find the Rust library; without it, it is the one that is built into the app.
  RustBackend({
    this._dataDir,
    this._receiveDir,
    ExternalLibrary Function()? library,
    this._supportFolder = const SupportFolder(),
  }) : _library = library ?? _builtIn;

  /// The name of Baton's folder inside `Library/Application Support`. The iPhone, iPad and Mac
  /// apps leave a folder of this name out of backups, in AppDelegate.swift and
  /// MainFlutterWindow.swift.
  static const folderName = 'baton-app';

  final String? _dataDir;
  final String? _receiveDir;
  final SupportFolder _supportFolder;
  final ExternalLibrary Function() _library;

  // The library is loaded once per process. A failed load is tried again, so that a person
  // who is shown the failure and asks to try again is not told the same thing at once.
  static Future<void>? _loading;

  // Where the library is when it is part of an iPhone, an iPad or a Mac app: a framework that
  // the build puts in the app, with Rust linked into it. Naming it keeps the loader from
  // looking anywhere else.
  static ExternalLibrary _builtIn() =>
      ExternalLibrary.open('baton_ffi.framework/baton_ffi');

  Future<void> _load() async {
    final loading = _loading ??= RustLib.init(externalLibrary: _library());
    try {
      await loading;
    } on Object {
      _loading = null;
      rethrow;
    }
  }

  /// The folder for Baton's files. It is made by Baton, private to the person who runs it.
  ///
  /// On an iPhone, an iPad and a Mac this is inside the app's own `Library/Application
  /// Support`, which is where an app keeps files that are not the person's documents, and
  /// [SupportFolder] says where that is. The folder of the `baton` command is `baton` in the
  /// same place, so the two are told apart even where an app is not sandboxed.
  Future<String> _folder() async {
    final dataDir = _dataDir;
    if (dataDir != null) return dataDir;
    return '${await _supportFolder.path()}/$folderName';
  }

  @override
  Future<String?> deviceName() async {
    await _load();
    return api.deviceName(dataDir: await _folder());
  }

  @override
  String suggestedName() {
    var host = '';
    try {
      host = io.Platform.localHostname.replaceFirst(RegExp(r'\.local$'), '');
    } on Object {
      // No host name to offer. The person types one.
    }
    if (host.isNotEmpty && host != 'localhost') return host;
    return io.Platform.isIOS ? 'My iPhone' : 'My Mac';
  }

  @override
  Future<void> setDeviceName(String name) async {
    await _load();
    await api.setDeviceName(dataDir: await _folder(), name: name);
  }

  @override
  Future<Session> start() async {
    await _load();
    // An iPhone or an iPad may use the multicast messages that finding devices on the local
    // network starts with, and that asking the router for a port starts with, only if Apple
    // has approved the app for it. So it finds the other device through the relay, and then
    // moves to a direct path.
    final phone = io.Platform.isIOS;
    final (receiveDir, receiveProblem) = await _receiveFolder();
    final baton = await api.Baton.start(
      dataDir: await _folder(),
      lanDiscovery: phone ? false : null,
      portMapping: phone ? false : null,
      receiveDir: receiveDir,
    );
    return _RustSession(baton, receiveDir, receiveProblem);
  }

  /// Where the files that other devices send are put, or a sentence that says why there is no
  /// such place. Not finding it does not stop Baton: pairing and sending do not need it, and the
  /// session tells the screens that this device takes no files, and why.
  Future<(String?, String?)> _receiveFolder() async {
    try {
      return (_receiveDir ?? await _supportFolder.receiveFolder(), null);
    } on StateError catch (e) {
      return (null, e.message);
    } on UnsupportedError catch (e) {
      return (null, e.message ?? 'Baton does not run on this system yet.');
    }
  }

  @override
  List<String> recentLog() {
    try {
      return startup.recentLog();
    } on Object {
      // The library never loaded, so nothing was logged that could be read.
      return const [];
    }
  }
}

class _RustSession implements Session {
  _RustSession(this._baton, this._receiveFolder, this._receiveProblem);

  final api.Baton _baton;
  final String? _receiveFolder;
  final String? _receiveProblem;

  @override
  Me get me => _baton.me();

  @override
  Future<Stream<Event>> events() => _baton.events();

  @override
  Future<List<Device>> devices() => _baton.devices();

  @override
  Future<Network> network() => _baton.network();

  @override
  Future<void> unpair(String id) => _baton.unpair(id: id);

  @override
  Future<Offer> offerPairing() => _baton.offerPairing();

  @override
  void cancelOffer() => _baton.cancelOffer();

  @override
  bool answerPairing(int request, {required bool accept}) =>
      _baton.answerPairing(request: request, accept: accept);

  @override
  Stream<JoinProgress> joinPairing(String ticket) =>
      _baton.joinPairing(ticket: ticket);

  @override
  void cancelJoin() => _baton.cancelJoin();

  @override
  Receiving receiving() {
    final problem = _receiveProblem;
    if (problem != null) return Receiving.unusable(message: problem);
    return _baton.receiving();
  }

  @override
  String? get receiveFolder =>
      receiving() is Receiving_On ? _receiveFolder : null;

  @override
  bool answerTransfer(int request, {required bool accept}) =>
      _baton.answerTransfer(request: request, accept: accept);

  @override
  Future<FileTransfer> sendFiles(String id, List<String> paths) async =>
      _RustFileTransfer(await _baton.sendFiles(id: id, paths: paths));

  @override
  Future<void> sendLink(String id, String url, {String? title}) =>
      _baton.sendLink(id: id, url: url, title: title);

  @override
  Future<void> sendText(String id, String text) =>
      _baton.sendText(id: id, text: text);

  @override
  Future<Reach> ping(String id) => _baton.ping(id: id);

  @override
  WindowAbilities windowAbilities() => _baton.windowAbilities();

  @override
  bool answerWindow(int request, {required bool accept}) =>
      _baton.answerWindow(request: request, accept: accept);

  @override
  Future<WindowShared> shareWindow(String id, {bool control = false}) =>
      _baton.shareWindow(id: id, control: control);

  @override
  Future<ScreenStarted> startScreen(String id, {bool control = false}) =>
      _baton.startScreen(id: id, control: control);

  @override
  StartAccess startAccess(String id) => _baton.startAccess(id: id);

  @override
  Future<void> setStartAccess(String id, StartAccess access) =>
      _baton.setStartAccess(id: id, access: access);

  @override
  bool? hearSetting(String id) => _baton.hearSetting(id: id);

  @override
  Future<void> setHearSetting(String id, bool? setting) =>
      _baton.setHearSetting(id: id, setting: setting);

  @override
  bool endWindow(int session) => _baton.endWindow(session: session);

  @override
  bool endWindowLeaving(int session) =>
      _baton.endWindowLeaving(session: session);

  @override
  bool windowInput(int session, WindowInput input) =>
      _baton.windowInput(session: session, input: input);

  @override
  void releaseWindowInput(int session) =>
      _baton.releaseWindowInput(session: session);

  @override
  bool windowLift(int session, {required bool screen, required bool input}) =>
      _baton.windowLift(session: session, screen: screen, input: input);

  @override
  bool hearWindow(int session, {required bool listen}) =>
      _baton.hearWindow(session: session, listen: listen);

  @override
  Future<List<Received>> inbox() => _baton.inbox();

  @override
  Future<bool> removeReceived(int number) =>
      _baton.removeReceived(number: number);

  @override
  Future<int> clearInbox() async => (await _baton.clearInbox()).toInt();

  @override
  String? webAddress(String url) => api.webAddress(url: url);

  @override
  List<String> recentLog() => startup.recentLog();

  @override
  Future<void> stop() => _baton.stop();
}

class _RustFileTransfer implements FileTransfer {
  _RustFileTransfer(this._send);

  final api.FileSend _send;

  @override
  FileProgress progress() => _send.progress();

  @override
  void cancel() => _send.cancel();

  @override
  Future<FilesSent> outcome() => _send.outcome();
}
