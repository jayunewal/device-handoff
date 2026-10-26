// SPDX-License-Identifier: Apache-2.0

// The ways to lock a Mac, and the question whether it is locked, which the private session
// (BatonPrivacyMac.swift) and the guard (apps/baton/macos/Guard) both need: the guard locks the Mac
// when the app is gone, and it has to do it the way the app does, with the same three ways in the
// same order. This file talks to the system, so it is built on a Mac and nowhere else, and what it
// does there is checked on one. It imports no more than Foundation and CoreGraphics, since the
// guard is a small program that has no use for the rest.
//
// What is typed at a Mac is a secret. Nothing here logs a key, only that a way was tried.

#if os(macOS)

import CoreGraphics
import Foundation

/// Whether the session says that the screen is locked, or that another user switched the session
/// away from the console. A session that cannot be read is not said to be locked.
func macSessionLocked() -> Bool {
  guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
  // The key for the lock screen is not documented, and has been there as long as the lock screen
  // has.
  if (session["CGSSessionScreenIsLocked"] as? NSNumber)?.boolValue == true { return true }
  return (session[kCGSessionOnConsoleKey as String] as? NSNumber)?.boolValue == false
}

/// Whether the system says anything about the session of this process. When it does not, the Mac is
/// never found to be locked, whatever is tried.
func macSessionReadable() -> Bool {
  CGSessionCopyCurrentDictionary() != nil
}

/// The three ways to lock this Mac, tried in turn: the call of the system, the shortcut posted as
/// keys, and the sleep of the display.
final class MacLock: MacLocking {
  private let noteKeyDown: () -> Void
  private let log: (String) -> Void

  /// `noteKeyDown` is called before each key that the shortcut posts goes to the system, so that a
  /// private session does not take a key of its own for the key of somebody at the Mac. `log` is
  /// told what keeps the Mac from being found locked, and never what is typed.
  init(noteKeyDown: @escaping () -> Void = {}, log: @escaping (String) -> Void = { _ in }) {
    self.noteKeyDown = noteKeyDown
    self.log = log
  }

  var locked: Bool { macSessionLocked() }

  func lock(attempt: Int) {
    // A session that cannot be read is never found to be locked, and the ways then go round for as
    // long as the guard runs: the first try says so, where the log of the guard shows it.
    if attempt == 0 && !macSessionReadable() {
      log("the session of the Mac cannot be read, so it cannot be told when the Mac is locked")
    }
    // The call of the system first, which locks at once and does not depend on a shortcut that the
    // person may have changed; then the shortcut; then the sleep of the display, which locks a Mac
    // that asks for its password at once.
    switch attempt % 3 {
    case 0: lockByCall()
    case 1: lockByKeys()
    default: lockByDisplaySleep()
    }
  }

  private typealias LockScreenCall = @convention(c) () -> Int32

  /// The call of the system that locks the screen, which is not in a header: it is looked for, and
  /// a Mac that does not have it is locked another way.
  private func lockByCall() {
    guard let handle = dlopen("/System/Library/PrivateFrameworks/login.framework/login", RTLD_NOW),
      let symbol = dlsym(handle, "SACLockScreenImmediate")
    else { return }
    _ = unsafeBitCast(symbol, to: LockScreenCall.self)()
  }

  /// Control, Command and Q, which is the shortcut of the system for locking the screen, posted as
  /// a person's keys would be. They pass the hold, since this app posted them.
  private func lockByKeys() {
    guard CGPreflightPostEventAccess(), let source = CGEventSource(stateID: .hidSystemState) else {
      return
    }
    let steps: [(CGKeyCode, Bool, CGEventFlags)] = [
      (59, true, [.maskControl]),
      (55, true, [.maskControl, .maskCommand]),
      (12, true, [.maskControl, .maskCommand]),
      (12, false, [.maskControl, .maskCommand]),
      (55, false, [.maskControl]),
      (59, false, []),
    ]
    for (code, down, flags) in steps {
      if down { noteKeyDown() }
      let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down)
      event?.flags = flags
      event?.post(tap: .cghidEventTap)
      Thread.sleep(forTimeInterval: 0.02)
    }
  }

  private func lockByDisplaySleep() {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/pmset")
    process.arguments = ["displaysleepnow"]
    do { try process.run() } catch { return }
    process.waitUntilExit()
  }
}

#endif
