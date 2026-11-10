// SPDX-License-Identifier: Apache-2.0

// Tests of staying awake while plugged in (BatonStayAwake.swift): the hold is taken when the person
// asked and the Mac is plugged in, let go of when either stops being so, taken once and let go of
// once, tried again when the system refused, and the choice is kept. The power and the hold are
// played by a fake Mac. What the Mac itself says is checked in StayAwakeMacTests.swift.

import Foundation

/// A Mac whose power and whose hold are played: what it says of its power, whether the system gives
/// a hold, and the holds that were taken and let go of.
final class SleepyMac {
  /// What the system says of the power: plugged in, on the battery, or nothing.
  var plugged: Bool? = true

  /// Whether the system gives a hold.
  var gives = true

  private(set) var taken = 0
  private(set) var released = 0

  /// The choices that were kept, in order.
  private(set) var remembered: [Bool] = []

  /// How many holds are on now.
  var held: Int { taken - released }

  func stay(asked: Bool) -> StayAwake {
    StayAwake(
      asked: asked, remember: { self.remembered.append($0) },
      pluggedIn: { self.plugged },
      hold: {
        guard self.gives else { return nil }
        self.taken += 1
        return { self.released += 1 }
      })
  }
}

func testNothingIsHeldUntilThePersonAsks() {
  let mac = SleepyMac()
  let stay = mac.stay(asked: false)
  check(stay.state == .off, "\(stay.state)")
  check(mac.taken == 0, "a hold that nobody asked for: \(mac.taken)")
  stay.powerChanged()
  check(
    mac.taken == 0 && stay.state == .off, "a change of power held the Mac: \(mac.taken) holds")
}

func testAMacThatIsPluggedInIsHeldWhenAskedAndOnlyOnce() {
  let mac = SleepyMac()
  let stay = mac.stay(asked: false)
  stay.ask(true)
  check(stay.state == .holding, "\(stay.state)")
  check(mac.held == 1, "held \(mac.held)")
  stay.ask(true)
  stay.powerChanged()
  stay.powerChanged()
  check(mac.taken == 1, "a hold for each call: \(mac.taken)")
  check(mac.held == 1, "held \(mac.held)")
  check(mac.remembered == [true, true], "\(mac.remembered)")
}

func testAChoiceFromBeforeIsHeldAtTheStart() {
  let mac = SleepyMac()
  let stay = mac.stay(asked: true)
  check(stay.state == .holding, "\(stay.state)")
  check(mac.held == 1, "held \(mac.held)")
  check(mac.remembered.isEmpty, "a choice that was kept again at the start: \(mac.remembered)")
}

func testAMacOnItsBatteryIsNotHeldAndIsHeldOnceItIsPluggedIn() {
  let mac = SleepyMac()
  mac.plugged = false
  let stay = mac.stay(asked: true)
  check(stay.state == .waiting, "\(stay.state)")
  check(mac.taken == 0, "a Mac on its battery was held: \(mac.taken)")
  mac.plugged = true
  stay.powerChanged()
  check(stay.state == .holding, "\(stay.state)")
  check(mac.held == 1, "held \(mac.held)")
}

func testAHoldIsLetGoOfWhenTheMacIsUnpluggedAndTakenAgainWhenItIsPluggedIn() {
  let mac = SleepyMac()
  let stay = mac.stay(asked: true)
  check(mac.held == 1, "held \(mac.held)")
  mac.plugged = false
  stay.powerChanged()
  check(stay.state == .waiting, "\(stay.state)")
  check(mac.held == 0 && mac.released == 1, "held \(mac.held), let go of \(mac.released)")
  stay.powerChanged()
  check(mac.released == 1, "let go of twice: \(mac.released)")
  mac.plugged = true
  stay.powerChanged()
  check(stay.state == .holding, "\(stay.state)")
  check(mac.taken == 2 && mac.held == 1, "taken \(mac.taken), held \(mac.held)")
}

func testAHoldIsLetGoOfWhenThePersonTurnsItOffAndStaysOff() {
  let mac = SleepyMac()
  let stay = mac.stay(asked: true)
  stay.ask(false)
  check(stay.state == .off, "\(stay.state)")
  check(mac.held == 0 && mac.released == 1, "held \(mac.held), let go of \(mac.released)")
  check(mac.remembered == [false], "\(mac.remembered)")
  // The Mac is unplugged and plugged in again, and the hold that the person turned off stays off.
  mac.plugged = false
  stay.powerChanged()
  mac.plugged = true
  stay.powerChanged()
  check(mac.taken == 1, "a hold came back that the person turned off: \(mac.taken) holds")
}

func testAHoldIsLetGoOfWhenThePersonTurnsItOffOnTheBattery() {
  let mac = SleepyMac()
  mac.plugged = false
  let stay = mac.stay(asked: true)
  stay.ask(false)
  check(stay.state == .off, "\(stay.state)")
  check(mac.taken == 0 && mac.released == 0, "taken \(mac.taken), let go of \(mac.released)")
}

func testASystemThatRefusesTheHoldIsSaidSoAndAskedAgain() {
  let mac = SleepyMac()
  mac.gives = false
  let stay = mac.stay(asked: true)
  check(stay.state == .refused, "\(stay.state)")
  check(mac.taken == 0, "taken \(mac.taken)")
  // The system gives it the next time that it is asked, whether the power changes or the person asks
  // again.
  mac.gives = true
  stay.powerChanged()
  check(stay.state == .holding, "\(stay.state)")
  check(mac.held == 1, "held \(mac.held)")

  let other = SleepyMac()
  other.gives = false
  let again = other.stay(asked: false)
  again.ask(true)
  check(again.state == .refused, "\(again.state)")
  other.gives = true
  again.ask(true)
  check(again.state == .holding && other.held == 1, "\(again.state), held \(other.held)")
}

func testAPowerThatTheSystemDoesNotSayIsNotHeldAndIsSaidSo() {
  let mac = SleepyMac()
  mac.plugged = nil
  let stay = mac.stay(asked: true)
  check(stay.state == .refused, "\(stay.state)")
  check(mac.taken == 0, "a Mac whose power is not known was held: \(mac.taken)")
  mac.plugged = true
  stay.powerChanged()
  check(stay.state == .holding && mac.held == 1, "\(stay.state), held \(mac.held)")
  // The system stops saying: what was held is let go of, since nobody knows that the Mac is plugged in.
  mac.plugged = nil
  stay.powerChanged()
  check(stay.state == .refused, "\(stay.state)")
  check(mac.held == 0, "a Mac whose power is not known is still held: \(mac.held)")
  // A Mac that the person did not ask for says nothing of it.
  let off = SleepyMac()
  off.plugged = nil
  check(off.stay(asked: false).state == .off, "a Mac that was not asked for")
}

func testTheWordsOfTheStatesAreTheOnesOfTheApp() {
  // They are written in lib/src/stay_awake.dart, and a word that differs is a state that the app does
  // not know.
  check(StayAwakeState.off.rawValue == "off")
  check(StayAwakeState.holding.rawValue == "holding")
  check(StayAwakeState.waiting.rawValue == "waiting")
  check(StayAwakeState.refused.rawValue == "refused")
}
