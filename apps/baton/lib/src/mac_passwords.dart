// SPDX-License-Identifier: Apache-2.0

import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'rust/api/mac_unlock.dart';

/// A Mac that this phone can unlock after a restart, and how to reach it. Its password is kept
/// apart, behind the person's face ([MacPasswordKeeper.open]).
@immutable
class SavedMac {
  const SavedMac({
    required this.id,
    required this.host,
    required this.account,
    required this.keys,
    this.port = 22,
  });

  /// Names this Mac among the saved ones. It means nothing outside this phone.
  final String id;

  /// The Mac's name on the network, such as `Jays-MBP.local`, or its address.
  final String host;

  /// The port of its SSH server.
  final int port;

  /// The short name of the account whose password unlocks the Mac.
  final String account;

  /// The fingerprints of the host keys that the person trusts for this Mac, as `ssh-keygen -l`
  /// writes them. A Mac that answers with another key is given nothing.
  final List<String> keys;

  SavedMac copyWith({
    String? host,
    int? port,
    String? account,
    List<String>? keys,
  }) => SavedMac(
    id: id,
    host: host ?? this.host,
    port: port ?? this.port,
    account: account ?? this.account,
    keys: keys ?? this.keys,
  );

  String toJson() => jsonEncode({
    'id': id,
    'host': host,
    'port': port,
    'account': account,
    'keys': keys,
  });

  /// The record that [toJson] wrote, or `null` for text that is not one.
  static SavedMac? fromJson(String text) {
    try {
      final map = jsonDecode(text);
      if (map is! Map<String, dynamic>) return null;
      final keys = map['keys'];
      final mac = SavedMac(
        id: map['id'] as String,
        host: map['host'] as String,
        port: map['port'] as int,
        account: map['account'] as String,
        keys: keys is List ? keys.whereType<String>().toList() : const [],
      );
      return mac;
    } on Object {
      return null;
    }
  }

  @override
  bool operator ==(Object other) =>
      other is SavedMac &&
      other.id == id &&
      other.host == host &&
      other.port == port &&
      other.account == account &&
      listEquals(other.keys, keys);

  @override
  int get hashCode =>
      Object.hash(id, host, port, account, Object.hashAll(keys));
}

/// Why a password could not be kept or opened, in a sentence for the person.
class MacPasswordProblem implements Exception {
  const MacPasswordProblem(this.message);

  final String message;

  @override
  String toString() => message;
}

/// Where the phone keeps the saved Macs and their passwords.
abstract class MacPasswordKeeper {
  const MacPasswordKeeper();

  /// The saved Macs.
  Future<List<SavedMac>> list();

  /// Saves `mac`, with `password` when there is one. Without one, the password that is kept
  /// stays as it is. Throws a [MacPasswordProblem] when it cannot be kept.
  Future<void> save(SavedMac mac, {String? password});

  /// The password of the Mac `id`, once the person has shown their face, or `null` if they
  /// stopped. Throws a [MacPasswordProblem] when it cannot be opened.
  Future<String?> open(String id, {required String reason});

  /// Forgets the Mac `id` and its password.
  Future<void> forget(String id);
}

/// The keychain of an iPhone or an iPad, through the code in ios/Runner/AppDelegate.swift.
class PlatformMacPasswordKeeper extends MacPasswordKeeper {
  const PlatformMacPasswordKeeper();

  /// The name must be the one in ios/Runner/AppDelegate.swift.
  static const channelName = 'io.github.jayunewal.baton/mac-passwords';

  static const _channel = MethodChannel(channelName);

  /// The keeper of this device, or `null` where there is none: only an iPhone or an iPad unlocks
  /// a Mac after a restart for now.
  static MacPasswordKeeper? forThisDevice() =>
      defaultTargetPlatform == TargetPlatform.iOS
      ? const PlatformMacPasswordKeeper()
      : null;

  @override
  Future<List<SavedMac>> list() async {
    final records = await _channel.invokeListMethod<String>('list') ?? const [];
    return records.map(SavedMac.fromJson).whereType<SavedMac>().toList();
  }

  @override
  Future<void> save(SavedMac mac, {String? password}) async {
    try {
      await _channel.invokeMethod<void>('save', {
        'id': mac.id,
        'record': mac.toJson(),
        'password': ?password,
      });
    } on PlatformException catch (e) {
      throw MacPasswordProblem(
        e.code == 'no_face'
            ? 'Set up Face ID and a passcode on this device first. '
                  'The password is kept behind your face.'
            : 'The password could not be saved (${e.code}).',
      );
    }
  }

  @override
  Future<String?> open(String id, {required String reason}) async {
    try {
      return await _channel.invokeMethod<String>('open', {
        'id': id,
        'reason': reason,
      });
    } on PlatformException catch (e) {
      switch (e.code) {
        case 'cancelled':
          return null;
        case 'gone':
          throw const MacPasswordProblem(
            'The saved password is gone. This happens when a face is added or removed '
            'on this device. Enter the password again.',
          );
        case 'not_recognised':
          throw const MacPasswordProblem('Your face was not recognised.');
        default:
          throw MacPasswordProblem(
            'The password could not be opened (${e.code}).',
          );
      }
    }
  }

  @override
  Future<void> forget(String id) =>
      _channel.invokeMethod<void>('forget', {'id': id});
}

/// Talks to the Mac over SSH.
abstract class SshUnlocker {
  const SshUnlocker();

  /// Asks the Mac for its host key and gives it nothing.
  Future<SshKeyCheck> key(String host, int port);

  /// Gives the Mac its password, if it answers with one of the keys that `mac` trusts.
  Future<SshUnlock> unlock(SavedMac mac, String password);
}

/// The SSH client in the Rust library.
class RustSshUnlocker extends SshUnlocker {
  const RustSshUnlocker();

  @override
  Future<SshKeyCheck> key(String host, int port) =>
      sshMacKey(host: host, port: port);

  @override
  Future<SshUnlock> unlock(SavedMac mac, String password) => sshUnlockMac(
    host: mac.host,
    port: mac.port,
    account: mac.account,
    password: password,
    trusted: mac.keys,
  );
}
