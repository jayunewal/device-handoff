// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/material.dart';

import '../awake.dart';
import '../backend.dart';
import '../continue_reader.dart';
import '../continue_server.dart';
import '../control_access.dart';
import '../file_chooser.dart';
import '../file_shower.dart';
import '../format.dart';
import '../inbox_controller.dart';
import '../links.dart';
import '../mac_passwords.dart';
import '../notice_access.dart';
import '../owner_check.dart';
import '../scanner.dart';
import '../transfers_controller.dart';
import '../unlock_notices.dart';
import '../windows_controller.dart';
import 'devices_screen.dart';
import 'inbox_screen.dart';
import 'settings_screen.dart';
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
    this.controlAccess = const PlatformControlAccess(),
    this.screenAccess = const PlatformScreenAccess(),
    this.notices = const PlatformNoticeKeeper(),
    this.ownerCheck = const PlatformOwnerCheck(),
    this.continueReader = const PlatformContinueReader(),
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

  /// What the system says about letting another device control a window or a screen of this one.
  /// Tests give it one of their own.
  final ControlAccessKeeper controlAccess;

  /// What the system says about letting Baton record the screen of this device. Tests give it one
  /// of their own.
  final ScreenAccessKeeper screenAccess;

  /// What the system says about showing notifications, and what shows them: a paired device that
  /// unlocked this one, or tried to, is told of that way. Tests give it one of their own.
  final NoticeKeeper notices;

  /// Asks the person for their face, or the passcode of this device, before this device asks
  /// another for its screen. Tests give it one of their own.
  final OwnerCheck ownerCheck;

  /// Reads the page in front in the browser and the text on the clipboard, for a paired device
  /// that the person allowed to ask what they are working on. Tests give it one of their own.
  final ContinueReader continueReader;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

enum _Tab { devices, inbox, settings }

/// From this width on, the places are listed in a sidebar beside the one shown, as on an iPad or a
/// Mac, and not in a bar at the bottom.
const sidebarFrom = 700.0;

class _HomeScreenState extends State<HomeScreen> {
  late final InboxController _inbox = InboxController(widget.session);
  late final TransfersController _transfers = TransfersController(
    widget.session,
    shower: widget.shower,
  );
  late final WindowsController _windows = WindowsController(
    widget.session,
    screen: widget.screen,
    ownerCheck: widget.ownerCheck,
  );
  late final UnlockNotices _unlockNotices = UnlockNotices(
    widget.session,
    widget.notices,
  );
  late final ContinueServer _continueServer = ContinueServer(
    widget.session,
    widget.continueReader,
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
    unawaited(_unlockNotices.start());
    unawaited(_continueServer.start());
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
    _unlockNotices.dispose();
    _continueServer.dispose();
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

  /// Keeps the Macs to unlock after a restart. Only an iPhone or iPad has one.
  late final MacPasswordKeeper? _macPasswords =
      PlatformMacPasswordKeeper.forThisDevice();

  /// Keeps the three places when the window crosses [sidebarFrom], where they move between the
  /// bar layout and the sidebar layout. Without it they would be built anew, and a dialog that one
  /// of them opened would answer a screen that is gone.
  final _placesKey = GlobalKey();

  @override
  Widget build(BuildContext context) {
    final places = IndexedStack(
      key: _placesKey,
      index: _tab.index,
      children: [
        DevicesScreen(
          session: widget.session,
          now: widget.now,
          scanner: widget.scanner,
          chooser: widget.chooser,
          controlAccess: widget.controlAccess,
          screenAccess: widget.screenAccess,
          notices: widget.notices,
          onShareWindow: _windows.abilities.canShare
              ? (device, {required control}) =>
                    unawaited(_windows.share(device, control: control))
              : null,
          onStartScreen: _windows.abilities.canShow
              ? (device, {required control}) =>
                    unawaited(_windows.startScreen(device, control: control))
              : null,
          canGiveScreen: _windows.abilities.canGiveScreen,
          canBeUnlocked: _windows.abilities.canBeUnlocked,
        ),
        InboxScreen(
          controller: _inbox,
          now: widget.now,
          openLink: widget.openLink,
        ),
        SettingsScreen(
          session: widget.session,
          now: widget.now,
          screenAccess: widget.screenAccess,
          notices: widget.notices,
          macPasswords: _macPasswords,
        ),
      ],
    );
    final panels = [
      WindowsPanel(controller: _windows, onOpen: _showWindow),
      TransfersPanel(controller: _transfers),
    ];
    if (MediaQuery.sizeOf(context).width >= sidebarFrom) {
      return Scaffold(
        body: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ListenableBuilder(
              listenable: _inbox,
              builder: (context, _) => _Sidebar(
                selected: _tab,
                unseen: _inbox.unseen,
                onSelected: _show,
              ),
            ),
            const VerticalDivider(width: 1),
            Expanded(
              child: Column(
                children: [
                  Expanded(child: places),
                  ...panels,
                ],
              ),
            ),
          ],
        ),
      );
    }
    return Scaffold(
      body: places,
      bottomNavigationBar: Column(
        mainAxisSize: MainAxisSize.min,
        children: [...panels, _navigation()],
      ),
    );
  }

  Widget _navigation() {
    return ListenableBuilder(
      listenable: _inbox,
      builder: (context, _) => NavigationBar(
        selectedIndex: _tab.index,
        onDestinationSelected: (index) => _show(_Tab.values[index]),
        destinations: [
          for (final place in _Tab.values)
            NavigationDestination(
              icon: _icon(place, unseen: _inbox.unseen, selected: false),
              selectedIcon: _icon(place, unseen: _inbox.unseen, selected: true),
              label: _label(place, unseen: _inbox.unseen),
            ),
        ],
      ),
    );
  }
}

/// What a place is called. The inbox says how many things in it are new, which a screen reader
/// reads out with the name.
String _label(_Tab place, {required int unseen}) => switch (place) {
  _Tab.devices => 'Devices',
  _Tab.inbox => unseen > 0 ? 'Inbox, $unseen new' : 'Inbox',
  _Tab.settings => 'Settings',
};

Widget _icon(_Tab place, {required int unseen, required bool selected}) {
  final icon = Icon(switch (place) {
    _Tab.devices => selected ? Icons.devices : Icons.devices_outlined,
    _Tab.inbox => selected ? Icons.inbox : Icons.inbox_outlined,
    _Tab.settings => selected ? Icons.settings : Icons.settings_outlined,
  });
  if (place != _Tab.inbox) return icon;
  return Badge(isLabelVisible: unseen > 0, label: Text('$unseen'), child: icon);
}

/// The places, listed down the side of a wide window: the name of the app at the top, then one
/// row for each place, the one shown marked in the signature colour.
class _Sidebar extends StatelessWidget {
  const _Sidebar({
    required this.selected,
    required this.unseen,
    required this.onSelected,
  });

  final _Tab selected;
  final int unseen;
  final void Function(_Tab place) onSelected;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return Material(
      color: scheme.surfaceContainerLow,
      child: SafeArea(
        right: false,
        child: SizedBox(
          width: 232,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(12, 20, 12, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(12, 0, 12, 20),
                  child: Row(
                    children: [
                      Container(
                        width: 10,
                        height: 10,
                        decoration: BoxDecoration(
                          color: scheme.primary,
                          shape: BoxShape.circle,
                        ),
                      ),
                      const SizedBox(width: 10),
                      Text('Baton', style: theme.textTheme.headlineSmall),
                    ],
                  ),
                ),
                for (final place in _Tab.values)
                  _SidebarRow(
                    icon: _icon(
                      place,
                      unseen: unseen,
                      selected: place == selected,
                    ),
                    label: place == _Tab.inbox
                        ? 'Inbox'
                        : _label(place, unseen: unseen),
                    semanticsLabel: _label(place, unseen: unseen),
                    selected: place == selected,
                    onTap: () => onSelected(place),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _SidebarRow extends StatelessWidget {
  const _SidebarRow({
    required this.icon,
    required this.label,
    required this.semanticsLabel,
    required this.selected,
    required this.onTap,
  });

  final Widget icon;
  final String label;

  /// What a screen reader says: the name, and for the inbox how many things in it are new.
  final String semanticsLabel;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final color = selected ? scheme.primary : scheme.onSurface;
    return Padding(
      padding: const EdgeInsets.only(bottom: 2),
      child: Semantics(
        selected: selected,
        button: true,
        child: Material(
          color: selected
              ? scheme.primary.withValues(alpha: 0.12)
              : Colors.transparent,
          borderRadius: BorderRadius.circular(10),
          child: InkWell(
            borderRadius: BorderRadius.circular(10),
            onTap: onTap,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
              child: Row(
                children: [
                  IconTheme(
                    data: IconThemeData(color: color, size: 22),
                    child: icon,
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      label,
                      semanticsLabel: semanticsLabel,
                      style: theme.textTheme.bodyLarge?.copyWith(
                        color: color,
                        fontWeight: selected
                            ? FontWeight.w600
                            : FontWeight.w500,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
