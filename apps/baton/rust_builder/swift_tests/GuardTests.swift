// SPDX-License-Identifier: Apache-2.0

// Tests of BatonGuard.swift: the rules of the guard, the program that reads from the app, and the
// case that the guard is for, an app that is killed, which is tried with processes of the kind that
// a Mac has. The program of the tests starts itself as the app and as the guard
// (`guardParentMain` and `guardChildMain`, which main.swift runs when it is given their names), and
// the Mac that the guard locks is a file.

import Foundation

#if canImport(Darwin)
  import Darwin
#elseif canImport(Glibc)
  import Glibc
#endif

// MARK: - The Mac and the pipes

/// A Mac that is locked by the try with this number, counted from 0.
final class FakeMac: MacLocking, @unchecked Sendable {
  private let mutex = NSLock()
  private var tries: [Int] = []
  private let locksAtAttempt: Int?

  init(locksAtAttempt: Int? = 0) {
    self.locksAtAttempt = locksAtAttempt
  }

  var locked: Bool {
    mutex.lock()
    defer { mutex.unlock() }
    guard let locksAtAttempt, let last = tries.last else { return false }
    return last >= locksAtAttempt
  }

  func lock(attempt: Int) {
    mutex.lock()
    tries.append(attempt)
    mutex.unlock()
  }

  var attempts: [Int] {
    mutex.lock()
    defer { mutex.unlock() }
    return tries
  }
}

/// A Mac that is locked when a file is there, which a try makes. It is how a test sees, from
/// another process, that the guard locked the Mac.
final class FileMac: MacLocking {
  private let path: String

  init(path: String) {
    self.path = path
  }

  var locked: Bool { FileManager.default.fileExists(atPath: path) }

  func lock(attempt: Int) {
    _ = FileManager.default.createFile(atPath: path, contents: Data("locked\n".utf8))
  }
}

/// Lines that come from a pipe, with a time to wait for each.
private final class Lines {
  private let descriptor: Int32
  private var buffer = LineBuffer()
  private var queue: [String] = []

  init(_ descriptor: Int32) {
    self.descriptor = descriptor
  }

  func next(within seconds: TimeInterval) -> String? {
    if !queue.isEmpty { return queue.removeFirst() }
    let end = Date().addingTimeInterval(seconds)
    while Date() < end {
      var one = [pollfd(fd: descriptor, events: Int16(POLLIN), revents: 0)]
      let milliseconds = Int32(min(100, max(1, end.timeIntervalSinceNow * 1000)))
      guard poll(&one, 1, milliseconds) > 0 else { continue }
      var bytes = [UInt8](repeating: 0, count: 256)
      let count = read(descriptor, &bytes, bytes.count)
      if count == 0 { return nil }
      if count > 0 {
        queue += buffer.add(Array(bytes[0..<count]))
        if !queue.isEmpty { return queue.removeFirst() }
      }
    }
    return nil
  }
}

private func temporaryPath(_ name: String) -> String {
  FileManager.default.temporaryDirectory.appendingPathComponent("baton-guard-\(name)-\(UUID())")
    .path
}

private func fast(lapse: TimeInterval = 3, patience: TimeInterval = 10) -> GuardTiming {
  GuardTiming(
    lapse: lapse, answer: 5, patience: patience,
    lock: LockTiming(settle: 0.05, poll: 0.005))
}

// MARK: - The rules

func testTheGuardLocksWhenTheAppIsGoneAfterItWasHandedTheMac() {
  var machine = GuardMachine(lapse: 3, patience: 10, now: 0)
  check(machine.handle(.arm, now: 1) == [.answer("ok")])
  check(machine.phase == .armed)
  check(machine.handle(.beat, now: 2).isEmpty)
  check(machine.handle(.closed, now: 2.1) == [.lockTheMacAndEnd])
  check(machine.phase == .over)
  check(machine.handle(.tick, now: 100).isEmpty, "a guard that is over does nothing more")
  check(machine.handle(.closed, now: 100).isEmpty)

  var told = GuardMachine(lapse: 3, patience: 10, now: 0)
  _ = told.handle(.arm, now: 0)
  check(
    told.handle(.terminated, now: 1) == [.lockTheMacAndEnd], "a termination is an end of the app")
}

func testAnArmedGuardLocksWhenNoLineComesForTheLapse() {
  var machine = GuardMachine(lapse: 3, patience: 10, now: 0)
  _ = machine.handle(.arm, now: 1)
  check(machine.handle(.tick, now: 3.9).isEmpty)
  check(machine.handle(.beat, now: 4).isEmpty)
  // Three seconds after the beat is the lapse, and not yet over it.
  check(machine.handle(.tick, now: 7).isEmpty, "the lapse is over only when it is passed")
  check(machine.handle(.tick, now: 7.01) == [.lockTheMacAndEnd])
  check(machine.phase == .over)
}

func testBeatsKeepAnArmedGuardFromLocking() {
  var machine = GuardMachine(lapse: 1, patience: 10, now: 0)
  _ = machine.handle(.arm, now: 0)
  var time = 0.0
  for _ in 0..<100 {
    time += 0.5
    check(machine.handle(.beat, now: time).isEmpty)
    check(machine.handle(.tick, now: time + 0.1).isEmpty)
  }
  check(machine.phase == .armed)
}

func testTheGuardEndsWithoutLockingWhenTheSessionEndsCleanly() {
  var machine = GuardMachine(lapse: 3, patience: 10, now: 0)
  _ = machine.handle(.arm, now: 0)
  check(machine.handle(.done, now: 1) == [.answer("ok"), .end])
  check(machine.phase == .over)
  check(machine.handle(.closed, now: 2).isEmpty, "the pipe that closes after done locks nothing")
  check(machine.handle(.terminated, now: 2).isEmpty)
  check(machine.handle(.tick, now: 100).isEmpty)
}

func testAGuardThatWasNeverHandedTheMacLocksNothing() {
  for input in [GuardInput.closed, .terminated, .done] {
    var machine = GuardMachine(lapse: 3, patience: 10, now: 0)
    check(machine.handle(input, now: 1) == [.end], "\(input)")
    check(machine.phase == .over)
  }
  var patient = GuardMachine(lapse: 3, patience: 10, now: 0)
  check(patient.handle(.beat, now: 1).isEmpty, "a beat before arm is not heard")
  check(patient.handle(.tick, now: 10).isEmpty)
  check(patient.handle(.tick, now: 10.01) == [.end], "an app that never arms is not waited for")
  // The app can still arm in the meantime, and then the lapse counts from the arm.
  var late = GuardMachine(lapse: 3, patience: 10, now: 0)
  check(late.handle(.arm, now: 9) == [.answer("ok")])
  check(late.handle(.tick, now: 11).isEmpty)
}

func testAnAppThatArmsTwiceIsAnsweredTwice() {
  var machine = GuardMachine(lapse: 3, patience: 10, now: 0)
  check(machine.handle(.arm, now: 1) == [.answer("ok")])
  check(machine.handle(.arm, now: 2) == [.answer("ok")])
  check(machine.handle(.tick, now: 4.9).isEmpty, "the second arm counts as hearing the app")
  check(machine.handle(.tick, now: 5.1) == [.lockTheMacAndEnd])
}

func testLinesAreCutOutOfWhateverComesThroughAPipe() {
  var buffer = LineBuffer()
  check(buffer.add(Array("ar".utf8)).isEmpty)
  check(buffer.add(Array("m\nbe".utf8)) == ["arm"])
  check(buffer.add(Array("at\ndone\n".utf8)) == ["beat", "done"])
  let long = String(repeating: "x", count: 500)
  check(buffer.add(Array((long + "\nbeat\n").utf8)) == [String(repeating: "x", count: 64), "beat"])
  check(GuardInput(line: "arm") == .arm)
  check(GuardInput(line: "beat") == .beat)
  check(GuardInput(line: "done") == .done)
  check(GuardInput(line: "lock the Mac") == nil, "a line that the guard does not know is not heard")
  check(GuardInput(line: "") == nil)
}

func testLockingGoesRoundTheWaysUntilTheMacIsLocked() {
  let clock = FakeClock()
  let mac = FakeMac(locksAtAttempt: 4)
  lockUntilLocked(mac, timing: LockTiming(settle: 2, poll: 0.1), sleep: clock.sleep, now: clock.now)
  check(mac.attempts == [0, 1, 2, 3, 4], "\(mac.attempts)")
  // Each way that did not work was given its time, and the one that did was not waited for.
  check(clock.sleeps.reduce(0, +) > 7.9, "\(clock.sleeps.reduce(0, +))")
  check(clock.sleeps.reduce(0, +) < 8.5, "\(clock.sleeps.reduce(0, +))")

  let locked = FakeMac(locksAtAttempt: 0)
  locked.lock(attempt: 0)
  let before = locked.attempts
  lockUntilLocked(locked, sleep: clock.sleep, now: clock.now)
  check(locked.attempts == before, "a Mac that is locked is not locked again")

  var lines: [String] = []
  let logged = FakeMac(locksAtAttempt: 1)
  lockUntilLocked(logged, sleep: clock.sleep, now: clock.now, log: { lines.append($0) })
  check(lines == ["locking the Mac, way 0", "locking the Mac, way 1"], "\(lines)")
}

// MARK: - The program, with pipes

/// The guard, run in a thread of this program, with pipes for its input and output.
private final class ChildRig: @unchecked Sendable {
  let toChild = Pipe()
  let fromChild = Pipe()
  let signals = Pipe()
  let mac: FakeMac
  private let timing: GuardTiming
  private let ended = DispatchSemaphore(value: 0)
  private let mutex = NSLock()
  private var result: Int32?
  private var heard: Lines?
  private var parentNumber: pid_t = 100

  init(timing: GuardTiming, mac: FakeMac = FakeMac(locksAtAttempt: 0)) {
    self.timing = timing
    self.mac = mac
  }

  /// The program that started the guard is another one from now on.
  func changeParent() {
    mutex.lock()
    parentNumber = 1
    mutex.unlock()
  }

  func start() {
    heard = Lines(fromChild.fileHandleForReading.fileDescriptor)
    let child = GuardChild(
      input: toChild.fileHandleForReading.fileDescriptor,
      output: fromChild.fileHandleForWriting.fileDescriptor,
      signals: signals.fileHandleForReading.fileDescriptor, mac: mac, timing: timing,
      parent: { [self] in
        mutex.lock()
        defer { mutex.unlock() }
        return parentNumber
      })
    let thread = Thread { [self] in
      let status = child.run()
      mutex.lock()
      result = status
      mutex.unlock()
      ended.signal()
    }
    thread.start()
  }

  func send(_ line: String) {
    let bytes = Array((line + "\n").utf8)
    _ = write(toChild.fileHandleForWriting.fileDescriptor, bytes, bytes.count)
  }

  func closeInput() {
    try? toChild.fileHandleForWriting.close()
  }

  func terminate() {
    _ = write(signals.fileHandleForWriting.fileDescriptor, [1] as [UInt8], 1)
  }

  func line(within seconds: TimeInterval = 5) -> String? {
    heard?.next(within: seconds)
  }

  /// Whether the guard ended within `seconds`, and with what.
  func end(within seconds: TimeInterval = 5) -> Int32? {
    guard ended.wait(timeout: .now() + seconds) == .success else { return nil }
    mutex.lock()
    defer { mutex.unlock() }
    return result
  }
}

func testAGuardSaysItIsReadyIsHandedTheMacAndEndsWhenTheSessionEnds() {
  let rig = ChildRig(timing: fast())
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("arm")
  check(rig.line() == "ok")
  for _ in 0..<5 {
    rig.send("beat")
    Thread.sleep(forTimeInterval: 0.01)
  }
  rig.send("done")
  check(rig.line() == "ok")
  check(rig.end() == 0, "the guard ended")
  rig.closeInput()
  check(
    rig.mac.attempts.isEmpty, "a session that ended cleanly locked the Mac: \(rig.mac.attempts)")
}

func testAGuardLocksTheMacWhenItsInputClosesAfterItWasHandedTheMac() {
  let rig = ChildRig(timing: fast())
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("arm")
  check(rig.line() == "ok")
  let began = Date()
  rig.closeInput()
  check(rig.end() == 0)
  check(rig.mac.attempts == [0], "\(rig.mac.attempts)")
  check(Date().timeIntervalSince(began) < 1, "a closed pipe is heard at once")
}

func testAGuardLocksTheMacWhenTheAppStopsSayingThatItIsWell() {
  let rig = ChildRig(timing: fast(lapse: 0.5))
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("arm")
  check(rig.line() == "ok")
  let began = Date()
  // The pipe stays open and nothing comes, as it does from an app that hangs.
  check(rig.end() == 0)
  check(rig.mac.attempts == [0], "\(rig.mac.attempts)")
  let waited = Date().timeIntervalSince(began)
  check(waited >= 0.3, "the lapse was not waited for: \(waited)")
  check(waited < 3, "the lapse was waited for too long: \(waited)")
  rig.closeInput()
}

func testBeatsKeepAGuardFromLockingForAsLongAsTheyComeAndOnlyThen() {
  let rig = ChildRig(timing: fast(lapse: 1))
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("arm")
  check(rig.line() == "ok")
  // The beats go on for longer than the lapse.
  for _ in 0..<65 {
    rig.send("beat")
    Thread.sleep(forTimeInterval: 0.02)
  }
  check(rig.mac.attempts.isEmpty, "a guard that was told that the app is well locked the Mac")
  check(rig.end(within: 0.05) == nil, "the guard ended while it was told that the app is well")
  check(rig.end() == 0, "the guard did not lock the Mac when the beats stopped")
  check(rig.mac.attempts == [0])
  rig.closeInput()
}

func testAGuardThatWasNeverHandedTheMacEndsAndLocksNothing() {
  let closed = ChildRig(timing: fast())
  closed.start()
  check(closed.line() == "ready 1")
  closed.closeInput()
  check(closed.end() == 0)
  check(closed.mac.attempts.isEmpty)

  let patient = ChildRig(timing: fast(patience: 0.3))
  patient.start()
  check(patient.line() == "ready 1")
  check(patient.end() == 0, "an app that never arms is not waited for")
  check(patient.mac.attempts.isEmpty)
  patient.closeInput()

  let terminated = ChildRig(timing: fast())
  terminated.start()
  check(terminated.line() == "ready 1")
  terminated.terminate()
  check(terminated.end() == 0)
  check(terminated.mac.attempts.isEmpty)
  terminated.closeInput()
}

func testATerminationLocksTheMacOfAGuardThatWasHandedIt() {
  let rig = ChildRig(timing: fast())
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("arm")
  check(rig.line() == "ok")
  rig.terminate()
  check(rig.end() == 0)
  check(rig.mac.attempts == [0], "\(rig.mac.attempts)")
  rig.closeInput()
}

func testAGuardWhoseParentChangesLocksTheMacEvenIfItsInputStaysOpen() {
  // Something else may hold the pipe of the app open after the app is gone. The guard has been
  // given to the system then, and it is told by that.
  let rig = ChildRig(timing: fast())
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("arm")
  check(rig.line() == "ok")
  rig.changeParent()
  check(rig.end() == 0)
  check(rig.mac.attempts == [0], "\(rig.mac.attempts)")
  rig.closeInput()

  // A guard that was never handed the Mac ends, and locks nothing.
  let idle = ChildRig(timing: fast())
  idle.start()
  check(idle.line() == "ready 1")
  idle.changeParent()
  check(idle.end() == 0)
  check(idle.mac.attempts.isEmpty, "\(idle.mac.attempts)")
  idle.closeInput()
}

func testAGuardKeepsTryingToLockTheMacUntilItIsLocked() {
  let rig = ChildRig(timing: fast(), mac: FakeMac(locksAtAttempt: 3))
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("arm")
  check(rig.line() == "ok")
  rig.closeInput()
  check(rig.end() == 0)
  check(rig.mac.attempts == [0, 1, 2, 3], "\(rig.mac.attempts)")
}

func testAGuardIgnoresLinesItDoesNotKnow() {
  let rig = ChildRig(timing: fast())
  rig.start()
  check(rig.line() == "ready 1")
  rig.send("hello")
  rig.send("arm")
  check(rig.line() == "ok", "a line that is not known was answered")
  rig.send("unlock the Mac")
  rig.send("done")
  check(rig.line() == "ok")
  check(rig.end() == 0)
  check(rig.mac.attempts.isEmpty)
  rig.closeInput()
}

// MARK: - The program, as processes

/// The value after `name` in the arguments of this program.
private func value(of name: String) -> String? {
  let arguments = CommandLine.arguments
  guard let at = arguments.firstIndex(of: name), at + 1 < arguments.count else { return nil }
  return arguments[at + 1]
}

/// This program, started as the guard: `--guard-child --lock-file PATH [--lapse SECONDS]`.
func guardChildMain() -> Int32 {
  var timing = GuardTiming()
  timing.lapse = value(of: "--lapse").flatMap(Double.init) ?? 3
  timing.lock = LockTiming(settle: 0.05, poll: 0.005)
  let mac = FileMac(path: value(of: "--lock-file") ?? "/nonexistent/lock")
  return runGuardProgram(mac: mac, timing: timing)
}

/// This program, started as the app: it hands the Mac to a guard, says so, and then says that it is
/// well until it is killed or told to end the session (`--disarm-after SECONDS`). With `--deaf` the
/// guard is one that closes its input after it was armed and stays, so that the app's next write is
/// to a pipe that nobody reads.
func guardParentMain() -> Int32 {
  // An app starts with the default for a write to a pipe that nobody reads, which ends it, and the
  // link has to deal with that by itself. (The program that started this one ignored it, and what
  // is ignored stays ignored across an exec.)
  signal(SIGPIPE, SIG_DFL)
  var arguments = ["--guard-child", "--lock-file", value(of: "--lock-file") ?? ""]
  if let lapse = value(of: "--lapse") { arguments += ["--lapse", lapse] }
  let deaf = CommandLine.arguments.contains("--deaf")
  let link = GuardLink(
    executable: URL(fileURLWithPath: deaf ? "/bin/sh" : CommandLine.arguments[0]),
    arguments: deaf
      ? [
        "-c",
        "echo ready \(guardProtocolVersion); read line; echo ok; "
          + "exec 0<&- 3<&- 4<&- 5<&- 6<&- 7<&-; exec sleep 10",
      ]
      : arguments,
    timing: fast())
  do {
    try link.arm(lost: {
      print("lost")
      fflush(stdout)
    })
  } catch {
    print("failed")
    fflush(stdout)
    return 1
  }
  print("armed \(link.guardProcessID ?? -1)")
  fflush(stdout)
  let began = Date()
  let disarmAfter = value(of: "--disarm-after").flatMap(Double.init)
  while true {
    link.beat()
    Thread.sleep(forTimeInterval: 0.02)
    if let disarmAfter, Date().timeIntervalSince(began) > disarmAfter {
      link.disarm()
      print("disarmed")
      fflush(stdout)
      return 0
    }
  }
}


/// An app and its guard, as processes: `lockFile` is where the guard's Mac is.
private final class AppRig {
  let process = Process()
  let lockFile: String
  private let output = Pipe()
  private var lines: Lines
  private(set) var guardID: Int32 = -1

  init(lapse: TimeInterval = 3, disarmAfter: TimeInterval? = nil, deaf: Bool = false) {
    lockFile = temporaryPath("lock")
    process.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
    var arguments = ["--guard-parent", "--lock-file", lockFile, "--lapse", "\(lapse)"]
    if let disarmAfter { arguments += ["--disarm-after", "\(disarmAfter)"] }
    if deaf { arguments += ["--deaf"] }
    process.arguments = arguments
    process.standardOutput = output
    lines = Lines(output.fileHandleForReading.fileDescriptor)
  }

  /// Starts the app and waits until it has handed the Mac to its guard.
  func start() -> Bool {
    do { try process.run() } catch { return false }
    try? output.fileHandleForWriting.close()
    guard let armed = lines.next(within: 10), armed.hasPrefix("armed ") else { return false }
    guardID = Int32(armed.dropFirst(6)) ?? -1
    return guardID > 0
  }

  func line(within seconds: TimeInterval = 5) -> String? {
    lines.next(within: seconds)
  }

  var locked: Bool { FileManager.default.fileExists(atPath: lockFile) }

  /// Waits until the Mac is locked, and says how long it took, or nil.
  func waitForTheLock(from began: Date, seconds: TimeInterval = 8) -> TimeInterval? {
    let end = Date().addingTimeInterval(seconds)
    while Date() < end {
      if locked { return Date().timeIntervalSince(began) }
      Thread.sleep(forTimeInterval: 0.002)
    }
    return nil
  }

  /// Whether the process with this number is gone.
  static func gone(_ number: Int32) -> Bool {
    eventually { kill(number, 0) != 0 || isZombie(number) }
  }

  func clean() {
    if process.isRunning { kill(process.processIdentifier, SIGKILL) }
    if guardID > 0 { kill(guardID, SIGKILL) }
    try? FileManager.default.removeItem(atPath: lockFile)
  }
}

/// Whether a process has ended and nobody has asked for its status. On Linux a killed child that is
/// not waited for stays in the list, and `kill(number, 0)` still finds it.
private func isZombie(_ number: Int32) -> Bool {
  #if os(Linux)
    guard let status = try? String(contentsOfFile: "/proc/\(number)/status", encoding: .utf8) else {
      return true
    }
    return status.contains("State:\tZ")
  #else
    return false
  #endif
}

func testAnAppThatIsKilledLeavesAGuardThatLocksTheMac() {
  let app = AppRig()
  defer { app.clean() }
  check(app.start(), "the app did not hand the Mac to its guard")
  Thread.sleep(forTimeInterval: 0.3)
  check(!app.locked, "the Mac was locked while the app was well")
  let began = Date()
  kill(app.process.processIdentifier, SIGKILL)
  let took = app.waitForTheLock(from: began)
  check(took != nil, "the guard did not lock the Mac when the app was killed")
  check((took ?? 99) < 2, "the guard was slow to lock the Mac: \(took ?? 99)")
  check(AppRig.gone(app.guardID), "the guard did not end after it locked the Mac")
}

func testAnAppThatEndsTheSessionLeavesAGuardThatLocksNothing() {
  let app = AppRig(disarmAfter: 0.3)
  defer { app.clean() }
  check(app.start(), "the app did not hand the Mac to its guard")
  check(app.line() == "disarmed")
  check(AppRig.gone(app.guardID), "the guard did not end with the session")
  Thread.sleep(forTimeInterval: 0.5)
  check(!app.locked, "the guard locked the Mac after the session ended")
}

func testAnAppThatHangsLeavesAGuardThatLocksTheMacWhenTheLapseIsOver() {
  let app = AppRig(lapse: 0.6)
  defer { app.clean() }
  check(app.start(), "the app did not hand the Mac to its guard")
  let began = Date()
  // A stopped process is one that hangs: it holds its pipes and says nothing.
  kill(app.process.processIdentifier, SIGSTOP)
  let took = app.waitForTheLock(from: began)
  check(took != nil, "the guard did not lock the Mac when the app hung")
  check((took ?? 0) >= 0.3, "the guard locked the Mac before the lapse: \(took ?? 0)")
  check((took ?? 99) < 4, "the guard was slow to lock the Mac: \(took ?? 99)")
  kill(app.process.processIdentifier, SIGKILL)
}

func testAGuardThatIsTerminatedWhileItHoldsTheMacLocksIt() {
  let app = AppRig()
  defer { app.clean() }
  check(app.start(), "the app did not hand the Mac to its guard")
  let began = Date()
  kill(app.guardID, SIGTERM)
  check(app.waitForTheLock(from: began) != nil, "a terminated guard did not lock the Mac")
  // The app is told that its guard is gone.
  check(app.line() == "lost", "the app was not told that its guard locked the Mac")
}

func testAGuardThatIsKilledWhileItHoldsTheMacIsReportedToTheApp() {
  let app = AppRig()
  defer { app.clean() }
  check(app.start(), "the app did not hand the Mac to its guard")
  kill(app.guardID, SIGKILL)
  check(app.line() == "lost", "the app was not told that its guard is gone")
  check(app.line(within: 0.5) == nil, "the app was told twice")
}

func testAGuardWhosePipeBreaksWhileItStaysIsReportedToTheApp() {
  // The app hears that a guard ended, but a pipe can break before it does, and then a write to it
  // is an error to deal with and not the end of the app.
  let app = AppRig(deaf: true)
  defer { app.clean() }
  check(app.start(), "the app did not hand the Mac to its guard")
  check(app.line() == "lost", "the app was not told that its guard is gone")
  check(app.line(within: 0.5) == nil, "the app was told twice")
  check(app.process.isRunning, "the app was ended by a write to a pipe that nobody reads")
}

func testAGuardThatIsNotThereOrDoesNotAnswerIsNotArmed() {
  let notGuards: [(String, URL?, [String], TimeInterval)] = [
    ("no program", nil, [], 5),
    ("a program that is not there", URL(fileURLWithPath: "/nonexistent/guard"), [], 5),
    // It starts, and ends at once.
    ("a program that ends", URL(fileURLWithPath: "/bin/sh"), ["-c", "exit 0"], 5),
    // It starts, and says nothing in the time.
    ("a program that is silent", URL(fileURLWithPath: "/bin/cat"), [], 0.3),
    // It starts, and speaks another version.
    (
      "a program of another version", URL(fileURLWithPath: "/bin/sh"),
      ["-c", "echo ready 99; exec cat"], 5
    ),
  ]
  for (name, executable, arguments, answer) in notGuards {
    let link = GuardLink(
      executable: executable, arguments: arguments, timing: GuardTiming(answer: answer))
    let began = Date()
    do {
      try link.arm(lost: { check(false, "\(name) was lost, and it was never armed") })
      check(false, "\(name) was armed")
    } catch let failure as GuardFailure {
      check(!failure.sentence.isEmpty, name)
      check(!failure.sentence.contains("\u{2014}"), "a dash in \(failure.sentence)")
    } catch {
      check(false, "\(name): another kind of error: \(error)")
    }
    check(Date().timeIntervalSince(began) < 3, "\(name) was waited for")
    check(link.guardProcessID == nil, "\(name) was left running")
    // A link that was never armed is quiet.
    link.beat()
    link.disarm()
  }
  // Nothing was handed to any of them, so none locked anything: a guard that was given up on before
  // it was armed ends and does not touch the Mac (the process tests above show it).
}

func testAnArmThatIsAnsweredTooLateIsTakenBackAndLocksNothing() {
  // A guard that is slow to say that it is armed is given up on, and a guard that was armed locks
  // the Mac when its input closes: the app has to say that the session is over first. The program
  // here answers after a while, and writes down what it was told.
  let told = temporaryPath("told")
  defer { try? FileManager.default.removeItem(atPath: told) }
  let script = "echo 'ready 1'; while read line; do echo \"$line\" >> '\(told)'; "
    + "if [ \"$line\" = arm ]; then sleep 1.5; echo ok; fi; done"
  let link = GuardLink(
    executable: URL(fileURLWithPath: "/bin/sh"), arguments: ["-c", script],
    timing: GuardTiming(answer: 1))
  do {
    try link.arm(lost: { check(false, "a guard that was given up on was lost") })
    check(false, "a guard that answered too late was armed")
  } catch is GuardFailure {
  } catch {
    check(false, "another kind of error: \(error)")
  }
  check(
    eventually {
      (try? String(contentsOfFile: told, encoding: .utf8)) == "arm\ndone\n"
    }, "the guard was told: \((try? String(contentsOfFile: told, encoding: .utf8)) ?? "nothing")")
}

func testALinkIsArmedOnceAndTheGuardEndsWhenItIsDisarmed() {
  let lockFile = temporaryPath("once")
  defer { try? FileManager.default.removeItem(atPath: lockFile) }
  let link = GuardLink(
    executable: URL(fileURLWithPath: CommandLine.arguments[0]),
    arguments: ["--guard-child", "--lock-file", lockFile], timing: fast())
  do {
    try link.arm(lost: { check(false, "the guard was lost") })
  } catch {
    check(false, "the guard was not armed: \(error)")
    return
  }
  let number = link.guardProcessID ?? -1
  check(number > 0)
  do {
    try link.arm(lost: {})
    check(false, "a link was armed twice")
  } catch is GuardFailure {
  } catch {
    check(false, "another kind of error: \(error)")
  }
  for _ in 0..<10 {
    link.beat()
    Thread.sleep(forTimeInterval: 0.01)
  }
  link.disarm()
  check(AppRig.gone(number), "the guard did not end when the link was disarmed")
  link.beat()
  link.disarm()
  Thread.sleep(forTimeInterval: 0.2)
  check(
    !FileManager.default.fileExists(atPath: lockFile), "the guard locked a Mac whose session ended")
}

