// SPDX-License-Identifier: Apache-2.0

// The rules of the privacy mode (docs/adr/0011-privacy-mode.md): a person who uses their Mac from a
// phone or a tablet while other people are at home. While the session lasts the screens of the Mac
// show nothing, nobody at the Mac can use its keyboard or its pointer, and when the session ends,
// whatever the reason, the Mac is locked again and only then does it show anything. The one end
// that does not lock it is the person's own at the other device: asked when they end the session,
// they may choose to leave the Mac as it is (docs/adr/0014-lock-the-mac-or-leave-it.md).
//
// This file holds what does not depend on the frameworks of a Mac: when the cover goes up and comes
// down, what ends a session, how the Mac is locked until the system says that it is, how it is left
// as it is when the person chose that, and what the guard (BatonGuard.swift) is told. The Mac itself
// (the cover, the tap that holds the input, the calls that lock) is given to it as a protocol, and
// is in BatonPrivacyMac.swift. So that this file can be built and tested anywhere
// (apps/baton/rust_builder/swift_tests), nothing here may import more than Foundation.

import Foundation

// MARK: - What the Mac does

/// Something at the Mac that ends a private session. It is also what a curtain throws when a part
/// of the privacy that was lifted cannot be put back.
enum PrivacyAlarm: Error, Equatable {
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
  /// The guard went away, or locked the Mac, while the session was on: nothing would lock the Mac
  /// if this app went away as well.
  case guardLost

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
    case .guardLost:
      return "The Mac lost its guard. The session ended, and the Mac locks."
    }
  }
}

/// What a private session uses of the Mac. BatonPrivacyMac.swift has the real one.
protocol Curtain: MacLocking {
  /// Covers every screen with black, takes the keyboard and the pointer out of anyone's hands but
  /// this app's, and keeps the screens awake. `alarm` is called, from any thread and without
  /// waiting, when something at the Mac needs the session ended. Throws `SharerFailure` with a
  /// sentence if it cannot, and then nothing is left raised. Called once.
  func raise(alarm: @escaping (PrivacyAlarm) -> Void) throws

  /// Lifts or puts back the two parts of the privacy that the person at the other device may switch
  /// on and off as often as they like: with `screen` the screens show what is on them, and with
  /// `input` the keyboard and the pointer of the Mac can be used at it. Everything that is lifted
  /// is given each time, and a call replaces the one before; a curtain that is raised has both
  /// off. A part that is lifted is not judged, since nobody is suspected of a key or a click that
  /// they were let make. Throws the alarm of the part that could not be put back, and then the
  /// curtain is as private as it could be made. Does nothing when nothing is raised.
  func lift(screen: Bool, input: Bool) throws

  /// Takes the cover down and lets go of the keyboard, the pointer and the screens. It may be
  /// called when nothing is raised.
  func lower()

  /// Whether the cover and the hold are as they were put, and are still being looked at. Once the
  /// cover is up, a session tells its guard that it is well only while this is true, so that a
  /// curtain that nobody looks after any more ends in the lock of the guard.
  var healthy: Bool { get }
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
  /// How often the guard is told that the session is well. It is a real time, since the guard
  /// counts real time, and it is far shorter than the lapse that the guard waits for.
  var beat: TimeInterval = 0.25
  /// How long the guard is told that the app is at work, whatever the curtain says, while the cover
  /// goes up: a Mac that is busy takes longer than the guard waits for a first word, and a cover
  /// that takes longer than this ends in the lock of the guard. It is a real time too.
  var raise: TimeInterval = 10
}

/// The privacy of one session that another device controls: the cover goes up when the session
/// starts and comes down only when the Mac is locked, or when the person at the other device chose,
/// at the end, to leave the Mac as it is.
///
/// A Mac that cannot be locked keeps its cover and goes on trying. That is the way of failing that
/// shows nothing to the people who are at home, and the person at the Mac has the power button.
///
/// The guard is handed the Mac before the cover goes up and let go of after it comes down. In
/// between, an app that goes away without ending the session (a crash, a kill, a hang) is the
/// guard's to answer for: it locks the Mac.
///
/// The person at the other device can lift the privacy for a while, the screens and the keyboard
/// and the pointer each by itself and as often as they like (`lift`), to show the Mac to somebody
/// or to use it from where they sit. Nothing that is lifted outlasts the session: whatever ends it,
/// what was lifted is put back before the Mac is locked, and a session starts with nothing lifted.
final class PrivateSession {
  /// Why a session is finished.
  enum Cause: Equatable {
    /// The session is over, whoever ended it.
    case sessionOver
    /// The session is over, and the person at the other device chose, when they ended it, that the
    /// Mac stay as it is: what they lifted is put back, the cover comes down, and the Mac is not
    /// locked (docs/adr/0014-lock-the-mac-or-leave-it.md). A Mac that is locked stays locked.
    case left
    /// The window is let go of: it never ran, or it did not start.
    case released
    /// Something at the Mac ended it.
    case alarm(PrivacyAlarm)

    /// Whether this is the end of the session, which waits for the end that began before it.
    var endsTheSession: Bool { self == .sessionOver || self == .left }
  }

  /// The two parts of the privacy that the person at the other device can lift, each by itself.
  struct Lifted: Equatable {
    /// The screens show what is on them.
    var screen = false
    /// The keyboard and the pointer of the Mac can be used at it.
    var input = false
  }

  private enum State {
    case idle, raised, finishing, done
  }

  private let curtain: Curtain
  private let guardian: SessionGuard
  private let end: (String) -> Void
  private let timing: PrivacyTiming
  private let sleep: (TimeInterval) -> Void
  private let now: () -> TimeInterval
  private let log: (String) -> Void

  /// Held for the whole of `begin`, so that a finish that is asked for meanwhile (an alarm can come
  /// from the first moment of the hold) waits for the cover to be up.
  private let condition = NSCondition()
  private var state = State.idle
  /// What the person at the other device has lifted of the privacy and not put back, under
  /// `condition`: a lift holds it for as long as it takes, so that no finish begins half way
  /// through one.
  private var lifted = Lifted()

  /// Whether the guard is told that the session is well. Apart from `condition`, since the thread
  /// that tells it must never wait for a `begin` that is raising the cover.
  private let beating = NSLock()
  private var beatingOn = false
  /// When the guard was first told, and whether the cover is up, so that the curtain is judged.
  private var beatingSince: TimeInterval = 0
  private var judging = false

  /// `guardian` is the guard, which is handed the Mac for as long as the cover is up. `end` ends
  /// the session at the other device with a sentence for the person there. `sleep` and `now` are
  /// the clocks of the locking, which the tests replace.
  init(
    curtain: Curtain, guardian: SessionGuard, timing: PrivacyTiming = PrivacyTiming(),
    end: @escaping (String) -> Void,
    sleep: @escaping (TimeInterval) -> Void = { Thread.sleep(forTimeInterval: $0) },
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
    log: @escaping (String) -> Void = { _ in }
  ) {
    self.curtain = curtain
    self.guardian = guardian
    self.timing = timing
    self.end = end
    self.sleep = sleep
    self.now = now
    self.log = log
  }

  /// Hands the Mac to the guard, puts the cover up and takes the keyboard and the pointer. Throws
  /// `SharerFailure` if the guard or the Mac cannot, and then nothing is left raised and the
  /// session cannot be raised again.
  func begin() throws {
    condition.lock()
    defer { condition.unlock() }
    guard state == .idle else {
      throw SharerFailure(sentence: "This Mac could not keep its screen private.")
    }
    // The guard first: a cover that nothing would lock the Mac behind, if this app went away, is
    // not a private session, and a guard that cannot be had is told to the person before anything
    // is covered.
    do {
      try guardian.arm { [weak self] in self?.alarmed(.guardLost) }
    } catch {
      state = .done
      condition.broadcast()
      if let failure = error as? GuardFailure { throw SharerFailure(sentence: failure.sentence) }
      throw error
    }
    // The guard hears from the app while the cover goes up, which can take longer on a Mac that is
    // busy than the guard waits for a first word.
    startBeating()
    do {
      try curtain.raise { [weak self] alarm in self?.alarmed(alarm) }
    } catch {
      stopBeating()
      curtain.lower()
      guardian.disarm()
      state = .done
      condition.broadcast()
      throw error
    }
    state = .raised
    judgeBeats()
    log("the cover is up")
  }

  /// The person at the other device switched what keeps the Mac private: `screen` lets the screens
  /// show what is on them, and `input` lets the keyboard and the pointer be used at the Mac. Both
  /// are given each time and the last call decides, so a part is put back by a call that leaves it
  /// off. A call that changes nothing is not passed on. It does nothing before the cover is up, nor
  /// once the session is being finished: a Mac that is being locked has nothing lifted. A part that
  /// cannot be put back ends the session, and the Mac is locked.
  func lift(screen: Bool, input: Bool) {
    condition.lock()
    defer { condition.unlock() }
    let wanted = Lifted(screen: screen, input: input)
    guard state == .raised, wanted != lifted else { return }
    do {
      try curtain.lift(screen: screen, input: input)
      lifted = wanted
      log(
        "the screens \(screen ? "show what is on them" : "are dark"), the keys and the pointer "
          + "\(input ? "can be used at the Mac" : "are held")")
    } catch {
      alarmed((error as? PrivacyAlarm) ?? .coverLost)
    }
  }

  /// The session is over, or the window is let go of, or something at the Mac ended it: the Mac is
  /// locked, and then the cover comes down. Returns when that is done, or after the patience of the
  /// timing, in which case the Mac goes on being locked on a thread of its own.
  ///
  /// The one cause that does not lock the Mac is `left`, the choice of the person at the other
  /// device: what they lifted is put back and the cover comes down, with nothing locked. It is the
  /// session's first end or it is nothing: a session that began to end some other way ends with the
  /// lock, and `left` only waits for it.
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
      guard cause.endsTheSession else {
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
  /// ended the session, and locking the Mac, or leaving it as it is if the person at the other
  /// device chose that. Neither waits for the other. The other device is told through the library,
  /// which may be waiting for the thread that stops the capture, and that thread waits for the
  /// lock.
  private func launch(_ cause: Cause) {
    if case .alarm(let alarm) = cause {
      log("the session is ended: \(alarm)")
      let teller = Thread { [self] in end(alarm.sentence) }
      teller.name = "Baton privacy end"
      teller.start()
    }
    let leaving = cause == .left
    let worker = Thread { [self] in
      if leaving { leaveTheMac() } else { lockTheMac() }
    }
    worker.name = leaving ? "Baton privacy leave" : "Baton privacy lock"
    worker.start()
  }

  /// Puts back whatever the person at the other device lifted, before the Mac is locked: nobody at
  /// the Mac can get in the way of the lock, and the lock screen is drawn behind the cover and not
  /// in front of a screen that shows what was on it. A part that cannot be put back does not stop
  /// the lock. Returns whether everything was put back.
  @discardableResult
  private func putBack() -> Bool {
    condition.lock()
    let was = lifted
    condition.unlock()
    guard was != Lifted() else { return true }
    var done = true
    do {
      try curtain.lift(screen: false, input: false)
    } catch {
      log("what was lifted could not be put back before the Mac was locked")
      done = false
    }
    condition.lock()
    lifted = Lifted()
    condition.unlock()
    return done
  }

  /// The person at the other device chose that the Mac stay as it is: what they lifted is put back,
  /// as before a lock, and the cover comes down. Nothing is locked, woken or unlocked, so a Mac
  /// that is locked stays locked, and the cover stays for a moment on one that is, so that its lock
  /// screen is drawn before the cover goes. A part that cannot be put back is not a reason to leave
  /// the Mac open: it is locked as it is at every other end.
  private func leaveTheMac() {
    guard putBack() else {
      log("what was lifted could not be put back, so the Mac is locked and not left")
      lockTheMac()
      return
    }
    if curtain.locked { sleep(timing.afterLock) }
    curtain.lower()
    // The cover is down and nothing is covered, so the guard has nothing left to guard.
    stopBeating()
    guardian.disarm()
    log("the Mac is left as it is and the cover is down")
    condition.lock()
    state = .done
    condition.broadcast()
    condition.unlock()
  }

  private func lockTheMac() {
    putBack()
    lockUntilLocked(
      curtain, timing: LockTiming(settle: timing.settle, poll: timing.poll), sleep: sleep, now: now,
      log: log)
    sleep(timing.afterLock)
    curtain.lower()
    // The Mac is locked and nothing is covered, so the guard has nothing left to guard.
    stopBeating()
    guardian.disarm()
    log("the Mac is locked and the cover is down")
    condition.lock()
    state = .done
    condition.broadcast()
    condition.unlock()
  }

  // MARK: The beat

  /// Tells the guard that the session is well, several times a second, for as long as the curtain
  /// is: the guard locks the Mac when it hears nothing for its lapse, which is how an app that
  /// hangs is dealt with. The thread does not keep the session alive.
  private func startBeating() {
    beating.lock()
    beatingOn = true
    judging = false
    beatingSince = ProcessInfo.processInfo.systemUptime
    beating.unlock()
    let interval = timing.beat
    let thread = Thread { [weak self] in
      while self?.beatOnce() == true { Thread.sleep(forTimeInterval: interval) }
    }
    thread.name = "Baton privacy beat"
    thread.qualityOfService = .userInteractive
    thread.start()
  }

  /// One beat, if the curtain is well, or if the cover is still going up and has not been for too
  /// long. false when there is no beating any more.
  private func beatOnce() -> Bool {
    beating.lock()
    let on = beatingOn
    let judged = judging
    let since = beatingSince
    beating.unlock()
    guard on else { return false }
    let well =
      judged
      ? curtain.healthy : ProcessInfo.processInfo.systemUptime - since < timing.raise
    if well { guardian.beat() }
    return true
  }

  /// The cover is up: from now on the curtain has to be well for the guard to be told.
  private func judgeBeats() {
    beating.lock()
    judging = true
    beating.unlock()
  }

  private func stopBeating() {
    beating.lock()
    beatingOn = false
    beating.unlock()
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
