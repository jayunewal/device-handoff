// SPDX-License-Identifier: Apache-2.0

// Tests of the places that windows are drawn on, and of trying again until they are there
// (BatonViewer.swift). A Mac's Flutter gives no texture until its engine runs, which is after the
// plugin is registered, and no test but the app on a Mac can see that. These play it.

import Foundation

/// A place that a window is drawn on, standing in for a texture.
final class Place {
  let name: String
  init(_ name: String) { self.name = name }
}

/// A `make` that answers what a script says, one answer for each call, and then nothing.
final class ScriptedMake {
  private let lock = NSLock()
  private var script: [Place?]
  private(set) var calls = 0

  init(_ script: [Place?]) { self.script = script }

  func callAsFunction() -> Place? {
    lock.lock()
    defer { lock.unlock() }
    calls += 1
    return script.isEmpty ? nil : script.removeFirst()
  }
}

func testPlacesAreMadeWhenAskedAndOnlyTheMissingOnesAre() {
  let make = ScriptedMake([Place("a"), Place("b"), Place("c")])
  let places = ScreenPlaces<Place>(count: 2) { make() }
  check(places.status == PlacesStatus(made: 0, wanted: 2, tries: 0), "\(places.status)")
  check(places.take() == nil, "nothing is made before fill is called")
  check(places.fill())
  check(places.status == PlacesStatus(made: 2, wanted: 2, tries: 1), "\(places.status)")
  check(make.calls == 2, "\(make.calls) places were made for two wanted")
  check(places.fill())
  check(make.calls == 2, "a full set is not made again")
  check(places.status.tries == 2)
}

func testAPlaceThatCannotBeMadeStopsTheFillingAndIsTriedAgain() {
  let make = ScriptedMake([Place("a"), nil, Place("b"), Place("c")])
  let places = ScreenPlaces<Place>(count: 3) { make() }
  check(!places.fill(), "the second place was refused")
  check(places.status.made == 1, "\(places.status)")
  check(make.calls == 2, "no third call after the refusal in the same fill, but \(make.calls)")
  check(places.take()?.name == "a", "the place that was made is there before the others are")
  check(places.fill(), "the places that were missing are made the next time")
  check(places.status == PlacesStatus(made: 3, wanted: 3, tries: 2), "\(places.status)")
  check(make.calls == 4, "only the missing places are asked for, but \(make.calls) calls")
  check(places.take()?.name == "b")
  check(places.take()?.name == "c")
  check(places.take() == nil)
}

func testAPlaceIsUsedByOneWindowAtATime() {
  let a = Place("a")
  let b = Place("b")
  let make = ScriptedMake([a, b])
  let places = ScreenPlaces<Place>(count: 2) { make() }
  check(places.fill())
  check(places.take() === a)
  check(places.take() === b)
  check(places.take() == nil, "both are in use")
  places.give(a)
  check(places.take() === a, "a place that is given back is there for the next window")
  check(places.take() == nil)
  places.give(a)
  places.give(a)
  check(places.take() === a, "giving a place back twice frees it once")
  check(places.take() == nil, "and b is still in use")
  places.give(Place("c"))
  check(places.take() == nil, "a place that is not one of these changes nothing")
}

func testNoPlaceIsTakenBeforeAnyIsMade() {
  let places = ScreenPlaces<Place>(count: 2) { nil }
  check(!places.fill())
  check(places.take() == nil)
  check(places.status == PlacesStatus(made: 0, wanted: 2, tries: 1), "\(places.status)")
}

func testACountBelowOneWantsNothing() {
  let make = ScriptedMake([Place("a")])
  let places = ScreenPlaces<Place>(count: -3) { make() }
  check(places.fill(), "nothing is wanted, so nothing is missing")
  check(make.calls == 0)
  check(places.status == PlacesStatus(made: 0, wanted: 0, tries: 1), "\(places.status)")
  check(places.take() == nil)
}

/// Who holds which place, seen from the threads that hold them.
final class Holders: @unchecked Sendable {
  private let lock = NSLock()
  private var held = Set<ObjectIdentifier>()
  private(set) var most = 0
  private(set) var twice = 0

  func took(_ place: Place) {
    lock.lock()
    defer { lock.unlock() }
    if !held.insert(ObjectIdentifier(place)).inserted { twice += 1 }
    most = max(most, held.count)
  }

  func gaveBack(_ place: Place) {
    lock.lock()
    held.remove(ObjectIdentifier(place))
    lock.unlock()
  }
}

/// A `make` that takes its time, and keeps count of how many are being made at once.
final class SlowMake: @unchecked Sendable {
  private let lock = NSLock()
  private var active = 0
  private(set) var most = 0
  private(set) var calls = 0

  func callAsFunction() -> Place? {
    lock.lock()
    active += 1
    calls += 1
    most = max(most, active)
    lock.unlock()
    usleep(2000)
    lock.lock()
    active -= 1
    lock.unlock()
    return Place("slow")
  }
}

func testFillsAtTheSameTimeMakeEachPlaceOnce() {
  let make = SlowMake()
  let places = ScreenPlaces<Place>(count: 3) { make() }
  let group = DispatchGroup()
  for _ in 0..<4 {
    group.enter()
    Thread.detachNewThread {
      places.fill()
      group.leave()
    }
  }
  check(group.wait(timeout: .now() + 30) == .success, "the threads finished")
  check(make.calls == 3, "\(make.calls) places were made for three wanted")
  check(make.most == 1, "\(make.most) places were being made at the same time")
  check(places.status == PlacesStatus(made: 3, wanted: 3, tries: 4), "\(places.status)")
}

/// Counts, from any thread.
final class Counter: @unchecked Sendable {
  private let lock = NSLock()
  private var value = 0

  func next() -> Int {
    lock.lock()
    defer { lock.unlock() }
    value += 1
    return value
  }
}

func testAPlaceCanBeTakenWhileTheOthersAreStillBeingMade() {
  // Flutter can take a moment for a texture, and a window that is offered meanwhile does not wait
  // for the whole set.
  let second = DispatchSemaphore(value: 0)
  let counter = Counter()
  let places = ScreenPlaces<Place>(count: 2) {
    let number = counter.next()
    if number == 2 { second.wait() }
    return Place("place \(number)")
  }
  let group = DispatchGroup()
  group.enter()
  Thread.detachNewThread {
    places.fill()
    group.leave()
  }
  var first: Place?
  check(
    eventually {
      first = places.take()
      return first != nil
    }, "the first place is there while the second is being made")
  check(first?.name == "place 1")
  check(places.status.made == 1, "\(places.status)")
  check(places.take() == nil, "and the second is not there yet")
  second.signal()
  check(group.wait(timeout: .now() + 30) == .success, "the fill finished")
  check(places.take()?.name == "place 2")
  check(places.status.made == 2)
}

func testPlacesAreTakenAndGivenFromManyThreadsWithoutLosingOne() {
  let make = ScriptedMake([Place("a"), Place("b")])
  let places = ScreenPlaces<Place>(count: 2) { make() }
  check(places.fill())
  let holders = Holders()
  let group = DispatchGroup()
  for _ in 0..<8 {
    group.enter()
    Thread.detachNewThread {
      for turn in 0..<2000 {
        guard let place = places.take() else { continue }
        holders.took(place)
        if turn % 16 == 0 { usleep(100) }
        holders.gaveBack(place)
        places.give(place)
      }
      group.leave()
    }
  }
  check(group.wait(timeout: .now() + 30) == .success, "the threads finished")
  check(holders.most <= 2, "\(holders.most) windows at once for two places")
  check(holders.twice == 0, "\(holders.twice) times a place was given to two windows")
  check(places.take() != nil && places.take() != nil, "both places are free at the end")
  check(places.take() == nil)
}

/// `later` for a test: the work waits in a queue until the test runs it, and the delays asked for
/// are kept.
final class Waiting {
  private(set) var delays: [TimeInterval] = []
  private var queue: [() -> Void] = []

  func later(_ delay: TimeInterval, _ work: @escaping () -> Void) {
    delays.append(delay)
    queue.append(work)
  }

  /// Runs the work that is waiting, one piece. false if there was none.
  func runNext() -> Bool {
    if queue.isEmpty { return false }
    queue.removeFirst()()
    return true
  }

  var waiting: Int { queue.count }
}

func testAnAttemptThatWorksIsNotRepeated() {
  let waiting = Waiting()
  var attempts = 0
  var gaveUp = 0
  keepTrying(
    after: [1, 2], later: waiting.later,
    attempt: {
      attempts += 1
      return true
    }, giveUp: { gaveUp += 1 })
  check(attempts == 1)
  check(waiting.waiting == 0, "nothing was waited for")
  check(gaveUp == 0)
}

func testAnAttemptThatDoesNotWorkIsRepeatedAfterEachDelayAndThenGivenUp() {
  let waiting = Waiting()
  var attempts = 0
  var gaveUp = 0
  keepTrying(
    after: [0.5, 1, 2], later: waiting.later,
    attempt: {
      attempts += 1
      return false
    }, giveUp: { gaveUp += 1 })
  check(attempts == 1, "the first attempt is made at once, and the others are waited for")
  check(waiting.delays == [0.5], "\(waiting.delays)")
  check(waiting.runNext())
  check(attempts == 2 && waiting.delays == [0.5, 1], "\(attempts) \(waiting.delays)")
  check(waiting.runNext())
  check(attempts == 3 && waiting.delays == [0.5, 1, 2], "\(attempts) \(waiting.delays)")
  check(gaveUp == 0, "it has not given up while there is a delay left")
  check(waiting.runNext())
  check(attempts == 4, "one attempt for the first and one after each delay, but \(attempts)")
  check(gaveUp == 1, "it gives up after the last attempt")
  check(!waiting.runNext(), "and waits for nothing more")
}

func testTryingStopsAsSoonAsAnAttemptWorks() {
  let waiting = Waiting()
  var attempts = 0
  var gaveUp = 0
  keepTrying(
    after: [1, 1, 1, 1], later: waiting.later,
    attempt: {
      attempts += 1
      return attempts == 3
    }, giveUp: { gaveUp += 1 })
  while waiting.runNext() {}
  check(attempts == 3, "\(attempts)")
  check(waiting.delays == [1, 1], "\(waiting.delays)")
  check(gaveUp == 0)
}

func testNoDelaysMeansOneAttempt() {
  let waiting = Waiting()
  var attempts = 0
  var gaveUp = 0
  keepTrying(
    after: [], later: waiting.later,
    attempt: {
      attempts += 1
      return false
    }, giveUp: { gaveUp += 1 })
  check(attempts == 1)
  check(gaveUp == 1)
  check(waiting.waiting == 0)
}

func testPlacesAreMadeOnceTheEngineRunsAsTheMacDoes() {
  // The engine of Flutter on a Mac starts after the plugin is registered, and takes no texture
  // before: the first tries are refused, and a later one is not.
  var engineRuns = false
  var made = 0
  let places = ScreenPlaces<Place>(count: 2) {
    if !engineRuns { return nil }
    made += 1
    return Place("texture \(made)")
  }
  let waiting = Waiting()
  var gaveUp = 0
  keepTrying(
    after: [0.05, 0.1, 0.2, 0.4], later: waiting.later, attempt: { places.fill() },
    giveUp: { gaveUp += 1 })
  check(places.status.made == 0, "the engine does not run yet")
  check(waiting.runNext())
  check(places.status.made == 0)
  engineRuns = true
  check(waiting.runNext())
  check(places.status == PlacesStatus(made: 2, wanted: 2, tries: 3), "\(places.status)")
  check(!waiting.runNext(), "no more tries once the places are there")
  check(gaveUp == 0)
  check(places.take() != nil)
}

func testPlacesThatNeverComeAreGivenUpOnAndAskedForAgainWhenAWindowComes() {
  var engineRuns = false
  let places = ScreenPlaces<Place>(count: 1) { engineRuns ? Place("texture") : nil }
  let waiting = Waiting()
  var gaveUp = 0
  keepTrying(
    after: [0.05, 0.1], later: waiting.later, attempt: { places.fill() }, giveUp: { gaveUp += 1 })
  while waiting.runNext() {}
  check(gaveUp == 1)
  check(places.status.made == 0 && places.status.tries == 3, "\(places.status)")
  // A window is offered much later, when the engine runs: the device asks again, once.
  engineRuns = true
  check(places.fill())
  check(places.take() != nil)
}
