// SPDX-License-Identifier: Apache-2.0

// Tests of the Mac's sharer (BatonCapture.swift) with the system itself, which only a Mac has: on
// other systems this file is empty. What the system answers depends on what the person at the
// computer has allowed, so each answer that is right is accepted, and what is given is checked to
// be whole.

#if os(macOS)

import Foundation

func testTheMacGivesItsMainScreenOrSaysWhyItCannot() {
  guard #available(macOS 14.0, *) else { return }
  let sharer = MacSharer()
  check(sharer.canGiveScreen, "a Mac that cannot give its screen")
  do {
    let given = try sharer.screen()
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

#endif
