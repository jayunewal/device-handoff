// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import 'always_on.dart';
import 'backend.dart';
import 'controller.dart';
import 'file_chooser.dart';
import 'scanner.dart';
import 'screens/home_screen.dart';
import 'screens/name_screen.dart';
import 'screens/problem_screen.dart';
import 'theme.dart';

/// The whole app. The first time it asks for a name for this device. Then it starts Baton and
/// shows the devices and the inbox.
class BatonApp extends StatefulWidget {
  const BatonApp({
    required this.backend,
    this.scanner = cameraScanner,
    this.chooser = const SystemFileChooser(),
    this.alwaysOn = const PlatformAlwaysOnKeeper(),
    super.key,
  });

  final Backend backend;

  /// Reads QR codes with the camera, for pairing. Tests give it one of their own.
  final ScannerBuilder scanner;

  /// Lets the person choose files and folders to send. Tests give it one of their own.
  final FileChooser chooser;

  /// Hears that the home screen is up. A Mac keeps its window out of sight until then when a login
  /// item opened Baton. Tests give it one of their own.
  final AlwaysOnKeeper alwaysOn;

  @override
  State<BatonApp> createState() => _BatonAppState();
}

class _BatonAppState extends State<BatonApp> {
  late final AppController _controller = AppController(widget.backend);
  bool _toldUp = false;

  @override
  void initState() {
    super.initState();
    _controller.addListener(_tellWhenUp);
    unawaited(_controller.begin());
  }

  /// Says once, after the first frame that shows the devices, that the app is up. A Baton that a
  /// login item opened keeps its window out of sight until then and not longer, because Flutter
  /// builds no screen while no window is on the screen, and the screens are what listen for what
  /// other devices send.
  void _tellWhenUp() {
    if (_toldUp || _controller.phase != Phase.running) return;
    _toldUp = true;
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => unawaited(widget.alwaysOn.ready()),
    );
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Baton',
      theme: BatonTheme.light(),
      darkTheme: BatonTheme.dark(),
      home: ListenableBuilder(
        listenable: _controller,
        builder: (context, _) => switch (_controller.phase) {
          Phase.loading || Phase.starting => const WaitingScreen(),
          Phase.askName => NameScreen(
            suggestion: _controller.suggestedName,
            onChosen: _controller.chooseName,
          ),
          Phase.running => HomeScreen(
            session: _controller.session!,
            scanner: widget.scanner,
            chooser: widget.chooser,
            alwaysOn: widget.alwaysOn,
          ),
          Phase.failed => ProblemScreen(
            problem: _controller.problem,
            onTryAgain: _controller.begin,
            recentLog: widget.backend.recentLog,
          ),
        },
      ),
    );
  }
}
