// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/foundation.dart' show TargetPlatform;
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

/// Whether a device that runs this system gives its screen when it is asked to, so that it is
/// worth offering to ask it. A Mac does. The others have no way to yet.
bool givesScreenWhenAsked(Platform platform) => platform == Platform.macOs;

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

/// "1 file" and "3 files".
String fileCount(int n) => _count(n, 'file');

/// "1 folder" and "3 folders".
String folderCount(int n) => _count(n, 'folder');

/// "1 item" and "3 items".
String itemCount(int n) => _count(n, 'item');

/// A size as the Mac and the iPhone write one, in units of a thousand: "950 bytes", "1.2 MB",
/// "31 GB".
String fileSize(int bytes) {
  if (bytes < 1000) return bytes == 1 ? '1 byte' : '$bytes bytes';
  const units = ['KB', 'MB', 'GB', 'TB'];
  // One decimal below 10 and none above, and a size that rounds up to 1000 is the next unit.
  num rounded(double value) =>
      value < 10 ? (value * 10).round() / 10 : value.round();
  var value = bytes / 1000;
  var unit = 0;
  while (rounded(value) >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  final shown = rounded(value);
  return '${shown < 10 ? shown.toStringAsFixed(1) : shown.toStringAsFixed(0)} '
      '${units[unit]}';
}

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
//
// Control characters other than the tab and the line ends; the Bidi_Control characters of
// Unicode (the marks U+061C, U+200E and U+200F, which move digits and punctuation on their own,
// the embeddings and overrides U+202A to U+202E, and the isolates U+2066 to U+2069); and the
// ones that have no glyph: the zero-width space, the word joiner and the invisible operators
// (U+2060 to U+2064), the deprecated format characters (U+206A to U+206F), the byte order mark
// and the annotation marks (U+FFF9 to U+FFFB). The zero-width joiners U+200C and U+200D stay,
// because emoji sequences and several scripts need them, and so do variation selectors.
final _hidden = RegExp(
  r'[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F'
  r'\u061C\u200B\u200E\u200F\u202A-\u202E\u2060-\u2064\u2066-\u206F\uFEFF\uFFF9-\uFFFB]',
);

/// [text] from another device, made fit to show: line ends are the same everywhere, and each
/// character that cannot be seen or that turns text round is shown as a replacement mark (�).
/// Copy the original, not this.
String visibleText(String text) => text
    .replaceAll('\r\n', '\n')
    .replaceAll('\r', '\n')
    .replaceAll(_hidden, '\uFFFD');

/// What a transfer holds, in a few words: "3 files, 12 MB", or "3 files in 2 folders, 12 MB".
String describeContents({
  required int files,
  required int folders,
  required int bytes,
}) {
  final what = folders == 0
      ? fileCount(files)
      : '${fileCount(files)} in ${folderCount(folders)}';
  return '$what, ${fileSize(bytes)}';
}

/// What a transfer that is being sent is doing, in a sentence for the person.
String describeStage(SendStage stage, String device) => switch (stage) {
  SendStage.connecting => 'Reaching $device…',
  SendStage.asking => 'Waiting for $device to say yes…',
  SendStage.sending => 'Sending to $device…',
  SendStage.finishing => '$device is putting the files in place…',
  SendStage.waiting =>
    'The connection stopped. Baton will try again in a moment…',
  SendStage.over => 'Done.',
};

/// How a transfer that this device sent ended, as a sentence.
String describeSent(FilesSent sent, String device) {
  final arrived = sent.files - sent.failed;
  final sentences = <String>[
    if (sent.failed == 0)
      'Sent ${fileCount(sent.files)} (${fileSize(sent.bytes)}) to $device.'
    else if (arrived <= 0)
      '$device could not keep ${sent.failed == 1 ? 'the file' : 'the files'}.'
    else
      'Sent $arrived of ${fileCount(sent.files)} to $device. '
          '${fileCount(sent.failed)} could not be saved there.',
    if (sent.leftOut > 0)
      '${itemCount(sent.leftOut)} in what you chose '
          '${sent.leftOut == 1 ? 'was' : 'were'} left out: links inside '
          'folders, files that could not be read and files that are too large.',
  ];
  return sentences.join(' ');
}

/// Where received files are, in words for the person: "your Downloads folder" on a Mac and "the
/// Baton folder in the Files app" on an iPhone or an iPad. [path] is the folder that Baton was
/// given for them.
String describeReceiveFolder(String path, TargetPlatform platform) =>
    switch (platform) {
      TargetPlatform.iOS => 'the Baton folder in the Files app',
      TargetPlatform.macOS when path.endsWith('/Downloads') =>
        'your Downloads folder',
      _ => 'the folder $path',
    };

/// What the button that takes the person to received files says: it names the app that shows
/// them, where Baton knows which it is.
String describeShowFiles(TargetPlatform platform) => switch (platform) {
  TargetPlatform.iOS => 'Show in Files',
  TargetPlatform.macOS => 'Show in Finder',
  _ => 'Show files',
};
