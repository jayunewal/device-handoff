// SPDX-License-Identifier: Apache-2.0

// The Mac side of staying awake while plugged in (docs/adr/0018-stay-awake-while-plugged-in.md):
// what BatonStayAwake.swift asks of the Mac, done with the calls of the system. The power is what
// the system says of where the Mac's power comes from, the hold is an assertion of the power
// management of the system, and a change of power is told by the system to the main run loop.
//
// This file talks to the system, so it is built on a Mac and nowhere else, and what it does there is
// checked on one: StayAwakeMacTests.swift looks for the hold in the assertions that the system
// lists, as `pmset -g assertions` does. No test checks the read of the power in the sandbox, since
// a read that fails answers the wall (see `macIsPluggedIn`): only unplugging a laptop shows it,
// which is S2 of docs/adr/0018-stay-awake-while-plugged-in.md. What it decides is in
// BatonStayAwake.swift.

#if os(macOS)

import Foundation
import IOKit.ps
import IOKit.pwr_mgt
import os

/// The log of the system. The lines are public so that `log show` has them whole.
private let stayLog = Logger(subsystem: "io.github.jayunewal.baton", category: "stay-awake")

/// What Baton tells the system about why it holds the Mac awake. `pmset -g assertions` and the
/// battery menu of the Mac show it, so it says what it does and for whom.
let stayAwakeReason =
  "Baton keeps this Mac awake while it is plugged in, so that your other devices can reach it."

/// Whether the Mac runs on the power of the wall, as the system says. A Mac with no battery always
/// does, and so does a laptop that is plugged in, charging or not. The call that says it,
/// `IOPSGetProvidingPowerSourceType`, reads in Apple's published source as answering the wall when
/// it cannot read the power at all, and `IOPSCopyPowerSourcesInfo` as giving an empty list and not
/// nothing. So this is nil only if the system hands back nothing, which is not known to happen, and
/// a read that fails looks like the wall: only a laptop that is unplugged shows that it did (S2 of
/// docs/adr/0018-stay-awake-while-plugged-in.md). Nil is not the same as a Mac on its battery.
func macIsPluggedIn() -> Bool? {
  guard let snapshot = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
    let source = IOPSGetProvidingPowerSourceType(snapshot)?.takeUnretainedValue()
  else {
    stayLog.error("Baton: the system did not say where the power of this Mac comes from.")
    return nil
  }
  return (source as String) == kIOPSACPowerValue
}

/// Holds the Mac awake: an assertion against the sleep that the system starts when nobody has used
/// the Mac for a while. The screen is not held, so it goes dark as the person set it, and the
/// system still puts a laptop to sleep when its lid is closed or its battery is low, whatever is
/// held. Gives the way to let go, or nothing where the system refused.
func holdMacAwake() -> (() -> Void)? {
  var assertion = IOPMAssertionID(0)
  let made = IOPMAssertionCreateWithName(
    "PreventUserIdleSystemSleep" as CFString, IOPMAssertionLevel(kIOPMAssertionLevelOn),
    stayAwakeReason as CFString, &assertion)
  guard made == kIOReturnSuccess else {
    stayLog.error("Baton: the system would not keep this Mac awake.")
    return nil
  }
  let held = assertion
  stayLog.notice("Baton: this Mac is held awake while it is plugged in.")
  return {
    _ = IOPMAssertionRelease(held)
    stayLog.notice("Baton: this Mac is let go of, and sleeps as System Settings says.")
  }
}

/// Tells when the power of the Mac changes: it is plugged in or unplugged, or its battery moves.
/// The system tells on the main run loop, in every mode of it, so that a menu that is open does not
/// hold the news back.
final class PowerWatch {
  private let changed: () -> Void
  private var source: CFRunLoopSource?

  init(changed: @escaping () -> Void) {
    self.changed = changed
  }

  deinit {
    stop()
  }

  /// Starts to tell. Starting twice changes nothing.
  func start() {
    guard source == nil else { return }
    let context = Unmanaged.passUnretained(self).toOpaque()
    let callback: IOPowerSourceCallbackType = { pointer in
      guard let pointer = pointer else { return }
      Unmanaged<PowerWatch>.fromOpaque(pointer).takeUnretainedValue().changed()
    }
    guard let made = IOPSNotificationCreateRunLoopSource(callback, context)?.takeRetainedValue()
    else {
      stayLog.error("Baton: the system would not tell when the power of this Mac changes.")
      return
    }
    CFRunLoopAddSource(CFRunLoopGetMain(), made, .commonModes)
    source = made
  }

  func stop() {
    guard let source = source else { return }
    CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes)
    self.source = nil
  }
}

/// The one thing of the app that keeps the Mac awake while it is plugged in: the choice of the
/// person, which the app keeps in its defaults so that it holds from the start of the app, before
/// any screen is up and after the Mac restarts, the hold, and the news of the power.
final class MacStayAwake {
  /// The key of the choice in the defaults of the app.
  static let choiceKey = "stayAwakeWhilePluggedIn"

  /// The one of the process: every engine of the app answers through it.
  static let shared = MacStayAwake()

  private let stay: StayAwake
  private let watch: PowerWatch

  init(defaults: UserDefaults = .standard) {
    let stay = StayAwake(
      asked: defaults.bool(forKey: MacStayAwake.choiceKey),
      remember: { defaults.set($0, forKey: MacStayAwake.choiceKey) },
      pluggedIn: macIsPluggedIn, hold: holdMacAwake)
    self.stay = stay
    watch = PowerWatch { stay.powerChanged() }
  }

  /// Starts to follow the power of the Mac. Starting twice changes nothing.
  func start() {
    watch.start()
  }

  var state: StayAwakeState { stay.state }

  func ask(_ on: Bool) {
    stay.ask(on)
  }
}

#endif
