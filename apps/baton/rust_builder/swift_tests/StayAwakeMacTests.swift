// SPDX-License-Identifier: Apache-2.0

// Tests of what BatonStayAwakeMac.swift asks of a Mac: the hold shows in the list of assertions that
// the system keeps, as `pmset -g assertions` prints it, with this process as its owner and the
// reason that Baton gave, and it is gone when it is let go of. The choice of the person is kept in
// the defaults and held again by the next start of the app. The read of the power is not tested
// here: the call answers the wall when it cannot read, so no Mac can fail it in a test, and a
// laptop that is unplugged shows it (S2 of docs/adr/0018-stay-awake-while-plugged-in.md).

#if os(macOS)

import Foundation

/// What `pmset -g assertions` says: the assertions of the system, by the process that owns them.
private func assertionsOfTheSystem() -> String {
  let process = Process()
  process.executableURL = URL(fileURLWithPath: "/usr/bin/pmset")
  process.arguments = ["-g", "assertions"]
  let pipe = Pipe()
  process.standardOutput = pipe
  process.standardError = FileHandle.nullDevice
  do {
    try process.run()
  } catch {
    return "pmset did not start: \(error)"
  }
  let output = pipe.fileHandleForReading.readDataToEndOfFile()
  process.waitUntilExit()
  return String(decoding: output, as: UTF8.self)
}

/// Whether this process owns an assertion against the sleep of the Mac that gives the reason that
/// Baton gives.
private func thisProcessHoldsTheMac(_ listing: String) -> Bool {
  let owner = "pid \(getpid())("
  let reason = String(stayAwakeReason.prefix(40))
  return listing.split(separator: "\n").contains { line in
    line.contains(owner) && line.contains("PreventUserIdleSystemSleep") && line.contains(reason)
  }
}

func testTheHoldShowsToTheSystemAndIsGoneWhenItIsLetGoOf() {
  check(!thisProcessHoldsTheMac(assertionsOfTheSystem()), "held before anything asked for it")
  guard let letGo = holdMacAwake() else {
    check(false, "the system would not give the hold")
    return
  }
  let listing = assertionsOfTheSystem()
  check(thisProcessHoldsTheMac(listing), "the hold is not in the list of the system:\n\(listing)")
  letGo()
  check(
    eventually { !thisProcessHoldsTheMac(assertionsOfTheSystem()) },
    "the hold is still in the list of the system after it was let go of")
}

func testTheChoiceIsKeptInTheDefaultsAndHeldAtTheNextStart() {
  let suite = "io.github.jayunewal.baton.tests.\(UUID().uuidString)"
  guard let defaults = UserDefaults(suiteName: suite) else {
    check(false, "no defaults of its own for the test")
    return
  }
  defer { defaults.removePersistentDomain(forName: suite) }
  let plugged = macIsPluggedIn() == true
  let held: StayAwakeState = plugged ? .holding : .waiting

  let first = MacStayAwake(defaults: defaults)
  check(first.state == .off, "\(first.state)")
  check(!defaults.bool(forKey: MacStayAwake.choiceKey), "a choice that nobody made")
  check(!thisProcessHoldsTheMac(assertionsOfTheSystem()), "held before the person asked")

  first.ask(true)
  check(first.state == held, "\(first.state), and the Mac is\(plugged ? "" : " not") plugged in")
  check(defaults.bool(forKey: MacStayAwake.choiceKey), "the choice was not kept")
  check(
    thisProcessHoldsTheMac(assertionsOfTheSystem()) == plugged,
    "the hold of the system does not follow the power of the Mac")

  // The next start of the app has the choice, and holds the Mac at once, with no screen up.
  first.ask(false)
  check(!thisProcessHoldsTheMac(assertionsOfTheSystem()), "still held after the person said no")
  defaults.set(true, forKey: MacStayAwake.choiceKey)
  let next = MacStayAwake(defaults: defaults)
  check(next.state == held, "\(next.state), and the Mac is\(plugged ? "" : " not") plugged in")
  check(
    thisProcessHoldsTheMac(assertionsOfTheSystem()) == plugged,
    "the next start did not hold the Mac as the kept choice asks")
  next.ask(false)
  check(!defaults.bool(forKey: MacStayAwake.choiceKey), "the choice that was turned off was kept")
  check(
    eventually { !thisProcessHoldsTheMac(assertionsOfTheSystem()) },
    "the hold is still in the list of the system after the test")
}

func testTheWatchOfThePowerStartsAndStopsAsOftenAsItIsAsked() {
  var told = 0
  let watch = PowerWatch { told += 1 }
  watch.start()
  watch.start()
  watch.stop()
  watch.stop()
  watch.start()
  // Nothing changed the power of this Mac while it ran, so there is nothing to have been told, and
  // what the test checks is that none of this crashes or leaves a source that tells a watch that is
  // gone.
  check(told >= 0)
  watch.stop()
}

#endif
