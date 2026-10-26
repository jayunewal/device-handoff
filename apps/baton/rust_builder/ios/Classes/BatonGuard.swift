// SPDX-License-Identifier: Apache-2.0

// The guard of the privacy mode (docs/adr/0011-privacy-mode.md). While another device controls the
// Mac, the app keeps its screens dark and its keys and pointer held, and that stops the moment the
// app does: an app that crashes, is killed or hangs gives the screens back as its process ends, and
// the Mac is left unlocked and lit. The guard is a small program that the app starts for each
// private session, and that locks the Mac when the app goes away without ending the session.
//
// The app and the guard talk over the pipes of the guard's standard input and output, in lines:
//
//   guard -> app    ready 1     the first line that the guard writes
//   app -> guard    arm         from now on, lock the Mac if the app goes away
//   guard -> app    ok
//   app -> guard    beat        the privacy of the app is well; sent several times a second
//   app -> guard    done        the session is over, the Mac is locked and the cover is down
//   guard -> app    ok          and the guard ends
//
// An armed guard locks the Mac, and ends, when its input closes (the system closes the app's end of
// the pipe when the app's process ends, however it ends), when no line comes for `lapse` seconds
// (the app hangs), or when it is told to terminate. A guard that is not armed ends when its input
// closes, so the start of the app that finds nothing to do leaves nothing behind.
//
// This file holds what does not depend on the frameworks of a Mac, so that it can be built and
// tested anywhere (apps/baton/rust_builder/swift_tests), including the case that matters: a parent
// that is killed. Nothing here may import more than Foundation. The locking itself is
// BatonLock.swift, and the program is apps/baton/macos/Guard/main.swift.

import Foundation

#if canImport(Darwin)
  import Darwin
#elseif canImport(Glibc)
  import Glibc
#endif

/// The version of what the app and the guard say to each other.
let guardProtocolVersion = 1

// MARK: - Locking the Mac

/// What locking a Mac asks of it. BatonLock.swift has the real one.
protocol MacLocking: AnyObject {
  /// Whether the system says that the Mac is locked.
  var locked: Bool { get }

  /// One try at locking the Mac. The Mac has more than one way, and the later numbers are tried
  /// when the earlier ones did not lock it. It does not wait for the Mac to be locked.
  func lock(attempt: Int)
}

/// The time that locking the Mac gives each step, in seconds. The tests make it short.
struct LockTiming {
  /// How long the Mac has to say that it is locked after a try, before the next way is tried.
  var settle: TimeInterval = 2
  /// How often the Mac is asked while it is waited for.
  var poll: TimeInterval = 0.1
}

/// Locks the Mac with the first of its ways that works, and goes on with the ways, round and round,
/// until the Mac says that it is locked. It returns then and not before.
func lockUntilLocked(
  _ mac: MacLocking, timing: LockTiming = LockTiming(),
  sleep: (TimeInterval) -> Void = { Thread.sleep(forTimeInterval: $0) },
  now: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
  log: (String) -> Void = { _ in }
) {
  var attempt = 0
  while !mac.locked {
    log("locking the Mac, way \(attempt)")
    mac.lock(attempt: attempt)
    attempt += 1
    let until = now() + timing.settle
    while now() < until && !mac.locked {
      sleep(timing.poll)
    }
  }
}

// MARK: - What a private session asks of its guard

/// The guard could not be started or did not answer. `sentence` is for the person at the other
/// device.
struct GuardFailure: Error {
  let sentence: String
}

/// What a private session uses of the guard. `GuardLink` is the real one.
protocol SessionGuard: AnyObject {
  /// Starts the guard and hands the Mac to it: from now on a guard that stops hearing from the app
  /// locks the Mac. `lost` is called, once and from any thread, if the guard goes away or locks the
  /// Mac while it is armed. Throws `GuardFailure` if the guard is not there, and then nothing is
  /// left running. Called once.
  func arm(lost: @escaping () -> Void) throws

  /// Says that the privacy of the app is well. It does not wait.
  func beat()

  /// The session is over, the Mac is locked and the cover is down: the guard ends without locking
  /// anything. `lost` is not called after this.
  func disarm()
}

/// The time the guard and the app give each other, in seconds. The tests make it short.
struct GuardTiming {
  /// How long an armed guard waits for a line from the app before it takes the app for hung.
  var lapse: TimeInterval = 3
  /// How long the app waits for the guard to say that it is ready, and then to answer. A program
  /// that starts for the first time can take a while on a Mac that is busy, and a guard that is
  /// gone is seen at once and not waited for.
  var answer: TimeInterval = 5
  /// How long a guard that is not armed waits to be, before it ends.
  var patience: TimeInterval = 10
  var lock = LockTiming()
}

// MARK: - The rules of the guard

/// What the guard is told, by the app or by the clock.
enum GuardInput: Equatable {
  case arm, beat, done
  /// The input of the guard closed: the app is gone.
  case closed
  /// The guard was told to terminate.
  case terminated
  /// The clock moved on.
  case tick

  /// A line of the app, or nil if the guard does not know it.
  init?(line: String) {
    switch line {
    case "arm": self = .arm
    case "beat": self = .beat
    case "done": self = .done
    default: return nil
    }
  }
}

/// What the guard does about it.
enum GuardOutput: Equatable {
  /// Writes a line to the app.
  case answer(String)
  /// Locks the Mac, and then ends.
  case lockTheMacAndEnd
  case end
}

/// When the guard locks the Mac and when it ends, without any I/O, so that every case is tested.
struct GuardMachine {
  enum Phase: Equatable {
    /// Started, and the app has not handed the Mac to it yet.
    case waiting
    case armed
    /// It is over: it ended, or it locks the Mac and then ends.
    case over
  }

  private(set) var phase = Phase.waiting
  private let lapse: TimeInterval
  private let patience: TimeInterval
  private let started: TimeInterval
  private var lastHeard: TimeInterval

  init(lapse: TimeInterval, patience: TimeInterval, now: TimeInterval) {
    self.lapse = lapse
    self.patience = patience
    started = now
    lastHeard = now
  }

  mutating func handle(_ input: GuardInput, now: TimeInterval) -> [GuardOutput] {
    switch (phase, input) {
    case (.over, _):
      return []
    case (.waiting, .arm):
      phase = .armed
      lastHeard = now
      return [.answer("ok")]
    case (.waiting, .closed), (.waiting, .terminated), (.waiting, .done):
      // Nothing was handed to the guard, so there is nothing to lock for.
      phase = .over
      return [.end]
    case (.waiting, .beat):
      return []
    case (.waiting, .tick):
      guard now - started > patience else { return [] }
      phase = .over
      return [.end]
    case (.armed, .arm):
      lastHeard = now
      return [.answer("ok")]
    case (.armed, .beat):
      lastHeard = now
      return []
    case (.armed, .done):
      phase = .over
      return [.answer("ok"), .end]
    case (.armed, .closed), (.armed, .terminated):
      phase = .over
      return [.lockTheMacAndEnd]
    case (.armed, .tick):
      guard now - lastHeard > lapse else { return [] }
      phase = .over
      return [.lockTheMacAndEnd]
    }
  }
}

/// Lines out of the bytes of a pipe. A line that is too long to be one of ours is cut.
struct LineBuffer {
  private var pending: [UInt8] = []
  private static let longest = 64

  mutating func add(_ bytes: [UInt8]) -> [String] {
    var lines: [String] = []
    for byte in bytes {
      if byte == 10 {
        lines.append(String(decoding: pending, as: UTF8.self))
        pending.removeAll(keepingCapacity: true)
      } else if pending.count < LineBuffer.longest {
        pending.append(byte)
      }
    }
    return lines
  }
}

// MARK: - The pipes

private enum Pulled {
  case data([UInt8])
  case end
  case nothing
}

private func pull(_ descriptor: Int32) -> Pulled {
  var bytes = [UInt8](repeating: 0, count: 256)
  let count = read(descriptor, &bytes, bytes.count)
  if count > 0 { return .data(Array(bytes[0..<count])) }
  if count == 0 { return .end }
  return errno == EINTR || errno == EAGAIN ? .nothing : .end
}

private enum Pushed {
  case sent, full, broken
}

/// Writes a line. A line is small enough to go into a pipe whole or not at all.
private func push(_ descriptor: Int32, _ line: String) -> Pushed {
  let bytes = Array((line + "\n").utf8)
  let count = write(descriptor, bytes, bytes.count)
  if count == bytes.count { return .sent }
  if count < 0 && (errno == EAGAIN || errno == EINTR) { return .full }
  return .broken
}

/// Waits until `descriptor` can be read, or has closed, or `seconds` are over.
private func waitToRead(_ descriptor: Int32, seconds: TimeInterval) -> Bool {
  var one = [pollfd(fd: descriptor, events: Int16(POLLIN), revents: 0)]
  let milliseconds = Int32(max(0, min(seconds, 600)) * 1000)
  return poll(&one, 1, milliseconds) > 0
}

/// The first line that comes on `descriptor` within `seconds`, or nil.
private func firstLine(from descriptor: Int32, within seconds: TimeInterval) -> String? {
  let until = Date(timeIntervalSinceNow: seconds)
  var lines = LineBuffer()
  while true {
    let left = until.timeIntervalSinceNow
    guard left > 0, waitToRead(descriptor, seconds: left) else { return nil }
    switch pull(descriptor) {
    case .data(let bytes):
      if let line = lines.add(bytes).first { return line }
    case .end:
      return nil
    case .nothing:
      break
    }
  }
}

// MARK: - The guard

/// The program that guards a private session: what it reads, what it writes, when it locks the Mac.
final class GuardChild {
  private let input: Int32
  private let output: Int32
  private let signals: Int32?
  private let mac: MacLocking
  private let timing: GuardTiming
  private let now: () -> TimeInterval
  private let parent: () -> pid_t
  private let log: (String) -> Void

  /// `input` and `output` are the pipes to the app, the standard input and output of the program.
  /// `signals` is a descriptor that becomes readable when the program is told to terminate.
  /// `parent` is the number of the process that started the program.
  init(
    input: Int32 = 0, output: Int32 = 1, signals: Int32? = nil, mac: MacLocking,
    timing: GuardTiming = GuardTiming(),
    now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
    parent: @escaping () -> pid_t = { getppid() },
    log: @escaping (String) -> Void = { _ in }
  ) {
    self.input = input
    self.output = output
    self.signals = signals
    self.mac = mac
    self.timing = timing
    self.now = now
    self.parent = parent
    self.log = log
  }

  /// Runs until the session is over or the Mac is locked, and returns the status of the program.
  func run() -> Int32 {
    _ = push(output, "ready \(guardProtocolVersion)")
    var machine = GuardMachine(lapse: timing.lapse, patience: timing.patience, now: now())
    var lines = LineBuffer()
    // The program that started this one. When it is another, the app is gone, and this one has been
    // given to the system, whatever else holds the pipe open.
    let started = parent()
    while true {
      var watched = [pollfd(fd: input, events: Int16(POLLIN), revents: 0)]
      if let signals { watched.append(pollfd(fd: signals, events: Int16(POLLIN), revents: 0)) }
      _ = poll(&watched, nfds_t(watched.count), 100)
      var heard: [GuardInput] = []
      if watched[0].revents != 0 {
        switch pull(input) {
        case .data(let bytes):
          for line in lines.add(bytes) {
            if let known = GuardInput(line: line) { heard.append(known) }
          }
        case .end:
          heard.append(.closed)
        case .nothing:
          break
        }
      }
      if watched.count > 1, watched[1].revents != 0 { heard.append(.terminated) }
      if parent() != started { heard.append(.closed) }
      heard.append(.tick)
      for what in heard {
        for action in machine.handle(what, now: now()) {
          switch action {
          case .answer(let line):
            _ = push(output, line)
          case .lockTheMacAndEnd:
            log("the app is not well (\(what)), and the Mac is locked")
            lockUntilLocked(mac, timing: timing.lock, log: log)
            return 0
          case .end:
            return 0
          }
        }
      }
    }
  }
}

// MARK: - The program

/// The write end of the pipe that a signal is told to the guard by. Set once, before the signals
/// are taken, and read by the handler only.
private var signalPipeWrite: Int32 = -1

/// Runs the guard as a program, and returns its status. The standard input and output are the pipes
/// to the app. A termination is taken for the end of the app, as far as the guard can tell: the
/// person who kills the app with a signal kills the guard with the same signal, and the Mac is
/// locked then too.
func runGuardProgram(
  mac: MacLocking, timing: GuardTiming = GuardTiming(), log: @escaping (String) -> Void = { _ in }
) -> Int32 {
  var ends: [Int32] = [0, 0]
  guard pipe(&ends) == 0 else { return 1 }
  _ = fcntl(ends[1], F_SETFL, fcntl(ends[1], F_GETFL) | O_NONBLOCK)
  signalPipeWrite = ends[1]
  let told: @convention(c) (Int32) -> Void = { _ in
    var byte: UInt8 = 1
    _ = write(signalPipeWrite, &byte, 1)
  }
  for number in [SIGTERM, SIGHUP, SIGINT] { signal(number, told) }
  // A write to an app that is gone is an error to ignore, and not the end of the guard.
  signal(SIGPIPE, SIG_IGN)
  return GuardChild(signals: ends[0], mac: mac, timing: timing, log: log).run()
}

// MARK: - The app's end

#if !os(iOS)

  /// What the app has of the guard: the program, started for one private session, and the pipes to
  /// it. It starts the guard when it is armed, and the guard is the only thing that lives past it.
  final class GuardLink: SessionGuard {
    private enum State {
      case idle, armed, disarmed, lost
    }

    private struct Started {
      let process: Process
      let toGuard: Pipe
      let fromGuard: Pipe
    }

    /// A write to a pipe whose reader is gone raises SIGPIPE, which ends a program that leaves it
    /// as it is. A guard that is gone is an error to deal with here, and not the end of the app,
    /// which would leave nothing to lock the Mac. It is set once, for the whole process.
    private static let ignoredBrokenPipes: Void = {
      _ = signal(SIGPIPE, SIG_IGN)
    }()

    private let executable: URL?
    private let arguments: [String]
    private let timing: GuardTiming
    private let mutex = NSLock()
    private var state = State.idle
    private var started: Started?
    private var lost: (() -> Void)?

    /// `executable` is the guard, which is inside the app. A program that is not there arms
    /// nothing. `arguments` are for tests, which start themselves as the guard.
    init(executable: URL?, arguments: [String] = [], timing: GuardTiming = GuardTiming()) {
      self.executable = executable
      self.arguments = arguments
      self.timing = timing
    }

    deinit {
      if let started { GuardLink.give(up: started) }
    }

    /// The number of the guard's process while it is armed, for the tests to find it by.
    var guardProcessID: Int32? {
      mutex.lock()
      defer { mutex.unlock() }
      return started?.process.processIdentifier
    }

    func arm(lost: @escaping () -> Void) throws {
      mutex.lock()
      guard state == .idle else {
        mutex.unlock()
        throw GuardFailure(sentence: "This Mac could not keep its screen private.")
      }
      mutex.unlock()
      let begun: Started
      do {
        begun = try start()
      } catch let failure as GuardFailure {
        throw failure
      } catch {
        NSLog("Baton: the guard could not be started (%@).", String(describing: error))
        throw GuardFailure(
          sentence: "This Mac could not start the guard that keeps a private session safe.")
      }
      let heard = begun.fromGuard.fileHandleForReading.fileDescriptor
      let toGuard = begun.toGuard.fileHandleForWriting.fileDescriptor
      guard firstLine(from: heard, within: timing.answer) == "ready \(guardProtocolVersion)",
        push(toGuard, "arm") == .sent, firstLine(from: heard, within: timing.answer) == "ok"
      else {
        // A guard that was armed and is slow to say so must not lock the Mac when its input closes:
        // nothing is private yet.
        _ = push(toGuard, "done")
        GuardLink.give(up: begun)
        throw GuardFailure(sentence: "The guard of this Mac did not answer.")
      }
      mutex.lock()
      state = .armed
      started = begun
      self.lost = lost
      mutex.unlock()
      // A guard that ended between the answer and here is told of.
      if !begun.process.isRunning { guardEnded() }
    }

    func beat() {
      mutex.lock()
      guard state == .armed, let started else {
        mutex.unlock()
        return
      }
      let sent = push(started.toGuard.fileHandleForWriting.fileDescriptor, "beat")
      mutex.unlock()
      // A pipe that is full means a guard that does not read, and it locks the Mac when the lapse
      // is over. One that is broken means a guard that is gone.
      if sent == .broken { guardEnded() }
    }

    func disarm() {
      mutex.lock()
      guard state == .armed, let started else {
        mutex.unlock()
        return
      }
      state = .disarmed
      lost = nil
      self.started = nil
      mutex.unlock()
      _ = push(started.toGuard.fileHandleForWriting.fileDescriptor, "done")
      _ = firstLine(
        from: started.fromGuard.fileHandleForReading.fileDescriptor, within: timing.answer)
      GuardLink.give(up: started)
    }

    /// Starts the guard and gives back its pipes. The first line it writes is not read yet.
    private func start() throws -> Started {
      guard let executable, FileManager.default.isExecutableFile(atPath: executable.path) else {
        throw GuardFailure(sentence: "This Mac has no guard to keep a private session safe.")
      }
      let process = Process()
      process.executableURL = executable
      process.arguments = arguments
      let toGuard = Pipe()
      let fromGuard = Pipe()
      process.standardInput = toGuard
      process.standardOutput = fromGuard
      process.standardError = FileHandle.nullDevice
      // The ends that stay here are not for the guard to keep: a guard that held the end its input
      // is written to would never see it close.
      _ = fcntl(toGuard.fileHandleForWriting.fileDescriptor, F_SETFD, FD_CLOEXEC)
      _ = fcntl(fromGuard.fileHandleForReading.fileDescriptor, F_SETFD, FD_CLOEXEC)
      _ = GuardLink.ignoredBrokenPipes
      let flags = fcntl(toGuard.fileHandleForWriting.fileDescriptor, F_GETFL)
      _ = fcntl(toGuard.fileHandleForWriting.fileDescriptor, F_SETFL, flags | O_NONBLOCK)
      process.terminationHandler = { [weak self] _ in self?.guardEnded() }
      try process.run()
      // The ends that went to the guard are closed here, so that a guard that is gone is seen as
      // one: the write fails, and the read ends.
      try? toGuard.fileHandleForReading.close()
      try? fromGuard.fileHandleForWriting.close()
      return Started(process: process, toGuard: toGuard, fromGuard: fromGuard)
    }

    /// The guard went away while it was armed: the session is told, once.
    private func guardEnded() {
      mutex.lock()
      guard state == .armed else {
        mutex.unlock()
        return
      }
      state = .lost
      let call = lost
      lost = nil
      let gone = started
      started = nil
      mutex.unlock()
      if let gone { GuardLink.give(up: gone) }
      call?()
    }

    /// Closes the pipes, which ends a guard that is still there, and makes sure it is gone.
    private static func give(up started: Started) {
      started.process.terminationHandler = nil
      try? started.toGuard.fileHandleForWriting.close()
      try? started.fromGuard.fileHandleForReading.close()
      let until = Date(timeIntervalSinceNow: 1)
      while started.process.isRunning && until.timeIntervalSinceNow > 0 {
        Thread.sleep(forTimeInterval: 0.01)
      }
      // A guard that was armed and did not end after `done` is told to, which locks nothing.
      if started.process.isRunning { started.process.terminate() }
    }
  }

#endif
