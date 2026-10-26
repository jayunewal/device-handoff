// SPDX-License-Identifier: Apache-2.0

// The rules of the privacy mode (docs/adr/0011-privacy-mode.md): a person who uses their Mac from a
// phone or a tablet while other people are at home. While the session lasts the screens of the Mac
// show nothing, nobody at the Mac can use its keyboard or its pointer, and when the session ends,
// whatever the reason, the Mac is locked again and only then does it show anything.
//
// This file holds what does not depend on the frameworks of a Mac: when the cover goes up and comes
// down, what ends a session, how the Mac is locked until the system says that it is. The Mac itself
// (the cover, the tap that holds the input, the calls that lock) is given to it as a protocol, and
// is in BatonPrivacyMac.swift. So that this file can be built and tested anywhere
// (apps/baton/rust_builder/swift_tests), nothing here may import more than Foundation.

import Foundation

// MARK: - What the Mac does

/// Something at the Mac that ends a private session.
enum PrivacyAlarm: Equatable {
  /// The person at the Mac pressed Escape three times, which is the way out of a Mac that shows
  /// nothing and takes no keys.
  case escape
  /// A key went down at the Mac that this app did not hand to the system and that the tap did not
  /// hold, as a key does while another program has the keyboard to itself (a password field).
  case foreignKey
  /// The hold on the keyboard and the pointer was lost, for example because the person took the
  /// consent back.
  case holdLost
  /// A screen could not be kept dark.
  case coverLost

  /// What the person at the other device is told.
  var sentence: String {
    switch self {
    case .escape:
      return "Somebody at the Mac pressed Escape three times. The session ended, and the Mac locks."
    case .foreignKey:
      return "A key was pressed at the Mac. The session ended, and the Mac locks."
    case .holdLost:
      return "The Mac could not keep its keyboard and pointer out of use. The session ended, and "
        + "the Mac locks."
    case .coverLost:
      return "The Mac could not keep its screens dark. The session ended, and the Mac locks."
    }
  }
}

/// What a private session uses of the Mac. BatonPrivacyMac.swift has the real one.
protocol Curtain: AnyObject {
  /// Covers every screen with black, takes the keyboard and the pointer out of anyone's hands but
  /// this app's, and keeps the screens awake. `alarm` is called, from any thread and without
  /// waiting, when something at the Mac needs the session ended. Throws `SharerFailure` with a
  /// sentence if it cannot, and then nothing is left raised. Called once.
  func raise(alarm: @escaping (PrivacyAlarm) -> Void) throws

  /// Takes the cover down and lets go of the keyboard, the pointer and the screens. It may be
  /// called when nothing is raised.
  func lower()

  /// Whether the system says that the Mac is locked.
  var locked: Bool { get }

  /// One try at locking the Mac. The Mac has more than one way, and the later numbers are tried
  /// when the earlier ones did not lock it. It does not wait for the Mac to be locked.
  func lock(attempt: Int)
}

// MARK: - The rules

/// The time a private session gives each step, in seconds. The tests make it short.
struct PrivacyTiming {
  /// How long the Mac has to say that it is locked after a try, before the next way is tried.
  var settle: TimeInterval = 2
  /// How often the Mac is asked while it is waited for.
  var poll: TimeInterval = 0.1
  /// How long the cover stays after the Mac says that it is locked, so that the lock screen is
  /// drawn before the cover goes: a screen that shows the desktop for a moment is not private.
  var afterLock: TimeInterval = 1
  /// How long `finish` waits for the Mac to be locked and the cover to come down before it
  /// returns. The locking goes on, and the cover stays, until the Mac is locked.
  var patience: TimeInterval = 10
}

/// The privacy of one session that another device controls: the cover goes up when the session
/// starts and comes down only when the Mac is locked.
///
/// A Mac that cannot be locked keeps its cover and goes on trying. That is the way of failing that
/// shows nothing to the people who are at home, and the person at the Mac has the power button.
final class PrivateSession {
  /// Why a session is finished.
  enum Cause: Equatable {
    /// The session is over, whoever ended it.
    case sessionOver
    /// The window is let go of: it never ran, or it did not start.
    case released
    /// Something at the Mac ended it.
    case alarm(PrivacyAlarm)
  }

  private enum State {
    case idle, raised, finishing, done
  }

  private let curtain: Curtain
  private let end: (String) -> Void
  private let timing: PrivacyTiming
  private let sleep: (TimeInterval) -> Void
  private let now: () -> TimeInterval
  private let log: (String) -> Void

  /// Held for the whole of `begin`, so that a finish that is asked for meanwhile (an alarm can come
  /// from the first moment of the hold) waits for the cover to be up.
  private let condition = NSCondition()
  private var state = State.idle

  /// `end` ends the session at the other device with a sentence for the person there. `sleep` and
  /// `now` are the clocks, which the tests replace.
  init(
    curtain: Curtain, timing: PrivacyTiming = PrivacyTiming(), end: @escaping (String) -> Void,
    sleep: @escaping (TimeInterval) -> Void = { Thread.sleep(forTimeInterval: $0) },
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
    log: @escaping (String) -> Void = { _ in }
  ) {
    self.curtain = curtain
    self.timing = timing
    self.end = end
    self.sleep = sleep
    self.now = now
    self.log = log
  }

  /// Puts the cover up and takes the keyboard and the pointer. Throws `SharerFailure` if the Mac
  /// cannot, and then nothing is left raised and the session cannot be raised again.
  func begin() throws {
    condition.lock()
    defer { condition.unlock() }
    guard state == .idle else {
      throw SharerFailure(sentence: "This Mac could not keep its screen private.")
    }
    do {
      try curtain.raise { [weak self] alarm in self?.alarmed(alarm) }
    } catch {
      curtain.lower()
      state = .done
      condition.broadcast()
      throw error
    }
    state = .raised
    log("the cover is up")
  }

  /// The session is over, or the window is let go of, or something at the Mac ended it: the Mac is
  /// locked, and then the cover comes down. Returns when that is done, or after the patience of the
  /// timing, in which case the Mac goes on being locked on a thread of its own.
  ///
  /// Every cause finishes the session once. A second call returns at once, unless it is the end of
  /// the session and the first call was not: that one waits for the first, so that nothing that
  /// follows the end of a session happens while the Mac is not yet locked. An alarm never waits,
  /// since nobody is held up by it.
  func finish(_ cause: Cause) {
    condition.lock()
    switch state {
    case .idle:
      // Nothing was raised, and nothing is, ever: a window that was only given and let go of
      // changes nothing at the Mac.
      state = .done
      condition.broadcast()
      condition.unlock()
      return
    case .done:
      condition.unlock()
      return
    case .raised:
      state = .finishing
      condition.unlock()
      launch(cause)
      if case .alarm = cause { return }
      condition.lock()
    case .finishing:
      guard cause == .sessionOver else {
        condition.unlock()
        return
      }
    }
    let deadline = Date(timeIntervalSinceNow: timing.patience)
    while state != .done {
      if !condition.wait(until: deadline) { break }
    }
    condition.unlock()
  }

  /// Something at the Mac needs the session ended. It does not wait: it is called from the thread
  /// of the tap, which must answer at once.
  private func alarmed(_ alarm: PrivacyAlarm) {
    DispatchQueue.global(qos: .userInitiated).async { [self] in
      finish(.alarm(alarm))
    }
  }

  /// Starts the two things that finishing is: telling the other device, if something at the Mac
  /// ended the session, and locking the Mac. Neither waits for the other. The other device is told
  /// through the library, which may be waiting for the thread that stops the capture, and that
  /// thread waits for the lock.
  private func launch(_ cause: Cause) {
    if case .alarm(let alarm) = cause {
      log("the session is ended: \(alarm)")
      let teller = Thread { [self] in end(alarm.sentence) }
      teller.name = "Baton privacy end"
      teller.start()
    }
    let worker = Thread { [self] in lockTheMac() }
    worker.name = "Baton privacy lock"
    worker.start()
  }

  private func lockTheMac() {
    var attempt = 0
    while !curtain.locked {
      log("locking the Mac, way \(attempt)")
      curtain.lock(attempt: attempt)
      attempt += 1
      let until = now() + timing.settle
      while now() < until && !curtain.locked {
        sleep(timing.poll)
      }
    }
    sleep(timing.afterLock)
    curtain.lower()
    log("the Mac is locked and the cover is down")
    condition.lock()
    state = .done
    condition.broadcast()
    condition.unlock()
  }
}

// MARK: - The quit of the app

/// Calls `action` when the app is about to quit, until it is stopped. A quit gives the screens of the
/// Mac back, since the process that kept them dark ends, so a private session locks the Mac first:
/// `action` runs on the thread that quits the app, and the quit goes on when it returns.
final class QuitWatch {
  private let center: NotificationCenter
  private let lock = NSLock()
  private var token: NSObjectProtocol?

  /// `name` is what the app posts when it is about to quit (`NSApplication.willTerminateNotification`
  /// on a Mac).
  init(
    name: Notification.Name, center: NotificationCenter = .default, action: @escaping () -> Void
  ) {
    self.center = center
    token = center.addObserver(forName: name, object: nil, queue: nil) { _ in action() }
  }

  /// Stops watching. It may be called again, and from the action.
  func stop() {
    lock.lock()
    let observer = token
    token = nil
    lock.unlock()
    if let observer { center.removeObserver(observer) }
  }

  deinit {
    stop()
  }
}

// MARK: - What is judged from the system's counters

/// When this app last handed a key to the system. A key that this app did not press is what the
/// tripwire looks for, so it has to know which keys were its own.
final class PostLedger: @unchecked Sendable {
  private let lock = NSLock()
  private var last: TimeInterval?
  private let now: () -> TimeInterval

  init(now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
    self.now = now
  }

  /// A key is about to be handed to the system.
  func notedKeyDown() {
    lock.lock()
    last = now()
    lock.unlock()
  }

  /// How long ago a key was last handed to the system, or nil if none was.
  var secondsSinceKeyDown: TimeInterval? {
    lock.lock()
    defer { lock.unlock() }
    return last.map { now() - $0 }
  }
}

/// A key at the Mac that the tap did not hold, which happens while a program has the keyboard to
/// itself (a password field): the system then does not show the keys to the taps of other programs,
/// but how long ago it counted the last key press still moves.
///
/// That is the time since the last key press and not a count of them, so it sees the last key only:
/// a key at the Mac that is followed by a key from the other device before the next look is not
/// seen (docs/adr/0011-privacy-mode.md). Reading the count of key presses instead is for the next
/// checks on a Mac.
enum Tripwire {
  /// How much later than the last key of this app the count may be, in seconds, before the key
  /// counts as another's: the system stamps a key a moment after it was handed over.
  static let slack: TimeInterval = 0.1

  /// Whether a key went down at the Mac that this app did not post since the hold began.
  ///
  /// `lastKeyAge` is how long ago the system counted a key going down (nil if it never has),
  /// `sinceHold` how long ago the hold began, and `postAge` how long ago this app last handed a key
  /// to the system (nil if never). A key that went down before the hold, or at the time of the last
  /// key of this app, is not another's.
  static func foreignKey(lastKeyAge: TimeInterval?, sinceHold: TimeInterval, postAge: TimeInterval?)
    -> Bool
  {
    guard let lastKeyAge else { return false }
    let from = min(sinceHold, postAge ?? sinceHold)
    return lastKeyAge < from - slack
  }
}

/// Three presses of Escape within two seconds: the way out for the person at a Mac that shows
/// nothing. A key that is held down repeats, and the repeats are not presses.
struct EscapeHatch {
  static let presses = 3
  static let within: TimeInterval = 2

  private var times: [TimeInterval] = []

  /// Escape went down at `time`, which only ever grows. true when it is the third press within
  /// two seconds, and the count starts again.
  mutating func pressed(at time: TimeInterval, repeating: Bool) -> Bool {
    guard !repeating else { return false }
    times = times.filter { time - $0 < EscapeHatch.within } + [time]
    guard times.count >= EscapeHatch.presses else { return false }
    times = []
    return true
  }
}
