// SPDX-License-Identifier: Apache-2.0

import 'dart:io' as io;

import 'package:flutter/foundation.dart';

import 'backend.dart';
import 'format.dart';

/// What the operating system says it is, on one line.
String describeSystem() {
  final system =
      '${io.Platform.operatingSystem} ${io.Platform.operatingSystemVersion}';
  return system.replaceAll(RegExp(r'\s+'), ' ');
}

/// What the Diagnostics screen shows and copies: a few facts about this device and how it is
/// connected, and the last lines the library logged.
@immutable
class Diagnostics {
  const Diagnostics({required this.facts, required this.log});

  /// When it was read, then facts about the system, this device and its connection, one to a
  /// line.
  final List<String> facts;

  /// The library's log, oldest line first.
  final List<String> log;

  /// How much of the log there is, in words.
  String get logSummary => log.isEmpty
      ? 'Nothing has been logged.'
      : '${_lines(log.length)}, oldest first.';

  /// All of it as text, for a bug report.
  String get report {
    final text = StringBuffer('Baton diagnostics\n\n');
    for (final fact in facts) {
      text.writeln(fact);
    }
    text
      ..writeln()
      ..writeln('Log')
      ..writeln(logSummary);
    for (final line in log) {
      text.writeln(line);
    }
    return text.toString();
  }
}

String _lines(int n) => n == 1 ? '1 line' : '$n lines';

/// The short form of a device's id, which is what the library's log calls a device: the first
/// ten digits. The log lines of this device and of the ones paired with it are told apart by it.
String shortId(String id) => id.length > 10 ? id.substring(0, 10) : id;

/// What to show of [address], one of the addresses that other devices can try, in a report that
/// is pasted into a public place.
///
/// A public address names the network this device is on and, with the port mapping that Baton
/// asks the router for, a port on the router that leads to it. It is shown by its kind and its
/// port: `public IPv4 (hidden):41641`. The addresses that belong to no one network in
/// particular (this host, a private or shared network, link-local, multicast and the ones set
/// aside for documents) stay as they are, because they answer most questions about why a device
/// cannot be reached. Text that is not an address is not shown at all, because there is no
/// telling what it holds. The library's log is held to the same rule on its side (`hide.rs`).
String describeAddress(String address) {
  const unknown = 'unrecognised address (hidden)';
  final parts = _splitAddress(address);
  if (parts == null) return unknown;
  final (:host, :port) = parts;
  // A link-local address can carry the interface it is on: `fe80::1%2`.
  final zone = host.indexOf('%');
  final ip = io.InternetAddress.tryParse(
    zone < 0 ? host : host.substring(0, zone),
  );
  if (ip == null) return unknown;
  if (!_isPublic(ip.rawAddress)) return visibleText(address);
  final kind = ip.type == io.InternetAddressType.IPv4 ? 'IPv4' : 'IPv6';
  return 'public $kind (hidden)${port == null ? '' : ':$port'}';
}

/// The address and the port of text like `192.168.1.5:4000`, `[fe80::1%2]:4000` or `::1`, or
/// `null` if it is neither with a port that could be one.
({String host, String? port})? _splitAddress(String address) {
  final String host;
  String? port;
  if (address.startsWith('[')) {
    final close = address.indexOf(']');
    if (close < 0) return null;
    host = address.substring(1, close);
    final after = address.substring(close + 1);
    if (after.isNotEmpty) {
      if (!after.startsWith(':')) return null;
      port = after.substring(1);
    }
  } else if (':'.allMatches(address).length == 1) {
    final colon = address.indexOf(':');
    host = address.substring(0, colon);
    port = address.substring(colon + 1);
  } else {
    // An address on its own, with no port (an IPv6 one has colons of its own).
    host = address;
  }
  if (port != null && !RegExp(r'^[0-9]{1,5}$').hasMatch(port)) return null;
  if (port != null && int.parse(port) > 65535) return null;
  return (host: host, port: port);
}

bool _isPublic(Uint8List bytes) {
  if (bytes.length == 4) return _isPublicV4(bytes);
  // `::ffff:8.8.8.8` is an IPv4 address in an IPv6 form.
  final mapped =
      bytes.sublist(0, 10).every((byte) => byte == 0) &&
      bytes[10] == 0xff &&
      bytes[11] == 0xff;
  return mapped ? _isPublicV4(bytes.sublist(12)) : _isPublicV6(bytes);
}

bool _isPublicV4(Uint8List ip) {
  final [a, b, c, d] = ip;
  return !(
  // 0.0.0.0, this host, a private network, link-local, multicast and broadcast.
  (a == 0 && b == 0 && c == 0 && d == 0) ||
      a == 127 ||
      a == 10 ||
      (a == 172 && b >= 16 && b < 32) ||
      (a == 192 && b == 168) ||
      (a == 169 && b == 254) ||
      (a >= 224 && a <= 239) ||
      (a == 255 && b == 255 && c == 255 && d == 255) ||
      // The ones set aside for documents.
      (a == 192 && b == 0 && c == 2) ||
      (a == 198 && b == 51 && c == 100) ||
      (a == 203 && b == 0 && c == 113) ||
      // The range for carrier-grade NAT, where VPNs such as Tailscale put their addresses.
      (a == 100 && b >= 64 && b < 128));
}

bool _isPublicV6(Uint8List ip) {
  final first = (ip[0] << 8) | ip[1];
  final second = (ip[2] << 8) | ip[3];
  // 64:ff9b::/96 is how an IPv6-only mobile network reaches IPv4 addresses. The address holds one
  // in its last four bytes and is as public as that one, as ::ffff:0:0/96 is.
  if (first == 0x64 &&
      second == 0xff9b &&
      ip.sublist(4, 12).every((byte) => byte == 0)) {
    return _isPublicV4(ip.sublist(12));
  }
  // Only 2000::/3 is handed out to networks. Everything else is this host, a private network,
  // link-local, multicast or not assigned. The library's log is held to the same rule, where it
  // keeps the short forms that the names of modules make (e:: in core::) out of the way.
  final global = (first & 0xe000) == 0x2000;
  // 2001:db8::/32 and 3fff::/20 are set aside for documents.
  final documentation =
      (first == 0x2001 && second == 0x0db8) ||
      (first == 0x3fff && (second & 0xf000) == 0);
  return global && !documentation;
}

// The folder after /Users/ or /home/, and after \Users\ on Windows (also as it looks when a path
// is printed with its backslashes doubled). A Windows name can hold spaces, and a Unix one ends
// at the end of the word. \x22 is a double quote and \x27 a single one.
final _unixHome = RegExp(r'(/Users/|/home/)([^/\\\x22\x27\s)\],;]+)');
final _windowsHome = RegExp(r'(\\+Users\\+)([^/\\\x22\x27]+)');

/// [text] with the account name in a path replaced by `~`. The folder of the app is under the
/// home folder of the account, and an account is often named after a person. The library's log
/// is held to the same rule on its side (`hide.rs`).
String hideAccountNames(String text) => text
    .replaceAllMapped(_unixHome, _keepFolderHideName)
    .replaceAllMapped(_windowsHome, _keepFolderHideName);

String _keepFolderHideName(Match match) =>
    match[2] == '~' ? match[0]! : '${match[1]}~';

/// `2026-10-15T08:44:54Z`: the time the way the library's own log lines write it, so that the
/// two can be put side by side.
String _seconds(DateTime time) =>
    '${time.toUtc().toIso8601String().split('.').first}Z';

/// Reads the facts and the log for the Diagnostics screen, and again when the person asks.
///
/// The screen can be opened without a [session] when Baton could not start: the log of the
/// failed start is what a person then needs to show.
class DiagnosticsController extends ChangeNotifier {
  DiagnosticsController({
    required this.recentLog,
    required this.system,
    this.session,
    this.problem,
    this.now = DateTime.now,
  });

  /// The Baton that is running, or `null` if there is none.
  final Session? session;

  /// Where the lines of the log come from.
  final List<String> Function() recentLog;

  /// What the operating system says it is, on one line.
  final String system;

  /// What went wrong, if the screen is opened because Baton could not start.
  final String? problem;

  /// The time. Tests give it a clock of their own.
  final DateTime Function() now;

  Diagnostics? _diagnostics;
  int _reads = 0;
  bool _disposed = false;

  /// What was read last, or `null` before the first read ends.
  Diagnostics? get diagnostics => _diagnostics;

  /// Reads everything now. The log is read last, so that it holds what happened while the
  /// facts were read.
  Future<void> refresh() async {
    final read = ++_reads;
    final facts = <String>[
      'Taken: ${_seconds(now())}',
      'System: ${visibleText(system)}',
    ];
    final said = problem;
    if (said != null) {
      facts.add('Problem: ${hideAccountNames(visibleText(said))}');
    }
    final running = session;
    if (running == null) {
      facts.add('Baton is not running.');
    } else {
      final me = running.me;
      facts.add(
        'This device: ${visibleText(me.name)} (${platformName(me.platform)}), '
        'id ${shortId(me.id)}',
      );
      facts.addAll(await _connection(running));
    }
    // A newer read is on its way, or the screen is gone.
    if (_disposed || read != _reads) return;
    _diagnostics = Diagnostics(
      facts: facts,
      log: [for (final line in recentLog()) visibleText(line)],
    );
    notifyListeners();
  }

  Future<List<String>> _connection(Session running) async {
    try {
      final devices = await running.devices();
      final network = await running.network();
      final addresses = network.addresses;
      return [
        'Paired devices: ${devices.length}',
        'Relay: ${network.relay == null ? 'not connected' : visibleText(network.relay!)}',
        'Addresses other devices can try: '
            '${addresses.isEmpty ? 'none found' : addresses.map(describeAddress).join(', ')}',
        'Finding devices on the local network: ${network.lanDiscovery ? 'on' : 'off'}',
      ];
    } on BatonError catch (e) {
      final said = hideAccountNames(visibleText(e.message));
      return ['Connection: could not be read ($said)'];
    } on Object catch (e) {
      // Only what kind of failure it was: what it says might not be fit to paste.
      return ['Connection: could not be read (${e.runtimeType})'];
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
