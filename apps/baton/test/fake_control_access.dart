// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/control_access.dart';

/// Says what a test wants the system to say about controlling windows of this device.
class FakeControlAccess extends ControlAccessKeeper {
  FakeControlAccess([this.access = ControlAccess.allowed]);

  /// What `state` gives.
  ControlAccess access;

  /// How many times `state` was asked.
  int asked = 0;

  /// How many times `request` was called.
  int requests = 0;

  @override
  Future<ControlAccess> state() async {
    asked++;
    return access;
  }

  @override
  Future<void> request() async => requests++;
}

/// Says what a test wants the system to say about recording the screen of this device.
class FakeScreenAccess extends ScreenAccessKeeper {
  FakeScreenAccess([this.access = ScreenAccess.allowed]);

  /// What `state` gives.
  ScreenAccess access;

  /// How many times `state` was asked.
  int asked = 0;

  /// How many times `request` was called.
  int requests = 0;

  @override
  Future<ScreenAccess> state() async {
    asked++;
    return access;
  }

  @override
  Future<void> request() async => requests++;
}
