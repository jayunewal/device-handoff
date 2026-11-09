// SPDX-License-Identifier: Apache-2.0

import 'package:baton/src/permissions.dart';

/// Says what it is told about the camera and Face ID, and keeps what it was asked to open.
class FakeSystemSettings extends SystemSettingsKeeper {
  FakeSystemSettings({this.cameraState, this.faceIdState});

  PermissionState? cameraState;
  PermissionState? faceIdState;

  /// What a request turns a permission that was not asked yet into.
  PermissionState answer = PermissionState.on;

  /// Whether Settings opens.
  bool opens = true;

  final List<Permission> opened = [];
  int cameraRequests = 0;
  int faceIdRequests = 0;

  @override
  Future<PermissionState?> camera() async => cameraState;

  @override
  Future<PermissionState?> requestCamera() async {
    cameraRequests++;
    if (cameraState == PermissionState.notAsked) cameraState = answer;
    return cameraState;
  }

  @override
  Future<PermissionState?> faceId() async => faceIdState;

  @override
  Future<PermissionState?> requestFaceId() async {
    faceIdRequests++;
    if (faceIdState == PermissionState.notAsked) faceIdState = answer;
    return faceIdState;
  }

  @override
  Future<bool> open(Permission permission) async {
    opened.add(permission);
    return opens;
  }
}
