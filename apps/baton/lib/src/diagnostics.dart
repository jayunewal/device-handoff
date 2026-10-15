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
    if (said != null) facts.add('Problem: ${visibleText(said)}');
    final running = session;
    if (running == null) {
      facts.add('Baton is not running.');
    } else {
      final me = running.me;
      facts.add(
        'This device: ${visibleText(me.name)} (${platformName(me.platform)}), '
        'fingerprint ${me.fingerprint}',
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
            '${addresses.isEmpty ? 'none found' : addresses.map(visibleText).join(', ')}',
        'Finding devices on the local network: ${network.lanDiscovery ? 'on' : 'off'}',
      ];
    } on BatonError catch (e) {
      return ['Connection: could not be read (${visibleText(e.message)})'];
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
