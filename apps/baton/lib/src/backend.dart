// SPDX-License-Identifier: Apache-2.0

// What the screens need from Baton, as two small interfaces. The app is written against these,
// so that widget tests can run it on any computer with a fake behind them, and so that the one
// place that touches the Rust library is rust_backend.dart.

import 'rust/api/model.dart';

export 'rust/api/error.dart' show BatonError, ErrorKind;
export 'rust/api/model.dart'
    show
        Device,
        Event,
        Event_DevicesChanged,
        Event_Resync,
        Me,
        Network,
        Platform;

/// Opens the folder where Baton keeps its files and starts Baton in it.
abstract interface class Backend {
  /// The name saved for this device, or `null` if the person has not chosen one yet.
  Future<String?> deviceName();

  /// A name to offer the person who has to choose one.
  String suggestedName();

  /// Saves the name for this device. Throws a `BatonError` of kind `badName`, whose message can
  /// be shown as it is, if there is nothing visible in it.
  Future<void> setDeviceName(String name);

  /// Starts Baton. Throws a `BatonError` that says why not.
  Future<Session> start();
}

/// A Baton that is running.
abstract interface class Session {
  /// This device.
  Me get me;

  /// Events from now on. Wait for the stream before reading the lists, so that nothing that
  /// happens in between is lost: once it is here, Baton is sending to it. It ends when Baton
  /// stops.
  Future<Stream<Event>> events();

  /// The devices paired with this one.
  Future<List<Device>> devices();

  /// Where this device can be reached right now.
  Future<Network> network();

  /// Stops Baton and lets go of its folder.
  Future<void> stop();
}
