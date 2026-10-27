// SPDX-License-Identifier: Apache-2.0

// Tests of what a private session says of whether its privacy is in place (BatonPrivacy.swift): the
// word that an unlock of the screen from another device goes by. It is yes only while the cover,
// the hold of the keys and the pointer, and the guard are all in place and the person at the other
// device lifted nothing, and the library is told each time that changes, in the order it changed,
// and never with a lock of the session held. The curtain is played by the fake of PrivacyTests.swift.

import Foundation

/// What `placed` was told, in order, from whatever thread.
final class Placed: @unchecked Sendable {
  private let mutex = NSLock()
  private var words: [Bool] = []

  func tell(_ word: Bool) {
    mutex.lock()
    words.append(word)
    mutex.unlock()
  }

  var all: [Bool] {
    mutex.lock()
    defer { mutex.unlock() }
    return words
  }
}

private func sessionTelling(
  _ placed: Placed, curtain: Curtain, told: Told = Told(), clock: FakeClock = FakeClock()
) -> PrivateSession {
  PrivateSession(
    curtain: curtain, guardian: FakeGuard(), timing: PrivacyTiming(patience: 5, beat: 0.005),
    end: told.tell, sleep: clock.sleep, now: clock.now, placed: placed.tell)
}

func testTheSessionSaysItsPrivacyIsInPlaceOnlyWhileEverythingIsAndTellsEachChange() {
  let curtain = FakeCurtain()
  let placed = Placed()
  let privacy = sessionTelling(placed, curtain: curtain)
  check(!privacy.inPlace, "before the cover is up")
  check(placed.all.isEmpty, "told before anything happened: \(placed.all)")
  do {
    try privacy.begin()
  } catch {
    check(false, "the cover could not be raised: \(error)")
  }
  check(privacy.inPlace, "with the cover up")
  check(placed.all == [true], "\(placed.all)")

  // The person at the other device lifts the screens, then the keys too, and puts both back.
  privacy.lift(screen: true, input: false)
  check(!privacy.inPlace && placed.all == [true, false], "\(placed.all)")
  privacy.lift(screen: true, input: true)
  check(!privacy.inPlace && placed.all == [true, false], "a word for no change: \(placed.all)")
  privacy.lift(screen: false, input: false)
  check(privacy.inPlace && placed.all == [true, false, true], "\(placed.all)")
  privacy.lift(screen: false, input: true)
  check(!privacy.inPlace && placed.all == [true, false, true, false], "\(placed.all)")
  privacy.lift(screen: false, input: false)
  check(privacy.inPlace && placed.all == [true, false, true, false, true], "\(placed.all)")

  // A curtain that is not looked after is not in place, but nothing is told: the word is for the
  // changes that the session makes, and the platform asks again before it acts.
  curtain.healthy = false
  check(!privacy.inPlace, "a curtain that nobody looks after")
  check(placed.all == [true, false, true, false, true], "\(placed.all)")
  curtain.healthy = true
  check(privacy.inPlace)

  privacy.finish(.sessionOver)
  check(!privacy.inPlace, "after the end")
  check(
    placed.all == [true, false, true, false, true, false],
    "the end was not told: \(placed.all)")
  // Nothing is lifted or raised once it is over.
  privacy.lift(screen: true, input: true)
  check(!privacy.inPlace && placed.all.count == 6, "\(placed.all)")
}

func testASessionThatCouldNotBeRaisedNeverSaysThatItsPrivacyIsInPlace() {
  let curtain = FakeCurtain()
  curtain.raiseFailure = SharerFailure(sentence: "The screens did not go dark.")
  let placed = Placed()
  let privacy = sessionTelling(placed, curtain: curtain)
  do {
    try privacy.begin()
    check(false, "a cover that could not be raised was raised")
  } catch {}
  check(!privacy.inPlace)
  check(placed.all.isEmpty, "told that it is in place: \(placed.all)")
  // And one that was only given and let go of.
  let released = Placed()
  let other = sessionTelling(released, curtain: FakeCurtain())
  other.finish(.released)
  check(!other.inPlace && released.all.isEmpty, "\(released.all)")
}

func testSomethingAtTheMacTakesThePrivacyOutOfPlaceBeforeTheMacIsLockedAndNothingPutsItBack() {
  let curtain = FakeCurtain(locksAtAttempt: 0)
  let placed = Placed()
  let told = Told()
  let privacy = sessionTelling(placed, curtain: curtain, told: told)
  try? privacy.begin()
  check(placed.all == [true])
  curtain.ring(.coverLost)
  check(told.wait(), "the other device was not told")
  check(eventually { placed.all == [true, false] }, "\(placed.all)")
  check(!privacy.inPlace)
  check(eventually { curtain.lowered == 1 }, "\(curtain.log)")
  // This is no private session any more, whatever the person at the other device does.
  privacy.lift(screen: false, input: false)
  check(!privacy.inPlace && placed.all == [true, false], "\(placed.all)")
}

func testTheLibraryIsNeverToldWithALockOfTheSessionHeld() {
  let curtain = FakeCurtain()
  let clock = FakeClock()
  let results = Placed()
  var privacy: PrivateSession?
  // The session is asked for its own state from another thread while it speaks: a lock that is
  // held by the thread that speaks would not let that thread answer.
  let placed: (Bool) -> Void = { _ in
    guard let session = privacy else { return }
    let done = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
      _ = session.inPlace
      done.signal()
    }
    results.tell(done.wait(timeout: .now() + 5) == .success)
  }
  privacy = PrivateSession(
    curtain: curtain, guardian: FakeGuard(), timing: PrivacyTiming(patience: 5, beat: 0.005),
    end: { _ in }, sleep: clock.sleep, now: clock.now, placed: placed)
  try? privacy?.begin()
  privacy?.lift(screen: true, input: false)
  privacy?.lift(screen: false, input: false)
  privacy?.finish(.sessionOver)
  check(results.all == [true, true, true, true], "\(results.all)")
  privacy = nil
}

func testWordsThatComeFromManyThreadsAreToldInOrderAndTheLastIsHowThingsStand() {
  let curtain = FakeCurtain()
  let placed = Placed()
  let privacy = sessionTelling(placed, curtain: curtain)
  try? privacy.begin()
  DispatchQueue.concurrentPerform(iterations: 4) { thread in
    for round in 0..<100 {
      privacy.lift(screen: (round + thread) % 2 == 0, input: (round + thread) % 3 == 0)
    }
  }
  // Whatever order the lifts were made in, the words are changes: no word repeats the one before,
  // and the last one says what the session says now.
  let words = placed.all
  check(
    zip(words, words.dropFirst()).allSatisfy { $0 != $1 }, "a word that changed nothing: \(words)")
  privacy.lift(screen: false, input: false)
  check(privacy.inPlace)
  check(placed.all.last == true, "\(placed.all.suffix(4))")
  check(placed.all.last == privacy.inPlace)
}

func testTheCoverIsPutOnAgainOnlyForASessionThatIsRaisedAndTheAnswerIsWhatTheCurtainSays() {
  let curtain = FakeCurtain()
  let privacy = sessionTelling(Placed(), curtain: curtain)
  check(!privacy.darken(), "before the cover is up")
  check(curtain.darkens == 0, "the curtain was asked though nothing is raised")
  try? privacy.begin()
  check(privacy.darken(), "with the cover up")
  check(curtain.darkens == 1)
  curtain.screensAreDark = false
  check(!privacy.darken(), "a screen that is not dark")
  curtain.screensAreDark = true
  privacy.finish(.sessionOver)
  let asked = curtain.darkens
  check(!privacy.darken(), "after the end")
  check(curtain.darkens == asked, "the curtain was asked after the end")
}

/// A curtain that says nothing of putting the cover on once more: the default answers for it.
final class BareCurtain: Curtain {
  var healthy = true
  var locked: Bool { true }
  func raise(alarm: @escaping (PrivacyAlarm) -> Void) throws {}
  func lift(screen: Bool, input: Bool) throws {}
  func lower() {}
  func lock(attempt: Int) {}
}

func testACurtainThatCannotPutTheCoverOnAgainIsDarkWhileItIsWell() {
  let curtain = BareCurtain()
  check(curtain.darken(), "a curtain that is well")
  curtain.healthy = false
  check(!curtain.darken(), "a curtain that is not")
}
