// SPDX-License-Identifier: Apache-2.0

// The checks for the privacy mode of a Mac (docs/research/privacy-mode/README.md). run.sh builds this
// into an app that is sandboxed as the app of Baton is, and runs the stages with a person at the
// Mac. The person is asked what the checks cannot see for themselves. Nothing here is part of the
// app of Baton.
//
// For a few seconds at a time it holds the keys and the pointer of the Mac, covers the screens
// with black, hides the pointer and turns the backlight down. Each of these ends by itself, and
// `probe restore` undoes what a crash could leave.

import AppKit
import Carbon.HIToolbox
import CoreGraphics
import Foundation
import ScreenCaptureKit
import ServiceManagement

/// Where the app and run.sh leave each other's notes. The sandbox of the app lets it use this folder
/// and no other.
let dir = "/private/tmp/baton-probe"
let begun = Date()

// MARK: - What is written down

final class Report {
  /// What earlier launches of the app wrote, since run.sh starts the app once for each stage, so that
  /// a stage that ends the app does not take the others with it.
  private var text = (try? String(contentsOfFile: dir + "/report.txt", encoding: .utf8)) ?? ""

  var isEmpty: Bool { text.isEmpty }

  func line(_ line: String) {
    let seconds = String(format: "%6.1f", Date().timeIntervalSince(begun))
    text += "[\(seconds)] \(line)\n"
    try? text.write(toFile: dir + "/report.txt", atomically: true, encoding: .utf8)
  }
}

let report = Report()

/// Tells run.sh where the checks are. It reads the file twice a second, so a phase is not left again
/// at once.
func phase(_ name: String) {
  try? name.write(toFile: dir + "/phase", atomically: true, encoding: .utf8)
}

func seconds(_ value: Double) -> String { String(format: "%.2f", value) }

/// How long ago the keys or the pointer were last used, as the system counts it: for the Mac itself
/// (`hidSystemState`) or for the session with what programs post (`combinedSessionState`).
func since(_ state: CGEventSourceStateID, _ type: CGEventType) -> Double {
  CGEventSource.secondsSinceLastEventType(state, eventType: type)
}

// MARK: - The person at the Mac

/// Lets the run loop go on for a while: the events of the tap, the windows and the timers are all
/// served by it.
func spin(_ time: Double) { RunLoop.current.run(until: Date(timeIntervalSinceNow: time)) }

/// A window with words, for the checks that leave the screen as it is.
final class Panel {
  private let window: NSWindow
  private let label: NSTextField

  init() {
    window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 760, height: 320), styleMask: [.titled],
      backing: .buffered, defer: false)
    window.title = "Baton privacy checks"
    window.level = .floating
    window.isReleasedWhenClosed = false
    label = NSTextField(wrappingLabelWithString: "")
    label.font = NSFont.systemFont(ofSize: 20)
    label.frame = NSRect(x: 24, y: 20, width: 712, height: 270)
    window.contentView?.addSubview(label)
    window.center()
  }

  func show(_ text: String) {
    label.stringValue = text
    window.orderFrontRegardless()
  }

  func hide() { window.orderOut(nil) }
}

private var madePanel: Panel?

/// The panel is made when it is first used, which is after the application exists: a window may not
/// be made before.
var panel: Panel {
  if let madePanel { return madePanel }
  let made = Panel()
  madePanel = made
  return made
}

/// Asks what the person saw, and writes down the answer.
@discardableResult
func ask(_ question: String, _ choices: [String]) -> String {
  panel.hide()
  NSApp.activate(ignoringOtherApps: true)
  let alert = NSAlert()
  alert.messageText = question
  for choice in choices { alert.addButton(withTitle: choice) }
  let index = alert.runModal().rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
  let answer = choices.indices.contains(index) ? choices[index] : "?"
  report.line("answer: \(question) -> \(answer)")
  return answer
}

// MARK: - The Mac

func displayID(of screen: NSScreen) -> CGDirectDisplayID {
  (screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value ?? 0
}

func accessLine() -> String {
  "input monitoring \(CGPreflightListenEventAccess()), post events \(CGPreflightPostEventAccess()), "
    + "accessibility \(AXIsProcessTrusted()), screen recording \(CGPreflightScreenCaptureAccess())"
}

func describeTheMac() {
  let info = ProcessInfo.processInfo
  let sandboxed = info.environment["APP_SANDBOX_CONTAINER_ID"] != nil
  report.line("macOS \(info.operatingSystemVersionString), sandboxed \(sandboxed)")
  let identity = (try? String(contentsOfFile: dir + "/.identity", encoding: .utf8)) ?? "?"
  report.line("signed with \(identity.trimmingCharacters(in: .whitespacesAndNewlines))")
  for screen in NSScreen.screens {
    let id = displayID(of: screen)
    report.line(
      "display \(id): \(Int(screen.frame.width))x\(Int(screen.frame.height)), "
        + "built in \(CGDisplayIsBuiltin(id) != 0), main \(id == CGMainDisplayID())")
  }
  report.line("access: \(accessLine())")
}

// MARK: - A tap that holds the input

let typeNames: [UInt32: String] = [
  CGEventType.keyDown.rawValue: "keyDown", CGEventType.keyUp.rawValue: "keyUp",
  CGEventType.flagsChanged.rawValue: "flagsChanged",
  CGEventType.leftMouseDown.rawValue: "leftMouseDown",
  CGEventType.leftMouseUp.rawValue: "leftMouseUp",
  CGEventType.rightMouseDown.rawValue: "rightMouseDown",
  CGEventType.rightMouseUp.rawValue: "rightMouseUp",
  CGEventType.otherMouseDown.rawValue: "otherMouseDown",
  CGEventType.otherMouseUp.rawValue: "otherMouseUp",
  CGEventType.mouseMoved.rawValue: "mouseMoved",
  CGEventType.leftMouseDragged.rawValue: "leftMouseDragged",
  CGEventType.rightMouseDragged.rawValue: "rightMouseDragged",
  CGEventType.otherMouseDragged.rawValue: "otherMouseDragged",
  CGEventType.scrollWheel.rawValue: "scrollWheel", 14: "systemDefined",
]

/// What the tap has seen. The tap is on the main run loop, so this is used by one thread.
final class TapState {
  var port: CFMachPort?
  /// After this the tap lets everything through. A tap that holds the input must not be able to
  /// hold it for long, whatever goes wrong.
  var until = Date.distantPast
  var panic = false
  var escapes: [Date] = []
  var hardware: [String: Int] = [:]
  var others = 0
  var ours = 0
  var disabled = 0

  var hardwareCount: Int { hardware.values.reduce(0, +) }

  func begin(maxSeconds: Double) {
    until = Date(timeIntervalSinceNow: maxSeconds)
    panic = false
    escapes = []
    hardware = [:]
    others = 0
    ours = 0
    disabled = 0
  }

  func handle(_ type: CGEventType, _ event: CGEvent) -> Unmanaged<CGEvent>? {
    if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
      disabled += 1
      if let port { CGEvent.tapEnable(tap: port, enable: true) }
      return nil
    }
    // The process that posted an event is told by the window server and not by the poster. It is
    // zero for the keys and the pointer of the Mac.
    let poster = event.getIntegerValueField(.eventSourceUnixProcessID)
    if poster == Int64(getpid()) {
      ours += 1
      return Unmanaged.passUnretained(event)
    }
    if poster == 0 {
      hardware[typeNames[type.rawValue] ?? "type \(type.rawValue)", default: 0] += 1
    } else {
      others += 1
    }
    // Three presses of Escape within two seconds end the hold.
    if type == .keyDown, event.getIntegerValueField(.keyboardEventKeycode) == 53 {
      let now = Date()
      escapes = escapes.filter { now.timeIntervalSince($0) < 2 } + [now]
      if escapes.count >= 3 { panic = true }
    }
    if panic || Date() > until { return Unmanaged.passUnretained(event) }
    return nil
  }
}

let tapState = TapState()

func tapCallback(
  proxy: CGEventTapProxy, type: CGEventType, event: CGEvent, refcon: UnsafeMutableRawPointer?
) -> Unmanaged<CGEvent>? {
  tapState.handle(type, event)
}

struct InstalledTap {
  let port: CFMachPort
  let source: CFRunLoopSource
  let place: String
}

let tapMask: CGEventMask = {
  var mask: CGEventMask = 0
  for raw in typeNames.keys { mask |= CGEventMask(1) << CGEventMask(raw) }
  return mask
}()

/// An active tap, which can drop what it is handed, as early in the path of the events as the
/// system allows.
func installTap(quiet: Bool) -> InstalledTap? {
  let places: [(CGEventTapLocation, String)] = [(.cghidEventTap, "hid"), (.cgSessionEventTap, "session")]
  for (location, name) in places {
    guard
      let port = CGEvent.tapCreate(
        tap: location, place: .headInsertEventTap, options: .defaultTap, eventsOfInterest: tapMask,
        callback: tapCallback, userInfo: nil)
    else {
      if !quiet { report.line("tap: not made at the \(name) level") }
      continue
    }
    guard let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, port, 0) else {
      if !quiet { report.line("tap: made at the \(name) level, but it has no run loop source") }
      continue
    }
    tapState.port = port
    CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
    CGEvent.tapEnable(tap: port, enable: true)
    if !quiet { report.line("tap: made at the \(name) level, active") }
    return InstalledTap(port: port, source: source, place: name)
  }
  return nil
}

func remove(_ tap: InstalledTap) {
  CGEvent.tapEnable(tap: tap.port, enable: false)
  CFRunLoopRemoveSource(CFRunLoopGetMain(), tap.source, .commonModes)
  CFMachPortInvalidate(tap.port)
  tapState.port = nil
}

/// Makes the tap, and when the system does not allow it, asks the person to allow the app and tries
/// again for a while.
func installTapAskingForAccess() -> InstalledTap? {
  if let tap = installTap(quiet: false) { return tap }
  report.line("tap: access is \(accessLine())")
  _ = CGRequestListenEventAccess()
  _ = CGRequestPostEventAccess()
  panel.show(
    "This check needs the app to be allowed.\n\nIn System Settings, under Privacy & Security, turn "
      + "on BatonProbe in Input Monitoring and in Accessibility (add it with + when it is not "
      + "listed). The check goes on by itself for 90 seconds.")
  let end = Date(timeIntervalSinceNow: 90)
  while Date() < end {
    spin(2)
    if let tap = installTap(quiet: true) {
      report.line("tap: made after the person allowed the app, access is \(accessLine())")
      remove(tap)
      return installTap(quiet: true)
    }
  }
  return nil
}

// MARK: - Stages that hold the input

/// Holds the input for ten seconds, with the person typing, clicking and swiping, and writes down
/// what the tap saw, what got through, and what the counters of the keyboard and pointer say.
func hold(_ name: String) {
  var expected = CGEvent(source: nil)?.location ?? .zero
  var flip: CGFloat = 3
  var tried = 0
  var moved = 0

  func nudge(_ state: CGEventSourceStateID) {
    let target = CGPoint(x: expected.x + flip, y: expected.y)
    CGEvent(
      mouseEventSource: CGEventSource(stateID: state), mouseType: .mouseMoved,
      mouseCursorPosition: target, mouseButton: .left)?.post(tap: .cghidEventTap)
    spin(0.05)
    let there = CGEvent(source: nil)?.location ?? .zero
    tried += 1
    if abs(there.x - target.x) <= 1.5 && abs(there.y - target.y) <= 1.5 { moved += 1 }
    expected = target
    flip = -flip
  }

  // First the person keeps their hands off, and moves posted by this app are made from the two
  // kinds of source. If the counters of the keys and pointer of the Mac show these moves, they
  // cannot tell the person from the app, and the app has to keep its own count.
  func quiet(_ kind: String, _ state: CGEventSourceStateID) {
    var hidAge = Double.infinity
    var sessionAge = Double.infinity
    let end = Date(timeIntervalSinceNow: 3)
    while Date() < end {
      nudge(state)
      hidAge = min(hidAge, since(.hidSystemState, .mouseMoved))
      sessionAge = min(sessionAge, since(.combinedSessionState, .mouseMoved))
      spin(0.2)
    }
    report.line(
      "\(name): hands off, moves from a \(kind) source: the pointer of the Mac was last moved "
        + "at least \(seconds(hidAge)) s before a look (\(seconds(sessionAge)) s for the session)")
  }
  panel.show("Hands off the keys, the pointer and the trackpad for 8 seconds.")
  spin(2)
  quiet("private", .privateState)
  quiet("hid", .hidSystemState)

  panel.show("Get ready. The keys and the pointer of this Mac stop working in 3 seconds.")
  spin(3)
  guard let tap = installTapAskingForAccess() else {
    report.line("\(name): the tap could not be made, access is \(accessLine())")
    return
  }
  tapState.begin(maxSeconds: 13)
  var keyAge = since(.hidSystemState, .keyDown)
  var keyMin = keyAge
  var keyResets = 0
  let start = Date()
  while Date().timeIntervalSince(start) < 10 && !tapState.panic {
    let left = Int(10 - Date().timeIntervalSince(start))
    panel.show(
      "NOW: type letters (not the Command key) in the field that is open, click, move the pointer, "
        + "scroll, swipe with three and with four fingers, pinch, press the volume and brightness "
        + "keys. Nothing should happen.\n\n\(left) seconds left. Escape three times ends it.")
    nudge(.privateState)
    let age = since(.hidSystemState, .keyDown)
    keyMin = min(keyMin, age)
    // The counter starts again from zero at a key press, so a fall is a key press that it saw.
    if age + 0.05 < keyAge { keyResets += 1 }
    keyAge = age
    spin(0.15)
  }
  let seen = tapState.hardware.sorted { $0.key < $1.key }.map { "\($0.key) \($0.value)" }
  let place = tap.place
  remove(tap)
  panel.hide()
  report.line("\(name): the tap was at the \(place) level, held by this app for \(seconds(Date().timeIntervalSince(start))) s")
  report.line("\(name): events of the Mac that the tap saw: \(seen.isEmpty ? "none" : seen.joined(separator: ", "))")
  report.line("\(name): events of other programs that the tap saw: \(tapState.others)")
  report.line("\(name): moves posted by this app: \(tried) tried, \(tapState.ours) seen by the tap, \(moved) moved the pointer")
  let there = CGEvent(source: nil)?.location ?? .zero
  report.line(
    "\(name): the pointer ended \(seconds(Double(there.x - expected.x))), "
      + "\(seconds(Double(there.y - expected.y))) from where this app's moves leave it")
  report.line(
    "\(name): counter of the keys: shortest time since a key press \(seconds(keyMin)) s, "
      + "\(keyResets) key presses seen by it")
  report.line("\(name): the system switched the tap off \(tapState.disabled) times, escape pressed to end it: \(tapState.panic)")
  report.line("\(name): secure input is on: \(IsSecureEventInputEnabled())")
}

/// A field in another process that has the keyboard. With `kind` `secure` it is a field for a
/// password, which turns on the secure input of the system as every such field does.
func stageField(_ name: String, kind: String, intro: String) {
  phase(name)
  report.line("== \(name)")
  let files = FileManager.default
  for file in ["field-ready", "field-stop", "field-result"] {
    try? files.removeItem(atPath: dir + "/" + file)
  }
  panel.show("Check \(name): getting a field ready.")
  phase("field-" + kind)
  var ready = false
  for _ in 0..<60 {
    spin(0.25)
    if files.fileExists(atPath: dir + "/field-ready") {
      ready = true
      break
    }
  }
  report.line("\(name): the field window \(ready ? "is up" : "did not come up")")
  if kind == "secure" {
    var on = false
    for _ in 0..<20 {
      if IsSecureEventInputEnabled() {
        on = true
        break
      }
      spin(0.25)
    }
    report.line("\(name): secure input is \(on ? "on" : "not on") with the password field open")
  }
  panel.show(intro)
  spin(6)
  hold(name)
  try? "stop".write(toFile: dir + "/field-stop", atomically: true, encoding: .utf8)
  var result: String?
  for _ in 0..<40 {
    spin(0.25)
    result = try? String(contentsOfFile: dir + "/field-result", encoding: .utf8)
    if result != nil { break }
  }
  report.line("\(name): the field says: \(result?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "nothing")")
  if kind == "secure" {
    var off = false
    for _ in 0..<20 {
      if !IsSecureEventInputEnabled() {
        off = true
        break
      }
      spin(0.25)
    }
    report.line("\(name): secure input is \(off ? "off" : "still on") after the field is gone")
  }
  ask(
    "While the input was held, what got through to the screen?",
    ["Nothing", "Keys", "The pointer or the trackpad", "Keys and pointer or trackpad"])
}

// MARK: - A cover over the screens

func makeCover(on screen: NSScreen) -> NSWindow {
  let cover = NSWindow(
    contentRect: screen.frame, styleMask: [.borderless], backing: .buffered, defer: false,
    screen: screen)
  cover.level = NSWindow.Level(rawValue: Int(CGShieldingWindowLevel()))
  cover.backgroundColor = .black
  cover.isOpaque = true
  cover.hasShadow = false
  // A click goes through to the window below, as the posted clicks of a remote person have to.
  cover.ignoresMouseEvents = true
  cover.isReleasedWhenClosed = false
  cover.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenAuxiliary]
  cover.setFrame(screen.frame, display: true)
  cover.orderFrontRegardless()
  return cover
}

func words(on cover: NSWindow, _ text: String) -> NSTextField {
  let label = NSTextField(wrappingLabelWithString: text)
  label.font = NSFont.systemFont(ofSize: 26)
  label.textColor = .white
  label.alignment = .center
  label.frame = NSRect(
    x: 80, y: cover.frame.height / 2 - 150, width: cover.frame.width - 160, height: 300)
  cover.contentView?.addSubview(label)
  return label
}

struct Listed {
  let owner: String
  let process: Int
  let layer: Int
  let width: Int
  let height: Int
  let alpha: Double
}

func listedWindows() -> [Listed] {
  guard
    let all = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID)
      as? [[String: Any]]
  else { return [] }
  return all.map { info in
    let bounds = info[kCGWindowBounds as String] as? [String: Any]
    return Listed(
      owner: info[kCGWindowOwnerName as String] as? String ?? "?",
      process: (info[kCGWindowOwnerPID as String] as? NSNumber)?.intValue ?? 0,
      layer: (info[kCGWindowLayer as String] as? NSNumber)?.intValue ?? 0,
      width: (bounds?["Width"] as? NSNumber)?.intValue ?? 0,
      height: (bounds?["Height"] as? NSNumber)?.intValue ?? 0,
      alpha: (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1)
  }
}

func stageShield() {
  phase("shield")
  report.line("== shield: a black cover at the shielding level \(CGShieldingWindowLevel()), input free")
  panel.hide()
  let covers = NSScreen.screens.map { makeCover(on: $0) }
  let base =
    "Check shield. For the next 36 seconds, one after the other:\n"
    + "1. Command+Space (Spotlight), then Escape.\n"
    + "2. Control+Up Arrow (Mission Control), then Escape.\n"
    + "3. Pointer to the top edge, then to the bottom edge.\n"
    + "4. Wait for two notification banners (Focus must be off).\n\n"
    + "This cover goes away by itself."
  let label = covers.first.map { words(on: $0, base) }
  phase("shield-banners")
  struct Seen {
    var layer: Int
    var width: Int
    var height: Int
    var polls: Int
  }
  var seen: [String: Seen] = [:]
  var ownLayer = Int.min
  let end = Date(timeIntervalSinceNow: 36)
  while Date() < end {
    for window in listedWindows() {
      if window.process == Int(getpid()) {
        if window.width >= 100 { ownLayer = max(ownLayer, window.layer) }
        continue
      }
      guard window.alpha > 0.05, window.width >= 10, window.height >= 10 else { continue }
      let key = "\(window.owner), layer \(window.layer)"
      var entry = seen[key] ?? Seen(layer: window.layer, width: 0, height: 0, polls: 0)
      entry.width = max(entry.width, window.width)
      entry.height = max(entry.height, window.height)
      entry.polls += 1
      seen[key] = entry
    }
    label?.stringValue = base + " \(Int(end.timeIntervalSinceNow)) seconds left."
    spin(0.25)
  }
  covers.forEach { $0.orderOut(nil) }
  report.line("shield: the covers sit at layer \(ownLayer)")
  for (key, entry) in seen.sorted(by: { $0.value.layer > $1.value.layer }).prefix(30)
  where entry.layer >= 0 {
    let flag = entry.layer >= ownLayer ? "  <== AT OR ABOVE THE COVER" : ""
    report.line(
      "shield: other window: \(key), up to \(entry.width)x\(entry.height), listed \(entry.polls) times\(flag)")
  }
  ask(
    "Spotlight (Command+Space) while the black cover was up:",
    ["Did not show over the cover", "Showed over the cover", "Did not open"])
  ask(
    "Mission Control (Control+Up Arrow) while the black cover was up:",
    ["Did not show over the cover", "Showed over the cover", "Did not open"])
  ask(
    "Notification banners while the black cover was up:",
    ["Showed over the cover", "Were hidden by the cover", "None came up at all"])
  ask(
    "The menu bar and the Dock at the edges of the screen while the cover was up:",
    ["Neither showed", "The menu bar showed", "The Dock showed", "Both showed"])
}

// MARK: - The pointer and the backlight

@_silgen_name("CGSMainConnectionID")
func CGSMainConnectionID() -> Int32

@_silgen_name("CGSSetConnectionProperty")
func CGSSetConnectionProperty(_ connection: Int32, _ target: Int32, _ key: CFString, _ value: CFTypeRef) -> Int32

typealias GetBrightness = @convention(c) (UInt32, UnsafeMutablePointer<Float>) -> Int32
typealias SetBrightness = @convention(c) (UInt32, Float) -> Int32

let displayServices = dlopen(
  "/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices", RTLD_NOW)

func brightness(of id: CGDirectDisplayID) -> Float? {
  guard let displayServices, let symbol = dlsym(displayServices, "DisplayServicesGetBrightness") else {
    return nil
  }
  var value: Float = 0
  return unsafeBitCast(symbol, to: GetBrightness.self)(id, &value) == 0 ? value : nil
}

func setBrightness(_ value: Float, of id: CGDirectDisplayID) -> Bool {
  guard let displayServices, let symbol = dlsym(displayServices, "DisplayServicesSetBrightness") else {
    return false
  }
  return unsafeBitCast(symbol, to: SetBrightness.self)(id, value) == 0
}

/// What the backlight was before it was turned down, kept in a file as well so that `restore` can
/// put it back after a crash.
var savedBrightness: [(CGDirectDisplayID, Float)] = []

func restoreWhatIsLeft() {
  for (id, value) in savedBrightness { _ = setBrightness(value, of: id) }
  savedBrightness = []
  CGDisplayRestoreColorSyncSettings()
}

func restoreFromFile() {
  CGDisplayRestoreColorSyncSettings()
  guard let text = try? String(contentsOfFile: dir + "/brightness", encoding: .utf8) else { return }
  for line in text.split(separator: "\n") {
    let parts = line.split(separator: " ")
    if parts.count == 2, let id = UInt32(parts[0]), let value = Float(parts[1]) {
      _ = setBrightness(value, of: id)
    }
  }
  try? FileManager.default.removeItem(atPath: dir + "/brightness")
}

/// Moves the pointer round the main screen the way a remote person's moves would, with posted
/// events when this app may post them, and with warps in any case.
func wander(seconds time: Double) {
  let bounds = CGDisplayBounds(CGMainDisplayID())
  let source = CGEventSource(stateID: .privateState)
  let post = CGPreflightPostEventAccess()
  let end = Date(timeIntervalSinceNow: time)
  var angle = 0.0
  while Date() < end {
    let point = CGPoint(
      x: bounds.midX + bounds.width * 0.3 * CGFloat(cos(angle)),
      y: bounds.midY + bounds.height * 0.3 * CGFloat(sin(angle)))
    CGWarpMouseCursorPosition(point)
    if post {
      CGEvent(
        mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point,
        mouseButton: .left)?.post(tap: .cghidEventTap)
    }
    angle += 0.12
    spin(0.04)
  }
}

func stageCursor() {
  phase("cursor")
  report.line("== cursor: hiding the pointer from a program that is not in front, and the backlight")
  panel.hide()
  let covers = NSScreen.screens.map { makeCover(on: $0) }
  let label = covers.first.map {
    words(on: $0, "Check cursor. Watch this black screen: is the pointer visible while it moves?")
  }
  let main = CGMainDisplayID()

  label?.stringValue = "Part 1 of 2. Is the pointer visible while it moves? (10 seconds)"
  let hidden = CGDisplayHideCursor(main)
  report.line("cursor: CGDisplayHideCursor from a program not in front: \(hidden.rawValue)")
  wander(seconds: 10)
  _ = CGDisplayShowCursor(main)
  covers.forEach { $0.orderOut(nil) }
  ask("Plain hide, pointer moving over the black cover:", ["Not visible", "Visible", "Flickered"])

  let connection = CGSMainConnectionID()
  let yes: CFTypeRef = NSNumber(value: true)
  let property = CGSSetConnectionProperty(connection, connection, "SetsCursorInBackground" as CFString, yes)
  report.line("cursor: SetsCursorInBackground: \(property)")
  let again = NSScreen.screens.map { makeCover(on: $0) }
  let second = again.first.map { words(on: $0, "Part 2 of 2. Is the pointer visible while it moves? (10 seconds)") }
  _ = CGDisplayHideCursor(main)
  wander(seconds: 10)
  _ = CGDisplayShowCursor(main)
  second?.stringValue = "Backlight. Is the built-in screen fully dark? (8 seconds)"

  var saved: [(CGDirectDisplayID, Float)] = []
  for screen in NSScreen.screens {
    let id = displayID(of: screen)
    if let value = brightness(of: id) { saved.append((id, value)) }
  }
  savedBrightness = saved
  try? saved.map { "\($0.0) \($0.1)" }.joined(separator: "\n")
    .write(toFile: dir + "/brightness", atomically: true, encoding: .utf8)
  report.line("cursor: backlight before: \(saved.isEmpty ? "unreadable" : saved.map { "display \($0.0) \($0.1)" }.joined(separator: ", "))")
  for (id, _) in saved {
    let ok = setBrightness(0, of: id)
    spin(0.5)
    report.line("cursor: backlight of display \(id) set to 0: \(ok), reads \(brightness(of: id).map { String($0) } ?? "nothing")")
  }
  spin(8)
  restoreWhatIsLeft()
  again.forEach { $0.orderOut(nil) }
  ask("Pointer after the call that lets a program not in front hide it:", ["Not visible", "Visible", "Flickered"])
  ask(
    "The backlight was turned down to zero for 8 seconds:",
    ["Fully dark", "Dimmed but still lit", "No change", "There is no built-in screen"])
  try? FileManager.default.removeItem(atPath: dir + "/brightness")
}

// MARK: - The guard

func describe(_ status: SMAppService.Status) -> String {
  switch status {
  case .notRegistered: return "not registered"
  case .enabled: return "enabled"
  case .requiresApproval: return "waiting for the person to approve it"
  case .notFound: return "not found"
  @unknown default: return "status \(status.rawValue)"
  }
}

/// What the agent wrote down last, as name and value.
func guardState() -> [String: String] {
  guard let text = try? String(contentsOfFile: dir + "/guard-heartbeat", encoding: .utf8) else {
    return [:]
  }
  var fields: [String: String] = [:]
  for line in text.split(separator: "\n") {
    let parts = line.split(separator: "=", maxSplits: 1)
    if parts.count == 2 { fields[String(parts[0])] = String(parts[1]) }
  }
  return fields
}

func guardLine(_ fields: [String: String]) -> String {
  fields.isEmpty
    ? "nothing"
    : fields.sorted { $0.key < $1.key }.map { "\($0.key)=\($0.value)" }.joined(separator: " ")
}

/// A datagram to the agent on the loopback, which the sandbox lets an app send without a file.
func sendDatagram(_ text: String) -> Bool {
  let descriptor = socket(AF_INET, SOCK_DGRAM, 0)
  guard descriptor >= 0 else { return false }
  defer { close(descriptor) }
  var address = sockaddr_in()
  address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
  address.sin_family = sa_family_t(AF_INET)
  address.sin_port = UInt16(47831).bigEndian
  address.sin_addr.s_addr = inet_addr("127.0.0.1")
  let bytes = Array(text.utf8)
  let sent = withUnsafePointer(to: &address) {
    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
      sendto(descriptor, bytes, bytes.count, 0, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
    }
  }
  return sent == bytes.count
}

func stageGuard() {
  phase("guard")
  report.line("== guard: an agent that the system starts for the person, sandboxed as the app is")
  panel.hide()
  let files = FileManager.default
  for file in ["guard-kill.txt", "lease", "private-flag", "lock-request", "lock-result"] {
    try? files.removeItem(atPath: dir + "/" + file)
  }
  let service = SMAppService.agent(plistName: "io.github.jayunewal.baton.probe.guard.plist")
  report.line("guard: status before: \(describe(service.status))")
  do {
    try service.register()
    report.line("guard: register() worked")
  } catch {
    report.line("guard: register() failed: \(error)")
  }
  var status = service.status
  report.line("guard: status after: \(describe(status))")
  if status == .notFound {
    report.line(
      "guard: the system does not know the agent. Run it again from another place: "
        + "BATON_PROBE_APPS=$HOME/Applications, then /Applications")
    return
  }
  if status == .requiresApproval {
    panel.show(
      "Guard check: turn on BatonProbe in System Settings, General, Login Items & Extensions, under "
        + "Allow in the Background. The check waits for 2 minutes.")
    SMAppService.openSystemSettingsLoginItems()
    for _ in 0..<60 {
      spin(2)
      status = service.status
      if status == .enabled { break }
    }
    panel.hide()
    report.line("guard: status after the wait: \(describe(status))")
  }
  guard status == .enabled else {
    try? service.unregister()
    return
  }

  var fields: [String: String] = [:]
  for _ in 0..<80 {
    spin(0.25)
    fields = guardState()
    if fields["pid"] != nil { break }
  }
  report.line("guard: the agent says: \(guardLine(fields))")
  if fields["tap"] != "true" {
    panel.show(
      "Guard check: if macOS asks to allow \"guard\" (Input Monitoring, Accessibility), allow it. The "
        + "check waits for up to a minute for the agent to be able to make a tap.")
    for _ in 0..<30 {
      spin(2)
      fields = guardState()
      if fields["tap"] == "true" { break }
    }
    panel.hide()
    report.line("guard: the agent can make a tap: \(fields["tap"] ?? "?"); \(guardLine(fields))")
  }

  // The lease: the app renews it for 6 seconds, in a file and in datagrams, and then lets it go.
  var sent = 0
  let renewing = Date(timeIntervalSinceNow: 6)
  while Date() < renewing {
    try? "\(sent)".write(toFile: dir + "/lease", atomically: true, encoding: .utf8)
    if sendDatagram("lease \(sent)") { sent += 1 }
    spin(0.5)
  }
  report.line("guard: the lease was renewed 12 times in 6 s, \(sent) datagrams could be sent")
  spin(8)
  fields = guardState()
  report.line("guard: 8 s after the last renewal: \(guardLine(fields))")

  // Killed, the agent should be started again by the system, and find the flag that a private
  // session leaves, which it takes for a crash.
  try? "private".write(toFile: dir + "/private-flag", atomically: true, encoding: .utf8)
  phase("guard-kill")
  var killed: String?
  for _ in 0..<240 {
    spin(0.25)
    killed = try? String(contentsOfFile: dir + "/guard-kill.txt", encoding: .utf8)
    if killed != nil { break }
  }
  report.line("guard: killed with signal 9: \(killed?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "run.sh did not say")")
  spin(2)
  report.line("guard: the new agent says: \(guardLine(guardState()))")
  try? files.removeItem(atPath: dir + "/private-flag")

  // The agent locks the Mac when it is asked to, each way in turn, as a guard would when the app is
  // gone: from the session of the person, with the consents of the agent and not of this app.
  for method in ["keys", "sac", "pmset"] {
    try? files.removeItem(atPath: dir + "/lock-result")
    panel.show(
      "Guard check, \(method): in 5 seconds the agent tries to lock the Mac. Unlock it when it "
        + "locks or its display goes dark; the check goes on once you are back. If nothing happens, "
        + "wait 10 seconds.")
    spin(5)
    panel.hide()
    try? method.write(toFile: dir + "/lock-request", atomically: true, encoding: .utf8)
    var answer: String?
    for _ in 0..<880 {
      spin(0.25)
      answer = try? String(contentsOfFile: dir + "/lock-result", encoding: .utf8)
      if answer != nil { break }
    }
    report.line(
      "guard: locked by the agent, \(answer?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "no answer from the agent")")
  }
  report.line("guard: the agent says: \(guardLine(guardState()))")

  do {
    try service.unregister()
    report.line("guard: unregistered")
  } catch {
    report.line("guard: unregister failed: \(error)")
  }
}

// MARK: - Locking the Mac from inside the sandbox

/// Whether the session says that the screen is locked. This is the key that Baton reads.
func sessionLocked() -> Bool {
  guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
  return (session["CGSSessionScreenIsLocked"] as? NSNumber)?.boolValue == true
}

typealias LockScreen = @convention(c) () -> Int32

/// Control, Command and Q, which is the shortcut of the system for locking the screen, posted as a
/// person's keys would be.
func lockByKeys() -> String {
  guard CGPreflightPostEventAccess() else { return "this app may not post events" }
  let source = CGEventSource(stateID: .hidSystemState)
  let steps: [(CGKeyCode, Bool, CGEventFlags)] = [
    (59, true, [.maskControl]),
    (55, true, [.maskControl, .maskCommand]),
    (12, true, [.maskControl, .maskCommand]),
    (12, false, [.maskControl, .maskCommand]),
    (55, false, [.maskControl]),
    (59, false, []),
  ]
  for (code, down, flags) in steps {
    let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)
    event?.flags = flags
    event?.post(tap: .cghidEventTap)
    spin(0.02)
  }
  return "posted"
}

func lockByCall() -> String {
  guard let handle = dlopen("/System/Library/PrivateFrameworks/login.framework/login", RTLD_NOW),
    let symbol = dlsym(handle, "SACLockScreenImmediate")
  else { return "the call is not there" }
  return "returned \(unsafeBitCast(symbol, to: LockScreen.self)())"
}

func lockByPmset() -> String {
  let process = Process()
  process.executableURL = URL(fileURLWithPath: "/usr/bin/pmset")
  process.arguments = ["displaysleepnow"]
  do { try process.run() } catch { return "could not be run: \(error)" }
  process.waitUntilExit()
  return "exited with \(process.terminationStatus)"
}

/// Waits for the person to unlock the Mac and wake the display, for up to three minutes.
func waitForThePerson() -> Double {
  let waited = Date()
  while Date().timeIntervalSince(waited) < 180 {
    if !sessionLocked() && CGDisplayIsAsleep(CGMainDisplayID()) == 0 { break }
    spin(0.5)
  }
  return Date().timeIntervalSince(waited)
}

func stageSandboxLock() {
  phase("sblock")
  report.line("== sblock: locking the Mac from inside the sandbox of this app, three ways")
  report.line("sblock: access: \(accessLine())")
  for method in ["keys", "sac", "pmset"] {
    panel.show(
      "Check sblock, \(method): in 5 seconds the Mac tries to lock itself. Unlock it when it locks "
        + "or its display goes dark; the check goes on once you are back. If nothing happens, "
        + "wait 10 seconds.")
    spin(5)
    panel.hide()
    let before = sessionLocked()
    let began = Date()
    let result: String
    switch method {
    case "keys": result = lockByKeys()
    case "sac": result = lockByCall()
    default: result = lockByPmset()
    }
    var after: Double?
    for _ in 0..<80 {
      if sessionLocked() {
        after = Date().timeIntervalSince(began)
        break
      }
      spin(0.1)
    }
    let asleep = CGDisplayIsAsleep(CGMainDisplayID()) != 0
    report.line(
      "sblock: \(method): locked before \(before), the call \(result), the session says locked "
        + "\(after.map { "after \(seconds($0)) s" } ?? "never within 8 s"), display asleep \(asleep)")
    if after != nil || asleep {
      report.line("sblock: \(method): the person was back after \(seconds(waitForThePerson())) s")
    }
  }
}

// MARK: - Black by the table of the display

func activeDisplays() -> [CGDirectDisplayID] {
  var ids = [CGDirectDisplayID](repeating: 0, count: 16)
  var count: UInt32 = 0
  CGGetActiveDisplayList(16, &ids, &count)
  return Array(ids.prefix(Int(count)))
}

/// Sets the output of every display to nought, which is black whatever a window shows. The system
/// puts it back when the program that set it ends.
func blackByTable() -> [String] {
  activeDisplays().map { id in "display \(id) \(CGSetDisplayTransferByFormula(id, 0, 0, 1, 0, 0, 1, 0, 0, 1).rawValue)" }
}

/// The highest output that the tables of the displays give now, from 0 to 1.
func tableMaximum() -> Float {
  var highest: Float = 0
  for id in activeDisplays() {
    var redMin: Float = 0, redMax: Float = 0, redGamma: Float = 0
    var greenMin: Float = 0, greenMax: Float = 0, greenGamma: Float = 0
    var blueMin: Float = 0, blueMax: Float = 0, blueGamma: Float = 0
    CGGetDisplayTransferByFormula(
      id, &redMin, &redMax, &redGamma, &greenMin, &greenMax, &greenGamma, &blueMin, &blueMax,
      &blueGamma)
    highest = max(highest, redMax, greenMax, blueMax)
  }
  return highest
}

func stageGamma() {
  phase("gamma")
  report.line("== gamma: the screens turned black by the table of each display, with no window")
  panel.show(
    "Check gamma. In 8 seconds every screen goes black for 20 seconds, by itself, and comes back by "
      + "itself. While it is black, one after the other: Command+Space (Spotlight), then Escape; "
      + "Control+Up Arrow (Mission Control), then Escape; the pointer to the top edge and to the "
      + "bottom edge. You cannot read this while it is black, so remember it.")
  spin(8)
  panel.hide()
  report.line("gamma: set the tables: \(blackByTable().joined(separator: ", "))")
  var highest: Float = 0
  var resets = 0
  let end = Date(timeIntervalSinceNow: 20)
  while Date() < end {
    spin(1)
    let now = tableMaximum()
    highest = max(highest, now)
    if now > 0.01 {
      resets += 1
      _ = blackByTable()
    }
  }
  CGDisplayRestoreColorSyncSettings()
  report.line("gamma: the highest output read back was \(highest), put back \(resets) times, restored")
  ask(
    "While the screens were black by the table of the display:",
    ["Fully black the whole time", "Black but something showed", "Only dimmed", "Black for a moment only"])
  ask(
    "Spotlight (Command+Space) and Mission Control (Control+Up Arrow) in that time:",
    ["Neither showed", "Spotlight showed", "Mission Control showed", "Both showed", "Neither opened"])
}

// MARK: - What the capture shows

/// An answer that arrives on a thread of the system, for the run loop to wait for.
final class Slot<T> {
  private let lock = NSLock()
  private var stored: T?
  private var finished = false

  func put(_ value: T?) {
    lock.lock()
    stored = value
    finished = true
    lock.unlock()
  }

  var done: Bool {
    lock.lock()
    defer { lock.unlock() }
    return finished
  }

  var value: T? {
    lock.lock()
    defer { lock.unlock() }
    return stored
  }
}

func received<T>(_ slot: Slot<T>, seconds time: Double = 10) -> T? {
  let end = Date(timeIntervalSinceNow: time)
  while !slot.done && Date() < end { spin(0.05) }
  return slot.value
}

func shareableContent(onScreenOnly: Bool = true) -> SCShareableContent? {
  let slot = Slot<SCShareableContent>()
  SCShareableContent.getExcludingDesktopWindows(true, onScreenWindowsOnly: onScreenOnly) { content, _ in
    slot.put(content)
  }
  return received(slot)
}

func screenshot(_ filter: SCContentFilter, cursor: Bool) -> CGImage? {
  let configuration = SCStreamConfiguration()
  let scale = Double(filter.pointPixelScale)
  configuration.width = max(1, Int((Double(filter.contentRect.width) * scale).rounded()))
  configuration.height = max(1, Int((Double(filter.contentRect.height) * scale).rounded()))
  configuration.showsCursor = cursor
  let slot = Slot<CGImage>()
  SCScreenshotManager.captureImage(contentFilter: filter, configuration: configuration) { image, _ in
    slot.put(image)
  }
  return received(slot)
}

/// The picture, or a part of it, as `size` by `size` grey values.
func grey(_ image: CGImage, crop: CGRect?, size: Int) -> [UInt8]? {
  var pixels = [UInt8](repeating: 0, count: size * size)
  var drawn = image
  if let crop {
    let inside = crop.intersection(CGRect(x: 0, y: 0, width: image.width, height: image.height))
    guard !inside.isNull, let part = image.cropping(to: inside) else { return nil }
    drawn = part
  }
  let made = pixels.withUnsafeMutableBytes { buffer -> Bool in
    guard
      let context = CGContext(
        data: buffer.baseAddress, width: size, height: size, bitsPerComponent: 8, bytesPerRow: size,
        space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue)
    else { return false }
    context.interpolationQuality = .medium
    context.draw(drawn, in: CGRect(x: 0, y: 0, width: size, height: size))
    return true
  }
  return made ? pixels : nil
}

/// How light the picture is, from 0 (black) to 255.
func lightness(_ image: CGImage?) -> String {
  guard let image, let pixels = grey(image, crop: nil, size: 16) else { return "no picture" }
  return String(format: "%.1f", Double(pixels.reduce(0) { $0 + Int($1) }) / Double(pixels.count))
}

/// How far two pictures differ in the square round a place, in grey values from 0 to 255.
func difference(_ first: CGImage?, _ second: CGImage?, around place: CGPoint) -> String {
  let square = CGRect(x: place.x - 16, y: place.y - 16, width: 96, height: 96)
  guard let first, let second, let one = grey(first, crop: square, size: 48),
    let two = grey(second, crop: square, size: 48)
  else { return "no picture" }
  var total = 0
  for index in one.indices { total += abs(Int(one[index]) - Int(two[index])) }
  return String(format: "%.2f", Double(total) / Double(one.count))
}

func stageCapture() {
  phase("capture")
  report.line("== capture: what the capture of the screen shows while the covers are up")
  guard CGPreflightScreenCaptureAccess() else {
    _ = CGRequestScreenCaptureAccess()
    report.line(
      "capture: this app may not record the screen yet. Turn on BatonProbe under Privacy & "
        + "Security, Screen & System Audio Recording, open the checks again, and run: "
        + "sh run.sh capture")
    return
  }
  guard let content = shareableContent(),
    let display = content.displays.first(where: { $0.displayID == CGMainDisplayID() })
  else {
    report.line("capture: the system did not list the screens")
    return
  }
  panel.show("Check capture: leave the pointer and the keys alone for about 40 seconds.")
  spin(2)
  panel.hide()
  let everything = SCContentFilter(display: display, excludingWindows: [])
  let scale = Double(everything.pointPixelScale)
  let base = screenshot(everything, cursor: false)
  report.line("capture: no cover, lightness of the picture: \(lightness(base)) (0 is black, 255 white)")

  let covers = NSScreen.screens.map { makeCover(on: $0) }
  spin(1.5)
  let after = shareableContent()
  var me = after?.applications.first(where: { $0.processID == getpid() })
  if me == nil {
    // The list of what is on screen may leave out a program whose windows sit this high.
    me = shareableContent(onScreenOnly: false)?.applications.first(where: { $0.processID == getpid() })
    report.line("capture: this app was not in the list of what is on screen; in the full list: \(me != nil)")
  }
  let ours = Set(covers.map { CGWindowID($0.windowNumber) })
  let theirs = after?.windows.filter { ours.contains($0.windowID) } ?? []
  report.line(
    "capture: its covers are in the list of windows: \(theirs.count) of \(covers.count)")
  report.line(
    "capture: cover up, nothing left out: \(lightness(screenshot(everything, cursor: false)))")
  var without: SCContentFilter?
  if let me {
    let filter = SCContentFilter(display: display, excludingApplications: [me], exceptingWindows: [])
    without = filter
    report.line(
      "capture: cover up, this application left out: \(lightness(screenshot(filter, cursor: false)))")
  }
  if !theirs.isEmpty {
    let filter = SCContentFilter(display: display, excludingWindows: theirs)
    report.line("capture: cover up, its windows left out: \(lightness(screenshot(filter, cursor: false)))")
  }

  // The tables of the displays black too: does the capture see what the tables hide?
  if let without {
    _ = blackByTable()
    spin(1)
    report.line(
      "capture: cover up and the tables black, application left out: "
        + "\(lightness(screenshot(without, cursor: false)))")
    CGDisplayRestoreColorSyncSettings()
    spin(0.5)
  }

  // The pointer: is it in the capture when the pointer of the Mac is hidden?
  if let without {
    let bounds = CGDisplayBounds(CGMainDisplayID())
    let middle = CGPoint(x: bounds.midX, y: bounds.midY)
    CGWarpMouseCursorPosition(middle)
    spin(0.6)
    let place = CGPoint(x: middle.x * CGFloat(scale), y: middle.y * CGFloat(scale))
    let plain = screenshot(without, cursor: false)
    let shown = screenshot(without, cursor: true)
    report.line(
      "capture: pointer showing: the capture with the pointer differs from the one without near the "
        + "pointer by \(difference(shown, plain, around: place)) (above about 2 means it is drawn)")
    _ = CGDisplayHideCursor(CGMainDisplayID())
    spin(0.6)
    let hidden = screenshot(without, cursor: true)
    report.line(
      "capture: pointer hidden with CGDisplayHideCursor: the capture with the pointer differs from the "
        + "one without by \(difference(hidden, plain, around: place))")
    _ = CGDisplayShowCursor(CGMainDisplayID())
  }
  covers.forEach { $0.orderOut(nil) }
}

// MARK: - The helper that has the keyboard

/// A second instance of this app with one field in a window. It is another process on purpose: the
/// system turns off the keys of an event tap for other processes while a field for a password has
/// the keyboard, and it may not do so for the process that turned it on.
func runField(kind: String) -> Never {
  let app = NSApplication.shared
  app.setActivationPolicy(.regular)
  let window = NSWindow(
    contentRect: NSRect(x: 0, y: 0, width: 560, height: 170), styleMask: [.titled],
    backing: .buffered, defer: false)
  window.title = kind == "secure" ? "Baton check: password field" : "Baton check: text field"
  window.level = .floating
  window.isReleasedWhenClosed = false
  let field: NSTextField =
    kind == "secure"
    ? NSSecureTextField(frame: NSRect(x: 20, y: 90, width: 520, height: 28))
    : NSTextField(frame: NSRect(x: 20, y: 90, width: 520, height: 28))
  let note = NSTextField(labelWithString: "Type letters here while the check runs. Not the Command key.")
  note.frame = NSRect(x: 20, y: 40, width: 520, height: 24)
  window.contentView?.addSubview(field)
  window.contentView?.addSubview(note)
  window.center()
  window.makeKeyAndOrderFront(nil)
  app.activate(ignoringOtherApps: true)
  DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
    window.makeFirstResponder(field)
    try? "ready".write(toFile: dir + "/field-ready", atomically: true, encoding: .utf8)
  }
  let started = Date()
  Timer.scheduledTimer(withTimeInterval: 0.4, repeats: true) { _ in
    let stopped = FileManager.default.fileExists(atPath: dir + "/field-stop")
    guard stopped || Date().timeIntervalSince(started) > 120 else { return }
    // Takes the text out of the field editor, so that its length is the number of keys that
    // reached the field.
    window.makeFirstResponder(nil)
    let result = "characters typed into the field: \(field.stringValue.count); secure input on: \(IsSecureEventInputEnabled()); app in front: \(app.isActive)"
    try? result.write(toFile: dir + "/field-result", atomically: true, encoding: .utf8)
    exit(0)
  }
  app.run()
  exit(0)
}

// MARK: - The stages

signal(SIGTERM) { _ in
  restoreWhatIsLeft()
  exit(1)
}
signal(SIGINT) { _ in
  restoreWhatIsLeft()
  exit(1)
}

let arguments = CommandLine.arguments
if arguments.count > 1, arguments[1] == "restore" {
  restoreFromFile()
  exit(0)
}
if arguments.count > 1, arguments[1] == "field" {
  runField(kind: arguments.count > 2 ? arguments[2] : "plain")
}

let application = NSApplication.shared
application.setActivationPolicy(.accessory)

let wanted =
  ((try? String(contentsOfFile: dir + "/stages", encoding: .utf8)) ?? "tap")
  .split(whereSeparator: { $0 == " " || $0 == "\n" }).map(String.init)

DispatchQueue.main.async {
  if report.isEmpty { describeTheMac() }
  report.line("stages: \(wanted.joined(separator: " "))")
  for stage in wanted {
    switch stage {
    case "tap":
      stageField(
        "tap", kind: "plain",
        intro: "Check 1: the input block.\n\nFirst 8 seconds with your hands off the keys, the pointer and the trackpad. Then 10 seconds in which nothing of what you do at this Mac should have an effect: type in the field that is open, click, swipe.")
    case "secure":
      stageField(
        "secure", kind: "secure",
        intro: "Check 2: the same, with a password field open that has the keyboard.\n\nFirst 8 seconds with your hands off. Then 10 seconds in which you type letters in the password field, move the pointer and swipe.")
    case "shield": stageShield()
    case "cursor": stageCursor()
    case "guard": stageGuard()
    case "gamma": stageGamma()
    case "capture": stageCapture()
    case "sblock": stageSandboxLock()
    default: report.line("no stage called \(stage)")
    }
  }
  report.line("done")
  phase("done")
  NSApp.terminate(nil)
}
application.run()
