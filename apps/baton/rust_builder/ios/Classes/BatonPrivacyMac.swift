// SPDX-License-Identifier: Apache-2.0

// The Mac side of the privacy mode (docs/adr/0011-privacy-mode.md): what BatonPrivacy.swift asks of
// the Mac, done with the calls of the system. Every screen is turned black by its table, which
// nothing shows over, not Spotlight and not Mission Control, and which the capture does not see.
// The keys and the pointer of the Mac are held by a tap that drops every event that this app did
// not post. When the session is over the Mac is locked, with the first of three ways that does it
// (BatonLock.swift), and only then do the screens and the keys come back. If the app goes away
// before that, the guard (BatonGuard.swift) locks the Mac.
//
// The app is sandboxed, and what is done here was found to work inside the sandbox, on a Mac, by
// the checks in docs/research/privacy-mode. This file talks to the system, so it is built on a Mac
// and nowhere else, and what it does there is checked on one.
//
// What the person at the other device types is a secret, and so is what the people at the Mac do.
// Nothing here logs a key or a place, only that something happened.

#if os(macOS)

import AppKit
import CoreGraphics
import Foundation
import os

/// The log of the system. The lines are public so that `log show` has them whole.
private let privacyLog = Logger(subsystem: "io.github.jayunewal.baton", category: "privacy")

// MARK: - The consent

/// Where System Settings lists the apps that may receive the keys of the Mac, which is where the
/// consent to hold them is given.
private let inputMonitoringSettingsAddress =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"

/// Where System Settings lists the apps that may control this computer.
private let accessibilitySettingsAddress =
  "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"

/// Whether the system lets this app hold the keyboard and the pointer. The answer is the system's:
/// the tap is made, and let go of at once, and a list of consents that says yes is no promise that
/// the tap can be made.
func macCanHoldTheInput() -> Bool {
  guard
    let port = CGEvent.tapCreate(
      tap: .cghidEventTap, place: .headInsertEventTap, options: .defaultTap,
      eventsOfInterest: heldEventMask, callback: holdCallback, userInfo: nil)
  else { return false }
  // Never serviced, so it is switched off before it is let go of, and holds nothing up.
  CGEvent.tapEnable(tap: port, enable: false)
  CFMachPortInvalidate(port)
  return true
}

/// Asks the system to let the app hold the keyboard and the pointer. It shows its own questions
/// once, and does not wait for the answers: the person gives them in System Settings, and they
/// take effect when the app is opened again. The lists in System Settings are hard to find by
/// hand, so the one that lacks the app is opened for the person.
func requestMacInputHoldAccess() {
  DispatchQueue.global(qos: .userInitiated).async {
    let listening = CGRequestListenEventAccess()
    let posting = CGRequestPostEventAccess()
    guard !(listening && posting) else { return }
    let address = posting ? inputMonitoringSettingsAddress : accessibilitySettingsAddress
    guard let settings = URL(string: address) else { return }
    DispatchQueue.main.async { _ = NSWorkspace.shared.open(settings) }
  }
}

// MARK: - The hold on the keys and the pointer

/// The kinds of event that the hold takes: the keys, the pointer, the wheel, a tablet, the gestures
/// of a trackpad, and the keys of the system (volume, brightness, media). The system numbers the
/// kinds after the first group, and has no names for them in Swift.
private let heldEventTypes: [UInt32] = [
  CGEventType.keyDown.rawValue, CGEventType.keyUp.rawValue, CGEventType.flagsChanged.rawValue,
  CGEventType.leftMouseDown.rawValue, CGEventType.leftMouseUp.rawValue,
  CGEventType.rightMouseDown.rawValue, CGEventType.rightMouseUp.rawValue,
  CGEventType.otherMouseDown.rawValue, CGEventType.otherMouseUp.rawValue,
  CGEventType.mouseMoved.rawValue, CGEventType.leftMouseDragged.rawValue,
  CGEventType.rightMouseDragged.rawValue, CGEventType.otherMouseDragged.rawValue,
  CGEventType.scrollWheel.rawValue, CGEventType.tabletPointer.rawValue,
  CGEventType.tabletProximity.rawValue,
  // System defined: the keys for volume, brightness and media.
  14,
  // Rotate, the start and the end of a gesture, a gesture, magnify, swipe, smart magnify, quick
  // look, pressure and direct touch.
  18, 19, 20, 29, 30, 31, 32, 33, 34, 37,
]

private let heldEventMask: CGEventMask = {
  var mask: CGEventMask = 0
  for type in heldEventTypes { mask |= CGEventMask(1) << CGEventMask(type) }
  return mask
}()

/// The key whose three presses end the session, which is Escape.
private let escapeKeyCode: Int64 = 53

private func holdCallback(
  proxy: CGEventTapProxy, type: CGEventType, event: CGEvent, context: UnsafeMutableRawPointer?
) -> Unmanaged<CGEvent>? {
  // Without its hold there is nothing to ask, and what is not known to be this app's is dropped.
  guard let context else { return nil }
  return Unmanaged<InputHold>.fromOpaque(context).takeUnretainedValue().handle(type, event)
}

/// An active tap at the lowest level that the system allows, on a thread of its own, that drops
/// every event of the keys and the pointer except those that this app posted. The window server
/// names the process that posted an event, and it is zero for the keys and the pointer of the Mac.
final class InputHold {
  private let alarm: (PrivacyAlarm) -> Void
  private let lock = NSLock()
  private var port: CFMachPort?
  private var started = false
  private var stopping = false
  /// Whether the keys and the pointer of the Mac pass: the person at the other device let them be
  /// used at the Mac. The tap stays on, so that holding them again is this and nothing else.
  private var passing = false
  /// Used by the thread of the tap only.
  private var hatch = EscapeHatch()
  private let ready = DispatchSemaphore(value: 0)
  private let finished = DispatchSemaphore(value: 0)

  /// `alarm` is called from the thread of the tap, and does not wait.
  init(alarm: @escaping (PrivacyAlarm) -> Void) {
    self.alarm = alarm
  }

  /// Makes the tap and returns when it holds. Throws if the system will not make it.
  func start() throws {
    let thread = Thread { [self] in run() }
    thread.name = "Baton input hold"
    thread.qualityOfService = .userInteractive
    thread.start()
    ready.wait()
    lock.lock()
    let made = port != nil
    started = made
    lock.unlock()
    guard made else {
      // The thread has nothing to run and ends.
      finished.wait()
      throw SharerFailure(
        sentence: "This Mac did not let Baton keep its keyboard and pointer out of use.")
    }
  }

  /// Lets go of the keys and the pointer, and returns when no event is being judged any more. It
  /// may be called when nothing is held.
  func stop() {
    lock.lock()
    guard started, !stopping else {
      lock.unlock()
      return
    }
    stopping = true
    lock.unlock()
    finished.wait()
  }

  /// Lets the keys and the pointer of the Mac through while `passing`, or holds them again. The tap
  /// is on all the while, and a hold that is lost is still found. Three presses of Escape are no
  /// way out while the keys pass, since the person at the Mac is allowed to use them. Whatever was
  /// pressed or clicked when the hold begins again may stay as it is until it is next pressed or
  /// clicked: the release that the hold drops is not seen by the system.
  func pass(_ passing: Bool) {
    lock.lock()
    self.passing = passing
    lock.unlock()
  }

  /// Whether the tap is on. A tap that the system switched off is switched on once more.
  var holding: Bool {
    lock.lock()
    let port = self.port
    lock.unlock()
    guard let port else { return false }
    if CGEvent.tapIsEnabled(tap: port) { return true }
    CGEvent.tapEnable(tap: port, enable: true)
    return CGEvent.tapIsEnabled(tap: port)
  }

  private func run() {
    let context = Unmanaged.passUnretained(self).toOpaque()
    guard
      let made = CGEvent.tapCreate(
        tap: .cghidEventTap, place: .headInsertEventTap, options: .defaultTap,
        eventsOfInterest: heldEventMask, callback: holdCallback, userInfo: context),
      let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, made, 0)
    else {
      ready.signal()
      finished.signal()
      return
    }
    CFRunLoopAddSource(CFRunLoopGetCurrent(), source, .commonModes)
    lock.lock()
    port = made
    lock.unlock()
    CGEvent.tapEnable(tap: made, enable: true)
    ready.signal()
    // The loop runs in short turns, so that a stop that comes before the loop runs is not lost.
    while true {
      lock.lock()
      let over = stopping
      lock.unlock()
      if over { break }
      // A loop with nothing to serve returns at once, as one does when the system took the tap
      // away, and must not be spun on: `holding` tells the session that the hold is gone.
      if CFRunLoopRunInMode(.defaultMode, 0.1, false) == .finished {
        Thread.sleep(forTimeInterval: 0.05)
      }
    }
    CGEvent.tapEnable(tap: made, enable: false)
    CFRunLoopRemoveSource(CFRunLoopGetCurrent(), source, .commonModes)
    CFMachPortInvalidate(made)
    lock.lock()
    port = nil
    lock.unlock()
    finished.signal()
  }

  /// Judges one event, on the thread of the tap: nil drops it.
  fileprivate func handle(_ type: CGEventType, _ event: CGEvent) -> Unmanaged<CGEvent>? {
    if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
      // The system switched the tap off, and everything passes until it is on again.
      lock.lock()
      let port = self.port
      lock.unlock()
      if let port { CGEvent.tapEnable(tap: port, enable: true) }
      return nil
    }
    lock.lock()
    let letThrough = passing
    lock.unlock()
    if letThrough {
      // Presses of Escape that were counted before do not count towards the ones after.
      hatch = EscapeHatch()
      return Unmanaged.passUnretained(event)
    }
    if event.getIntegerValueField(.eventSourceUnixProcessID) == Int64(getpid()) {
      return Unmanaged.passUnretained(event)
    }
    if type == .keyDown, event.getIntegerValueField(.keyboardEventKeycode) == escapeKeyCode {
      let repeating = event.getIntegerValueField(.keyboardEventAutorepeat) != 0
      if hatch.pressed(at: ProcessInfo.processInfo.systemUptime, repeating: repeating) {
        alarm(.escape)
      }
    }
    return nil
  }
}

// MARK: - The cover

/// Every screen turned black by its own table, which is black whatever a window shows, and which
/// nothing shows over. The capture does not see the table, so the other device gets the screen as
/// it is.
final class ScreenCover {
  /// Sets every display that shows something to black, and says whether every one is. A display
  /// that is asleep shows nothing and is left alone; it is covered when it wakes, since this is
  /// called again and again. One that is online and not drawing, but not asleep, is part of a
  /// mirror and shows what the main one does.
  func apply() -> Bool {
    guard let displays = ScreenCover.online() else { return false }
    var dark = true
    for display in displays {
      let shows = CGDisplayIsActive(display) != 0 || CGDisplayIsAsleep(display) == 0
      guard shows else { continue }
      let set = CGSetDisplayTransferByFormula(display, 0, 0, 1, 0, 0, 1, 0, 0, 1)
      if set != .success {
        dark = false
        continue
      }
      // The read can fail, and then the set that worked is all there is to go by.
      if let highest = ScreenCover.highestOutput(of: display), highest > 0.001 { dark = false }
    }
    return dark
  }

  /// Gives the screens back what the person set for them.
  func restore() {
    CGDisplayRestoreColorSyncSettings()
  }

  private static func online() -> [CGDirectDisplayID]? {
    var ids = [CGDirectDisplayID](repeating: 0, count: 32)
    var count: UInt32 = 0
    guard CGGetOnlineDisplayList(UInt32(ids.count), &ids, &count) == .success else { return nil }
    return Array(ids.prefix(Int(count)))
  }

  /// The highest output of the table of a display, from 0 to 1, or nil if the system does not say.
  private static func highestOutput(of display: CGDirectDisplayID) -> Float? {
    var redMin: Float = 0
    var redMax: Float = 0
    var redGamma: Float = 0
    var greenMin: Float = 0
    var greenMax: Float = 0
    var greenGamma: Float = 0
    var blueMin: Float = 0
    var blueMax: Float = 0
    var blueGamma: Float = 0
    guard
      CGGetDisplayTransferByFormula(
        display, &redMin, &redMax, &redGamma, &greenMin, &greenMax, &greenGamma, &blueMin,
        &blueMax, &blueGamma) == .success
    else { return nil }
    return max(redMax, greenMax, blueMax)
  }
}

// MARK: - The curtain

/// What a private session uses of this Mac: the hold on the keys and the pointer, the cover, the
/// screens kept awake, and the ways to lock the Mac.
final class MacCurtain: Curtain {
  /// How often the hold, the cover and the keys of the Mac are looked at.
  private static let watchInterval: TimeInterval = 0.25
  /// How many looks in a row may find a screen that is not dark before the session is ended.
  private static let darknessMisses = 2
  /// How long the looks may stop before the curtain is no longer said to be well: four looks.
  private static let lookGrace: TimeInterval = 1

  private let ledger: PostLedger
  private let mac: MacLock
  private let cover = ScreenCover()
  private let queue = DispatchQueue(label: "io.github.jayunewal.baton.privacy")
  private let mutex = NSLock()
  private var alarm: ((PrivacyAlarm) -> Void)?
  private var told = false
  private var hold: InputHold?
  private var covered = false
  private var activity: NSObjectProtocol?
  private var watcher: DispatchSourceTimer?
  private var quitWatch: QuitWatch?
  private var heldSince: TimeInterval = 0
  /// What the person at the other device lifted: the screens are let show what is on them, and the
  /// keys and the pointer of the Mac are let through. A lifted part is not looked at.
  private var screenLifted = false
  private var inputLifted = false
  /// When the last look was made, or the cover went up.
  private var lastLook: TimeInterval = 0
  /// Used by the queue only.
  private var misses = 0

  /// What happens when the app is about to quit while the cover is up. It is set once, before
  /// `raise`: the quit gives the screens back as its process ends, so it has to be the end of the
  /// session, with the Mac locked first.
  var whenTheAppQuits: (() -> Void)?

  /// `ledger` is told of the keys that this curtain posts to lock the Mac, so that they are not
  /// taken for the keys of somebody at the Mac.
  init(ledger: PostLedger) {
    self.ledger = ledger
    mac = MacLock(noteKeyDown: { ledger.notedKeyDown() })
  }

  deinit {
    lower()
  }

  func raise(alarm: @escaping (PrivacyAlarm) -> Void) throws {
    mutex.lock()
    let first = self.alarm == nil
    if first { self.alarm = alarm }
    mutex.unlock()
    guard first else {
      throw SharerFailure(sentence: "This Mac could not keep its screen private.")
    }
    // The person at the other device can quit Baton by mistake (Command-Q, its menu, its window),
    // and the Mac must not be left open by that: a quit is heard, unlike a crash.
    let quits = QuitWatch(name: NSApplication.willTerminateNotification) { [weak self] in
      self?.whenTheAppQuits?()
    }
    mutex.lock()
    quitWatch = quits
    mutex.unlock()
    // The hold comes first, so that nobody can type or click in the moment that the screens are
    // dark.
    let made = InputHold { [weak self] alarm in self?.tell(alarm) }
    try made.start()
    mutex.lock()
    hold = made
    heldSince = ProcessInfo.processInfo.systemUptime
    covered = true
    mutex.unlock()
    var dark = false
    for _ in 0..<3 where !dark {
      dark = cover.apply()
      if !dark { Thread.sleep(forTimeInterval: 0.05) }
    }
    guard dark else {
      throw SharerFailure(sentence: "This Mac could not make its screens dark.")
    }
    // The screens stay awake: a screen that sleeps is not captured, and the session would end. The
    // activity is one that the person started, which the system does not slow down (App Nap), since
    // an app with nothing on show that is slowed down would not look at the cover in time, and
    // would not tell its guard that it is well.
    let kept = ProcessInfo.processInfo.beginActivity(
      options: [.userInitiated, .idleDisplaySleepDisabled, .idleSystemSleepDisabled],
      reason: "Baton keeps this Mac awake while another device uses it.")
    mutex.lock()
    activity = kept
    lastLook = ProcessInfo.processInfo.systemUptime
    mutex.unlock()
    watch()
    privacyLog.notice("Baton: the screens are dark and the keys and the pointer are held.")
  }

  func lower() {
    mutex.lock()
    let timer = watcher
    watcher = nil
    let held = hold
    hold = nil
    let kept = activity
    activity = nil
    let quits = quitWatch
    quitWatch = nil
    let wasCovered = covered
    covered = false
    screenLifted = false
    inputLifted = false
    mutex.unlock()
    quits?.stop()
    timer?.cancel()
    // A look that is running is waited for, so that it does not put the cover back after it is
    // down.
    queue.sync {}
    if wasCovered { cover.restore() }
    held?.stop()
    if let kept { ProcessInfo.processInfo.endActivity(kept) }
    if wasCovered || held != nil {
      privacyLog.notice("Baton: the screens and the keys and the pointer are given back.")
    }
  }

  func lift(screen: Bool, input: Bool) throws {
    // On the queue of the looks, so that a look is never half way through when the hold or the
    // cover changes, and never puts back a cover that was just lifted.
    try queue.sync {
      mutex.lock()
      let held = hold
      let raised = covered
      let wasScreen = screenLifted
      let wasInput = inputLifted
      mutex.unlock()
      guard raised, let held else { return }
      // What is put back goes in the order of `raise`, the hold first, so that nobody can type or
      // click in the moment that the screens are dark. What is lifted goes after it.
      if wasInput && !input {
        // The keys that went down at the Mac while it was open are not the tripwire's to judge.
        mutex.lock()
        heldSince = ProcessInfo.processInfo.systemUptime
        inputLifted = false
        mutex.unlock()
        held.pass(false)
        privacyLog.notice("Baton: the keys and the pointer are held again.")
      }
      if wasScreen && !screen {
        mutex.lock()
        screenLifted = false
        mutex.unlock()
        misses = 0
        var dark = false
        for _ in 0..<3 where !dark {
          dark = cover.apply()
          if !dark { Thread.sleep(forTimeInterval: 0.05) }
        }
        guard dark else { throw PrivacyAlarm.coverLost }
        privacyLog.notice("Baton: the screens are dark again.")
      }
      if !wasScreen && screen {
        mutex.lock()
        screenLifted = true
        mutex.unlock()
        cover.restore()
        privacyLog.notice("Baton: the screens show what is on them.")
      }
      if !wasInput && input {
        mutex.lock()
        inputLifted = true
        mutex.unlock()
        held.pass(true)
        privacyLog.notice("Baton: the keys and the pointer can be used at the Mac.")
      }
    }
  }

  var locked: Bool { mac.locked }

  func lock(attempt: Int) {
    mac.lock(attempt: attempt)
  }

  /// Whether the cover and the hold are up and the looks at them go on. The guard hears that the
  /// session is well only while this is true.
  var healthy: Bool {
    mutex.lock()
    defer { mutex.unlock() }
    return covered && hold != nil
      && ProcessInfo.processInfo.systemUptime - lastLook < MacCurtain.lookGrace
  }

  // MARK: Looking

  private func watch() {
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(
      deadline: .now() + MacCurtain.watchInterval, repeating: MacCurtain.watchInterval,
      leeway: .milliseconds(20))
    timer.setEventHandler { [weak self] in self?.look() }
    mutex.lock()
    watcher = timer
    mutex.unlock()
    timer.resume()
  }

  /// Whether the hold and the cover are as they were put, and whether a key went down that the hold
  /// did not take. A part that the person at the other device lifted is not looked at, except that
  /// the tap must be there: the Mac is held again when the part is put back.
  private func look() {
    mutex.lock()
    let held = hold
    let since = heldSince
    let showing = screenLifted
    let usable = inputLifted
    mutex.unlock()
    guard let held else { return }
    defer {
      mutex.lock()
      lastLook = ProcessInfo.processInfo.systemUptime
      mutex.unlock()
    }
    if !held.holding { tell(.holdLost) }
    if showing {
      misses = 0
    } else if cover.apply() {
      misses = 0
    } else {
      misses += 1
      if misses >= MacCurtain.darknessMisses { tell(.coverLost) }
    }
    // Keys are what the person at the Mac is let press.
    if usable { return }
    // How long ago the system counted the last key press, which moves for a key that the tap was
    // blind to. It is negative if there never was one. It is the last key only: see `Tripwire`.
    let age = CGEventSource.secondsSinceLastEventType(.hidSystemState, eventType: .keyDown)
    if Tripwire.foreignKey(
      lastKeyAge: age >= 0 ? age : nil, sinceHold: ProcessInfo.processInfo.systemUptime - since,
      postAge: ledger.secondsSinceKeyDown)
    {
      tell(.foreignKey)
    }
  }

  /// The first thing that needs the session ended is told, and nothing after it: the session ends
  /// once, and the cover stays until the Mac is locked.
  private func tell(_ alarm: PrivacyAlarm) {
    mutex.lock()
    let call = told ? nil : self.alarm
    told = true
    mutex.unlock()
    guard let call else { return }
    let why = String(describing: alarm)
    privacyLog.error("Baton: the private session is ended because of \(why, privacy: .public).")
    call(alarm)
  }
}

/// The privacy of a session that another device controls, on this Mac. `ledger` is the one that
/// the poster of the events uses, so that the keys the other device sends are not taken for the
/// keys of somebody at the Mac. `end` ends the session with a sentence for the person there.
func makePrivateSession(ledger: PostLedger, end: @escaping (String) -> Void) -> PrivateSession {
  let curtain = MacCurtain(ledger: ledger)
  // The guard is inside the app, next to its own program. A build without it has no private
  // session.
  let guardian = GuardLink(executable: Bundle.main.url(forAuxiliaryExecutable: "BatonGuard"))
  let session = PrivateSession(
    curtain: curtain, guardian: guardian, end: end,
    log: { privacyLog.notice("Baton: \($0, privacy: .public)") })
  // A quit of the app is an end of the session like any other: `finish` returns when the Mac is
  // locked and the cover is down, and only then does the quit go on and the process end.
  curtain.whenTheAppQuits = { [weak session] in session?.finish(.sessionOver) }
  return session
}

#endif
