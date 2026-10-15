// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'backend.dart';

/// Where the app is, from opening it to showing the devices.
enum Phase {
  /// Looking for the name that was saved.
  loading,

  /// The first time: the person names this device.
  askName,

  /// Starting Baton.
  starting,

  /// Baton is running.
  running,

  /// Baton could not start.
  failed,
}

/// Takes the app from a closed folder to a running Baton and says which screen is wanted.
class AppController extends ChangeNotifier {
  AppController(this._backend);

  final Backend _backend;
  Phase _phase = Phase.loading;
  Session? _session;
  String _problem = '';
  bool _disposed = false;

  /// Which screen is wanted.
  Phase get phase => _phase;

  /// The running Baton, while [phase] is [Phase.running].
  Session? get session => _session;

  /// What to tell the person, while [phase] is [Phase.failed]. It is one sentence.
  String get problem => _problem;

  /// A name to offer, while [phase] is [Phase.askName].
  String get suggestedName => _backend.suggestedName();

  /// Looks for the saved name and, if there is one, starts Baton. Call it again to try again
  /// after a failure.
  Future<void> begin() async {
    _set(Phase.loading);
    await _guard(() async {
      if (await _backend.deviceName() == null) {
        _set(Phase.askName);
      } else {
        await _start();
      }
    });
  }

  /// Saves the name the person chose and starts Baton. Returns what to tell the person if the
  /// name cannot be used, and `null` if it was saved.
  Future<String?> chooseName(String name) async {
    String? refused;
    await _guard(() async {
      try {
        await _backend.setDeviceName(name);
      } on BatonError catch (e) {
        // Only the name is the person's to fix. A folder that cannot be written is not.
        if (e.kind != ErrorKind.badName) rethrow;
        refused = e.message;
        return;
      }
      await _start();
    });
    return refused;
  }

  Future<void> _start() async {
    _set(Phase.starting);
    final session = await _backend.start();
    if (_disposed) {
      unawaited(session.stop());
      return;
    }
    _session = session;
    _set(Phase.running);
  }

  /// Runs [body], and shows the failure screen if it throws.
  Future<void> _guard(Future<void> Function() body) async {
    try {
      await body();
    } on BatonError catch (e) {
      _fail(e.message);
    } on Object catch (e) {
      // Not something Baton said, such as a library that could not be loaded. The person
      // cannot fix it, but can tell us what it was.
      _fail('Something unexpected went wrong: $e');
    }
  }

  void _fail(String problem) {
    _problem = problem;
    _set(Phase.failed);
  }

  void _set(Phase phase) {
    if (_disposed) return;
    _phase = phase;
    notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    final session = _session;
    _session = null;
    if (session != null) unawaited(session.stop());
    super.dispose();
  }
}

/// What the devices screen shows, kept up to date from a running Baton.
class DevicesController extends ChangeNotifier {
  DevicesController(
    this._session, {
    this._networkEvery = const Duration(seconds: 3),
  });

  final Session _session;
  final Duration _networkEvery;
  StreamSubscription<Event>? _events;
  Timer? _timer;
  bool _disposed = false;
  bool _readingDevices = false;
  bool _readingNetwork = false;
  List<Device>? _devices;
  Network? _network;

  /// This device.
  Me get me => _session.me;

  /// The devices paired with this one, or `null` until they have been read.
  List<Device>? get devices => _devices;

  /// Where this device can be reached, or `null` until it has been read.
  Network? get network => _network;

  /// Starts listening for events and reads what the screen shows.
  Future<void> start() async {
    // Listening comes first, so that nothing that happens while the lists are read is lost.
    final events = await _session.events();
    if (_disposed) {
      // Nobody is left to read them. A stream is told to stop only when someone cancels it.
      unawaited(events.listen(null).cancel());
      return;
    }
    _events = events.listen(
      _onEvent,
      onError: (Object e) => debugPrint('Baton: events: $e'),
    );
    await Future.wait([_readDevices(), _readNetwork()]);
    if (_disposed) return;
    // There is no event for the network. Asking is cheap, because Baton runs in this process.
    _timer = Timer.periodic(_networkEvery, (_) => unawaited(_readNetwork()));
  }

  void _onEvent(Event event) {
    switch (event) {
      case Event_DevicesChanged() || Event_Resync():
        unawaited(_readDevices());
      default:
        // The other events are for the screens that pair devices and show the inbox.
        break;
    }
  }

  Future<void> _readDevices() async {
    if (_readingDevices) return;
    _readingDevices = true;
    try {
      final devices = await _session.devices();
      if (_disposed) return;
      _devices = devices;
      notifyListeners();
    } on BatonError catch (e) {
      // Baton has stopped, which only happens as the app closes.
      debugPrint('Baton: devices: ${e.message}');
    } finally {
      _readingDevices = false;
    }
  }

  Future<void> _readNetwork() async {
    if (_readingNetwork) return;
    _readingNetwork = true;
    try {
      final network = await _session.network();
      if (_disposed) return;
      _network = network;
      notifyListeners();
    } on BatonError catch (e) {
      debugPrint('Baton: network: ${e.message}');
    } finally {
      _readingNetwork = false;
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    // A stream from the bridge finishes a cancel only when it next has an event, so do not
    // wait for it.
    unawaited(_events?.cancel());
    super.dispose();
  }
}
