// SPDX-License-Identifier: Apache-2.0

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'awake.dart';
import 'backend.dart';

/// Something to tell the person about a window that is over, or that did not begin.
sealed class WindowNote {
  const WindowNote();
}

/// A window that was shown here or shared from here, and that ended some other way than the
/// person ending it. [end] says how.
final class WindowOver extends WindowNote {
  const WindowOver(this.session, this.end);

  final WindowSession session;
  final WindowEnd end;
}

/// A window that the person said yes to, and that the other device took back before it began.
final class WindowWithdrawn extends WindowNote {
  const WindowWithdrawn(this.request);

  final WindowRequest request;
}

/// A window that the person said yes to, and that did not begin in time.
final class WindowNotOpened extends WindowNote {
  const WindowNotOpened(this.request);

  final WindowRequest request;
}

/// A sentence from Baton, already worded for the person, about a window that was offered or
/// answered and did not begin: the other device did not take it, could not be reached, or this
/// device could not show it.
final class WindowFailed extends WindowNote {
  const WindowFailed(this.message);

  final String message;
}

/// How large the picture of a window is, in pixels.
typedef PictureSize = ({int width, int height});

/// What the person has lifted of the privacy of a screen of another device that this one controls:
/// whether its screens show what is on them ([screen]), and whether its keyboard and trackpad can
/// be used at it ([input]). Both are off when the session starts.
typedef LiftedPrivacy = ({bool screen, bool input});

/// What the other device did not do of what the person did on a window that is shown here: it
/// dropped [count] events, and [reason] is the rule that dropped the most of them.
typedef WindowDropped = ({
  WindowSession session,
  WindowNoticeReason reason,
  int count,
});

/// What the screens show of the windows: the offers for the person to answer, the window that is
/// being opened, the one this device is offering, the ones that run, and how the last ones ended.
///
/// Nothing here draws a window. A window that another device shows on this one reaches
/// [started], and the screen that takes it draws the picture with `WindowSession.screen`, in the
/// shape that [pictureOf] says.
///
/// The screen of a phone stays on from an offer until the person has answered it and the window
/// has begun, because Baton on a phone works only while it is on the screen. The screen that
/// shows the window keeps it on from there; [screen] does the asking, and tests give it one of
/// their own.
class WindowsController extends ChangeNotifier {
  WindowsController(
    this._session, {
    ScreenKeeper screen = const PlatformScreenKeeper(),
    this._openingLimit = const Duration(seconds: 20),
  }) : _awake = AwakeHold(screen);

  /// How many notes are kept to be shown.
  static const keepNotes = 5;

  final Session _session;
  final AwakeHold _awake;
  final Duration _openingLimit;
  final StreamController<WindowRequest> _asked = StreamController.broadcast();
  final StreamController<WindowSession> _started = StreamController.broadcast();
  final StreamController<WindowDropped> _dropped = StreamController.broadcast();
  StreamSubscription<Event>? _events;
  Timer? _openingTimer;
  bool _disposed = false;
  final List<WindowRequest> _requests = [];
  WindowRequest? _opening;
  Device? _offering;
  bool _offeringControl = false;
  Device? _asking;
  bool _askingControl = false;
  final List<WindowSession> _sessions = [];
  final Map<int, PictureSize> _pictures = {};
  final Map<int, LiftedPrivacy> _lifted = {};
  final List<WindowNote> _notes = [];

  /// What this device does with windows. Offer only what is possible.
  WindowAbilities get abilities => _session.windowAbilities();

  /// The offers that wait for the person's answer, oldest first.
  List<WindowRequest> get requests => List.unmodifiable(_requests);

  /// The window that the person said yes to and that has not begun, or `null`.
  WindowRequest? get opening => _opening;

  /// The device that this one is offering a window to, from the moment the person asks to share
  /// one until the other device has answered, or `null`. The person chooses the window in that
  /// time.
  Device? get offering => _offering;

  /// Whether the window that is being offered is offered to be controlled as well as looked at.
  bool get offeringControl => _offeringControl;

  /// The device that this one is asking for its screen, from the moment the person asks until the
  /// other device has answered, or `null`.
  Device? get asking => _asking;

  /// Whether the screen that is being asked for is asked for to be controlled as well as looked at.
  bool get askingControl => _askingControl;

  /// The windows that run, shown here or shared from here, oldest first.
  List<WindowSession> get sessions => List.unmodifiable(_sessions);

  /// What the last windows came to, newest first, until the person puts it away.
  List<WindowNote> get notes => List.unmodifiable(_notes);

  /// How large the picture of [session] is, as its source said last, or `null` while it has said
  /// nothing. The window that was offered is another shape when the source scales it to what
  /// this device takes, and the picture changes shape when the window is resized, so a screen
  /// lays out the picture by this and not by `WindowSession.width` and `height`.
  PictureSize? pictureOf(WindowSession session) =>
      _pictures[session.id.toInt()];

  /// What the person has lifted of the privacy of [session], which is a screen of another device
  /// that this one controls (`WindowSession.private`): whether its screens show what is on them,
  /// and whether its keyboard and trackpad can be used at it. Both are off when the session
  /// starts, whatever they were in the last one: nothing is remembered for a device.
  LiftedPrivacy liftedOf(WindowSession session) =>
      _lifted[session.id.toInt()] ?? (screen: false, input: false);

  /// Each offer as it comes, so that a screen that covers the list can say that one waits.
  Stream<WindowRequest> get asked => _asked.stream;

  /// Each window that another device shows on this one, as it begins, so that a screen can take
  /// the person to it.
  Stream<WindowSession> get started => _started.stream;

  /// Each time the other device says that it did not do some of what the person did on a window
  /// that is shown here, so that a screen can say why a tap did nothing. The other device says it
  /// at most once a second for each window.
  Stream<WindowDropped> get dropped => _dropped.stream;

  /// Starts listening for events.
  Future<void> start() async {
    final events = await _session.events();
    if (_disposed) {
      // Nobody is left to read them. A stream is told to stop only when someone cancels it.
      unawaited(events.listen(null).cancel());
      return;
    }
    _events = events.listen(
      _onEvent,
      onError: (Object e) => debugPrint('Baton: events: $e'),
    );
  }

  void _onEvent(Event event) {
    if (_disposed) return;
    switch (event) {
      case Event_WindowRequested(:final request):
        if (_requests.any((r) => r.id == request.id)) return;
        _requests.add(request);
        _awake.renew();
        _asked.add(request);
        notifyListeners();
      case Event_WindowRequestEnded(:final id, :final failure):
        // The other device took the offer back, or the time ran out; or the person said yes and
        // this device could not show the window after all, which `failure` says.
        final waiting = _requests.length;
        _requests.removeWhere((r) => r.id == id);
        final answered = _opening?.id == id;
        if (answered) _stopOpening();
        if (failure != null) _note(WindowFailed(failure));
        if (_requests.length == waiting && !answered && failure == null) return;
        _settle();
      case Event_WindowStarted(:final session):
        if (_sessions.any((s) => s.id == session.id)) return;
        _sessions.add(session);
        if (session.role == WindowRole.viewing) {
          _stopOpening();
          // A window that was given up on and begins after all is not one that did not open.
          _notes.removeWhere(
            (note) =>
                note is WindowNotOpened &&
                note.request.deviceId == session.deviceId,
          );
          _started.add(session);
        }
        _settle();
      case Event_WindowPicture(:final id, :final width, :final height):
        // Only a window that runs here has a picture, and a size that is nothing cannot be laid
        // out. The same size again changes nothing to draw.
        final shown = _sessions.any(
          (s) => s.id == id && s.role == WindowRole.viewing,
        );
        if (!shown || width <= 0 || height <= 0) return;
        final size = (width: width, height: height);
        if (_pictures[id.toInt()] == size) return;
        _pictures[id.toInt()] = size;
        notifyListeners();
      case Event_WindowNotice(:final id, :final reason, :final count):
        // Only a window that runs here is touched, and a count of nothing says nothing.
        final index = _sessions.indexWhere(
          (s) => s.id == id && s.role == WindowRole.viewing,
        );
        if (index < 0 || count <= 0) return;
        _dropped.add((session: _sessions[index], reason: reason, count: count));
      case Event_WindowEnded(:final id, :final end):
        final index = _sessions.indexWhere((s) => s.id == id);
        if (index < 0) return;
        _pictures.remove(id.toInt());
        _lifted.remove(id.toInt());
        final session = _sessions.removeAt(index);
        // The person who ended it knows.
        if (end is! WindowEnd_ThisDevice) _note(WindowOver(session, end));
        _settle();
      default:
        // The other events are for the screens that pair devices, list them and show the inbox
        // and the files.
        break;
    }
  }

  /// Says yes or no to [request]. Returns `false` if it was not waiting any more, because the
  /// other device took it back.
  bool answer(WindowRequest request, {required bool accept}) {
    final index = _requests.indexWhere((r) => r.id == request.id);
    if (index < 0) return false;
    final waiting = _session.answerWindow(request.id.toInt(), accept: accept);
    _requests.removeAt(index);
    if (accept) {
      if (waiting) {
        _opening = request;
        _openingTimer?.cancel();
        _openingTimer = Timer(_openingLimit, _notOpened);
        _awake.renew();
      } else {
        _note(WindowWithdrawn(request));
      }
    }
    _settle();
    return waiting;
  }

  /// Lets the person choose a window of this device, and offers it to [device]. Returns when the
  /// other device has answered. What came of it is in [notes], if there was anything to say,
  /// and in [sessions] and [started] if the window began. Does nothing while an offer is going or
  /// a screen is being asked for.
  Future<void> share(Device device, {bool control = false}) async {
    if (_offering != null || _asking != null) return;
    _offering = device;
    _offeringControl = control;
    notifyListeners();
    try {
      final shared = await _session.shareWindow(device.id, control: control);
      // The session of a window that began comes as `Event_WindowStarted`, which is the only way
      // that a window is added: a result that arrived after the window had ended would bring
      // back a window that is over.
      if (!_disposed && shared is WindowShared_Declined) {
        _note(WindowFailed(shared.message));
      }
    } on BatonError catch (e) {
      if (!_disposed) _note(WindowFailed(e.message));
    } finally {
      if (!_disposed) {
        _offering = null;
        _offeringControl = false;
        notifyListeners();
      }
    }
  }

  /// Asks [device] to start sharing its screen with this one, and shows it when it does. Nobody
  /// at [device] is asked: the person there allowed this device beforehand, in the settings for
  /// it. Returns when the other device has answered. What came of it is in [notes], if there was
  /// anything to say, and in [sessions] and [started] if the screen began. Does nothing while a
  /// window is being offered or another screen is being asked for.
  ///
  /// The screen of a phone stays on from the moment the person asks, because Baton on a phone
  /// works only while it is on the screen.
  Future<void> startScreen(Device device, {bool control = false}) async {
    if (_asking != null || _offering != null) return;
    _asking = device;
    _askingControl = control;
    _awake.renew();
    notifyListeners();
    try {
      final started = await _session.startScreen(device.id, control: control);
      // The session of a screen that began comes as `Event_WindowStarted`, which is the only way
      // that a window is added: a result that arrived after the screen had ended would bring
      // back a window that is over.
      if (!_disposed && started is ScreenStarted_NotStarted) {
        _note(WindowFailed(started.message));
      }
    } on BatonError catch (e) {
      if (!_disposed) _note(WindowFailed(e.message));
    } finally {
      if (!_disposed) {
        _asking = null;
        _askingControl = false;
        _settle();
      }
    }
  }

  /// Ends [session], which is shown here or shared from here, and tells the other device.
  /// Returns whether it was running. The window is gone from [sessions] at once.
  bool end(WindowSession session) {
    final running = _session.endWindow(session.id.toInt());
    final kept = _sessions.length;
    _sessions.removeWhere((s) => s.id == session.id);
    _pictures.remove(session.id.toInt());
    _lifted.remove(session.id.toInt());
    if (_sessions.length != kept) _settle();
    return running;
  }

  /// Gives [session] what the person did on its picture: a touch, a click, a scroll, a key. It
  /// goes to the other device soon, in a batch with others, and is done there only if the person
  /// at that device let this one control the window (`WindowSession.control`). Returns whether
  /// Baton took it. None of the reasons for no ends the window, so a screen drops the event and
  /// goes on; what the other device drops comes back through [dropped]. The rules of what may be
  /// sent are kept in Rust, once, and not repeated here.
  bool input(WindowSession session, WindowInput input) =>
      _session.windowInput(session.id.toInt(), input);

  /// Lets go of every button and key that is down on [session], for example when the person
  /// leaves the screen with a finger on it.
  void releaseInput(WindowSession session) =>
      _session.releaseWindowInput(session.id.toInt());

  /// Switches what the people at the device that shares [session] may do while this one controls
  /// it: [screen] is whether they see its screens, [input] whether they use its keyboard and
  /// trackpad. A side that is left out stays as it is, and either may be switched on and off as
  /// often as the person likes. Returns whether the other device was told. When it was not (the
  /// session is not private, it is over, or the other device has stopped reading) things stay as
  /// they were, and [liftedOf] says so. Saying what already is tells the other device nothing. The
  /// other device puts both back when the session ends, and so does this one. The rules of what
  /// may be sent are kept in Rust, once, and not repeated here.
  bool lift(WindowSession session, {bool? screen, bool? input}) {
    final now = liftedOf(session);
    final next = (screen: screen ?? now.screen, input: input ?? now.input);
    if (next == now) return true;
    final told = _session.windowLift(
      session.id.toInt(),
      screen: next.screen,
      input: next.input,
    );
    if (!told) return false;
    _lifted[session.id.toInt()] = next;
    notifyListeners();
    return true;
  }

  /// How [session] ended, if it ended some other way than the person ending it and the person
  /// has not put it away.
  WindowOver? over(WindowSession session) {
    for (final note in _notes) {
      if (note is WindowOver && note.session.id == session.id) return note;
    }
    return null;
  }

  /// Puts away what is shown of a window that is over.
  void dismiss(WindowNote note) {
    if (_notes.remove(note)) notifyListeners();
  }

  void _note(WindowNote note) {
    _notes.insert(0, note);
    if (_notes.length > keepNotes) {
      _notes.removeRange(keepNotes, _notes.length);
    }
  }

  /// The window that the person said yes to has not begun in time. If it begins later, it is
  /// shown all the same.
  void _notOpened() {
    _openingTimer = null;
    final request = _opening;
    if (request == null || _disposed) return;
    _opening = null;
    _note(WindowNotOpened(request));
    _settle();
  }

  void _stopOpening() {
    _opening = null;
    _openingTimer?.cancel();
    _openingTimer = null;
  }

  /// The screen can turn off again when no offer waits, no window is being opened and no screen
  /// is being asked for.
  void _settle() {
    if (_requests.isEmpty && _opening == null && _asking == null) {
      _awake.release();
    }
    notifyListeners();
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _awake.release();
    _openingTimer?.cancel();
    // A stream from the bridge finishes a cancel only when it next has an event, so do not
    // wait for it.
    unawaited(_events?.cancel());
    unawaited(_asked.close());
    unawaited(_started.close());
    unawaited(_dropped.close());
    super.dispose();
  }
}
