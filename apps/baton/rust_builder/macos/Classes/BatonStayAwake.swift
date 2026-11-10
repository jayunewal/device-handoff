// SPDX-License-Identifier: Apache-2.0

// Whether a Mac stays awake while it is plugged in (docs/adr/0018-stay-awake-while-plugged-in.md).
// A Mac that sleeps cannot be reached by the other devices of the person, and nothing wakes it from
// outside its own network, so the person may ask Baton to hold the Mac awake for as long as it runs
// on the power of the wall. It is held by two things together: the person asked, and the Mac is
// plugged in. Either stopping lets the Mac sleep as System Settings says, and the hold comes back
// when both are so again.
//
// This file decides, and takes the power and the hold as functions, so that any computer with a
// Swift compiler tests it (StayAwakeTests.swift). The calls of the system, for the power and for the
// hold, are in BatonStayAwakeMac.swift.

import Foundation

/// Where staying awake stands, in the words of lib/src/stay_awake.dart.
enum StayAwakeState: String {
  /// The person did not ask for it. The Mac sleeps as System Settings says.
  case off

  /// The person asked, the Mac is plugged in, and it is held awake.
  case holding

  /// The person asked, and the Mac runs on its battery, so nothing is held until it is plugged in.
  case waiting

  /// The person asked, and Baton cannot do it: the system would not say whether the Mac is plugged
  /// in, or would not give the hold. It is tried again whenever the power changes or the person asks
  /// again.
  case refused
}

/// The hold of a Mac on its sleep, kept as the person's choice and the Mac's power ask for it. The
/// calls come from the main thread, which is where the app is answered and the system tells of a
/// change of power.
final class StayAwake {
  /// Holds the Mac awake and gives the way to let go of the hold, or nothing where the system
  /// refused.
  typealias Hold = () -> (() -> Void)?

  private let pluggedIn: () -> Bool?
  private let hold: Hold
  private let remember: (Bool) -> Void
  private var asked: Bool
  private var plugged: Bool?
  private var letGo: (() -> Void)?

  /// `asked` is the choice that the person made before, `remember` keeps the choice that they make
  /// now, `pluggedIn` says whether the Mac runs on the power of the wall (nothing where that is not
  /// known), and `hold` holds it. A Mac that is asked for at the start is held at once.
  init(
    asked: Bool, remember: @escaping (Bool) -> Void = { _ in },
    pluggedIn: @escaping () -> Bool?, hold: @escaping Hold
  ) {
    self.asked = asked
    self.remember = remember
    self.pluggedIn = pluggedIn
    self.hold = hold
    settle()
  }

  /// Where this stands now.
  var state: StayAwakeState {
    guard asked else { return .off }
    guard let plugged else { return .refused }
    guard plugged else { return .waiting }
    return letGo == nil ? .refused : .holding
  }

  /// The person's choice. It is kept for the next start of the app.
  func ask(_ on: Bool) {
    asked = on
    remember(on)
    settle()
  }

  /// The power of the Mac changed: it was plugged in or unplugged, or its battery moved.
  func powerChanged() {
    settle()
  }

  /// Makes the hold what the choice and the power ask for, and leaves it alone if it is.
  private func settle() {
    plugged = pluggedIn()
    if asked && plugged == true {
      if letGo == nil {
        letGo = hold()
      }
    } else if let release = letGo {
      letGo = nil
      release()
    }
  }
}
