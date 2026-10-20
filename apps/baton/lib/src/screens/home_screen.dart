// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../awake.dart';
import '../backend.dart';
import '../file_chooser.dart';
import '../file_shower.dart';
import '../format.dart';
import '../inbox_controller.dart';
import '../links.dart';
import '../scanner.dart';
import '../transfers_controller.dart';
import '../windows_controller.dart';
import 'devices_screen.dart';
import 'inbox_screen.dart';
import 'transfers_panel.dart';
import 'window_screen.dart';
import 'windows_panel.dart';

/// The two places the person starts from: the devices, to pair them and send to them, and the
/// inbox, for what they sent. The files that other devices send, and the windows that they offer
/// to show or that this device shares, show at the bottom of both.
class HomeScreen extends StatefulWidget {
  const HomeScreen({
    required this.session,
    this.now = DateTime.now,
    this.scanner = cameraScanner,
    this.openLink = openInBrowser,
    this.chooser = const SystemFileChooser(),
    this.shower = const SystemFileShower(),
    this.screen = const PlatformScreenKeeper(),
    super.key,
  });

  final Session session;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// Reads QR codes with the camera, for pairing. Tests give it one of their own.
  final ScannerBuilder scanner;

  /// Opens a link in the web browser. Tests give it one of their own.
  final OpenLink openLink;

  /// Lets the person choose files and folders to send. Tests give it one of their own.
  final FileChooser chooser;

  /// Takes the person to the files that arrived. Tests give it one of their own.
  final FileShower shower;

  /// Keeps the screen of the device on while a window is offered and shown. Tests give it one of
  /// their own.
  final ScreenKeeper screen;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

enum _Tab { devices, inbox }

class _HomeScreenState extends State<HomeScreen> {
  late final InboxController _inbox = InboxController(widget.session);
  late final TransfersController _transfers = TransfersController(
    widget.session,
    shower: widget.shower,
  );
  late final WindowsController _windows = WindowsController(
    widget.session,
    screen: widget.screen,
  );
  StreamSubscription<Received>? _arrivals;
  StreamSubscription<TransferRequest>? _requests;
  StreamSubscription<WindowRequest>? _windowRequests;
  StreamSubscription<WindowSession>? _windowsStarted;
  _Tab _tab = _Tab.devices;

  @override
  void initState() {
    super.initState();
    _arrivals = _inbox.arrivals.listen(_arrived);
    _requests = _transfers.asked.listen(_asked);
    _windowRequests = _windows.asked.listen(_windowAsked);
    _windowsStarted = _windows.started.listen(_showWindow);
    unawaited(_inbox.start());
    unawaited(_transfers.start());
    unawaited(_windows.start());
  }

  @override
  void dispose() {
    unawaited(_arrivals?.cancel());
    unawaited(_requests?.cancel());
    unawaited(_windowRequests?.cancel());
    unawaited(_windowsStarted?.cancel());
    _inbox.dispose();
    _transfers.dispose();
    _windows.dispose();
    super.dispose();
  }

  void _show(_Tab tab) {
    setState(() => _tab = tab);
    _inbox.looking = tab == _Tab.inbox;
  }

  /// Says that something came, when the inbox is not what the person is looking at. It says
  /// who sent what kind of thing and no more, because what came can be a password.
  void _arrived(Received received) {
    if (_tab == _Tab.inbox) return;
    final what = switch (received.item) {
      ReceivedItem_Link() => 'a link',
      ReceivedItem_Text() => 'some text',
    };
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(
        SnackBar(
          content: Text('${received.fromName} sent you $what.'),
          action: SnackBarAction(label: 'View', onPressed: _view),
        ),
      );
  }

  void _view() {
    if (!mounted) return;
    _goHome();
    _show(_Tab.inbox);
  }

  /// Leaves whatever is on top of the two places, such as a screen for pairing. A screen that
  /// asks before it is left, as the send screen does while files go, stops the way back and
  /// asks, because leaving it gives the transfer up.
  void _goHome() {
    final navigator = Navigator.of(context);
    navigator.popUntil(
      (route) =>
          route.isFirst ||
          (route is ModalRoute &&
              route.popDisposition == RoutePopDisposition.doNotPop),
    );
    // On such a screen this asks "Stop sending to …?". On the two places it does nothing.
    unawaited(navigator.maybePop());
  }

  /// Says that a device asks to send files, when a screen on top of the two places hides the
  /// question that waits at the bottom of them. It says who asks and nothing of what: the
  /// names of the files can be private.
  void _asked(TransferRequest request) {
    if (ModalRoute.of(context)?.isCurrent ?? true) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(
        SnackBar(
          content: Text('${request.deviceName} wants to send you files.'),
          duration: const Duration(seconds: 10),
          action: SnackBarAction(label: 'Review', onPressed: _review),
        ),
      );
  }

  void _review() {
    if (!mounted) return;
    _goHome();
  }

  /// Says that a device offers to show a window or its screen, when a screen on top of the two
  /// places hides the question that waits at the bottom of them. It says who offers and whether it
  /// is a window or a screen, and not the title of the window, which can be private.
  void _windowAsked(WindowRequest request) {
    if (ModalRoute.of(context)?.isCurrent ?? true) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(
        SnackBar(
          content: Text(
            '${visibleText(request.deviceName)} wants to show you '
            '${request.display ? 'its screen' : 'a window'}.',
          ),
          duration: const Duration(seconds: 10),
          action: SnackBarAction(label: 'Review', onPressed: _review),
        ),
      );
  }

  /// Takes the person to a window that another device shows on this one.
  void _showWindow(WindowSession session) {
    if (!mounted) return;
    unawaited(
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => WindowScreen(
            controller: _windows,
            session: session,
            screen: widget.screen,
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: IndexedStack(
        index: _tab.index,
        children: [
          DevicesScreen(
            session: widget.session,
            now: widget.now,
            scanner: widget.scanner,
            chooser: widget.chooser,
            onShareWindow: _windows.abilities.canShare
                ? (device) => unawaited(_windows.share(device))
                : null,
          ),
          InboxScreen(
            controller: _inbox,
            now: widget.now,
            openLink: widget.openLink,
          ),
        ],
      ),
      bottomNavigationBar: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          WindowsPanel(controller: _windows, onOpen: _showWindow),
          TransfersPanel(controller: _transfers),
          _navigation(),
        ],
      ),
    );
  }

  Widget _navigation() {
    return ListenableBuilder(
      listenable: _inbox,
      builder: (context, _) {
        Widget inbox(IconData icon) => Badge(
          isLabelVisible: _inbox.unseen > 0,
          label: Text('${_inbox.unseen}'),
          child: Icon(icon),
        );
        return NavigationBar(
          selectedIndex: _tab.index,
          onDestinationSelected: (index) => _show(_Tab.values[index]),
          destinations: [
            const NavigationDestination(
              icon: Icon(Icons.devices_outlined),
              selectedIcon: Icon(Icons.devices),
              label: 'Devices',
            ),
            NavigationDestination(
              icon: inbox(Icons.inbox_outlined),
              selectedIcon: inbox(Icons.inbox),
              label: _inbox.unseen > 0
                  ? 'Inbox, ${_inbox.unseen} new'
                  : 'Inbox',
            ),
          ],
        );
      },
    );
  }
}
