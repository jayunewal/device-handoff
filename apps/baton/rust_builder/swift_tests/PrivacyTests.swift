// SPDX-License-Identifier: Apache-2.0

// Tests of BatonPrivacy.swift: when the cover of a private session goes up and when it comes down,
// what ends the session, how the Mac is locked until it says that it is, and what the guard is
// told. The Mac itself is played by a curtain that does what the test says, and the guard by one
// that records what it is told; what the real ones do is checked on a Mac
// (docs/research/privacy-mode).

import Foundation

/// What a curtain and a guard were asked, in the order they were asked it.
final class Journal: @unchecked Sendable {
  private let mutex = NSLock()
  private var lines: [String] = []

  func add(_ line: String) {
    mutex.lock()
    lines.append(line)
    mutex.unlock()
  }

  var all: [String] {
    mutex.lock()
    defer { mutex.unlock() }
    return lines
  }
}

/// A guard that records what it is asked, and is lost when the test says.
final class FakeGuard: SessionGuard {
  private let mutex = NSLock()
  private let journal: Journal?
  private var lost: (() -> Void)?
  private var beated = 0
  private var armedCount = 0
  private var disarmedCount = 0

  /// What `arm` throws, if anything.
  var armFailure: GuardFailure?

  init(journal: Journal? = nil) {
    self.journal = journal
  }

  func arm(lost: @escaping () -> Void) throws {
    journal?.add("arm")
    if let armFailure { throw armFailure }
    mutex.lock()
    self.lost = lost
    armedCount += 1
    mutex.unlock()
  }

  func beat() {
    mutex.lock()
    beated += 1
    mutex.unlock()
  }

  func disarm() {
    journal?.add("disarm")
    mutex.lock()
    disarmedCount += 1
    mutex.unlock()
  }

  /// The guard went away, or locked the Mac.
  func lose() {
    mutex.lock()
    let call = lost
    mutex.unlock()
    call?()
  }

  var beats: Int {
    mutex.lock()
    defer { mutex.unlock() }
    return beated
  }

  var armed: Int {
    mutex.lock()
    defer { mutex.unlock() }
    return armedCount
  }

  var disarmed: Int {
    mutex.lock()
    defer { mutex.unlock() }
    return disarmedCount
  }
}

/// A curtain that records what it is asked, and is locked when the test says.
final class FakeCurtain: Curtain {
  private let mutex = NSLock()
  private let journal: Journal?
  private var events: [String] = []
  private var alarm: ((PrivacyAlarm) -> Void)?
  private var isLocked = false
  private var attempts: [Int] = []
  private var lockedAfter: Int?
  private var isHealthy = true

  var raiseFailure: Error?
  /// Rung from inside `raise`, as an alarm is when something at the Mac happens at once.
  var ringsWhileRaising: PrivacyAlarm?
  /// Done from inside `raise`, for something else that happens while the cover goes up.
  var whileRaising: (() -> Void)?
  /// What the call that leaves nothing lifted throws, if anything: the put-back of the privacy.
  var putBackFailure: PrivacyAlarm?
  /// Done from inside `lift`, with what it was asked, for something else that happens meanwhile.
  var whileLifting: ((Bool, Bool) -> Void)?

  /// The Mac is locked by the try with this number, counted from 0. nil: never, until `lockNow`.
  init(locksAtAttempt: Int? = 0, journal: Journal? = nil) {
    lockedAfter = locksAtAttempt
    self.journal = journal
  }

  /// Whether the cover and the hold are as they were put, and looked at.
  var healthy: Bool {
    get {
      mutex.lock()
      defer { mutex.unlock() }
      return isHealthy
    }
    set {
      mutex.lock()
      isHealthy = newValue
      mutex.unlock()
    }
  }

  /// What `darken` says of the screens when the curtain is well, and how often it was asked.
  var screensAreDark = true
  private var darkened = 0

  var darkens: Int {
    mutex.lock()
    defer { mutex.unlock() }
    return darkened
  }

  func darken() -> Bool {
    mutex.lock()
    defer { mutex.unlock() }
    darkened += 1
    return isHealthy && screensAreDark
  }

  func raise(alarm: @escaping (PrivacyAlarm) -> Void) throws {
    journal?.add("raise")
    mutex.lock()
    events.append("raise")
    mutex.unlock()
    if let raiseFailure { throw raiseFailure }
    mutex.lock()
    self.alarm = alarm
    mutex.unlock()
    whileRaising?()
    if let ringsWhileRaising {
      alarm(ringsWhileRaising)
      Thread.sleep(forTimeInterval: 0.05)
    }
  }

  func lift(screen: Bool, input: Bool) throws {
    let line = "lift \(screen) \(input)"
    journal?.add(line)
    mutex.lock()
    events.append(line)
    mutex.unlock()
    whileLifting?(screen, input)
    if !screen && !input, let putBackFailure { throw putBackFailure }
  }

  func lower() {
    journal?.add("lower")
    mutex.lock()
    events.append("lower")
    mutex.unlock()
  }

  var locked: Bool {
    mutex.lock()
    defer { mutex.unlock() }
    return isLocked
  }

  func lock(attempt: Int) {
    journal?.add("lock \(attempt)")
    mutex.lock()
    events.append("lock \(attempt)")
    attempts.append(attempt)
    if let lockedAfter, attempt >= lockedAfter { isLocked = true }
    mutex.unlock()
  }

  /// The Mac was locked by somebody, or by the way that finally works.
  func lockNow() {
    mutex.lock()
    isLocked = true
    mutex.unlock()
  }

  /// What the Mac at the other side of the protocol did, in order.
  var log: [String] {
    mutex.lock()
    defer { mutex.unlock() }
    return events
  }

  var lowered: Int { log.filter { $0 == "lower" }.count }

  /// Something at the Mac needs the session ended.
  func ring(_ alarm: PrivacyAlarm) {
    mutex.lock()
    let call = self.alarm
    mutex.unlock()
    call?(alarm)
  }
}

/// A clock that moves when the privacy sleeps, and takes next to no time for it.
final class FakeClock: @unchecked Sendable {
  private let mutex = NSLock()
  private var time: TimeInterval = 100
  private var slept: [TimeInterval] = []

  func now() -> TimeInterval {
    mutex.lock()
    defer { mutex.unlock() }
    return time
  }

  func sleep(_ seconds: TimeInterval) {
    mutex.lock()
    time += seconds
    slept.append(seconds)
    mutex.unlock()
    Thread.sleep(forTimeInterval: 0.0005)
  }

  var sleeps: [TimeInterval] {
    mutex.lock()
    defer { mutex.unlock() }
    return slept
  }
}

/// What a session told the other device, from whatever thread it did.
final class Told: @unchecked Sendable {
  private let mutex = NSLock()
  private var sentences: [String] = []
  private let arrived = DispatchSemaphore(value: 0)

  func tell(_ sentence: String) {
    mutex.lock()
    sentences.append(sentence)
    mutex.unlock()
    arrived.signal()
  }

  /// Whether a sentence arrived within `seconds`.
  func wait(seconds: TimeInterval = 5) -> Bool {
    arrived.wait(timeout: .now() + seconds) == .success
  }

  var all: [String] {
    mutex.lock()
    defer { mutex.unlock() }
    return sentences
  }
}

private func session(
  _ curtain: Curtain, guardian: SessionGuard = FakeGuard(), clock: FakeClock = FakeClock(),
  told: Told = Told(), patience: TimeInterval = 5
) -> PrivateSession {
  PrivateSession(
    curtain: curtain, guardian: guardian, timing: PrivacyTiming(patience: patience, beat: 0.005),
    end: told.tell, sleep: clock.sleep, now: clock.now)
}

// MARK: - When the cover goes up

func testTheCoverGoesUpOnceAndAFailureLeavesNothingRaised() {
  let curtain = FakeCurtain()
  let privacy = session(curtain)
  do {
    try privacy.begin()
  } catch {
    check(false, "the cover could not be raised: \(error)")
  }
  check(curtain.log == ["raise"], "\(curtain.log)")
  do {
    try privacy.begin()
    check(false, "the cover was raised twice")
  } catch let failure as SharerFailure {
    check(!failure.sentence.isEmpty)
  } catch {
    check(false, "another kind of error: \(error)")
  }
  check(curtain.log == ["raise"], "the second begin touched the Mac: \(curtain.log)")

  let refusing = FakeCurtain()
  refusing.raiseFailure = SharerFailure(sentence: "The screens did not go dark.")
  let failing = session(refusing)
  do {
    try failing.begin()
    check(false, "a cover that could not be raised was raised")
  } catch let failure as SharerFailure {
    check(failure.sentence == "The screens did not go dark.", failure.sentence)
  } catch {
    check(false, "another kind of error: \(error)")
  }
  // Whatever was done before the failure is undone, and the Mac is not locked: nothing began.
  check(refusing.log == ["raise", "lower"], "\(refusing.log)")
  // The session cannot be raised again, and asking to does not touch the Mac.
  do {
    try failing.begin()
    check(false, "a session whose cover failed was raised again")
  } catch is SharerFailure {
  } catch {
    check(false, "another kind of error: \(error)")
  }
  check(refusing.log == ["raise", "lower"], "the second begin touched the Mac: \(refusing.log)")
  failing.finish(.released)
  failing.finish(.sessionOver)
  check(
    refusing.log == ["raise", "lower"], "a session that did not begin finished: \(refusing.log)")
}

func testAWindowThatWasOnlyGivenAndLetGoOfRaisesNothingAndCannotBeRaisedAfterwards() {
  let curtain = FakeCurtain()
  let privacy = session(curtain)
  privacy.finish(.released)
  check(curtain.log.isEmpty, "\(curtain.log)")
  do {
    try privacy.begin()
    check(false, "a window that was let go of was raised")
  } catch is SharerFailure {
  } catch {
    check(false, "another kind of error: \(error)")
  }
  // A lock that a thread of its own had been started for would show by now.
  Thread.sleep(forTimeInterval: 0.1)
  check(curtain.log.isEmpty, "\(curtain.log)")
}

// MARK: - When the cover comes down

func testTheCoverComesDownOnlyAfterTheMacIsLockedAndTheLockScreenWasDrawn() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let clock = FakeClock()
  let privacy = session(curtain, clock: clock)
  try? privacy.begin()
  let began = Date()
  privacy.finish(.sessionOver)
  check(Date().timeIntervalSince(began) < 2, "the end of the session waited for more than the lock")
  check(curtain.log == ["raise", "lock 0", "lower"], "\(curtain.log)")
  // The cover stays for a while after the Mac says that it is locked, and for no longer.
  check(
    clock.sleeps.contains { $0 >= PrivacyTiming().afterLock }, "slept \(clock.sleeps)")
  check(
    clock.sleeps.reduce(0, +) < PrivacyTiming().afterLock + 0.5, "slept \(clock.sleeps)")
  // Finishing again, in any way, changes nothing.
  privacy.finish(.sessionOver)
  privacy.finish(.released)
  privacy.finish(.alarm(.escape))
  check(curtain.log == ["raise", "lock 0", "lower"], "\(curtain.log)")
}

func testAMacThatIsLockedAlreadyIsNotLockedAgainAndKeepsTheCoverUntilTheEnd() {
  let curtain = FakeCurtain(locksAtAttempt: nil)
  curtain.lockNow()
  let clock = FakeClock()
  let privacy = session(curtain, clock: clock)
  try? privacy.begin()
  privacy.finish(.sessionOver)
  check(curtain.log == ["raise", "lower"], "\(curtain.log)")
  check(clock.sleeps.contains { $0 >= PrivacyTiming().afterLock }, "slept \(clock.sleeps)")
}

func testAMacThatDoesNotLockWithTheFirstWayIsTriedWithTheNextOnesAndKeepsItsCoverMeanwhile() {
  let curtain = FakeCurtain(locksAtAttempt: 2)
  let clock = FakeClock()
  let privacy = session(curtain, clock: clock)
  try? privacy.begin()
  privacy.finish(.sessionOver)
  check(curtain.log == ["raise", "lock 0", "lock 1", "lock 2", "lower"], "\(curtain.log)")
  // Each way is given time to take effect before the next is tried.
  let settle = PrivacyTiming().settle
  check(
    clock.sleeps.reduce(0, +) >= 2 * settle + PrivacyTiming().afterLock - 0.01,
    "slept \(clock.sleeps.reduce(0, +)) s")
}

func testAMacThatCannotBeLockedKeepsItsCoverAndFinishReturnsAfterThePatience() {
  let curtain = FakeCurtain(locksAtAttempt: nil)
  let privacy = session(curtain, patience: 0.3)
  try? privacy.begin()
  let began = Date()
  privacy.finish(.sessionOver)
  let waited = Date().timeIntervalSince(began)
  check(waited >= 0.25 && waited < 3, "waited \(waited) s")
  check(curtain.lowered == 0, "the cover came down on a Mac that is not locked")
  // It goes on trying, and the cover goes when the Mac is locked at last.
  check(eventually { curtain.log.filter { $0.hasPrefix("lock") }.count >= 4 }, "\(curtain.log)")
  check(curtain.lowered == 0, "the cover came down on a Mac that is not locked")
  curtain.lockNow()
  check(eventually { curtain.lowered == 1 }, "\(curtain.log)")
}

// MARK: - What ends a session at the Mac

func testSomethingAtTheMacEndsTheSessionOnceTellsTheOtherDeviceAndLocksTheMac() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let told = Told()
  let privacy = session(curtain, told: told)
  try? privacy.begin()
  curtain.ring(.escape)
  check(told.wait(), "the other device was not told")
  // Whatever else happens at the Mac while the session ends does not end it again.
  curtain.ring(.foreignKey)
  curtain.ring(.escape)
  check(eventually { curtain.lowered == 1 }, "\(curtain.log)")
  privacy.finish(.sessionOver)
  Thread.sleep(forTimeInterval: 0.05)
  check(told.all == [PrivacyAlarm.escape.sentence], "\(told.all)")
  check(curtain.log == ["raise", "lock 0", "lower"], "\(curtain.log)")
}

func testSomethingThatHappensAtTheMacWhileTheCoverGoesUpEndsTheSessionAfterTheCoverIsUp() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  curtain.ringsWhileRaising = .holdLost
  let told = Told()
  let privacy = session(curtain, told: told)
  do {
    try privacy.begin()
  } catch {
    check(false, "the cover could not be raised: \(error)")
  }
  check(told.wait(), "the other device was not told")
  check(eventually { curtain.lowered == 1 }, "\(curtain.log)")
  // The Mac is locked and given back only after the cover was up, never before.
  check(curtain.log == ["raise", "lock 0", "lower"], "\(curtain.log)")
  check(told.all == [PrivacyAlarm.holdLost.sentence], "\(told.all)")
}

func testTheMacIsLockedWhileTheOtherDeviceIsStillBeingToldAndNeitherWaitsForTheOther() {
  // The library can be slow to end a session, for it waits for the thread that stops the capture,
  // and that thread waits for the lock: if the lock waited for the library the two would wait for
  // each other.
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let release = DispatchSemaphore(value: 0)
  let entered = DispatchSemaphore(value: 0)
  let clock = FakeClock()
  let privacy = PrivateSession(
    curtain: curtain, guardian: FakeGuard(), timing: PrivacyTiming(patience: 5),
    end: { _ in
      entered.signal()
      _ = release.wait(timeout: .now() + 10)
    }, sleep: clock.sleep, now: clock.now)
  try? privacy.begin()
  curtain.ring(.holdLost)
  check(entered.wait(timeout: .now() + 5) == .success, "the other device was not told")
  check(eventually { curtain.lowered == 1 }, "the Mac was not locked while the library was busy")
  release.signal()
}

func testAnAlarmDoesNotHoldUpWhoeverRingsItAndTheCoverStaysUntilTheMacIsLocked() {
  let curtain = FakeCurtain(locksAtAttempt: nil)
  let privacy = session(curtain, patience: 5)
  try? privacy.begin()
  // The thread of the tap rings the alarm and must answer the system at once.
  let began = Date()
  privacy.finish(.alarm(.escape))
  check(Date().timeIntervalSince(began) < 1, "an alarm waited for the lock")
  check(curtain.lowered == 0, "the cover came down on a Mac that is not locked")
  curtain.lockNow()
  check(eventually { curtain.lowered == 1 }, "\(curtain.log)")
}

func testTheEndOfASessionWaitsForTheLockAndAReleaseThatFollowsDoesNot() {
  let curtain = FakeCurtain(locksAtAttempt: nil)
  let privacy = session(curtain, patience: 5)
  try? privacy.begin()
  // Something at the Mac ended it first, and the Mac is not locked yet.
  curtain.ring(.coverLost)
  check(eventually { curtain.log.contains("lock 0") }, "\(curtain.log)")

  let ended = DispatchSemaphore(value: 0)
  let end = Thread {
    privacy.finish(.sessionOver)
    ended.signal()
  }
  end.start()
  check(ended.wait(timeout: .now() + 0.3) == .timedOut, "the end of the session did not wait")

  // A release that comes after does not hold anything up.
  let began = Date()
  privacy.finish(.released)
  check(Date().timeIntervalSince(began) < 1, "a release waited for the lock")
  check(curtain.lowered == 0)

  curtain.lockNow()
  check(ended.wait(timeout: .now() + 5) == .success, "the end of the session never returned")
  check(curtain.lowered == 1, "\(curtain.log)")
}

func testASessionThatDidNotStartAfterTheCoverWentUpIsLockedAndGivenBackWhenItIsReleased() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let privacy = session(curtain)
  try? privacy.begin()
  privacy.finish(.released)
  check(curtain.log == ["raise", "lock 0", "lower"], "\(curtain.log)")
}

// MARK: - What the person at the other device lifts

func testWhatIsLiftedReachesTheCurtainOnlyWhileTheCoverIsUpAndEachCallReplacesTheOneBefore() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let privacy = session(curtain)
  // Before the cover is up there is nothing to lift.
  privacy.lift(screen: true, input: true)
  check(curtain.log.isEmpty, "\(curtain.log)")
  try? privacy.begin()
  // A session starts with nothing lifted, so a call that lifts nothing changes nothing.
  privacy.lift(screen: false, input: false)
  check(curtain.log == ["raise"], "a lift of nothing was passed on: \(curtain.log)")
  // Either part by itself, both, and back, as often as the person likes. A call that says what
  // already is changes nothing and is not passed on.
  privacy.lift(screen: true, input: false)
  privacy.lift(screen: true, input: false)
  privacy.lift(screen: true, input: true)
  privacy.lift(screen: false, input: true)
  privacy.lift(screen: false, input: true)
  privacy.lift(screen: false, input: false)
  privacy.lift(screen: false, input: false)
  privacy.lift(screen: false, input: true)
  privacy.lift(screen: false, input: false)
  let lifts = [
    "lift true false", "lift true true", "lift false true", "lift false false", "lift false true",
    "lift false false",
  ]
  check(curtain.log == ["raise"] + lifts, "\(curtain.log)")
  // Nothing is lifted when the session ends, so nothing is put back first.
  privacy.finish(.sessionOver)
  check(curtain.log == ["raise"] + lifts + ["lock 0", "lower"], "\(curtain.log)")
  // And once it is over there is nothing to lift.
  privacy.lift(screen: true, input: true)
  Thread.sleep(forTimeInterval: 0.05)
  check(curtain.log == ["raise"] + lifts + ["lock 0", "lower"], "\(curtain.log)")
}

func testASessionThatEndsWithSomethingLiftedPutsItBackBeforeItLocksTheMac() {
  // However the session ends, what was lifted is put back first: nobody at the Mac can get in the
  // way of the lock, and the lock screen is drawn behind the cover and not in front of a screen
  // that shows what was on it.
  let ways: [(String, (PrivateSession, FakeCurtain) -> Void)] = [
    ("the end of the session", { privacy, _ in privacy.finish(.sessionOver) }),
    ("a release", { privacy, _ in privacy.finish(.released) }),
    ("an alarm", { _, curtain in curtain.ring(.holdLost) }),
  ]
  for (name, end) in ways {
    for (screen, input) in [(true, false), (false, true), (true, true)] {
      let curtain = FakeCurtain(locksAtAttempt: 0)
      let privacy = session(curtain)
      try? privacy.begin()
      privacy.lift(screen: screen, input: input)
      end(privacy, curtain)
      check(eventually { curtain.lowered == 1 }, "\(name): \(curtain.log)")
      check(
        curtain.log == ["raise", "lift \(screen) \(input)", "lift false false", "lock 0", "lower"],
        "\(name), \(screen) \(input): \(curtain.log)")
    }
  }
}

func testAPartThatCannotBePutBackEndsTheSessionTellsTheOtherDeviceAndLocksTheMacAnyway() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  curtain.putBackFailure = .coverLost
  let told = Told()
  let privacy = session(curtain, told: told)
  try? privacy.begin()
  privacy.lift(screen: true, input: false)
  privacy.lift(screen: false, input: false)
  check(told.wait(), "the other device was not told")
  check(told.all == [PrivacyAlarm.coverLost.sentence], "\(told.all)")
  check(eventually { curtain.lowered == 1 }, "\(curtain.log)")
  // What is still lifted is put back once more before the lock, and when that fails as well the
  // lock goes on.
  check(
    curtain.log == [
      "raise", "lift true false", "lift false false", "lift false false", "lock 0", "lower",
    ], "\(curtain.log)")
  // The session is over, and a lift after it changes nothing.
  privacy.lift(screen: true, input: true)
  Thread.sleep(forTimeInterval: 0.05)
  check(curtain.log.count == 6, "\(curtain.log)")
}

func testAFinishThatComesWhileSomethingIsBeingLiftedWaitsForTheLiftAndThenPutsItBack() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let privacy = session(curtain)
  try? privacy.begin()
  let inside = DispatchSemaphore(value: 0)
  let go = DispatchSemaphore(value: 0)
  // Only the lift that lifts something waits: the put-back, which the finish does, does not.
  curtain.whileLifting = { screen, input in
    guard screen || input else { return }
    inside.signal()
    _ = go.wait(timeout: .now() + 5)
  }
  let lifting = Thread { privacy.lift(screen: true, input: true) }
  lifting.start()
  check(inside.wait(timeout: .now() + 5) == .success, "the curtain was not asked to lift")
  let finishing = Thread { privacy.finish(.sessionOver) }
  finishing.start()
  // The session is not finished half way through a lift.
  Thread.sleep(forTimeInterval: 0.2)
  check(curtain.log == ["raise", "lift true true"], "\(curtain.log)")
  go.signal()
  check(eventually { curtain.lowered == 1 }, "\(curtain.log)")
  check(
    curtain.log == ["raise", "lift true true", "lift false false", "lock 0", "lower"],
    "\(curtain.log)")
}

// MARK: - When the person at the other device chose to leave the Mac as it is

func testALeaveLowersTheCoverAndLocksNothingAndDoesNotWaitForALockScreen() {
  let journal = Journal()
  // A Mac that would be locked at once, if anything asked it to be.
  let curtain = FakeCurtain(locksAtAttempt: 0, journal: journal)
  let guardian = FakeGuard(journal: journal)
  let clock = FakeClock()
  let privacy = session(curtain, guardian: guardian, clock: clock)
  try? privacy.begin()
  privacy.finish(.left)
  check(journal.all == ["arm", "raise", "lower", "disarm"], "\(journal.all)")
  check(!curtain.locked, "the Mac was locked")
  // The cover has no lock screen to wait for.
  check(clock.sleeps.isEmpty, "slept \(clock.sleeps)")
  check(guardian.armed == 1 && guardian.disarmed == 1)
  // Finishing again, in any way, changes nothing.
  privacy.finish(.left)
  privacy.finish(.sessionOver)
  privacy.finish(.released)
  privacy.finish(.alarm(.escape))
  Thread.sleep(forTimeInterval: 0.05)
  check(journal.all == ["arm", "raise", "lower", "disarm"], "\(journal.all)")
  check(guardian.disarmed == 1)
}

func testALeaveOfAMacThatIsLockedKeepsItLockedAndKeepsTheCoverUntilTheLockScreenIsDrawn() {
  let curtain = FakeCurtain(locksAtAttempt: nil)
  curtain.lockNow()
  let clock = FakeClock()
  let privacy = session(curtain, clock: clock)
  try? privacy.begin()
  privacy.finish(.left)
  // Nothing is locked again, woken or unlocked.
  check(curtain.log == ["raise", "lower"], "\(curtain.log)")
  check(curtain.locked, "the Mac is not locked any more")
  check(clock.sleeps.contains { $0 >= PrivacyTiming().afterLock }, "slept \(clock.sleeps)")
}

func testASessionThatEndsByLeavingWithSomethingLiftedPutsItBackAndLowersTheCoverWithoutALock() {
  for (screen, input) in [(true, false), (false, true), (true, true)] {
    let curtain = FakeCurtain(locksAtAttempt: 0)
    let privacy = session(curtain)
    try? privacy.begin()
    privacy.lift(screen: screen, input: input)
    privacy.finish(.left)
    check(
      curtain.log == ["raise", "lift \(screen) \(input)", "lift false false", "lower"],
      "\(screen) \(input): \(curtain.log)")
    check(!curtain.locked, "\(screen) \(input): the Mac was locked")
  }
}

func testAPartThatCannotBePutBackBeforeALeaveLocksTheMacAfterAll() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  curtain.putBackFailure = .coverLost
  let told = Told()
  let privacy = session(curtain, told: told)
  try? privacy.begin()
  privacy.lift(screen: true, input: false)
  privacy.finish(.left)
  // The Mac is not left open because something in the end went wrong, and what was lifted is not
  // put back twice.
  check(
    curtain.log == ["raise", "lift true false", "lift false false", "lock 0", "lower"],
    "\(curtain.log)")
  check(curtain.locked, "the Mac was left open")
  // The person ended the session, so nobody has to be told that it ended.
  Thread.sleep(forTimeInterval: 0.05)
  check(told.all.isEmpty, "\(told.all)")
}

func testALeaveThatComesAfterSomethingAtTheMacBeganToLockWaitsForTheLockAndLeavesNothing() {
  let curtain = FakeCurtain(locksAtAttempt: nil)
  let privacy = session(curtain, patience: 5)
  try? privacy.begin()
  // Something at the Mac ended it first, and the Mac is not locked yet.
  curtain.ring(.coverLost)
  check(eventually { curtain.log.contains("lock 0") }, "\(curtain.log)")

  let ended = DispatchSemaphore(value: 0)
  let end = Thread {
    privacy.finish(.left)
    ended.signal()
  }
  end.start()
  check(ended.wait(timeout: .now() + 0.3) == .timedOut, "the leave did not wait for the lock")
  check(curtain.lowered == 0)

  curtain.lockNow()
  check(ended.wait(timeout: .now() + 5) == .success, "the leave never returned")
  // The session ended the way it began to, and the cover came down once, on a Mac that is locked.
  check(curtain.lowered == 1, "\(curtain.log)")
  check(curtain.locked)
}

func testSomethingAtTheMacWhileItIsBeingLeftDoesNotLockIt() {
  // The put-back is where the Mac is being left: the person's choice came first.
  let curtain = FakeCurtain(locksAtAttempt: 0)
  curtain.whileLifting = { [weak curtain] screen, input in
    if !screen && !input { curtain?.ring(.escape) }
  }
  let told = Told()
  let privacy = session(curtain, told: told)
  try? privacy.begin()
  privacy.lift(screen: true, input: false)
  privacy.finish(.left)
  Thread.sleep(forTimeInterval: 0.1)
  check(
    curtain.log == ["raise", "lift true false", "lift false false", "lower"], "\(curtain.log)")
  check(!curtain.locked, "the Mac was locked")
  check(told.all.isEmpty, "\(told.all)")
}

func testTheEndOfASessionThatComesWhileTheMacIsBeingLeftWaitsForTheLeaveAndLocksNothing() {
  // A quit of the app is such an end: it waits for the cover to come down, and goes on.
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let inside = DispatchSemaphore(value: 0)
  let go = DispatchSemaphore(value: 0)
  curtain.whileLifting = { screen, input in
    guard !screen && !input else { return }
    inside.signal()
    _ = go.wait(timeout: .now() + 5)
  }
  let privacy = session(curtain, patience: 5)
  try? privacy.begin()
  privacy.lift(screen: true, input: false)
  let leaving = Thread { privacy.finish(.left) }
  leaving.start()
  check(inside.wait(timeout: .now() + 5) == .success, "the put-back never began")

  let quit = DispatchSemaphore(value: 0)
  let quitting = Thread {
    privacy.finish(.sessionOver)
    quit.signal()
  }
  quitting.start()
  check(quit.wait(timeout: .now() + 0.3) == .timedOut, "the end of the session did not wait")
  go.signal()
  check(quit.wait(timeout: .now() + 5) == .success, "the end of the session never returned")
  check(
    curtain.log == ["raise", "lift true false", "lift false false", "lower"], "\(curtain.log)")
  check(!curtain.locked, "the Mac was locked")
}

func testALeaveEndsTheSessionAndThenTheGuardIsLetGoOfAndNotToldAgain() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let guardian = FakeGuard()
  let privacy = session(curtain, guardian: guardian)
  try? privacy.begin()
  check(eventually { guardian.beats >= 3 }, "the guard was not told: \(guardian.beats)")
  let began = Date()
  privacy.finish(.left)
  // The session is over when the leave has been done, and does not wait for its patience.
  check(Date().timeIntervalSince(began) < 2, "the leave waited for its patience")
  check(guardian.disarmed == 1, "the guard was not let go of")
  Thread.sleep(forTimeInterval: 0.05)
  let ended = guardian.beats
  Thread.sleep(forTimeInterval: 0.15)
  check(guardian.beats == ended, "the guard was told after the Mac was left")
}

func testAWindowThatWasNeverRaisedIsNotLeftEither() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let privacy = session(curtain)
  privacy.finish(.left)
  Thread.sleep(forTimeInterval: 0.1)
  check(curtain.log.isEmpty, "\(curtain.log)")
  do {
    try privacy.begin()
    check(false, "a window that was left was raised afterwards")
  } catch is SharerFailure {
  } catch {
    check(false, "another kind of error: \(error)")
  }
}

// MARK: - The guard

func testTheGuardIsHandedTheMacBeforeTheCoverGoesUpAndLetGoOfAfterItComesDown() {
  let journal = Journal()
  let curtain = FakeCurtain(locksAtAttempt: 1, journal: journal)
  let guardian = FakeGuard(journal: journal)
  let privacy = session(curtain, guardian: guardian)
  do {
    try privacy.begin()
  } catch {
    check(false, "the cover could not be raised: \(error)")
  }
  // The guard is the first, so that nothing is ever covered that nothing would lock.
  check(journal.all == ["arm", "raise"], "\(journal.all)")
  privacy.finish(.sessionOver)
  // The guard is let go of last, when the Mac is locked and nothing is covered.
  check(
    journal.all == ["arm", "raise", "lock 0", "lock 1", "lower", "disarm"], "\(journal.all)")
  check(guardian.armed == 1 && guardian.disarmed == 1)
  // Finishing again, in any way, hands nothing back twice.
  privacy.finish(.sessionOver)
  privacy.finish(.alarm(.escape))
  Thread.sleep(forTimeInterval: 0.05)
  check(guardian.disarmed == 1, "\(journal.all)")
}

func testAGuardThatCannotBeHadRaisesNothingAndSaysWhy() {
  let journal = Journal()
  let curtain = FakeCurtain(journal: journal)
  let guardian = FakeGuard(journal: journal)
  guardian.armFailure = GuardFailure(sentence: "This Mac has no guard.")
  let privacy = session(curtain, guardian: guardian)
  do {
    try privacy.begin()
    check(false, "a session was begun that no guard watches")
  } catch let failure as SharerFailure {
    check(failure.sentence == "This Mac has no guard.", failure.sentence)
  } catch {
    check(false, "another kind of error: \(error)")
  }
  // Nothing was covered, held or locked, and there is nothing to give back.
  check(journal.all == ["arm"], "\(journal.all)")
  // The session cannot begin again, and finishing it does nothing.
  do {
    try privacy.begin()
    check(false, "a session that no guard watched was begun again")
  } catch is SharerFailure {
  } catch {
    check(false, "another kind of error: \(error)")
  }
  privacy.finish(.released)
  privacy.finish(.sessionOver)
  Thread.sleep(forTimeInterval: 0.05)
  check(journal.all == ["arm"], "\(journal.all)")
  check(guardian.disarmed == 0)
}

func testACoverThatDoesNotGoUpGivesTheGuardBackAndLocksNothing() {
  let journal = Journal()
  let curtain = FakeCurtain(journal: journal)
  curtain.raiseFailure = SharerFailure(sentence: "The screens did not go dark.")
  let guardian = FakeGuard(journal: journal)
  let privacy = session(curtain, guardian: guardian)
  do {
    try privacy.begin()
    check(false, "a cover that could not be raised was raised")
  } catch let failure as SharerFailure {
    check(failure.sentence == "The screens did not go dark.", failure.sentence)
  } catch {
    check(false, "another kind of error: \(error)")
  }
  check(journal.all == ["arm", "raise", "lower", "disarm"], "\(journal.all)")
  // The guard that was let go of is not heard from again.
  guardian.lose()
  Thread.sleep(forTimeInterval: 0.05)
  check(journal.all == ["arm", "raise", "lower", "disarm"], "\(journal.all)")
  // Nor is it told that the session is well: the session is done, and nothing is guarded.
  let told = guardian.beats
  Thread.sleep(forTimeInterval: 0.1)
  check(guardian.beats == told, "the guard was told after the cover failed")
}

func testAGuardThatIsLostEndsTheSessionTellsTheOtherDeviceAndLocksTheMac() {
  let journal = Journal()
  let curtain = FakeCurtain(locksAtAttempt: 0, journal: journal)
  let guardian = FakeGuard(journal: journal)
  let told = Told()
  let privacy = session(curtain, guardian: guardian, told: told)
  try? privacy.begin()
  guardian.lose()
  check(told.wait(), "the other device was not told")
  check(eventually { journal.all.last == "disarm" }, "\(journal.all)")
  check(journal.all == ["arm", "raise", "lock 0", "lower", "disarm"], "\(journal.all)")
  check(told.all == [PrivacyAlarm.guardLost.sentence], "\(told.all)")
  // A guard that is lost twice, or after the end, ends nothing again.
  guardian.lose()
  privacy.finish(.sessionOver)
  Thread.sleep(forTimeInterval: 0.05)
  check(told.all.count == 1, "\(told.all)")
  check(journal.all.count == 5, "\(journal.all)")
}

func testAGuardThatIsLostWhileTheCoverGoesUpEndsTheSessionAfterTheCoverIsUp() {
  let journal = Journal()
  let curtain = FakeCurtain(locksAtAttempt: 0, journal: journal)
  let guardian = FakeGuard(journal: journal)
  // The guard was armed and goes away while the cover is being raised.
  curtain.whileRaising = { guardian.lose() }
  let told = Told()
  let privacy = session(curtain, guardian: guardian, told: told)
  do {
    try privacy.begin()
  } catch {
    check(false, "the cover could not be raised: \(error)")
  }
  check(told.wait(), "the other device was not told")
  check(eventually { journal.all.last == "disarm" }, "\(journal.all)")
  // The Mac is locked after the cover was up, and the guard is let go of last.
  check(journal.all == ["arm", "raise", "lock 0", "lower", "disarm"], "\(journal.all)")
  check(told.all == [PrivacyAlarm.guardLost.sentence], "\(told.all)")
}

func testTheGuardIsToldThatTheSessionIsWellOnlyWhileTheCurtainIs() {
  let curtain = FakeCurtain()
  let guardian = FakeGuard()
  let privacy = session(curtain, guardian: guardian)
  Thread.sleep(forTimeInterval: 0.05)
  check(guardian.beats == 0, "a session that never began told the guard that it is well")
  try? privacy.begin()
  check(eventually { guardian.beats >= 3 }, "the guard was not told: \(guardian.beats)")

  curtain.healthy = false
  Thread.sleep(forTimeInterval: 0.05)
  let paused = guardian.beats
  Thread.sleep(forTimeInterval: 0.15)
  check(guardian.beats == paused, "the guard was told that a curtain that is not well is well")

  curtain.healthy = true
  check(eventually { guardian.beats > paused }, "the guard was not told again")

  privacy.finish(.sessionOver)
  let ended = guardian.beats
  Thread.sleep(forTimeInterval: 0.15)
  check(guardian.beats == ended, "the guard was told after the session was over")
}

func testTheGuardIsToldThatTheAppIsAtWorkWhileTheCoverGoesUp() {
  // A Mac that is busy takes longer to raise the cover than the guard waits for a first word, and
  // a guard that locked the Mac before the session began would be a Mac that locks for nothing.
  let curtain = FakeCurtain()
  let guardian = FakeGuard()
  curtain.healthy = false
  var during = 0
  curtain.whileRaising = {
    Thread.sleep(forTimeInterval: 0.3)
    during = guardian.beats
  }
  let privacy = session(curtain, guardian: guardian)
  do { try privacy.begin() } catch { check(false, "the cover could not be raised: \(error)") }
  check(during >= 3, "the guard was not told while the cover went up: \(during)")
  // Once the cover is up the curtain is judged, and one that is not well is not said to be.
  Thread.sleep(forTimeInterval: 0.05)
  let judged = guardian.beats
  Thread.sleep(forTimeInterval: 0.2)
  check(guardian.beats == judged, "the guard was told that a curtain that is not well is well")
  curtain.healthy = true
  Thread.sleep(forTimeInterval: 0.2)
  check(guardian.beats > judged, "the guard was not told that a curtain that is well is well")
  privacy.finish(.sessionOver)
}

func testTheGuardIsNotToldThatTheAppIsAtWorkForLongerThanTheBound() {
  // A cover that hangs while it goes up must end in the lock of the guard, and not be told well
  // for as long as it hangs.
  let curtain = FakeCurtain()
  let guardian = FakeGuard()
  curtain.healthy = false
  var first = 0
  var second = 0
  curtain.whileRaising = {
    Thread.sleep(forTimeInterval: 0.25)
    first = guardian.beats
    Thread.sleep(forTimeInterval: 0.25)
    second = guardian.beats
  }
  let clock = FakeClock()
  let privacy = PrivateSession(
    curtain: curtain, guardian: guardian,
    timing: PrivacyTiming(patience: 5, beat: 0.005, raise: 0.1), end: Told().tell,
    sleep: clock.sleep, now: clock.now)
  do { try privacy.begin() } catch { check(false, "the cover could not be raised: \(error)") }
  check(first >= 2, "the guard was not told while the cover went up: \(first)")
  check(second == first, "the guard was told after the bound: \(first) then \(second)")
  privacy.finish(.sessionOver)
}

func testTheGuardIsToldUntilTheMacIsLockedAndTheCoverIsDown() {
  // An app that hangs while it locks the Mac is the guard's to answer for, so the guard goes on
  // being told, and is not let go of, for as long as the Mac is not locked.
  let curtain = FakeCurtain(locksAtAttempt: nil)
  let guardian = FakeGuard()
  let privacy = session(curtain, guardian: guardian, patience: 0.2)
  try? privacy.begin()
  privacy.finish(.sessionOver)
  let during = guardian.beats
  check(
    eventually { guardian.beats > during + 3 }, "the guard was not told while the Mac was locked")
  check(guardian.disarmed == 0, "the guard was let go of on a Mac that is not locked")
  curtain.lockNow()
  check(eventually { guardian.disarmed == 1 }, "the guard was not let go of")
  check(curtain.lowered == 1)
  Thread.sleep(forTimeInterval: 0.05)
  let ended = guardian.beats
  Thread.sleep(forTimeInterval: 0.1)
  check(guardian.beats == ended, "the guard was told after it was let go of")
}

func testASessionThatNobodyHoldsDoesNotGoOnBeating() {
  let curtain = FakeCurtain()
  let guardian = FakeGuard()
  var privacy: PrivateSession? = session(curtain, guardian: guardian)
  try? privacy?.begin()
  check(eventually { guardian.beats >= 2 })
  privacy = nil
  Thread.sleep(forTimeInterval: 0.1)
  let ended = guardian.beats
  Thread.sleep(forTimeInterval: 0.15)
  check(guardian.beats == ended, "a session that nobody held went on beating")
}

// MARK: - A quit of the app

func testAQuitOfTheAppLocksTheMacBeforeItGoesOn() {
  // The quit gives the screens back as the process ends, so it finishes the session first, and
  // the notification does not return before the Mac is locked and the cover is down.
  let center = NotificationCenter()
  let quit = Notification.Name("test.the.app.quits")
  let curtain = FakeCurtain(locksAtAttempt: 1)
  let privacy = session(curtain)
  try? privacy.begin()
  let watch = QuitWatch(name: quit, center: center) { privacy.finish(.sessionOver) }
  center.post(name: quit, object: nil)
  check(curtain.log == ["raise", "lock 0", "lock 1", "lower"], "\(curtain.log)")

  // A watch that was stopped does nothing, and may be stopped again.
  watch.stop()
  watch.stop()
  center.post(name: quit, object: nil)
  check(curtain.log == ["raise", "lock 0", "lock 1", "lower"], "\(curtain.log)")
}

func testAWatchOfTheQuitCallsItsActionOnTheThreadThatQuitsAndStopsWhenItIsLetGoOf() {
  let center = NotificationCenter()
  let quit = Notification.Name("test.the.app.quits.again")
  let calls = LockedCounter()
  var watch: QuitWatch? = QuitWatch(name: quit, center: center) { calls.add() }
  center.post(name: quit, object: nil)
  check(calls.value == 1, "\(calls.value)")
  // It does not answer to another notification.
  center.post(name: Notification.Name("test.something.else"), object: nil)
  check(calls.value == 1, "\(calls.value)")
  // Let go of, it stops.
  watch = nil
  center.post(name: quit, object: nil)
  check(calls.value == 1, "a watch that was let go of acted: \(calls.value)")
}

/// A count that threads add to.
final class LockedCounter: @unchecked Sendable {
  private let mutex = NSLock()
  private var count = 0

  func add() {
    mutex.lock()
    count += 1
    mutex.unlock()
  }

  var value: Int {
    mutex.lock()
    defer { mutex.unlock() }
    return count
  }
}

// MARK: - What the system's counters say

func testThreePressesOfEscapeInTwoSecondsAreTheWayOut() {
  var hatch = EscapeHatch()
  check(!hatch.pressed(at: 10, repeating: false))
  check(!hatch.pressed(at: 10.5, repeating: false))
  check(hatch.pressed(at: 11, repeating: false), "the third press")
  // The count starts again.
  check(!hatch.pressed(at: 11.2, repeating: false))
  check(!hatch.pressed(at: 11.4, repeating: false))
  check(hatch.pressed(at: 11.6, repeating: false))

  // Presses too far apart are not three presses.
  hatch = EscapeHatch()
  check(!hatch.pressed(at: 0, repeating: false))
  check(!hatch.pressed(at: 1.5, repeating: false))
  check(!hatch.pressed(at: 3, repeating: false), "the first press is more than two seconds old")
  check(hatch.pressed(at: 3.2, repeating: false), "three within two seconds again")

  // A key that is held down repeats, and the repeats are not presses.
  hatch = EscapeHatch()
  for step in 0..<20 {
    check(!hatch.pressed(at: Double(step) * 0.03, repeating: true), "a repeat counted")
  }
  check(!hatch.pressed(at: 1, repeating: false))
  check(!hatch.pressed(at: 1.1, repeating: false))
  check(hatch.pressed(at: 1.2, repeating: false))
}

func testAKeyThatTheAppDidNotPostIsFoundAndTheKeysOfTheAppAreNot() {
  // No key has ever been pressed.
  check(!Tripwire.foreignKey(lastKeyAge: nil, sinceHold: 5, postAge: nil))
  // The last key was before the hold began.
  check(!Tripwire.foreignKey(lastKeyAge: 30, sinceHold: 5, postAge: nil))
  // A key since the hold began, and the app posted none.
  check(Tripwire.foreignKey(lastKeyAge: 2, sinceHold: 5, postAge: nil))
  // The key is the last one that the app posted, a moment later by the stamp of the system.
  check(!Tripwire.foreignKey(lastKeyAge: 1.95, sinceHold: 5, postAge: 2))
  check(!Tripwire.foreignKey(lastKeyAge: 2, sinceHold: 5, postAge: 2))
  // A key after the last one of the app.
  check(Tripwire.foreignKey(lastKeyAge: 0.5, sinceHold: 5, postAge: 2))
  // The last key of the app was before the hold, and a key came after the hold began.
  check(Tripwire.foreignKey(lastKeyAge: 1, sinceHold: 5, postAge: 60))
  // The slack is a moment, and not more.
  check(Tripwire.slack > 0 && Tripwire.slack <= 0.25, "\(Tripwire.slack)")
}

func testTheLedgerSaysHowLongAgoTheAppLastPostedAKey() {
  let clock = FakeClock()
  let ledger = PostLedger(now: clock.now)
  check(ledger.secondsSinceKeyDown == nil)
  ledger.notedKeyDown()
  clock.sleep(1.5)
  check(ledger.secondsSinceKeyDown == 1.5, "\(String(describing: ledger.secondsSinceKeyDown))")
  ledger.notedKeyDown()
  check(ledger.secondsSinceKeyDown == 0, "\(String(describing: ledger.secondsSinceKeyDown))")
}

func testWhatTheOtherDeviceIsToldIsAShortSentenceThatSaysTheMacLocks() {
  let all: [PrivacyAlarm] = [.escape, .foreignKey, .holdLost, .coverLost, .guardLost]
  var seen = Set<String>()
  for alarm in all {
    let sentence = alarm.sentence
    check(
      !sentence.isEmpty && sentence.utf8.count <= Int(BATON_MESSAGE_MAX),
      "\(alarm): \(sentence.utf8.count) bytes")
    check(sentence.hasSuffix("the Mac locks."), "\(alarm): \(sentence)")
    check(!sentence.contains("\u{2014}"), "\(alarm): a dash in \(sentence)")
    check(seen.insert(sentence).inserted, "\(alarm): the same sentence as another")
  }
}
