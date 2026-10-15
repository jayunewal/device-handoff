// SPDX-License-Identifier: Apache-2.0

// The one place that calls the Rust library. Everything else in the app goes through the
// interfaces in backend.dart.

import 'dart:io' as io;

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import 'backend.dart';
import 'rust/api/baton.dart' as api;
import 'rust/frb_generated.dart';

/// Baton as the Rust library runs it.
class RustBackend implements Backend {
  /// [dataDir] is the folder that belongs to Baton alone. Without one, the folder for this
  /// system is used. [library] says where to find the Rust library; without it, it is the one
  /// that is built into the app.
  RustBackend({this._dataDir, ExternalLibrary Function()? library})
    : _library = library ?? _builtIn;

  /// The name of Baton's folder inside `Library/Application Support`. The iPhone, iPad and Mac
  /// apps leave a folder of this name out of backups, in AppDelegate.swift and
  /// MainFlutterWindow.swift.
  static const folderName = 'baton-app';

  final String? _dataDir;
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
  /// Support`, which is where an app keeps files that are not the person's documents. The
  /// folder of the `baton` command is `baton` in the same place, so the two are told apart
  /// even where an app is not sandboxed.
  String _folder() {
    final dataDir = _dataDir;
    if (dataDir != null) return dataDir;
    if (!io.Platform.isIOS && !io.Platform.isMacOS) {
      throw UnsupportedError('Baton does not run on this system yet.');
    }
    final home = io.Platform.environment['HOME'];
    if (home == null || home.isEmpty) {
      throw StateError('The home folder of the app is not known.');
    }
    return '$home/Library/Application Support/$folderName';
  }

  @override
  Future<String?> deviceName() async {
    await _load();
    return api.deviceName(dataDir: _folder());
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
    await api.setDeviceName(dataDir: _folder(), name: name);
  }

  @override
  Future<Session> start() async {
    await _load();
    // An iPhone or an iPad may use the multicast messages that finding devices on the local
    // network starts with, and that asking the router for a port starts with, only if Apple
    // has approved the app for it. So it finds the other device through the relay, and then
    // moves to a direct path.
    final phone = io.Platform.isIOS;
    final baton = await api.Baton.start(
      dataDir: _folder(),
      lanDiscovery: phone ? false : null,
      portMapping: phone ? false : null,
    );
    return _RustSession(baton);
  }
}

class _RustSession implements Session {
  _RustSession(this._baton);

  final api.Baton _baton;

  @override
  Me get me => _baton.me();

  @override
  Stream<Event> events() => _baton.events();

  @override
  Future<List<Device>> devices() => _baton.devices();

  @override
  Future<Network> network() => _baton.network();

  @override
  Future<void> stop() => _baton.stop();
}
