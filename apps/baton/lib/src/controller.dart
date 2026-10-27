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
  bool _readDevicesAgain = false;
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
      case Event_UnlockAttempted():
        // The record of the device may have ended with it, and its history has a line more.
        notifyListeners();
      default:
        // The other events are for the screens that pair devices and show the inbox.
        break;
    }
  }

  /// Forgets [device]. Returns what to tell the person if that did not work, and `null` if it
  /// did.
  Future<String?> forget(Device device) async {
    try {
      await _session.unpair(device.id);
    } on BatonError catch (e) {
      // Gone already, say because the other device forgot this one first: that is what
      // was wanted.
      if (e.kind != ErrorKind.notPaired) return e.message;
    }
    unawaited(_readDevices());
    return null;
  }

  /// What [device] may do at this device without a person here being asked: ask for its screen.
  /// `null` if it is not paired with this device any more, say because it forgot this one first.
  StartAccess? startAccess(Device device) {
    try {
      return _session.startAccess(device.id);
    } on BatonError {
      return null;
    }
  }

  /// Lets [device] ask for the screen of this device as [access] says, or does not. Returns what
  /// to tell the person if that did not work, and `null` if it did.
  Future<String?> setStartAccess(Device device, StartAccess access) async {
    try {
      await _session.setStartAccess(device.id, access);
      return null;
    } on BatonError catch (e) {
      return e.message;
    }
  }

  /// What the person here said of [device] hearing the sound of this device's screen: `true` that
  /// it may, `false` that it may not, and `null` when they have not said (or the device is not
  /// paired with this one any more), and then it may in a session that lets it control the screen.
  bool? hearSetting(Device device) {
    try {
      return _session.hearSetting(device.id);
    } on BatonError {
      return null;
    }
  }

  /// Says whether [device] may hear the sound of this device's screen, or takes that back with
  /// `null`. Returns what to tell the person if that did not work, and `null` if it did.
  Future<String?> setHearSetting(Device device, bool? setting) async {
    try {
      await _session.setHearSetting(device.id, setting);
      return null;
    } on BatonError catch (e) {
      return e.message;
    }
  }

  /// How the unlock of [device] stands at this device: whether the person set it up so that the
  /// device can unlock this one while it is locked, and whether it ended since. `null` if it is not
  /// paired with this device any more, say because it forgot this one first.
  UnlockStatus? unlockStatus(Device device) {
    try {
      return _session.unlockStatus(device.id);
    } on BatonError {
      return null;
    }
  }

  /// Sets the unlock of this device up for [device], with [password], the password of the person
  /// who uses this device. The call waits for the person at [device] if the platform there asks
  /// them. The password is given to Baton and kept nowhere here. A failure comes as
  /// `UnlockSetUp_NotDone` with its sentence, so that the screen shows every way it can end alike.
  Future<UnlockSetUp> setUpUnlock(Device device, String password) async {
    try {
      return await _session.setUpUnlock(device.id, password);
    } on BatonError catch (e) {
      return UnlockSetUp.notDone(message: e.message);
    }
  }

  /// Turns the unlock for [device] off. Returns what to tell the person if that did not work, and
  /// `null` if it did, also when it was off already.
  Future<String?> turnOffUnlock(Device device) async {
    try {
      await _session.turnOffUnlock(device.id);
      return null;
    } on BatonError catch (e) {
      return e.message;
    }
  }

  /// The last attempts of [device] to unlock this one, the latest first, up to [limit].
  List<UnlockEntry> unlockAttempts(Device device, {int limit = 5}) => [
    for (final entry in _session.unlockHistory())
      if (entry.deviceId == device.id) entry,
  ].take(limit).toList();

  Future<void> _readDevices() async {
    if (_readingDevices) {
      // Something changed after the read that is going on began, so read again after it.
      _readDevicesAgain = true;
      return;
    }
    _readingDevices = true;
    try {
      do {
        _readDevicesAgain = false;
        final devices = await _session.devices();
        if (_disposed) return;
        _devices = devices;
        notifyListeners();
      } while (_readDevicesAgain);
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
