// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import 'backend.dart';
import 'controller.dart';
import 'file_chooser.dart';
import 'scanner.dart';
import 'screens/home_screen.dart';
import 'screens/name_screen.dart';
import 'screens/problem_screen.dart';

/// The whole app. The first time it asks for a name for this device. Then it starts Baton and
/// shows the devices and the inbox.
class BatonApp extends StatefulWidget {
  const BatonApp({
    required this.backend,
    this.scanner = cameraScanner,
    this.chooser = const SystemFileChooser(),
    super.key,
  });

  final Backend backend;

  /// Reads QR codes with the camera, for pairing. Tests give it one of their own.
  final ScannerBuilder scanner;

  /// Lets the person choose files and folders to send. Tests give it one of their own.
  final FileChooser chooser;

  @override
  State<BatonApp> createState() => _BatonAppState();
}

class _BatonAppState extends State<BatonApp> {
  late final AppController _controller = AppController(widget.backend);

  @override
  void initState() {
    super.initState();
    unawaited(_controller.begin());
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
      theme: _theme(Brightness.light),
      darkTheme: _theme(Brightness.dark),
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

ThemeData _theme(Brightness brightness) => ThemeData(
  colorSchemeSeed: const Color(0xFF3B5BDB),
  brightness: brightness,
  useMaterial3: true,
);
