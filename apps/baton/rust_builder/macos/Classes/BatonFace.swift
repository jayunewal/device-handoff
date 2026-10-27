// SPDX-License-Identifier: Apache-2.0

// The phone's side of unlocking a locked Mac with the face of its person
// (docs/adr/0013-unlock-from-a-phone.md). The Mac keeps its password in a box that only this device
// opens. For each Mac this device keeps a pair of keys whose private half is in the Secure Enclave
// and works only after the face of the person is recognised, and the Mac seals the key of the box
// to the public half. When the person taps Unlock, the Mac sends that sealed key, the system asks
// for the face, the Secure Enclave opens the sealed key, and the key of the box goes back to the
// Mac, once, inside the connection.
//
// This file holds what decides: the reading of what the Rust side passes, the one question that a
// screen has open and who may answer it, what is wiped and when, how a failure of the system
// becomes a number of the interface, and the answers to the Rust side. The pair of keys and the
// prompt for the face are given to it as protocols, and BatonFaceKeys.swift has the real ones. So
// that this file can be built and tested on any computer that has a Swift compiler
// (apps/baton/rust_builder/swift_tests), nothing here may import more than Foundation.
//
// A call from Rust must never end in a trap, because a trap ends the whole app: nothing here
// force-unwraps, indexes without looking, or asserts on what Rust passed. Nothing here writes the
// key of a box, a sealed key or the name of a device to a log.

import Foundation

// MARK: - What the Rust side passes

/// The other device: the one whose person sets an unlock up for this device's key, and whose
/// screen is unlocked. Its id is how this device finds the key it made for that device and no
/// other.
struct FaceDevice: Equatable, CustomStringConvertible {
  /// The bytes of the id of a device (BATON_DEVICE_ID_BYTES).
  static let idBytes = Int(BATON_DEVICE_ID_BYTES)
  /// The most bytes of a name that the protocol allows (MAX_NAME_BYTES in baton-proto). The Rust
  /// side checked it already, so a longer name is not a name that came from a device.
  static let mostNameBytes = 256

  var id: [UInt8]
  /// What the person called the device. It is shown in the prompt for the face and is not for a
  /// log.
  var name: String

  init(id: [UInt8], name: String) {
    self.id = id
    self.name = name
  }

  /// Reads the id and the name of the interface. nil if the id is not exactly `idBytes` long, the
  /// name is longer than `mostNameBytes`, or either is given a length and no pointer. A name that
  /// is not UTF-8 is repaired, not refused: it is only shown.
  init?(reading id: BatonBytes, name: BatonBytes) {
    guard let idView = UnsafeRawBufferPointer(reading: id), idView.count == FaceDevice.idBytes,
      let nameView = UnsafeRawBufferPointer(reading: name),
      nameView.count <= FaceDevice.mostNameBytes
    else { return nil }
    self.init(id: Array(idView), name: String(decoding: nameView, as: UTF8.self))
  }

  var description: String { "a device with an id of \(id.count) bytes" }
}

/// What the person at another device asks of this one when they set an unlock up: the key that the
/// unlock is sealed to (`BatonUnlockSource`).
struct FaceKeyRequest: Equatable, CustomStringConvertible {
  var device: FaceDevice
  /// Replace the pair of keys first, so that what was sealed to the old key opens nothing from then
  /// on. Without it the pair that was made before is given, and one is made if there is none.
  var renew: Bool

  init(device: FaceDevice, renew: Bool) {
    self.device = device
    self.renew = renew
  }

  /// Reads a request of the interface. nil if the struct is shorter than this version of the
  /// interface knows it, `renew` is neither 0 nor 1, or the id or the name is not as `FaceDevice`
  /// wants it. A key is replaced only when the Rust side clearly said so.
  init?(reading source: UnsafePointer<BatonUnlockSource>) {
    // The size is read alone first: a struct that is shorter than this version of the interface
    // knows it is not copied, since the copy would read past its end.
    guard source.pointee.size >= UInt32(MemoryLayout<BatonUnlockSource>.size) else { return nil }
    let c = source.pointee
    guard c.renew <= 1, let device = FaceDevice(reading: c.id, name: c.name) else { return nil }
    self.init(device: device, renew: c.renew == 1)
  }

  var description: String { "a request for a key, renew \(renew)" }
}

/// The question that the other device asks, which the person answers with their face
/// (`BatonUnlockQuestion`). Everything in it is a copy: the memory the Rust side passed is valid
/// only until the call returns. The sealed key is a secret, so it is not printed.
struct FaceQuestion: Equatable, CustomStringConvertible {
  /// The bytes of a point of P-256 and of the tag of AES-GCM: a key sealed with ECIES is longer
  /// than these two together, and one that is shorter cannot be opened, so the person is not asked
  /// for their face for it.
  static let leastSealedBytes = 65 + 16
  static let mostSealedBytes = Int(BATON_UNLOCK_SEALED_MAX)

  /// The number that the answer repeats.
  var number: UInt64
  var sealed: [UInt8]
  var device: FaceDevice

  init(number: UInt64, sealed: [UInt8], device: FaceDevice) {
    self.number = number
    self.sealed = sealed
    self.device = device
  }

  /// Reads a question of the interface. nil if the struct is shorter than this version of the
  /// interface knows it, the sealed key is not between `leastSealedBytes` and `mostSealedBytes`
  /// long, or the id or the name is not as `FaceDevice` wants it.
  init?(reading question: UnsafePointer<BatonUnlockQuestion>) {
    guard question.pointee.size >= UInt32(MemoryLayout<BatonUnlockQuestion>.size) else {
      return nil
    }
    let c = question.pointee
    guard let sealed = UnsafeRawBufferPointer(reading: c.sealed),
      sealed.count >= FaceQuestion.leastSealedBytes, sealed.count <= FaceQuestion.mostSealedBytes,
      let device = FaceDevice(reading: c.id, name: c.name)
    else { return nil }
    self.init(number: c.number, sealed: Array(sealed), device: device)
  }

  var description: String { "question \(number) with a sealed key of \(sealed.count) bytes" }
}

// MARK: - What the Rust side is given

/// The public key that an unlock is sealed to, and how it is sealed.
struct FacePublicKey: Equatable {
  /// A BATON_SEALING_ number.
  var sealing: UInt32
  var bytes: [UInt8]

  /// The bytes of a point of P-256 in the uncompressed form of ANSI X9.63: 04, X and Y.
  static let pointBytes = 65

  /// Whether `bytes` can be a point of P-256 in the uncompressed form. The point is not checked to
  /// be on the curve: the system does that when the Mac seals to it.
  static func isPointOfP256(_ bytes: [UInt8]) -> Bool {
    bytes.count == pointBytes && bytes.first == 0x04
  }

  init(sealing: UInt32, bytes: [UInt8]) {
    self.sealing = sealing
    self.bytes = bytes
  }

  /// A point of P-256, sealed to with ECIES.
  init(point: [UInt8]) {
    self.init(sealing: BATON_SEALING_ECIES_P256, bytes: point)
  }

  /// Writes the key into the room the Rust side gave, and says whether it did. Nothing is written
  /// if the struct is shorter than this version of the interface knows it, there is no room, or
  /// the key is empty or larger than BATON_UNLOCK_KEY_MAX, since the Rust side reads what an
  /// out argument holds only when the call succeeds.
  func write(to key: UnsafeMutablePointer<BatonUnlockKey>) -> Bool {
    guard key.pointee.size >= UInt32(MemoryLayout<BatonUnlockKey>.size),
      let room = key.pointee.key, !bytes.isEmpty, bytes.count <= Int(BATON_UNLOCK_KEY_MAX)
    else { return false }
    for (at, byte) in bytes.enumerated() {
      room[at] = byte
    }
    key.pointee.len = UInt32(bytes.count)
    key.pointee.sealing = sealing
    return true
  }
}

/// Why a key could not be given, as the number that `unlock_key` returns and a sentence. The Rust
/// side tells the other device only that it failed, and writes the sentence to the log of this
/// device, so the sentence holds no key and no name or id of a device. It is written for a person
/// all the same, in case a later version of the interface shows it.
enum FaceKeyFailure: Error, Equatable {
  /// The person at this device said no (BATON_KEY_REFUSED). Nothing asks them yet, so no key
  /// ends here; the number is kept for the day something does.
  case refused(String)
  /// The settings of this device do not let that device ask (BATON_KEY_BLOCKED).
  case blocked(String)
  /// This device cannot make such a key: it has nothing to keep one in, or no face that it
  /// recognises (BATON_KEY_UNSUPPORTED).
  case unsupported(String)
  /// Anything else that went wrong here (BATON_E_PLATFORM).
  case failed(String)

  var code: Int32 {
    switch self {
    case .refused: return Int32(BATON_KEY_REFUSED)
    case .blocked: return Int32(BATON_KEY_BLOCKED)
    case .unsupported: return Int32(BATON_KEY_UNSUPPORTED)
    case .failed: return Int32(BATON_E_PLATFORM)
    }
  }

  var sentence: String {
    switch self {
    case .refused(let sentence), .blocked(let sentence), .unsupported(let sentence),
      .failed(let sentence):
      return sentence
    }
  }

  static let noEnclave = FaceKeyFailure.unsupported(
    "This device has no Secure Enclave, so it cannot keep a key that only your face opens.")
  static let noPasscode = FaceKeyFailure.unsupported(
    "This device has no passcode. Set one in Settings, set up Face ID, and try again.")
  static let noFace = FaceKeyFailure.unsupported(
    "Face ID or Touch ID is not set up on this device. Set it up in Settings and try again.")
  static let faceUnavailable = FaceKeyFailure.unsupported(
    "Face ID or Touch ID is not available to Baton on this device. Check Settings and try again.")
  static let faceLocked = FaceKeyFailure.failed(
    "Face ID or Touch ID is locked after too many tries. Unlock this device with its passcode "
      + "and try again.")
  static let deviceLocked = FaceKeyFailure.failed(
    "This device is locked. Unlock it, keep Baton open, and try again.")
  static let cannotMake = FaceKeyFailure.failed(
    "The key for the unlock could not be made on this device.")
  static let cannotOpen = FaceKeyFailure.unsupported(
    "This device cannot open a key that is sealed in the way the unlock needs.")
  static let notAPoint = FaceKeyFailure.failed(
    "The key that this device made for the unlock is not one that the Mac can seal to.")
}

/// Why a question was not opened, as the BATON_FACE_ number that `baton_stream_unlock_declined`
/// is given.
enum FaceDecline: Equatable {
  /// The person did not agree, their face was not recognised, or they cancelled.
  case declined
  /// This device no longer has the key that opens the sealed key. Only the person at the other
  /// device can set the unlock up again, so this is said only when the system said so.
  case keyLost
  /// Anything else that went wrong here.
  case failed

  var reason: UInt32 {
    switch self {
    case .declined: return BATON_FACE_DECLINED
    case .keyLost: return BATON_FACE_KEY_LOST
    case .failed: return BATON_FACE_FAILED
    }
  }
}

// MARK: - What the system says when a key cannot be used

/// An error of the system, as the domain and the code that name it. The system gives the same
/// trouble in different clothes (a status of the keychain, with an error of LocalAuthentication
/// under it), so the whole chain is looked at, and only these two parts of it: nothing else in an
/// error is needed, and some of it names the app or the person.
struct FaceSystemError: Equatable {
  var domain: String
  var code: Int

  /// The most errors of one chain that are looked at.
  static let deepest = 8

  /// The error and the errors under it, outermost first.
  static func chain(of error: Error) -> [FaceSystemError] {
    var chain: [FaceSystemError] = []
    var next: NSError? = error as NSError
    while let current = next, chain.count < deepest {
      chain.append(FaceSystemError(domain: current.domain, code: current.code))
      next = current.userInfo[NSUnderlyingErrorKey] as? NSError
    }
    return chain
  }

  /// The chain as one line for a log: domains and codes, and nothing else.
  static func line(_ chain: [FaceSystemError]) -> String {
    chain.map { "\($0.domain) \($0.code)" }.joined(separator: ", under ")
  }
}

/// The domains and the codes that the classification below knows. The numbers are those of the
/// headers of the system (LAError.h, SecBase.h); a test that runs on a Mac holds them to the
/// constants of the SDK (FaceCodesTests.swift), since a wrong number here would only show as an
/// error that is sorted wrongly.
enum FaceSystemCodes {
  /// LAError.errorDomain.
  static let localAuthentication = "com.apple.LocalAuthentication"
  /// The domain of a status of Security.framework (NSOSStatusErrorDomain).
  static let osStatus = "NSOSStatusErrorDomain"

  // LocalAuthentication: LAError.Code.
  static let authenticationFailed = -1
  static let userCancel = -2
  static let userFallback = -3
  static let systemCancel = -4
  static let passcodeNotSet = -5
  static let biometryNotAvailable = -6
  static let biometryNotEnrolled = -7
  static let biometryLockout = -8
  static let appCancel = -9
  static let invalidContext = -10
  static let notInteractive = -1004

  // Security: OSStatus.
  static let unimplemented = -4
  static let param = -50
  static let userCanceled = -128
  static let authFailed = -25293
  static let itemNotFound = -25300
  static let interactionNotAllowed = -25308
}

extension FaceDecline {
  /// Sorts what the system said when the key could not be used into the three reasons.
  ///
  /// What is certain, from the documentation: LocalAuthentication names a cancel by the person
  /// (userCancel), by the system (systemCancel) and by the app (appCancel, which is what
  /// `invalidate()` causes), a face that was not recognised (authenticationFailed), a lockout
  /// after too many tries (biometryLockout), and no passcode or no enrolled face (passcodeNotSet,
  /// biometryNotEnrolled), which is what a key that needs a face has come to when the passcode was
  /// removed or every face was. An item that is not in the keychain is errSecItemNotFound, which is
  /// what a key that was never made, was deleted, or went with a removed passcode comes to
  /// (kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly deletes it).
  ///
  /// What is not certain, and is for the checks on a device (U5 of the record): which error the
  /// system gives for a key that was invalidated by a face that was added or removed
  /// (.biometryCurrentSet says only that the item "is invalidated"). If it is errSecItemNotFound
  /// the key is lost, as here. If it is errSecAuthFailed, nothing tells it from a face that was not
  /// recognised, and it is sorted with that one: the person tries again, and a key that is lost
  /// keeps failing until they set the unlock up again at the other device. An error that is none
  /// of these is `failed`, which ends nothing at the other device.
  init(system errors: [FaceSystemError]) {
    // LocalAuthentication is more specific than the status that wraps it, so it decides first.
    if let error = errors.first(where: { $0.domain == FaceSystemCodes.localAuthentication }) {
      self = FaceDecline.localAuthentication(error.code)
      return
    }
    for error in errors where error.domain == FaceSystemCodes.osStatus {
      if let known = FaceDecline.status(error.code) {
        self = known
        return
      }
    }
    self = .failed
  }

  private static func localAuthentication(_ code: Int) -> FaceDecline {
    switch code {
    case FaceSystemCodes.authenticationFailed, FaceSystemCodes.userCancel,
      FaceSystemCodes.userFallback, FaceSystemCodes.systemCancel, FaceSystemCodes.biometryLockout,
      FaceSystemCodes.appCancel:
      return .declined
    case FaceSystemCodes.passcodeNotSet, FaceSystemCodes.biometryNotEnrolled:
      return .keyLost
    default:
      return .failed
    }
  }

  private static func status(_ code: Int) -> FaceDecline? {
    switch code {
    case FaceSystemCodes.userCanceled, FaceSystemCodes.authFailed:
      return .declined
    case FaceSystemCodes.itemNotFound:
      return .keyLost
    default:
      return nil
    }
  }
}

extension FaceKeyFailure {
  /// What `canEvaluatePolicy` said when it would not allow the biometric policy: a key that needs
  /// a face cannot be made on a device that has no passcode, no face or no hardware for one.
  /// `error` is nil if it said no and gave no reason.
  init(biometry error: FaceSystemError?) {
    guard let error, error.domain == FaceSystemCodes.localAuthentication else {
      self = .faceUnavailable
      return
    }
    switch error.code {
    case FaceSystemCodes.passcodeNotSet: self = .noPasscode
    case FaceSystemCodes.biometryNotEnrolled: self = .noFace
    case FaceSystemCodes.biometryNotAvailable: self = .faceUnavailable
    case FaceSystemCodes.biometryLockout: self = .faceLocked
    default: self = .cannotMake
    }
  }

  /// What the keychain or Security.framework said when a step of making the key did not work.
  /// The step and the codes go to the log, which is where whoever tries the device looks.
  init(step: String, system errors: [FaceSystemError]) {
    NSLog(
      "Baton: the key for the unlock could not be made: it did not %@ (%@).", step,
      FaceSystemError.line(errors))
    if let found = errors.first(where: { $0.domain == FaceSystemCodes.localAuthentication }) {
      self.init(biometry: found)
      return
    }
    let statuses = errors.filter { $0.domain == FaceSystemCodes.osStatus }.map { $0.code }
    if statuses.contains(FaceSystemCodes.interactionNotAllowed) {
      self = .deviceLocked
    } else if statuses.contains(FaceSystemCodes.unimplemented) {
      self = .noEnclave
    } else {
      self = .cannotMake
    }
  }
}

// MARK: - The keys of this device and the face

/// What came of a try to open a sealed key.
enum FaceResult {
  /// The face was recognised and the sealed key is open: the key of the box.
  case opened(FaceBoxKey)
  case declined(FaceDecline)

  /// Wipes the key, if there is one: the answer is not sent.
  func discard() {
    if case .opened(let key) = self { key.wipe() }
  }
}

/// One try to open a sealed key with the private half of the pair of one device. A device that
/// asks for the face, and the system that shows the prompt, are behind it
/// (BatonFaceKeys.swift).
protocol FaceAttempt: AnyObject {
  /// Asks for the face and opens the sealed key. It waits for as long as the person takes, so it
  /// is called from a queue of its own and never from inside a call of the Rust side. Called at
  /// most once. It does not ask if `cancel` came first.
  func open() -> FaceResult

  /// The question is not open any more: takes the prompt down, and makes `open` end at once, or
  /// never begin. It does not wait for anything, since the Rust side calls it with the lock of the
  /// session held; it may be called from any thread, before, during or after `open`, and more
  /// than once.
  func cancel()
}

/// The pairs of keys that this device keeps, one for each device that it may unlock.
protocol FaceKeys: AnyObject {
  /// The 65 bytes of the public half of the pair for the device `id`: made if there is none, and
  /// replaced first if `renew`. The private half stays in the Secure Enclave and opens a sealed
  /// key only after the face of the person is recognised, with no passcode as a way round. Throws
  /// `FaceKeyFailure`. Called on a thread that may wait, one call at a time (`FaceUnlock`).
  func publicKey(for id: [UInt8], renew: Bool) throws -> [UInt8]

  /// A try to open `sealed` with the pair for the device `id` and with no other pair: a sealed key
  /// does not say which key it was sealed to, and a device that tried its keys in turn would open
  /// one that was copied from another device. `reason` is what the prompt for the face says.
  /// Returns at once and asks for nothing: the person is asked when `open` is called.
  func attempt(opening sealed: [UInt8], for id: [UInt8], reason: String) -> FaceAttempt
}

// MARK: - A secret that is wiped

/// The key of a box, in memory that is wiped when the key is let go of, and before that on
/// `wipe()`. Nothing about it is printed.
///
/// The key opens the password only together with the box that stays at the Mac, but a key that was
/// opened and not sent is good for one unlock without a face (the record says so), so every path
/// that ends with a key ends with a wipe: the one that sends it, the one that drops it, and the
/// one that finds it too long or too short.
final class FaceBoxKey: CustomStringConvertible {
  private let memory: UnsafeMutableRawBufferPointer
  private let release: (UnsafeMutableRawBufferPointer) -> Void
  private let lock = NSLock()
  private var wiped = false

  /// How many bytes the key has. It stays what it was after a wipe.
  let count: Int

  /// The most bytes a key may have. The key of a box has BATON_BOX_KEY_BYTES.
  static let mostBytes = 256

  /// Takes over `memory`, which holds the key. It is wiped, and then handed to `release`, which
  /// frees it, when the key is let go of. The tests give a `release` that looks at it first.
  init(
    adopting memory: UnsafeMutableRawBufferPointer,
    release: @escaping (UnsafeMutableRawBufferPointer) -> Void
  ) {
    self.memory = memory
    self.release = release
    self.count = memory.count
  }

  /// A copy of `bytes` in memory of its own. nil if there are none, or more than `mostBytes`.
  convenience init?(copying bytes: UnsafeRawBufferPointer) {
    guard bytes.count > 0, bytes.count <= FaceBoxKey.mostBytes else { return nil }
    let memory = UnsafeMutableRawBufferPointer.allocate(byteCount: bytes.count, alignment: 1)
    memory.copyMemory(from: bytes)
    self.init(adopting: memory, release: { $0.deallocate() })
  }

  /// Gives the bytes to `body` for as long as it runs, or nil, without calling it, if the key was
  /// wiped. The bytes are not to be kept.
  func withBytes<Output>(_ body: (UnsafeRawBufferPointer) -> Output) -> Output? {
    lock.lock()
    defer { lock.unlock() }
    if wiped { return nil }
    return body(UnsafeRawBufferPointer(memory))
  }

  var isWiped: Bool {
    lock.lock()
    defer { lock.unlock() }
    return wiped
  }

  /// Overwrites the key with zeros. More than once is fine.
  func wipe() {
    lock.lock()
    defer { lock.unlock() }
    if wiped { return }
    wiped = true
    guard let base = memory.baseAddress, memory.count > 0 else { return }
    // A call that the compiler may not leave out, as a plain loop of stores may be left out when
    // the memory is freed next. The sizes of `memset_s` are of the type `rsize_t`, which Swift
    // may not take for an `Int`, so they are converted to whatever type it names.
    #if canImport(Darwin)
      _ = memset_s(base, numericCast(memory.count), 0, numericCast(memory.count))
    #else
      explicit_bzero(base, memory.count)
    #endif
  }

  deinit {
    wipe()
    release(memory)
  }

  var description: String { "a key of \(count) bytes" }
}

// MARK: - The question that a screen has open

/// The one question that a screen has open, and the work of asking for the face.
///
/// The Rust side calls `ask` with the lock of the session held, so it hands the question to a queue
/// of its own and returns. The work there waits for the person, and then answers from that queue,
/// never from inside a call of the Rust side, and exactly once: the question is taken by the work
/// when it ends, or by `cancel` or `close`, which come from the Rust side, and whichever is first
/// decides. A question that was taken back is not answered at all, and the key that it opened, if
/// it opened one, is wiped.
///
/// The answer is made with no lock of this class held, because the Rust side holds its own lock
/// when it calls `cancel` and takes it when it is answered. So an answer that was taken just before
/// a cancel still goes out, and the Rust side refuses it with BATON_E_UNKNOWN_LINK, which is how it
/// treats every answer to a question that is not open. Nothing is sent that the Rust side would
/// take for the answer to another question: it is given the number that it asked with.
///
/// Each question is worked on by itself, so a prompt that does not go away after a cancel does not
/// hold back the next question.
final class FaceAsker {
  /// Where an answer goes. On a device it is the Rust side (`FaceAnswers.toRust`).
  struct Answers {
    /// The face was recognised and the sealed key is open. The key is wiped after this returns.
    var opened: (_ number: UInt64, _ key: FaceBoxKey) -> Void
    var declined: (_ number: UInt64, _ reason: FaceDecline) -> Void
  }

  private final class Asked {
    let number: UInt64
    let attempt: FaceAttempt

    init(number: UInt64, attempt: FaceAttempt) {
      self.number = number
      self.attempt = attempt
    }
  }

  private let keys: FaceKeys
  private let answers: Answers
  private let queue: DispatchQueue
  private let lock = NSLock()
  private var current: Asked?
  private var closed = false

  /// `queue` is where the work waits for the person. It is the asker's own unless the tests give
  /// one, and it must be able to run the work of two questions at once.
  init(keys: FaceKeys, answers: Answers, queue: DispatchQueue? = nil) {
    self.keys = keys
    self.answers = answers
    self.queue =
      queue
      ?? DispatchQueue(
        label: "io.github.jayunewal.baton.face", qos: .userInitiated, attributes: .concurrent)
  }

  /// The person tapped Unlock: takes the question to a queue of its own and returns at once.
  /// false if it cannot be taken: another is open, or the screen has ended. Then nothing is asked
  /// and nothing is answered.
  func ask(_ question: FaceQuestion) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    if closed || current != nil { return false }
    let attempt = keys.attempt(
      opening: question.sealed, for: question.device.id,
      reason: FacePrompt.reason(for: question.device.name))
    let asked = Asked(number: question.number, attempt: attempt)
    current = asked
    queue.async { [self] in run(asked) }
    return true
  }

  /// The question is not open any more: the prompt comes down and nothing is answered for it. A
  /// question that was answered already, or none, changes nothing.
  func cancel() {
    lock.lock()
    let taken = current
    current = nil
    lock.unlock()
    taken?.attempt.cancel()
  }

  /// The screen is over: as `cancel`, and no question is taken after this.
  func close() {
    lock.lock()
    closed = true
    let taken = current
    current = nil
    lock.unlock()
    taken?.attempt.cancel()
  }

  /// Waits until the work that was handed to the asker is done, for the tests.
  func idle() {
    queue.sync(flags: .barrier) {}
  }

  private func isCurrent(_ asked: Asked) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return !closed && current === asked
  }

  /// Takes the question to answer it. false if it was taken back first, or answered by another.
  private func claim(_ asked: Asked) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard !closed, current === asked else { return false }
    current = nil
    return true
  }

  private func run(_ asked: Asked) {
    // Taken back before this had its turn: the person is not asked for their face at all.
    guard isCurrent(asked) else { return }
    let result = asked.attempt.open()
    guard claim(asked) else {
      result.discard()
      return
    }
    switch result {
    case .opened(let key):
      defer { key.wipe() }
      // The key of a box has the length that the interface says, and a key of any other length is
      // not sent: the Mac would not know what to do with it, and it is no key of ours.
      if key.count == Int(BATON_BOX_KEY_BYTES) {
        answers.opened(asked.number, key)
      } else {
        answers.declined(asked.number, .failed)
      }
    case .declined(let reason):
      answers.declined(asked.number, reason)
    }
  }
}

// MARK: - A device

/// What a device does to let another device unlock its screen with the face of its person: it
/// keeps the pairs of keys, and makes the asker that each screen has.
final class FaceUnlock {
  private let keys: FaceKeys
  /// Held while a pair is made, found or replaced, so that two calls for the same device never run
  /// into each other: a pair is found by a tag, and two pairs under one tag could not be told
  /// apart.
  private let working = NSLock()

  init(keys: FaceKeys) {
    self.keys = keys
  }

  /// The key that the unlock of `request.device` is sealed to. Throws `FaceKeyFailure`. A key that
  /// is not a point of P-256 is not given on: the Mac would take a device that sends one for a
  /// device that broke the protocol.
  func key(for request: FaceKeyRequest) throws -> FacePublicKey {
    working.lock()
    defer { working.unlock() }
    let point = try keys.publicKey(for: request.device.id, renew: request.renew)
    guard FacePublicKey.isPointOfP256(point) else { throw FaceKeyFailure.notAPoint }
    return FacePublicKey(point: point)
  }

  /// The asker of the screen whose link is `link`.
  func asker(link: UInt64) -> FaceAsker {
    FaceAsker(keys: keys, answers: FaceAnswers.toRust(link: link))
  }
}

// MARK: - The answers to the Rust side

enum FaceAnswers {
  /// The answers that go to the Rust side, for the screen whose link is `link`. They are called
  /// from the queue of the asker and never from inside a call of the Rust side.
  static func toRust(link: UInt64) -> FaceAsker.Answers {
    FaceAsker.Answers(
      opened: { number, key in
        let code = key.withBytes { bytes -> Int32 in
          let view = BatonBytes(
            ptr: bytes.baseAddress?.assumingMemoryBound(to: UInt8.self), len: bytes.count)
          return withUnsafePointer(to: view) { baton_stream_unlock_opened(link, number, $0) }
        }
        if let code {
          note(code)
        } else {
          // The key was wiped before it was sent, which only a mistake in this file can cause.
          // The person is waiting, so the question is declined.
          note(baton_stream_unlock_declined(link, number, FaceDecline.failed.reason))
        }
      },
      declined: { number, reason in
        note(baton_stream_unlock_declined(link, number, reason.reason))
      })
  }

  /// What the Rust side said about an answer. One that came after the question was taken back is
  /// refused with BATON_E_UNKNOWN_LINK, which is as it should be and not worth a line in the log.
  private static func note(_ code: Int32) {
    if code == Int32(BATON_OK) || code == Int32(BATON_E_UNKNOWN_LINK) { return }
    NSLog("Baton: the answer to a question about a face was not taken (%d).", code)
  }
}

// MARK: - Names

/// The tag under which the pair of keys for one device is kept in the keychain: made of the id of
/// the device and of nothing else, so that a pair is found by the device it was made for.
enum FaceTag {
  static let prefix = "io.github.jayunewal.baton.unlock."

  /// nil if `id` is not exactly BATON_DEVICE_ID_BYTES long.
  static func make(for id: [UInt8]) -> String? {
    guard id.count == FaceDevice.idBytes else { return nil }
    let digits = Array("0123456789abcdef")
    var tag = prefix
    for byte in id {
      tag.append(digits[Int(byte >> 4)])
      tag.append(digits[Int(byte & 0x0f)])
    }
    return tag
  }
}

/// The reason that the prompt for the face gives, which names the device.
enum FacePrompt {
  /// What stands for a name that is empty, or has nothing in it that can be shown.
  static let unnamed = "your Mac"
  /// The most characters of a name that the prompt shows.
  static let mostCharacters = 48

  static func reason(for name: String) -> String {
    "Unlock \(shown(name))"
  }

  /// The name as the prompt shows it: the marks that change the way text runs or that break a line
  /// are taken out, so that a name cannot rearrange the prompt or pose as another sentence, runs of
  /// white space are one space, and a long name is cut.
  static func shown(_ name: String) -> String {
    var plain = String.UnicodeScalarView()
    for scalar in name.unicodeScalars {
      if isMarkOfLayout(scalar) {
        plain.append(" ")
      } else {
        plain.append(scalar)
      }
    }
    let words = String(plain).split(whereSeparator: { $0.isWhitespace })
    let joined = words.joined(separator: " ")
    if joined.isEmpty { return unnamed }
    if joined.count <= mostCharacters { return joined }
    return String(joined.prefix(mostCharacters)) + "\u{2026}"
  }

  /// Controls and line and paragraph separators, and the marks of the Unicode bidirectional
  /// algorithm: the Arabic letter mark, the left-to-right and right-to-left marks, the embeddings
  /// and overrides, and the isolates.
  private static func isMarkOfLayout(_ scalar: Unicode.Scalar) -> Bool {
    switch scalar.properties.generalCategory {
    case .control, .lineSeparator, .paragraphSeparator:
      return true
    default:
      break
    }
    switch scalar.value {
    case 0x061C, 0x200E, 0x200F, 0x202A...0x202E, 0x2066...0x2069:
      return true
    default:
      return false
    }
  }
}
