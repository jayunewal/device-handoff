// SPDX-License-Identifier: Apache-2.0

// Tests of the Mac's sharer (BatonCapture.swift) with the system itself, which only a Mac has: on
// other systems this file is empty. What the system answers depends on what the person at the
// computer has allowed, so each answer that is right is accepted, and what is given is checked to
// be whole.

#if os(macOS)

import CoreGraphics
import Foundation

func testTheMacGivesItsMainScreenOrSaysWhyItCannot() {
  guard #available(macOS 14.0, *) else { return }
  let sharer = MacSharer()
  check(sharer.canGiveScreen, "a Mac that cannot give its screen")
  check(sharer.canLeave, "a Mac that cannot leave itself as it is")
  do {
    let given = try sharer.screen(privacy: .open)
    check(given.info.display, "what was given is not a display")
    check(given.info.width >= 1 && given.info.height >= 1, "\(given.info)")
    check(given.info.title == "" && given.info.app == "", "a screen that was named: \(given.info)")
    // Given, and never used: it is let go of like a window.
    given.window.released()
  } catch let refusal as ScreenRefusal {
    check(refusal == .needsPermission || refusal == .unavailable, "\(refusal)")
  } catch let failure as SharerFailure {
    check(
      !failure.sentence.isEmpty && failure.sentence.utf8.count <= Int(BATON_MESSAGE_MAX),
      "a sentence of \(failure.sentence.utf8.count) bytes")
  } catch {
    check(false, "an error that is not told: \(error)")
  }
}

/// The highest output of the tables of the displays that are online, from 0 to 1, or nil if no
/// table can be read.
private func highestTableOutput() -> Float? {
  var ids = [CGDirectDisplayID](repeating: 0, count: 32)
  var count: UInt32 = 0
  guard CGGetOnlineDisplayList(UInt32(ids.count), &ids, &count) == .success else { return nil }
  var highest: Float?
  for id in ids.prefix(Int(count)) {
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
        id, &redMin, &redMax, &redGamma, &greenMin, &greenMax, &greenGamma, &blueMin, &blueMax,
        &blueGamma) == .success
    else { continue }
    highest = max(highest ?? 0, redMax, greenMax, blueMax)
  }
  return highest
}

func testAPrivateScreenIsGivenOnlyToAnAppThatCanKeepItPrivateAndChangesNothingUntilItStarts() {
  guard #available(macOS 14.0, *) else { return }
  let sharer = MacSharer()
  let before = highestTableOutput()
  do {
    // Given only if the system let the app hold the keys and the pointer, and record the screen.
    let given = try sharer.screen(privacy: .private)
    check(given.info.display, "what was given is not a display")
    // Given, and never started: nothing is raised, so there is nothing to lift, and a lift does
    // not change the screens.
    given.window.lift(screen: true, input: true)
    // Let go of, and it raised nothing.
    given.window.released()
  } catch let refusal as ScreenRefusal {
    // What the system has not allowed is told as such, and nothing is given in its place.
    check(
      refusal == .needsPermission || refusal == .needsInputPermission || refusal == .unavailable,
      "\(refusal)")
  } catch let failure as SharerFailure {
    check(!failure.sentence.isEmpty, "a failure with no sentence")
  } catch {
    check(false, "an error that is not told: \(error)")
  }
  check(highestTableOutput() == before, "a screen that never started changed the screens")
}

#endif
