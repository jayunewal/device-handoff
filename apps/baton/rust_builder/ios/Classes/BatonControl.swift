// SPDX-License-Identifier: Apache-2.0

// The calls of a Mac that BatonInput.swift needs to let the person at another device steer a window
// that is shared, or a whole display: what the window server says of windows, what the system says
// of displays and of the Mac, and the events that are handed to the system. What is asked, and what
// is done with the answers, is in BatonInput.swift and, on the Rust side, in
// crates/baton-stream/src/gate.rs. This file only talks to the system, so it is built on a Mac and
// nowhere else, and what it does there is checked on one (docs/adr/0009-window-streaming.md, R6 and
// R7).
//
// The app is sandboxed, and the Accessibility interface is not used: it would say which window of
// an application has the keyboard, but a sandboxed app is not let to ask. The window server is
// asked instead, and the keyboard is judged from what it lists (BatonInput.swift).
//
// What the person types and where they point is a secret. Nothing here logs it, and a failure is
// told without it.

#if os(macOS)

import AppKit
import CoreGraphics
import Foundation
import os

/// The log of the system, for what became of the first events and whether this app may post them.
/// The lines are public so that `log show` has them whole: a line made with `NSLog` and a string
/// argument may be shown as `<private>`, and a person who looks for why a tap did nothing has to
/// find them.
private let controlLog = Logger(subsystem: "io.github.jayunewal.baton", category: "control")

// MARK: - The window server

/// The layers of the window server, as this Mac numbers them.
func macWindowLayers() -> WindowLayers {
  func layer(_ key: CGWindowLevelKey) -> Int { Int(CGWindowLevelForKey(key)) }
  // The windows of applications, the panels that float over them, the modal panels and the utility
  // windows can hold the focus of their application. The Dock and the menu bar are above them.
  let lowest = layer(.normalWindow)
  let highest = max(lowest, layer(.utilityWindow))
  return WindowLayers(
    menu: layer(.popUpMenuWindow), keys: lowest...highest, untouchable: layer(.cursorWindow))
}

/// What the window server says of windows.
final class MacWindowListing: WindowListing {
  func describe(window id: UInt32) -> DescribedWindow {
    // There is no window with the number 0.
    guard id != 0 else { return .gone }
    // The array holds the window numbers themselves and not number objects, as the call wants.
    var number = UnsafeRawPointer(bitPattern: UInt(id))
    guard let ids = CFArrayCreate(kCFAllocatorDefault, &number, 1, nil),
      let found = CGWindowListCreateDescriptionFromArray(ids) as? [[String: Any]]
    else { return .unavailable }
    // A window that is gone is left out of the answer.
    guard let info = found.first(where: { MacWindowListing.number(of: $0) == id }) else {
      return .gone
    }
    guard let window = MacWindowListing.listed(info) else { return .unavailable }
    return .window(window)
  }

  func windowsOnScreen(inFrontOf id: UInt32) -> [ListedWindow]? {
    guard
      let found = CGWindowListCopyWindowInfo([.optionOnScreenAboveWindow], CGWindowID(id))
        as? [[String: Any]]
    else { return nil }
    return found.compactMap(MacWindowListing.listed)
  }

  private static func number(of info: [String: Any]) -> UInt32? {
    (info[kCGWindowNumber as String] as? NSNumber)?.uint32Value
  }

  /// nil for a window that does not say where it is, which cannot cover anything.
  private static func listed(_ info: [String: Any]) -> ListedWindow? {
    guard let id = number(of: info),
      let process = (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value,
      let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue,
      let bounds = screenRect(fromDictionary: info[kCGWindowBounds as String])
    else { return nil }
    // A window with no alpha is shown at full strength. The key is there for a window that is
    // on the screen only.
    let alpha = (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1
    let onScreen = (info[kCGWindowIsOnscreen as String] as? NSNumber)?.boolValue ?? false
    return ListedWindow(
      id: id, process: process, layer: layer, alpha: alpha, bounds: bounds, onScreen: onScreen)
  }
}

/// What the system says of displays.
final class MacDisplayListing: DisplayListing {
  func bounds(ofDisplay id: UInt32) -> ScreenRect? {
    // A display that was unplugged is not online, and what it had as bounds is not where anything
    // is.
    guard CGDisplayIsOnline(id) != 0 else { return nil }
    let rect = CGDisplayBounds(id)
    let bounds = ScreenRect(
      x: Double(rect.origin.x), y: Double(rect.origin.y), width: Double(rect.size.width),
      height: Double(rect.size.height))
    return bounds.isUsable ? bounds : nil
  }
}

// MARK: - The Mac

/// Whether the session says that the screen is locked, or that another user switched the session
/// away from the console. A session that cannot be read is not said to be locked.
func macSessionLocked() -> Bool {
  guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
  // The key for the lock screen is not documented, and has been there as long as the lock screen
  // has.
  if (session["CGSSessionScreenIsLocked"] as? NSNumber)?.boolValue == true { return true }
  return (session[kCGSessionOnConsoleKey as String] as? NSNumber)?.boolValue == false
}

/// What the system says of the Mac, and the way to bring an application to the front.
final class MacActivityReader: MacActivity {
  var locked: Bool {
    macSessionLocked() || CGDisplayIsAsleep(CGMainDisplayID()) != 0
  }

  var frontmostProcess: Int32? {
    NSWorkspace.shared.frontmostApplication?.processIdentifier
  }

  func bringToFront(process: Int32) {
    guard let app = NSRunningApplication(processIdentifier: process) else { return }
    // AppKit is used from the main thread, and this is called from the thread of the input. It
    // does not wait for the main thread for long, since that may be waiting for this one.
    let asked = DispatchSemaphore(value: 0)
    DispatchQueue.main.async {
      MacActivityReader.activate(app)
      asked.signal()
    }
    guard asked.wait(timeout: .now() + ActivationWait.mainThread) == .success else { return }
    if waitUntilInFront(process, seconds: ActivationWait.afterActivating) { return }
    // An application that is not itself in front may not be let to bring another there. The way
    // that the system opens an application is not held to that.
    guard let url = app.bundleURL else { return }
    let configuration = NSWorkspace.OpenConfiguration()
    configuration.activates = true
    configuration.addsToRecentItems = false
    let opened = DispatchSemaphore(value: 0)
    NSWorkspace.shared.openApplication(at: url, configuration: configuration) { _, error in
      if let error {
        let code = (error as NSError).code
        controlLog.error(
          "Baton: the application could not be brought to the front (\(code, privacy: .public)).")
      }
      opened.signal()
    }
    guard opened.wait(timeout: .now() + ActivationWait.opening) == .success else { return }
    _ = waitUntilInFront(process, seconds: ActivationWait.afterOpening)
  }

  private static func activate(_ app: NSRunningApplication) {
    if #available(macOS 14.0, *) {
      // This application lets the other take the front if this one has it. If it has not, the
      // system may ignore the request.
      NSApplication.shared.yieldActivation(to: app)
      _ = app.activate(from: NSRunningApplication.current, options: [])
    } else {
      _ = app.activate(options: [.activateIgnoringOtherApps])
    }
  }

  private func waitUntilInFront(_ process: Int32, seconds: TimeInterval) -> Bool {
    let end = Date().addingTimeInterval(seconds)
    while Date() < end {
      if frontmostProcess == process { return true }
      Thread.sleep(forTimeInterval: 0.02)
    }
    return frontmostProcess == process
  }
}

// MARK: - The consent

/// What this Mac says about letting the app hand input events to other applications. It is asked
/// again each time, since the person may change it in System Settings at any time.
func macControlAccess() -> ControlAccess {
  // The system tells which window the person picked only from macOS 15.2 on, and a window that is
  // not known cannot be steered.
  guard #available(macOS 15.2, *) else { return .unavailable }
  return CGPreflightPostEventAccess() ? .allowed : .needed
}

/// Where System Settings lists the apps that may control this computer, which is where consent to
/// hand input events to other applications is given.
private let controlSettingsAddress =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"

/// Asks the system to let the app hand input events to other applications. It shows its own
/// question once, and does not wait for the answer: the person gives it in System Settings, and
/// `macControlAccess` says what it came to. The question is shown once, and the list in System
/// Settings is hard to find by hand, so the list is opened for the person whenever the answer is
/// not yes. It is the same list for every build of the app, and an app that was built again and
/// installed over the old one may have to be taken out of it and put in anew.
func requestMacControlAccess() {
  guard #available(macOS 15.2, *) else { return }
  // The call is not known to wait for the person, but nothing guarantees it, and the thread of
  // the app's screens is not to be held.
  DispatchQueue.global(qos: .userInitiated).async {
    guard !CGRequestPostEventAccess(), let settings = URL(string: controlSettingsAddress) else {
      return
    }
    DispatchQueue.main.async { _ = NSWorkspace.shared.open(settings) }
  }
}

/// What this Mac says about letting the app record the screen. It is asked again each time, since
/// the person may change it in System Settings at any time.
func macScreenAccess() -> ScreenAccess {
  CGPreflightScreenCaptureAccess() ? .allowed : .needed
}

/// Where System Settings lists the apps that may record the screen, which is where the consent to
/// record it is given.
private let screenSettingsAddress =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"

/// Asks the system to let the app record the screen. It shows its own question once, and does not
/// wait for the answer: the person gives it in System Settings, and the system takes it into
/// effect when the app is opened again. As for the consent to control, the list in System Settings
/// is opened for the person whenever the answer is not yes, since it is hard to find by hand. The
/// question is asked on the main thread, as `MacSharer.screen` asks it.
func requestMacScreenAccess() {
  DispatchQueue.main.async {
    guard !CGRequestScreenCaptureAccess(), let settings = URL(string: screenSettingsAddress) else {
      return
    }
    _ = NSWorkspace.shared.open(settings)
  }
}

// MARK: - The events

/// Hands the events of the person at the other device to the system, as if the person at this Mac
/// did them: the pointer moves, and what lies under it or has the keyboard takes the event,
/// whichever application that is. A whole display is steered so, and so is a window; the rules of
/// the Rust side and of the translator (BatonInput.swift) keep what is put to a window inside it.
///
/// Events that were posted to the process of the shared window instead showed a tap on the phone
/// and did nothing in the window on a Mac, where the same tap handed to the system clicked on a
/// shared screen (docs/adr/0009-window-streaming.md, R6). Why is inferred: such an event does not
/// move the pointer, and a button that is pressed looks at where the pointer is, not at where the
/// event says.
///
/// The system asks the person at this Mac for consent to post events, and a sandboxed app may not
/// be let to at all (R6). Without consent nothing is posted and the post fails. A post returns
/// nothing to say whether it was taken, so the first time that the pointer is moved this way the
/// log says whether the pointer followed.
///
/// It is used by one thread at a time.
final class MacEventPoster: EventPoster {
  /// How long an answer about consent is used again, in seconds.
  private static let reuse: TimeInterval = 0.25

  private let source: CGEventSource?
  private let now: () -> TimeInterval
  private var consent: (given: Bool, at: TimeInterval)?
  /// Whether consent was told about last as given, so that it is told again when it changes.
  private var toldGiven: Bool?
  private var posted = false
  /// Whether the pointer was looked at after the first move that was posted to the system.
  private var pointerChecked = false
  /// Where the last mouse event was, for the movement that the next one carries.
  private var previous: CGPoint?
  /// Told when a key is handed to the system, in a session where nobody at the Mac may press one
  /// (BatonPrivacy.swift): the system counts that key as one of the Mac's.
  private let ledger: PostLedger?

  init(
    ledger: PostLedger? = nil,
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
  ) {
    self.ledger = ledger
    // Events for the system are those of a keyboard and a mouse that are there, and have the state
    // of those. The person at this Mac keeps their own pointer and keys: the system would
    // otherwise ignore them for a moment after each event that is posted, and a person who wants
    // to stop the sharing could not click while a finger moves at the other device.
    let hardware = CGEventSource(stateID: .hidSystemState)
    hardware?.localEventsSuppressionInterval = 0
    source = hardware
    self.now = now
  }

  func post(_ event: SyntheticEvent) -> Bool {
    guard consentGiven(), let source, let made = cgEvent(for: event, source) else { return false }
    if case .key(let key) = event, key.down { ledger?.notedKeyDown() }
    made.post(tap: .cghidEventTap)
    if !posted {
      posted = true
      controlLog.notice("Baton: the first input event was handed to the system.")
    }
    checkPointer(after: event)
    return true
  }

  /// The first time that the pointer is moved by a post, looks at whether it went there. Where it
  /// went is not told, only whether it did.
  private func checkPointer(after event: SyntheticEvent) {
    guard !pointerChecked, case .mouse(let mouse) = event else { return }
    pointerChecked = true
    let wanted = CGPoint(x: mouse.at.x, y: mouse.at.y)
    // The window server takes a moment to move the pointer.
    var followed = false
    for _ in 0..<10 {
      if let place = CGEvent(source: nil)?.location, abs(place.x - wanted.x) <= 2,
        abs(place.y - wanted.y) <= 2
      {
        followed = true
        break
      }
      Thread.sleep(forTimeInterval: 0.01)
    }
    let word = followed ? "followed" : "did not follow"
    controlLog.notice(
      "Baton: the pointer \(word, privacy: .public) the first mouse event handed to the system.")
  }

  private func consentGiven() -> Bool {
    let time = now()
    if let consent, time >= consent.at, time - consent.at < MacEventPoster.reuse {
      return consent.given
    }
    let given = CGPreflightPostEventAccess()
    consent = (given, time)
    if given != toldGiven {
      toldGiven = given
      controlLog.notice(
        "Baton: this app \(given ? "may" : "may not", privacy: .public) post input events.")
    }
    return given
  }

  // MARK: One event

  private func cgEvent(for event: SyntheticEvent, _ source: CGEventSource) -> CGEvent? {
    switch event {
    case .mouse(let mouse): return mouseEvent(mouse, source)
    case .scroll(let scroll): return scrollEvent(scroll, source)
    case .key(let key): return keyEvent(key, source)
    }
  }

  private func mouseEvent(_ mouse: SyntheticMouse, _ source: CGEventSource) -> CGEvent? {
    let type: CGEventType
    let button: CGMouseButton
    switch (mouse.kind, mouse.button) {
    case (.moved, _): (type, button) = (.mouseMoved, .left)
    case (.down, .left): (type, button) = (.leftMouseDown, .left)
    case (.down, .right): (type, button) = (.rightMouseDown, .right)
    case (.down, .middle): (type, button) = (.otherMouseDown, .center)
    case (.up, .left): (type, button) = (.leftMouseUp, .left)
    case (.up, .right): (type, button) = (.rightMouseUp, .right)
    case (.up, .middle): (type, button) = (.otherMouseUp, .center)
    case (.dragged, .left): (type, button) = (.leftMouseDragged, .left)
    case (.dragged, .right): (type, button) = (.rightMouseDragged, .right)
    case (.dragged, .middle): (type, button) = (.otherMouseDragged, .center)
    }
    let point = CGPoint(x: mouse.at.x, y: mouse.at.y)
    guard
      let made = CGEvent(
        mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: button)
    else { return nil }
    made.flags = flags(of: mouse.modifiers)
    made.setIntegerValueField(.mouseEventClickState, value: Int64(mouse.clickCount))
    made.setDoubleValueField(.mouseEventPressure, value: mouse.pressure)
    // Some views take their movement from the event, and not from where it is.
    if mouse.kind == .moved || mouse.kind == .dragged, let previous {
      made.setIntegerValueField(.mouseEventDeltaX, value: Int64((point.x - previous.x).rounded()))
      made.setIntegerValueField(.mouseEventDeltaY, value: Int64((point.y - previous.y).rounded()))
    }
    previous = point
    return made
  }

  private func scrollEvent(_ scroll: SyntheticScroll, _ source: CGEventSource) -> CGEvent? {
    // The first wheel is the vertical one, and the second the horizontal one. A positive number
    // moves the content down and to the right, as the finger of the person does.
    guard
      let made = CGEvent(
        scrollWheelEvent2Source: source, units: .pixel, wheelCount: 2, wheel1: scroll.dy,
        wheel2: scroll.dx, wheel3: 0)
    else { return nil }
    made.location = CGPoint(x: scroll.at.x, y: scroll.at.y)
    made.flags = flags(of: scroll.modifiers)
    return made
  }

  private func keyEvent(_ key: SyntheticKey, _ source: CGEventSource) -> CGEvent? {
    guard
      let made = CGEvent(
        keyboardEventSource: source, virtualKey: CGKeyCode(key.virtualKey), keyDown: key.down)
    else { return nil }
    made.flags = flags(of: key.modifiers).union(flags(of: key.keyFlags))
    if let text = key.text, !text.isEmpty {
      text.withUnsafeBufferPointer {
        made.keyboardSetUnicodeString(stringLength: $0.count, unicodeString: $0.baseAddress)
      }
    }
    if key.autorepeat {
      made.setIntegerValueField(.keyboardEventAutorepeat, value: 1)
    }
    return made
  }

  private func flags(of modifiers: InputModifiers) -> CGEventFlags {
    var flags = CGEventFlags()
    if modifiers.contains(.shift) { flags.insert(.maskShift) }
    if modifiers.contains(.control) { flags.insert(.maskControl) }
    if modifiers.contains(.option) { flags.insert(.maskAlternate) }
    if modifiers.contains(.command) { flags.insert(.maskCommand) }
    if modifiers.contains(.capsLock) { flags.insert(.maskAlphaShift) }
    return flags
  }

  /// What the keyboard of a Mac sets on some keys, which the shortcuts of the system look for.
  private func flags(of keyFlags: KeyFlags) -> CGEventFlags {
    var flags = CGEventFlags()
    if keyFlags.contains(.function) { flags.insert(.maskSecondaryFn) }
    if keyFlags.contains(.numericPad) { flags.insert(.maskNumericPad) }
    return flags
  }
}

// MARK: - The control of a window or a display

/// A line that says that control was set up, and whether this app may post, so that a person who
/// reads the log of the Mac finds out whether the lines of this app reach it at all.
private func noteControlSetUp(of what: String) {
  let access = macControlAccess() == .allowed ? "may post" : "may not post"
  let line = "Baton: control of the shared \(what) is set up, and this app \(access) input events."
  controlLog.notice("\(line, privacy: .public)")
}

/// What the Rust side asks of a window that is shared with control, for the window with this
/// number, which belongs to this process. Its events go to the system, as those of a display do,
/// but the shortcuts that the system keeps for itself are not let through: they act on the whole
/// Mac and not on the window.
func makeWindowControl(window: UInt32, process: Int32) -> WindowControl {
  noteControlSetUp(of: "window")
  return WindowControl(
    reader: SurroundingsReader(
      window: window, process: process, listing: MacWindowListing(), activity: MacActivityReader(),
      layers: macWindowLayers()),
    translator: InputTranslator(
      doubleClickInterval: NSEvent.doubleClickInterval, movesPointerToScroll: true,
      dropsSystemShortcuts: true),
    poster: MacEventPoster())
}

/// What the Rust side asks of a whole display that is shared with control, for the display with
/// this number. Its events go to the system, as if the person at this Mac did them, and so do the
/// shortcuts that the system keeps for itself. `ledger` is given for a private session, where the
/// keys that are handed to the system have to be told from those of somebody at the Mac.
func makeDisplayControl(display: UInt32, ledger: PostLedger? = nil) -> WindowControl {
  noteControlSetUp(of: "screen")
  return WindowControl(
    reader: DisplayReader(
      display: display, listing: MacDisplayListing(), activity: MacActivityReader()),
    translator: InputTranslator(
      doubleClickInterval: NSEvent.doubleClickInterval, movesPointerToScroll: true),
    poster: MacEventPoster(ledger: ledger))
}

#endif
