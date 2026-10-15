// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/material.dart' show IconData, Icons;

import 'backend.dart';

/// The system a device runs, as a person would say it.
String platformName(Platform platform) => switch (platform) {
  Platform.macOs => 'Mac',
  Platform.windows => 'Windows PC',
  Platform.linux => 'Linux PC',
  // An iPad says that it is an iPhone, until the app layer can tell them apart.
  Platform.ios => 'iPhone or iPad',
  Platform.ipadOs => 'iPad',
  Platform.android => 'Android',
  Platform.other => 'Other device',
};

/// An icon for the system a device runs.
IconData platformIcon(Platform platform) => switch (platform) {
  Platform.macOs => Icons.laptop_mac,
  Platform.windows => Icons.desktop_windows,
  Platform.linux => Icons.computer,
  Platform.ios => Icons.phone_iphone,
  Platform.ipadOs => Icons.tablet_mac,
  Platform.android => Icons.phone_android,
  Platform.other => Icons.devices_other,
};

const _months = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

/// When a device was last heard from, in words: "seen 5 minutes ago", "seen on Oct 14" or
/// "not seen yet". [seconds] counts from the Unix epoch, as the app layer gives it.
String lastSeen(int? seconds, DateTime now) {
  if (seconds == null) return 'not seen yet';
  final then = DateTime.fromMillisecondsSinceEpoch(seconds * 1000);
  final ago = now.difference(then);
  if (ago.inMinutes < 1) return 'seen just now';
  if (ago.inHours < 1) return 'seen ${_count(ago.inMinutes, 'minute')} ago';
  if (ago.inDays < 1) return 'seen ${_count(ago.inHours, 'hour')} ago';
  if (ago.inDays < 7) return 'seen ${_count(ago.inDays, 'day')} ago';
  final day = '${_months[then.month - 1]} ${then.day}';
  return then.year == now.year ? 'seen on $day' : 'seen on $day, ${then.year}';
}

String _count(int n, String unit) => n == 1 ? '1 $unit' : '$n ${unit}s';

/// How long is left until [expiresAt], counted in seconds from the Unix epoch as the app layer
/// gives it: "4:32", and "0:00" once it has passed.
String timeLeft(int expiresAt, DateTime now) {
  final milliseconds = expiresAt * 1000 - now.millisecondsSinceEpoch;
  final seconds = milliseconds <= 0 ? 0 : (milliseconds / 1000).ceil();
  return '${seconds ~/ 60}:${(seconds % 60).toString().padLeft(2, '0')}';
}

/// Makes a message from the library read as a sentence to the person: a capital letter at the
/// start, and a full stop at the end unless there is a mark there.
String sentence(String message) {
  final text = message.trim();
  if (text.isEmpty) return text;
  final capital = text[0].toUpperCase() + text.substring(1);
  return '.!?'.contains(capital[capital.length - 1]) ? capital : '$capital.';
}
