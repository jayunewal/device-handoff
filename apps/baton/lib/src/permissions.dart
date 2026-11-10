// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'notice_access.dart';

/// What Baton can ask the system to allow, one row each on the permissions screen.
enum Permission {
  /// Accessibility, on a Mac: a private screen holds the Mac's own keys and pointer still, and a
  /// device that is let control this Mac moves its pointer and types.
  accessibility,

  /// Input Monitoring, on a Mac: a private screen, the default for a session that a phone starts,
  /// cannot start without it, since it holds the Mac's own keys and pointer.
  inputMonitoring,

  /// Screen Recording, on a Mac: a device that is let look sees its screen.
  screenRecording,

  /// System Audio Recording, on a Mac: a device that is let look hears it too. The system asks the
  /// person once, the first time Baton reads the sound, and does not tell Baton the answer.
  systemAudio,

  /// Notifications, on a Mac: the person is told when a device unlocked it, and when a device
  /// sends something or asks to while Baton has no window in front.
  notifications,

  /// The camera, to read the code that another device shows when the two pair.
  camera,

  /// Face ID, on an iPhone or iPad: it checks that it is the person before a Mac is opened or
  /// unlocked.
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

/// What the system says about the permissions it keeps on a channel of Baton's own (all but
/// Notifications, which has a channel of its own), and what takes the person to the screen of
/// Settings where a permission is turned on. Tests give it one of their
/// own.
abstract class SystemSettingsKeeper {
  const SystemSettingsKeeper();

  /// Where [permission] stands, or null when the device cannot do it, such as a Mac with no camera
  /// or a device with no Face ID, or the system does not say.
  Future<PermissionState?> state(Permission permission);

  /// Asks the system for [permission]: the system asks the person once, and Baton opens Settings
  /// for any other time. Says where it then stands.
  Future<PermissionState?> request(Permission permission);

  /// Opens Settings where [permission] is turned on. Says whether it opened.
  Future<bool> open(Permission permission);
}

class PlatformSystemSettings extends SystemSettingsKeeper {
  const PlatformSystemSettings();

  /// The names of the channel and of the questions, the names of the permissions and the words of
  /// the answers must be the ones in macos/Runner/MainFlutterWindow.swift and
  /// ios/Runner/AppDelegate.swift.
  static const channelName = 'io.github.jayunewal.baton/permissions';
  static const stateQuestion = 'state';
  static const requestQuestion = 'request';
  static const openQuestion = 'openSettings';
  static const answers = {
    'allowed': PermissionState.on,
    'denied': PermissionState.off,
    'needed': PermissionState.notAsked,
    'notSetUp': PermissionState.notSetUp,
  };

  static const _channel = MethodChannel(channelName);

  @override
  Future<PermissionState?> state(Permission permission) async =>
      answers[await _ask(stateQuestion, permission)];

  @override
  Future<PermissionState?> request(Permission permission) async =>
      answers[await _ask(requestQuestion, permission)];

  @override
  Future<bool> open(Permission permission) async =>
      await _ask(openQuestion, permission) == true;

  Future<Object?> _ask(String question, Permission permission) async {
    if (defaultTargetPlatform != TargetPlatform.macOS &&
        defaultTargetPlatform != TargetPlatform.iOS) {
      return null;
    }
    try {
      return await _channel.invokeMethod<Object>(question, permission.name);
    } on PlatformException catch (e) {
      debugPrint('Baton: permissions: $question: $e');
    } on MissingPluginException catch (e) {
      debugPrint('Baton: permissions: $question: $e');
    }
    return null;
  }
}

/// Everything Baton can ask the system for on [platform], and where each stands, in the order the
/// permissions screen shows them. What the device cannot do is left out: a Mac with no camera has
/// no row for the camera, and an iPad with no Face ID none for Face ID.
Future<List<PermissionCheck>> checkPermissions({
  required TargetPlatform platform,
  required NoticeKeeper notices,
  required SystemSettingsKeeper system,
}) async {
  final checks = <PermissionCheck>[];
  Future<void> add(Permission permission, PermissionState? state) async {
    if (state != null) checks.add(PermissionCheck(permission, state));
  }

  switch (platform) {
    case TargetPlatform.macOS:
      await add(
        Permission.accessibility,
        await system.state(Permission.accessibility),
      );
      await add(
        Permission.inputMonitoring,
        await system.state(Permission.inputMonitoring),
      );
      await add(
        Permission.screenRecording,
        await system.state(Permission.screenRecording),
      );
      // The system does not say whether Baton may record the sound, so the person looks.
      await add(Permission.systemAudio, PermissionState.cannotCheck);
      await add(Permission.notifications, switch (await notices.state()) {
        NoticeAccess.allowed => PermissionState.on,
        NoticeAccess.needed => PermissionState.notAsked,
        NoticeAccess.denied => PermissionState.off,
        NoticeAccess.unavailable => null,
      });
      await add(Permission.camera, await system.state(Permission.camera));
    case TargetPlatform.iOS:
      await add(Permission.faceId, await system.state(Permission.faceId));
      await add(Permission.camera, await system.state(Permission.camera));
    default:
      return checks;
  }
  checks.add(
    const PermissionCheck(Permission.localNetwork, PermissionState.cannotCheck),
  );
  return checks;
}
