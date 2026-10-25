// SPDX-License-Identifier: Apache-2.0

// The calls of a Mac that BatonInput.swift needs to let the person at another device steer a window
// that is shared: what the window server says of windows, what the system says of the Mac, and the
// events that are handed to the process of the window. What is asked, and what is done with the
// answers, is in BatonInput.swift and, on the Rust side, in crates/baton-stream/src/gate.rs. This
// file only talks to the system, so it is built on a Mac and nowhere else, and what it does there
// is checked on one (docs/adr/0009-window-streaming.md, R6 and R7).
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

// MARK: - The Mac

/// What the system says of the Mac, and the way to bring an application to the front.
final class MacActivityReader: MacActivity {
  var locked: Bool {
    if let session = CGSessionCopyCurrentDictionary() as? [String: Any] {
      // The key for the lock screen is not documented, and has been there as long as the lock
      // screen has. A session that is not on the console is one that another user switched away
      // from.
      if (session["CGSSessionScreenIsLocked"] as? NSNumber)?.boolValue == true { return true }
      if (session[kCGSessionOnConsoleKey as String] as? NSNumber)?.boolValue == false {
        return true
      }
    }
    return CGDisplayIsAsleep(CGMainDisplayID()) != 0
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
        NSLog(
          "Baton: the application could not be brought to the front (%ld).",
          (error as NSError).code)
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

// MARK: - The events

/// Hands events to the process of a shared window. They are not posted to the system, so the
/// pointer of the person at this Mac does not move and the keys that they press are not mixed with
/// them: an event of its own state, to one process.
///
/// The system asks the person at this Mac for consent to post events, and a sandboxed app may not
/// be let to at all (docs/adr/0009-window-streaming.md, R6). Without consent nothing is posted and
/// the post fails.
///
/// It is used by one thread at a time.
final class MacEventPoster: EventPoster {
  /// How long an answer about consent is used again, in seconds.
  private static let reuse: TimeInterval = 0.25

  private let window: UInt32
  private let source = CGEventSource(stateID: .privateState)
  private let now: () -> TimeInterval
  private var consent: (given: Bool, at: TimeInterval)?
  /// Whether consent was told about last as given, so that it is told again when it changes.
  private var toldGiven: Bool?
  private var posted = false
  /// Where the last mouse event was, for the movement that the next one carries.
  private var previous: CGPoint?

  /// `window` is the number of the shared window, which the events are aimed at.
  init(
    window: UInt32, now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }
  ) {
    self.window = window
    self.now = now
  }

  func post(_ event: SyntheticEvent, toProcess process: Int32) -> Bool {
    guard consentGiven(), let source, let made = cgEvent(for: event, source) else { return false }
    made.postToPid(process)
    if !posted {
      posted = true
      NSLog("Baton: the first input event was handed to the window's process.")
    }
    return true
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
      NSLog("Baton: this app %@ post input events.", given ? "may" : "may not")
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
    aim(made)
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
    aim(made)
    return made
  }

  private func keyEvent(_ key: SyntheticKey, _ source: CGEventSource) -> CGEvent? {
    guard
      let made = CGEvent(
        keyboardEventSource: source, virtualKey: CGKeyCode(key.virtualKey), keyDown: key.down)
    else { return nil }
    made.flags = flags(of: key.modifiers)
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

  /// The event is for the shared window, which the system would otherwise look for under the
  /// pointer.
  private func aim(_ made: CGEvent) {
    made.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(window))
    made.setIntegerValueField(
      .mouseEventWindowUnderMousePointerThatCanHandleThisEvent, value: Int64(window))
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
}

// MARK: - The control of a window

/// What the Rust side asks of a window that is shared with control, for the window with this
/// number, which belongs to this process.
func makeWindowControl(window: UInt32, process: Int32) -> WindowControl {
  // A line that says that control was set up, and whether this app may post, so that a person who
  // reads the log of the Mac finds out whether the lines of this app reach it at all.
  if macControlAccess() == .allowed {
    NSLog("Baton: control of the shared window is set up, and this app may post input events.")
  } else {
    NSLog("Baton: control of the shared window is set up, and this app may not post input events.")
  }
  return WindowControl(
    reader: SurroundingsReader(
      window: window, process: process, listing: MacWindowListing(), activity: MacActivityReader(),
      layers: macWindowLayers()),
    translator: InputTranslator(doubleClickInterval: NSEvent.doubleClickInterval),
    poster: MacEventPoster(window: window), process: process)
}

#endif
