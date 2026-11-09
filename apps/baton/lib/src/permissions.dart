// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'control_access.dart';
import 'notice_access.dart';

/// What Baton can ask the system to allow, one row each on the permissions screen.
enum Permission {
  /// Accessibility, on a Mac: a device that is let control this Mac moves its pointer and types.
  control,

  /// Screen and System Audio Recording, on a Mac: a device that is let look sees and hears it.
  screenRecording,

  /// Notifications, on a Mac: the person is told when a device unlocked it.
  notifications,

  /// The camera, to read the code that another device shows when the two pair.
  camera,

  /// Face ID, on an iPhone or iPad: it checks that it is the person before a Mac is unlocked.
  faceId,

  /// The local network: devices on the same network connect straight to each other. The system
  /// says nothing about it to the app, so Baton can only take the person to where it is set.
  localNetwork,
}

/// Where a permission stands.
enum PermissionState {
  on,

  off,

  /// The system has not asked the person yet. Asking is one tap, with no trip to Settings.
  notAsked,

  /// Face ID is not set up on the device.
  notSetUp,

  /// The system does not say. The person can look in Settings.
  cannotCheck,
}

/// One permission and where it stands.
@immutable
class PermissionCheck {
  const PermissionCheck(this.permission, this.state);

  final Permission permission;
  final PermissionState state;

  /// Whether the person has something to do about it.
  bool get needsAttention =>
      state == PermissionState.off ||
      state == PermissionState.notAsked ||
      state == PermissionState.notSetUp;

  @override
  bool operator ==(Object other) =>
      other is PermissionCheck &&
      other.permission == permission &&
      other.state == state;

  @override
  int get hashCode => Object.hash(permission, state);

  @override
  String toString() => '${permission.name}: ${state.name}';
}

/// What the system says about the camera and Face ID, and what takes the person to the screen of
/// Settings where a permission is turned on. Tests give it one of their own.
abstract class SystemSettingsKeeper {
  const SystemSettingsKeeper();

  /// Where the camera stands, or null when the device has none or the system does not say.
  Future<PermissionState?> camera();

  /// Asks the person to let Baton use the camera, and says where it then stands.
  Future<PermissionState?> requestCamera();

  /// Where Face ID stands, or null when the device has no Face ID or this is not an iPhone or iPad.
  Future<PermissionState?> faceId();

  /// Asks the person to let Baton use Face ID, and says where it then stands.
  Future<PermissionState?> requestFaceId();

  /// Opens Settings where [permission] is turned on. Says whether it opened.
  Future<bool> open(Permission permission);
}

class PlatformSystemSettings extends SystemSettingsKeeper {
  const PlatformSystemSettings();

  static const channelName = 'io.github.jayunewal.baton/permissions';

  static const _channel = MethodChannel(channelName);

  @override
  Future<PermissionState?> camera() async => _state(await _ask('camera'));

  @override
  Future<PermissionState?> requestCamera() async =>
      _state(await _ask('requestCamera'));

  @override
  Future<PermissionState?> faceId() async => _state(await _ask('faceId'));

  @override
  Future<PermissionState?> requestFaceId() async =>
      _state(await _ask('requestFaceId'));

  @override
  Future<bool> open(Permission permission) async =>
      await _ask('openSettings', permission.name) == true;

  /// The words of the answers must be the ones in macos/Runner/MainFlutterWindow.swift and
  /// ios/Runner/AppDelegate.swift.
  static PermissionState? _state(Object? answer) => switch (answer) {
    'allowed' => PermissionState.on,
    'denied' => PermissionState.off,
    'needed' => PermissionState.notAsked,
    'notSetUp' => PermissionState.notSetUp,
    _ => null,
  };

  Future<Object?> _ask(String question, [Object? argument]) async {
    if (defaultTargetPlatform != TargetPlatform.macOS &&
        defaultTargetPlatform != TargetPlatform.iOS) {
      return null;
    }
    try {
      return await _channel.invokeMethod<Object>(question, argument);
    } on PlatformException catch (e) {
      debugPrint('Baton: permissions: $question: $e');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: permissions: $question: $e');
    }
    return null;
  }
}

/// Everything Baton can ask the system for on [platform], and where each stands, in the order the
/// permissions screen shows them. What the device cannot do is left out: a Mac too old to be
/// controlled has no row for Accessibility, and an iPad with no Face ID none for Face ID.
Future<List<PermissionCheck>> checkPermissions({
  required TargetPlatform platform,
  required ControlAccessKeeper controlAccess,
  required ScreenAccessKeeper screenAccess,
  required NoticeKeeper notices,
  required SystemSettingsKeeper system,
}) async {
  final checks = <PermissionCheck>[];
  switch (platform) {
    case TargetPlatform.macOS:
      final control = await controlAccess.state();
      if (control != ControlAccess.unavailable) {
        checks.add(
          PermissionCheck(
            Permission.control,
            control == ControlAccess.allowed
                ? PermissionState.on
                : PermissionState.off,
          ),
        );
      }
      final screen = await screenAccess.state();
      if (screen != ScreenAccess.unavailable) {
        checks.add(
          PermissionCheck(
            Permission.screenRecording,
            screen == ScreenAccess.allowed
                ? PermissionState.on
                : PermissionState.off,
          ),
        );
      }
      final notice = switch (await notices.state()) {
        NoticeAccess.allowed => PermissionState.on,
        NoticeAccess.needed => PermissionState.notAsked,
        NoticeAccess.denied => PermissionState.off,
        NoticeAccess.unavailable => null,
      };
      if (notice != null) {
        checks.add(PermissionCheck(Permission.notifications, notice));
      }
      final camera = await system.camera();
      if (camera != null) {
        checks.add(PermissionCheck(Permission.camera, camera));
      }
    case TargetPlatform.iOS:
      final faceId = await system.faceId();
      if (faceId != null) {
        checks.add(PermissionCheck(Permission.faceId, faceId));
      }
      final camera = await system.camera();
      if (camera != null) {
        checks.add(PermissionCheck(Permission.camera, camera));
      }
    default:
      return checks;
  }
  checks.add(
    const PermissionCheck(Permission.localNetwork, PermissionState.cannotCheck),
  );
  return checks;
}
