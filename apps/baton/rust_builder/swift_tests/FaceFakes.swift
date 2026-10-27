// SPDX-License-Identifier: Apache-2.0

// What the tests of the unlock with a face play with: ids and keys, a pair of keys that does what a
// test says, the room that the Rust side leaves for an answer, and a platform and a screen that
// take the calls of the table. The tests themselves are in FaceTests.swift, FaceAskerTests.swift
// and FaceShimTests.swift.

import Foundation

// MARK: - Small things

/// The id of a device: 32 bytes that differ with `seed`.
func faceId(_ seed: UInt8 = 1) -> [UInt8] {
  (0..<UInt8(FaceDevice.idBytes)).map { seed &+ $0 }
}

/// A public key as the Secure Enclave gives it: 04, then X and Y.
func facePoint(_ seed: UInt8 = 7) -> [UInt8] {
  [0x04] + [UInt8](repeating: seed, count: FacePublicKey.pointBytes - 1)
}

/// A key of a box with `count` bytes of `byte`.
func faceKey(_ byte: UInt8 = 0x5A, count: Int = Int(BATON_BOX_KEY_BYTES)) -> FaceBoxKey {
  let memory = UnsafeMutableRawBufferPointer.allocate(byteCount: max(count, 1), alignment: 1)
  memory.initializeMemory(as: UInt8.self, repeating: byte)
  return FaceBoxKey(
    adopting: UnsafeMutableRawBufferPointer(rebasing: memory[0..<count]),
    release: { _ in memory.deallocate() })
}

/// What the key of a box holds, or nil if it was wiped.
func faceBytes(_ key: FaceBoxKey) -> [UInt8]? {
  key.withBytes { Array($0) }
}

/// A question as the asker takes it.
func faceQuestion(
  _ number: UInt64 = 1, name: String = "Studio", seed: UInt8 = 1, sealedBytes: Int = 113
) -> FaceQuestion {
  FaceQuestion(
    number: number, sealed: [UInt8](repeating: 0x33, count: sealedBytes),
    device: FaceDevice(id: faceId(seed), name: name))
}

/// A value that one thread sets and another reads, behind a lock.
final class FaceShared<Value>: @unchecked Sendable {
  private let lock = NSLock()
  private var value: Value

  init(_ value: Value) {
    self.value = value
  }

  func get() -> Value {
    lock.lock()
    defer { lock.unlock() }
    return value
  }

  func set(_ new: Value) {
    lock.lock()
    value = new
    lock.unlock()
  }

  func update(_ change: (inout Value) -> Void) {
    lock.lock()
    change(&value)
    lock.unlock()
  }
}

/// Runs `body` on a thread of its own and says whether it returned within `seconds`: a call that
/// must not wait is shown not to, without the test waiting for it for ever.
func faceReturnsWithin(_ seconds: Double, _ body: @escaping () -> Void) -> Bool {
  let done = DispatchSemaphore(value: 0)
  Thread.detachNewThread {
    body()
    done.signal()
  }
  return done.wait(timeout: .now() + seconds) == .success
}

// MARK: - A pair of keys that is not real

/// One try to open a sealed key, as a test wants it to go.
final class FaceFakeAttempt: FaceAttempt {
  let sealed: [UInt8]
  let id: [UInt8]
  let reason: String

  /// What `open` gives when it ends. Set before the asker runs the attempt.
  var result: FaceResult = .declined(.declined)
  /// Whether `open` waits until it is let go: by `letGo`, or by a cancel if `endsOnCancel`.
  var waits = false
  /// Whether a cancel lets `open` go, as it does with a prompt that is taken down. A prompt that
  /// stays after a cancel is played by setting this to false.
  var endsOnCancel = true
  /// Signalled when `open` begins.
  let began = DispatchSemaphore(value: 0)

  private let gate = DispatchSemaphore(value: 0)
  private let lock = NSLock()
  private var opens = 0
  private var cancels = 0
  private var thread: Thread?

  init(sealed: [UInt8], id: [UInt8], reason: String) {
    self.sealed = sealed
    self.id = id
    self.reason = reason
  }

  func open() -> FaceResult {
    lock.lock()
    opens += 1
    thread = Thread.current
    lock.unlock()
    began.signal()
    if waits { gate.wait() }
    return result
  }

  func cancel() {
    lock.lock()
    cancels += 1
    lock.unlock()
    if endsOnCancel { gate.signal() }
  }

  /// Lets `open` go, as the person finishing with the prompt does.
  func letGo() {
    gate.signal()
  }

  var openCalls: Int {
    lock.lock()
    defer { lock.unlock() }
    return opens
  }

  var cancelCalls: Int {
    lock.lock()
    defer { lock.unlock() }
    return cancels
  }

  /// The thread that `open` ran on.
  var openThread: Thread? {
    lock.lock()
    defer { lock.unlock() }
    return thread
  }
}

/// The keys of a device, as a test wants them to go. It keeps what it was asked.
final class FaceFakeKeys: FaceKeys {
  private let lock = NSLock()
  private var requested: [FaceKeyRequestSeen] = []
  private var made: [FaceFakeAttempt] = []
  private var inside = 0
  private var most = 0

  /// What `publicKey` gives, unless `failure` is set.
  var point = facePoint()
  var failure: Error?
  /// How long `publicKey` takes, so that calls that may overlap do.
  var keyDelay: TimeInterval = 0
  /// Sets each attempt up as it is made: its number, from 0, and the attempt.
  var script: ((_ index: Int, _ attempt: FaceFakeAttempt) -> Void)?

  func publicKey(for id: [UInt8], renew: Bool) throws -> [UInt8] {
    lock.lock()
    requested.append(FaceKeyRequestSeen(id: id, renew: renew))
    inside += 1
    most = max(most, inside)
    let delay = keyDelay
    lock.unlock()
    if delay > 0 { Thread.sleep(forTimeInterval: delay) }
    lock.lock()
    inside -= 1
    let failure = self.failure
    let point = self.point
    lock.unlock()
    if let failure { throw failure }
    return point
  }

  func attempt(opening sealed: [UInt8], for id: [UInt8], reason: String) -> FaceAttempt {
    let attempt = FaceFakeAttempt(sealed: sealed, id: id, reason: reason)
    lock.lock()
    let index = made.count
    made.append(attempt)
    lock.unlock()
    script?(index, attempt)
    return attempt
  }

  var attempts: [FaceFakeAttempt] {
    lock.lock()
    defer { lock.unlock() }
    return made
  }

  var keyRequests: [FaceKeyRequestSeen] {
    lock.lock()
    defer { lock.unlock() }
    return requested
  }

  /// The most calls of `publicKey` that were running at the same moment.
  var mostAtOnce: Int {
    lock.lock()
    defer { lock.unlock() }
    return most
  }
}

/// A call of `publicKey` as the fake saw it.
struct FaceKeyRequestSeen: Equatable {
  var id: [UInt8]
  var renew: Bool
}

// MARK: - The answers

/// Where the answers of an asker go, in the tests that do not go to the Rust side. It keeps what
/// came, and the thread it came on, and the keys, so that a test can look at them afterwards.
final class FaceAnswerSink: @unchecked Sendable {
  enum Event: Equatable {
    case opened(number: UInt64, key: [UInt8])
    case declined(number: UInt64, reason: FaceDecline)
  }

  private let lock = NSLock()
  private var seen: [Event] = []
  private var threads: [Thread] = []
  private var keys: [FaceBoxKey] = []

  /// Run inside each answer, with no lock held. A test makes the answer wait here, as the call to
  /// the Rust side waits for the lock of the session. Set before the first question.
  var hook: (() -> Void)?

  var answers: FaceAsker.Answers {
    FaceAsker.Answers(
      opened: { [self] number, key in
        record(.opened(number: number, key: faceBytes(key) ?? []), key: key)
        hook?()
      },
      declined: { [self] number, reason in
        record(.declined(number: number, reason: reason), key: nil)
        hook?()
      })
  }

  private func record(_ event: Event, key: FaceBoxKey?) {
    lock.lock()
    seen.append(event)
    threads.append(Thread.current)
    if let key { keys.append(key) }
    lock.unlock()
  }

  var events: [Event] {
    lock.lock()
    defer { lock.unlock() }
    return seen
  }

  var answerThreads: [Thread] {
    lock.lock()
    defer { lock.unlock() }
    return threads
  }

  /// The keys that were opened, which the asker wipes once the answer is made.
  var openedKeys: [FaceBoxKey] {
    lock.lock()
    defer { lock.unlock() }
    return keys
  }
}

/// An asker over fake keys that answers into a sink, on a queue that the test can hold.
final class FaceRig {
  let keys = FaceFakeKeys()
  let sink = FaceAnswerSink()
  let queue = DispatchQueue(label: "io.github.jayunewal.baton.test.face", attributes: .concurrent)
  let asker: FaceAsker

  init() {
    asker = FaceAsker(keys: keys, answers: sink.answers, queue: queue)
  }
}

// MARK: - What the Rust side leaves and passes

/// A request for a key as the Rust side builds it.
func faceSource(
  _ arena: Arena, id: [UInt8] = faceId(), name: [UInt8] = Array("Studio".utf8), renew: UInt32 = 0
) -> BatonUnlockSource {
  BatonUnlockSource(
    size: UInt32(MemoryLayout<BatonUnlockSource>.size), renew: renew, id: arena.batonBytes(id),
    name: arena.batonBytes(name))
}

/// A question as the Rust side builds it.
func faceQuestionStruct(
  _ arena: Arena, number: UInt64 = 1, sealed: [UInt8] = [UInt8](repeating: 0x33, count: 113),
  id: [UInt8] = faceId(), name: [UInt8] = Array("Studio".utf8)
) -> BatonUnlockQuestion {
  BatonUnlockQuestion(
    size: UInt32(MemoryLayout<BatonUnlockQuestion>.size), reserved: 0, number: number,
    sealed: arena.batonBytes(sealed), id: arena.batonBytes(id), name: arena.batonBytes(name))
}

/// A question with the memory behind it, which is the Rust side's and is gone when the call
/// returns: `scrub` fills it with other bytes, as the Rust side may do the moment the call is over.
final class FaceQuestionMemory {
  private let sealed: UnsafeMutablePointer<UInt8>
  private let id: UnsafeMutablePointer<UInt8>
  private let name: UnsafeMutablePointer<UInt8>
  private let counts: (sealed: Int, id: Int, name: Int)
  var question: BatonUnlockQuestion

  init(number: UInt64, sealed: [UInt8], id: [UInt8], name: [UInt8]) {
    func copy(_ values: [UInt8]) -> UnsafeMutablePointer<UInt8> {
      let memory = UnsafeMutablePointer<UInt8>.allocate(capacity: max(values.count, 1))
      memory.initialize(repeating: 0, count: max(values.count, 1))
      for (at, byte) in values.enumerated() { memory[at] = byte }
      return memory
    }
    let sealedMemory = copy(sealed)
    let idMemory = copy(id)
    let nameMemory = copy(name)
    self.sealed = sealedMemory
    self.id = idMemory
    self.name = nameMemory
    counts = (sealed.count, id.count, name.count)
    question = BatonUnlockQuestion(
      size: UInt32(MemoryLayout<BatonUnlockQuestion>.size), reserved: 0, number: number,
      sealed: BatonBytes(ptr: sealedMemory, len: sealed.count),
      id: BatonBytes(ptr: idMemory, len: id.count),
      name: BatonBytes(ptr: nameMemory, len: name.count))
  }

  /// Overwrites what the question points to.
  func scrub() {
    sealed.initialize(repeating: 0xFF, count: max(counts.sealed, 1))
    id.initialize(repeating: 0xFF, count: max(counts.id, 1))
    name.initialize(repeating: 0xFF, count: max(counts.name, 1))
  }

  deinit {
    sealed.deallocate()
    id.deallocate()
    name.deallocate()
  }
}

/// Room for a public key as the Rust side leaves it: the struct and the bytes behind it, which are
/// exactly as many as the interface promises, so that a write past them is found by the address
/// sanitizer.
final class FaceKeyRoom {
  let capacity: Int
  private let bytes: UnsafeMutablePointer<UInt8>
  var key: BatonUnlockKey

  init(capacity: Int = Int(BATON_UNLOCK_KEY_MAX), fill: UInt8 = 0xEE) {
    let memory = UnsafeMutablePointer<UInt8>.allocate(capacity: max(capacity, 1))
    memory.initialize(repeating: fill, count: max(capacity, 1))
    self.capacity = capacity
    bytes = memory
    key = BatonUnlockKey(
      size: UInt32(MemoryLayout<BatonUnlockKey>.size), sealing: 0, len: 0, reserved: 0, key: memory)
  }

  /// All the bytes of the room.
  var contents: [UInt8] {
    Array(UnsafeBufferPointer(start: bytes, count: capacity))
  }

  /// The bytes that `len` says were written.
  var written: [UInt8] {
    Array(contents.prefix(Int(key.len)))
  }

  deinit {
    bytes.deallocate()
  }
}

/// Room for a sentence as the Rust side leaves it.
final class FaceMessageRoom {
  private let text: UnsafeMutablePointer<UInt8>
  var message: BatonMessage

  init(room: Int = Int(BATON_MESSAGE_MAX)) {
    let memory = UnsafeMutablePointer<UInt8>.allocate(capacity: max(room, 1))
    memory.initialize(repeating: 0, count: max(room, 1))
    text = memory
    message = BatonMessage(
      size: UInt32(MemoryLayout<BatonMessage>.size), cap: UInt32(room), len: 0, reserved: 0,
      text: memory)
  }

  /// What was written, never more than the room.
  var sentence: String {
    let count = Int(min(message.len, message.cap))
    return String(decoding: UnsafeBufferPointer(start: text, count: count), as: UTF8.self)
  }

  deinit {
    text.deallocate()
  }
}

// MARK: - A platform and a screen

/// A screen that keeps the calls about the face.
final class FaceScreen: ViewerScreen {
  private let lock = NSLock()
  private var asked: [FaceQuestion] = []
  private var cancels = 0

  /// What `unlockOpen` answers.
  var takes = true

  func decode(_ frame: FrameInput) -> Bool { true }
  func show(_ number: UInt64) {}
  func source(_ message: SourceInput) {}
  func state() -> DecoderReport { DecoderReport(decodeMicros: 0, thermal: 0) }
  func ended() {}
  func released() {}

  func unlockOpen(_ question: FaceQuestion) -> Bool {
    lock.lock()
    asked.append(question)
    lock.unlock()
    return takes
  }

  func unlockCancel() {
    lock.lock()
    cancels += 1
    lock.unlock()
  }

  var questions: [FaceQuestion] {
    lock.lock()
    defer { lock.unlock() }
    return asked
  }

  var cancelCalls: Int {
    lock.lock()
    defer { lock.unlock() }
    return cancels
  }
}

/// A platform that can unlock, or says it cannot, and gives the key a test wants.
final class FacePlatform: ViewerPlatform {
  let limits = ViewerLimits(maxWidth: 1, maxHeight: 1, maxFps: 1, codecs: 1)
  let canUnlock: Bool
  var playsSound = false
  let screen = FaceScreen()
  /// A screen that is not the one above, for a test about the screens that take no question.
  var otherScreen: ViewerScreen?

  private let lock = NSLock()
  private var seen: [FaceKeyRequest] = []
  private let outcome: Result<FacePublicKey, Error>

  init(
    canUnlock: Bool = true,
    outcome: Result<FacePublicKey, Error> = .success(FacePublicKey(point: facePoint()))
  ) {
    self.canUnlock = canUnlock
    self.outcome = outcome
  }

  func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen {
    MadeScreen(screen: otherScreen ?? screen, id: 3)
  }

  func unlockKey(_ request: FaceKeyRequest) throws -> FacePublicKey {
    lock.lock()
    seen.append(request)
    lock.unlock()
    return try outcome.get()
  }

  /// The requests that reached the platform.
  var requests: [FaceKeyRequest] {
    lock.lock()
    defer { lock.unlock() }
    return seen
  }
}

/// A platform as the device makes one: the screens it makes are windows with an asker of their own,
/// over the keys that a test gives, and the key is asked of the same object.
final class FaceUnlockPlatform: ViewerPlatform {
  let limits = ViewerLimits(maxWidth: 1, maxHeight: 1, maxFps: 1, codecs: 1)
  let canUnlock = true
  let unlock: FaceUnlock

  private let lock = NSLock()
  private var madeAskers: [FaceAsker] = []
  private var links: [UInt64] = []

  init(keys: FaceKeys) {
    unlock = FaceUnlock(keys: keys)
  }

  func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen {
    let asker = unlock.asker(link: link)
    lock.lock()
    madeAskers.append(asker)
    links.append(link)
    lock.unlock()
    let screen = WindowScreen(
      decoder: FakeDecoder(), present: { _ in }, failed: {}, thermal: { 0 }, onRelease: {},
      asker: asker)
    return MadeScreen(screen: screen, id: Int64(link))
  }

  func unlockKey(_ request: FaceKeyRequest) throws -> FacePublicKey {
    try unlock.key(for: request)
  }

  /// The askers of the screens that were made, in order.
  var askers: [FaceAsker] {
    lock.lock()
    defer { lock.unlock() }
    return madeAskers
  }
}
