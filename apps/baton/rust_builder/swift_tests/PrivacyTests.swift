// SPDX-License-Identifier: Apache-2.0

// Tests of BatonPrivacy.swift: when the cover of a private session goes up and when it comes down,
// what ends the session, and how the Mac is locked until it says that it is. The Mac itself is
// played by a curtain that does what the test says; what the real one does is checked on a Mac
// (docs/research/privacy-mode).

import Foundation

/// A curtain that records what it is asked, and is locked when the test says.
final class FakeCurtain: Curtain {
  private let mutex = NSLock()
  private var events: [String] = []
  private var alarm: ((PrivacyAlarm) -> Void)?
  private var isLocked = false
  private var attempts: [Int] = []
  private var lockedAfter: Int?

  var raiseFailure: Error?
  /// Rung from inside `raise`, as an alarm is when something at the Mac happens at once.
  var ringsWhileRaising: PrivacyAlarm?

  /// The Mac is locked by the try with this number, counted from 0. nil: never, until `lockNow`.
  init(locksAtAttempt: Int? = 0) {
    lockedAfter = locksAtAttempt
  }

  func raise(alarm: @escaping (PrivacyAlarm) -> Void) throws {
    mutex.lock()
    events.append("raise")
    mutex.unlock()
    if let raiseFailure { throw raiseFailure }
    mutex.lock()
    self.alarm = alarm
    mutex.unlock()
    if let ringsWhileRaising {
      alarm(ringsWhileRaising)
      Thread.sleep(forTimeInterval: 0.05)
    }
  }

  func lower() {
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
  _ curtain: Curtain, clock: FakeClock = FakeClock(), told: Told = Told(),
  patience: TimeInterval = 5
) -> PrivateSession {
  PrivateSession(
    curtain: curtain, timing: PrivacyTiming(patience: patience), end: told.tell,
    sleep: clock.sleep, now: clock.now)
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
  check(refusing.log == ["raise", "lower"], "a session that did not begin finished: \(refusing.log)")
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
    curtain: curtain, timing: PrivacyTiming(patience: 5),
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
  let all: [PrivacyAlarm] = [.escape, .foreignKey, .holdLost, .coverLost]
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
