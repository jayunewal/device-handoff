// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/permissions.dart';

/// Says what it is told about each permission, and keeps what it was asked for and to open.
class FakeSystemSettings extends SystemSettingsKeeper {
  FakeSystemSettings([Map<Permission, PermissionState>? states])
    : states = {...?states};

  final Map<Permission, PermissionState> states;

  /// What a request turns a permission that was not asked yet into.
  PermissionState answer = PermissionState.on;

  /// Whether Settings opens.
  bool opens = true;

  final List<Permission> requested = [];
  final List<Permission> opened = [];

  @override
  Future<PermissionState?> state(Permission permission) async =>
      states[permission];

  @override
  Future<PermissionState?> request(Permission permission) async {
    requested.add(permission);
    if (states[permission] == PermissionState.notAsked) {
      states[permission] = answer;
    }
    return states[permission];
  }

  @override
  Future<bool> open(Permission permission) async {
    opened.add(permission);
    return opens;
  }
}
