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

/// How long ago something happened, in words: "just now", "5 minutes ago" or "on Oct 14".
/// [seconds] counts from the Unix epoch, as the app layer gives it.
String since(int seconds, DateTime now) {
  final then = DateTime.fromMillisecondsSinceEpoch(seconds * 1000);
  final ago = now.difference(then);
  if (ago.inMinutes < 1) return 'just now';
  if (ago.inHours < 1) return '${_count(ago.inMinutes, 'minute')} ago';
  if (ago.inDays < 1) return '${_count(ago.inHours, 'hour')} ago';
  if (ago.inDays < 7) return '${_count(ago.inDays, 'day')} ago';
  final day = '${_months[then.month - 1]} ${then.day}';
  return then.year == now.year ? 'on $day' : 'on $day, ${then.year}';
}

/// When a device was last heard from, in words: "seen 5 minutes ago", "seen on Oct 14" or
/// "not seen yet".
String lastSeen(int? seconds, DateTime now) =>
    seconds == null ? 'not seen yet' : 'seen ${since(seconds, now)}';

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

/// What a ping found, as a sentence: "Kitchen iPad answered in 24 ms, over your local
/// network."
String describeReach(String name, Reach reach) {
  final route = switch (reach.route) {
    Route.lan => ', over your local network',
    Route.direct => ', directly over the internet',
    Route.relay => ', through the relay, which is slower',
    Route.unknown => '',
  };
  return '$name answered in ${reach.roundTripMs} ms$route.';
}

// What another device sends is shown as it is, except for the characters that change which
// way text runs or that no one can see. A line that reads "https://example.com" can be made to
// show something else with them, and the copy a person pastes would not be what they read.
final _hidden = RegExp(
  r'[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u202A-\u202E\u2066-\u2069]',
);

/// [text] from another device, made fit to show: line ends are the same everywhere, and each
/// character that cannot be seen or that turns text round is shown as a replacement mark (�).
/// Copy the original, not this.
String visibleText(String text) => text
    .replaceAll('\r\n', '\n')
    .replaceAll('\r', '\n')
    .replaceAll(_hidden, '\uFFFD');
