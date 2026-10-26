// SPDX-License-Identifier: Apache-2.0

// The helper of the checks for the privacy mode of a Mac (docs/research/privacy-mode/README.md). It is
// not sandboxed, and the system starts it for the person: the app of the checks registers it with
// SMAppService, so that it is what a guard of the privacy mode would be.
//
//   guard                 the agent: writes what it sees to a file once a second, makes a tap now and
//                         then to see whether it may, and takes a lease that the app renews, in a file
//                         and in a datagram on the loopback
//   guard lock <method>   one try at locking the Mac without the consent of the person: sac, cgsession
//                         or displaysleep, and how long the session takes to say that it is locked

import ApplicationServices
import CoreGraphics
import Foundation

let dir = "/private/tmp/baton-probe"
let leasePort: UInt16 = 47831

func note(_ name: String, _ text: String) {
  try? text.write(toFile: dir + "/" + name, atomically: true, encoding: .utf8)
}

func seconds(_ value: Double) -> String { String(format: "%.1f", value) }

/// Whether the session says that the screen is locked. This is the key that Baton reads.
func sessionLocked() -> Bool {
  guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
  return (session["CGSSessionScreenIsLocked"] as? NSNumber)?.boolValue == true
}

// MARK: - Locking without consent

typealias LockScreen = @convention(c) () -> Int32

func run(_ path: String, _ arguments: [String]) -> String {
  let process = Process()
  process.executableURL = URL(fileURLWithPath: path)
  process.arguments = arguments
  do { try process.run() } catch { return "could not be run: \(error)" }
  process.waitUntilExit()
  return "exited with \(process.terminationStatus)"
}

func lock(method: String) -> String {
  switch method {
  case "sac":
    guard let handle = dlopen("/System/Library/PrivateFrameworks/login.framework/login", RTLD_NOW),
      let symbol = dlsym(handle, "SACLockScreenImmediate")
    else { return "the call is not there" }
    return "returned \(unsafeBitCast(symbol, to: LockScreen.self)())"
  case "cgsession":
    return run(
      "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession", ["-suspend"])
  case "displaysleep":
    return run("/usr/bin/pmset", ["displaysleepnow"])
  default:
    return "no such method"
  }
}

func tryLock(method: String) {
  let before = sessionLocked()
  let began = Date()
  let result = lock(method: method)
  var after: Double?
  for _ in 0..<80 {
    if sessionLocked() {
      after = Date().timeIntervalSince(began)
      break
    }
    Thread.sleep(forTimeInterval: 0.1)
  }
  let asleep = CGDisplayIsAsleep(CGMainDisplayID()) != 0
  print(
    "lock \(method): locked before \(before), the call \(result), the session says locked "
      + "\(after.map { "after \(seconds($0)) s" } ?? "never within 8 s"), display asleep \(asleep)")
  guard after != nil || asleep else { return }
  // The person wakes the Mac and unlocks it, and the next try waits for that.
  let waited = Date()
  while Date().timeIntervalSince(waited) < 180 {
    if !sessionLocked() && CGDisplayIsAsleep(CGMainDisplayID()) == 0 { break }
    Thread.sleep(forTimeInterval: 0.5)
  }
  print("lock \(method): the person was back after \(seconds(Date().timeIntervalSince(waited))) s")
}

// MARK: - The agent

/// Whether an active tap can be made, which is what the privacy mode needs of a helper that holds
/// the input. It passes everything and is gone at once.
func tapMakes() -> Bool {
  let mask: CGEventMask = CGEventMask(1) << CGEventMask(CGEventType.mouseMoved.rawValue)
  guard
    let port = CGEvent.tapCreate(
      tap: .cghidEventTap, place: .headInsertEventTap, options: .defaultTap, eventsOfInterest: mask,
      callback: { _, _, event, _ in Unmanaged.passUnretained(event) }, userInfo: nil)
  else { return false }
  CFMachPortInvalidate(port)
  return true
}

func openLoopbackSocket() -> Int32 {
  let descriptor = socket(AF_INET, SOCK_DGRAM, 0)
  guard descriptor >= 0 else { return -1 }
  var address = sockaddr_in()
  address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
  address.sin_family = sa_family_t(AF_INET)
  address.sin_port = leasePort.bigEndian
  address.sin_addr.s_addr = inet_addr("127.0.0.1")
  let bound = withUnsafePointer(to: &address) {
    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
      bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
    }
  }
  guard bound == 0 else {
    close(descriptor)
    return -1
  }
  var timeout = timeval(tv_sec: 0, tv_usec: 250_000)
  setsockopt(descriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
  return descriptor
}

func runAgent() -> Never {
  let started = Date()
  // A flag that the app leaves while a private session lasts. One that is there when the agent
  // starts is one that a crash left, and the agent would lock the Mac before anything else.
  let flagAtStart = FileManager.default.fileExists(atPath: dir + "/private-flag")
  let listener = openLoopbackSocket()
  var datagrams = 0
  var lastDatagram: Date?
  var lastRenewal: Date?
  var leased = false
  var lapses = 0
  var lapsedAfter = -1.0
  var tap = false
  var tapChecked = Date.distantPast
  var written = Date.distantPast
  // The system asks the person once to allow what a guard would use.
  _ = CGRequestListenEventAccess()
  _ = CGRequestPostEventAccess()

  while true {
    if listener >= 0 {
      var buffer = [UInt8](repeating: 0, count: 64)
      let count = recvfrom(listener, &buffer, buffer.count, 0, nil, nil)
      if count > 0 {
        datagrams += 1
        lastDatagram = Date()
      }
    } else {
      Thread.sleep(forTimeInterval: 0.25)
    }
    let now = Date()
    var fileAge: Double?
    if let attributes = try? FileManager.default.attributesOfItem(atPath: dir + "/lease"),
      let date = attributes[.modificationDate] as? Date
    {
      fileAge = now.timeIntervalSince(date)
    }
    let datagramAge = lastDatagram.map { now.timeIntervalSince($0) }
    let renewedAge = min(fileAge ?? .infinity, datagramAge ?? .infinity)
    let on = renewedAge < 3
    if on { lastRenewal = now }
    if leased && !on {
      lapses += 1
      lapsedAfter = now.timeIntervalSince(lastRenewal ?? now)
    }
    leased = on
    if now.timeIntervalSince(tapChecked) > 4 {
      tap = tapMakes()
      tapChecked = now
    }
    if now.timeIntervalSince(written) >= 1 {
      written = now
      let lines = [
        "pid=\(getpid())",
        "time=\(Int(now.timeIntervalSince1970))",
        "running_seconds=\(seconds(now.timeIntervalSince(started)))",
        "sandboxed=\(ProcessInfo.processInfo.environment["APP_SANDBOX_CONTAINER_ID"] != nil)",
        "found_flag_at_start=\(flagAtStart)",
        "session_known=\(CGSessionCopyCurrentDictionary() != nil)",
        "locked=\(sessionLocked())",
        "accessibility=\(AXIsProcessTrusted())",
        "input_monitoring=\(CGPreflightListenEventAccess())",
        "post_events=\(CGPreflightPostEventAccess())",
        "tap=\(tap)",
        "loopback=\(listener >= 0)",
        "datagrams=\(datagrams)",
        "file_lease_age=\(fileAge.map(seconds) ?? "none")",
        "datagram_age=\(datagramAge.map(seconds) ?? "none")",
        "lapses=\(lapses)",
        "last_lapse_after=\(seconds(lapsedAfter))",
      ]
      note("guard-heartbeat", lines.joined(separator: "\n") + "\n")
    }
  }
}

// MARK: - Which one

let arguments = CommandLine.arguments
if arguments.count > 2, arguments[1] == "lock" {
  tryLock(method: arguments[2])
  exit(0)
}
runAgent()
