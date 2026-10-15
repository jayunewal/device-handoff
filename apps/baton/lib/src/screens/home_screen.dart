// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../backend.dart';
import '../inbox_controller.dart';
import '../links.dart';
import '../scanner.dart';
import 'devices_screen.dart';
import 'inbox_screen.dart';

/// The two places the person starts from: the devices, to pair them and send to them, and the
/// inbox, for what they sent.
class HomeScreen extends StatefulWidget {
  const HomeScreen({
    required this.session,
    this.now = DateTime.now,
    this.scanner = cameraScanner,
    this.openLink = openInBrowser,
    super.key,
  });

  final Session session;

  /// The time to measure "seen 5 minutes ago" from. Tests give it a clock of their own.
  final DateTime Function() now;

  /// Reads QR codes with the camera, for pairing. Tests give it one of their own.
  final ScannerBuilder scanner;

  /// Opens a link in the web browser. Tests give it one of their own.
  final OpenLink openLink;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

enum _Tab { devices, inbox }

class _HomeScreenState extends State<HomeScreen> {
  late final InboxController _inbox = InboxController(widget.session);
  StreamSubscription<Received>? _arrivals;
  _Tab _tab = _Tab.devices;

  @override
  void initState() {
    super.initState();
    _arrivals = _inbox.arrivals.listen(_arrived);
    unawaited(_inbox.start());
  }

  @override
  void dispose() {
    unawaited(_arrivals?.cancel());
    _inbox.dispose();
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
    // Whatever is on top of the two places, such as a screen for pairing, goes.
    Navigator.of(context).popUntil((route) => route.isFirst);
    _show(_Tab.inbox);
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
          ),
          InboxScreen(
            controller: _inbox,
            now: widget.now,
            openLink: widget.openLink,
          ),
        ],
      ),
      bottomNavigationBar: ListenableBuilder(
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
      ),
    );
  }
}
