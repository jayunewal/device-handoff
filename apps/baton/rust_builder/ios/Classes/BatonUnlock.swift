// SPDX-License-Identifier: Apache-2.0

// The Mac's side of being unlocked from a phone or a tablet (docs/adr/0013-unlock-from-a-phone.md).
// The person sets it up once, at this Mac, with the password of their account. The Mac puts the
// password in a box that only the other device opens, with the face of its person, and keeps the
// box. When the person taps Unlock at the other device, that device sends the key of the box, and
// this Mac types the password into its lock screen, once, and looks at what came of it.
//
// This file holds what decides, and none of it needs a framework of a Mac: the reading of what the
// Rust side passes, the secrets and when they are wiped, the six steps of an unlock and the order
// they come in, which keys are typed for a character, and the waiting. What needs the system (the
// box and the sealing of its key, the check of the password, the keys that reach the lock screen,
// the cover of the screens) is given to it as protocols, and BatonUnlockMac.swift has the real
// ones. So that this file can be built and tested on any computer that has a Swift compiler
// (apps/baton/rust_builder/swift_tests), nothing here may import more than Foundation.
//
// A call from Rust must never end in a trap, because a trap ends the whole app: nothing here
// force-unwraps, indexes without looking, or asserts on what Rust passed. Nothing here writes a
// password, a key, a box or a typed key to a log.
//
// A Mac that stays unlocked after the session, or that is unlocked for somebody else, is a
// security failure, so the rules are strict: the password is typed at most once, one key at a time,
// and never unless the attempt is live, the screen is locked and the privacy is in place.

import Foundation

// MARK: - Limits

/// The sizes that the interface allows, as numbers of bytes.
enum UnlockLimits {
  /// The key of a box.
  static let boxKeyBytes = Int(BATON_BOX_KEY_BYTES)
  /// The id of a device.
  static let deviceIdBytes = Int(BATON_DEVICE_ID_BYTES)
  /// The most bytes of a password.
  static let passwordMax = Int(BATON_UNLOCK_PASSWORD_MAX)
  /// The most bytes of a box, and of a sealed key.
  static let boxMax = Int(BATON_UNLOCK_BOX_MAX)
  static let sealedMax = Int(BATON_UNLOCK_SEALED_MAX)
  /// The most bytes of a public key.
  static let keyMax = Int(BATON_UNLOCK_KEY_MAX)
}

// MARK: - Secrets

/// Overwrites `count` bytes with zeros, with a call that the compiler may not leave out: a plain
/// loop of stores may be left out when the memory is freed next. The sizes of `memset_s` are of the
/// type `rsize_t`, which Swift may not take for an `Int`, so they are converted to whatever type it
/// names.
func wipeMemory(_ base: UnsafeMutableRawPointer, _ count: Int) {
  guard count > 0 else { return }
  #if canImport(Darwin)
    _ = memset_s(base, numericCast(count), 0, numericCast(count))
  #else
    explicit_bzero(base, count)
  #endif
}

/// Bytes that are a secret, in memory that is wiped when they are let go of, and before that on
/// `wipe()`. A password, the key of a box and the characters of a password are kept so, and never
/// in a `String`, in a log line or in anything that prints itself. Nothing about the bytes is
/// printed.
///
/// A copy of the bytes in an array or in `Data` cannot be wiped by this class, so a secret is
/// copied out only where the system needs it (BatonUnlockMac.swift), for as short a time as it
/// can be, and that copy is wiped there.
final class Secret: CustomStringConvertible {
  private let memory: UnsafeMutableRawBufferPointer
  private let release: (UnsafeMutableRawBufferPointer) -> Void
  private let lock = NSLock()
  private var wiped = false

  /// How many bytes it has. It stays what it was after a wipe.
  let count: Int

  /// The most bytes a secret may have: a password of 127 bytes, as 127 characters of 4 bytes.
  static let mostBytes = 1024

  /// Takes over `memory`, which holds the secret. It is wiped, and then handed to `release`, which
  /// frees it, when the secret is let go of. The tests give a `release` that looks at it first.
  init(
    adopting memory: UnsafeMutableRawBufferPointer,
    release: @escaping (UnsafeMutableRawBufferPointer) -> Void
  ) {
    self.memory = memory
    self.release = release
    self.count = memory.count
  }

  /// A secret of `count` bytes in memory of its own, which `fill` writes to (it starts as zeros).
  /// nil if `count` is not from 1 to `mostBytes`, or if `fill` says that it could not: what it
  /// wrote is wiped then.
  convenience init?(count: Int, fill: (UnsafeMutableRawBufferPointer) -> Bool) {
    guard count > 0, count <= Secret.mostBytes else { return nil }
    let memory = UnsafeMutableRawBufferPointer.allocate(byteCount: count, alignment: 8)
    _ = memory.initializeMemory(as: UInt8.self, repeating: 0)
    self.init(adopting: memory, release: { $0.deallocate() })
    if !fill(memory) {
      wipe()
      return nil
    }
  }

  /// A copy of `bytes` in memory of its own. nil if there are none, or more than `mostBytes`.
  convenience init?(copying bytes: UnsafeRawBufferPointer) {
    guard bytes.count > 0, bytes.count <= Secret.mostBytes else { return nil }
    let memory = UnsafeMutableRawBufferPointer.allocate(byteCount: bytes.count, alignment: 8)
    memory.copyMemory(from: bytes)
    self.init(adopting: memory, release: { $0.deallocate() })
  }

  /// A copy of the bytes the Rust side passed. nil if there is a length and no pointer, or the
  /// length is not within `range`.
  convenience init?(reading bytes: BatonBytes, within range: ClosedRange<Int>) {
    guard let view = UnsafeRawBufferPointer(reading: bytes), range.contains(view.count) else {
      return nil
    }
    self.init(copying: view)
  }

  /// Gives the bytes to `body` for as long as it runs, or nil, without calling it, if the secret
  /// was wiped. The bytes are not to be kept.
  func withBytes<Output>(_ body: (UnsafeRawBufferPointer) -> Output) -> Output? {
    lock.lock()
    defer { lock.unlock() }
    if wiped { return nil }
    return body(UnsafeRawBufferPointer(memory))
  }

  /// The 32-bit number at `index`, for a secret that is a list of them. nil if there is no such
  /// number, or the secret was wiped.
  func word(at index: Int) -> UInt32? {
    lock.lock()
    defer { lock.unlock() }
    guard !wiped, index >= 0, index < count / 4 else { return nil }
    return memory.load(fromByteOffset: index * 4, as: UInt32.self)
  }

  /// A copy of the bytes as `Data`, for a call of the system that wants one. nil if the secret was
  /// wiped. The copy is the caller's to wipe.
  func data() -> Data? {
    withBytes { Data($0) }
  }

  var isWiped: Bool {
    lock.lock()
    defer { lock.unlock() }
    return wiped
  }

  /// Overwrites the secret with zeros. More than once is fine.
  func wipe() {
    lock.lock()
    defer { lock.unlock() }
    if wiped { return }
    wiped = true
    guard let base = memory.baseAddress else { return }
    wipeMemory(base, memory.count)
  }

  deinit {
    wipe()
    release(memory)
  }

  var description: String { "a secret of \(count) bytes" }
}

// MARK: - What the Rust side passes

/// A copy of the bytes the Rust side passed, when their number is within `range`. nil if there is
/// a length and no pointer, or the number is not within `range`. For what is no secret.
func copyBytes(_ bytes: BatonBytes, within range: ClosedRange<Int>) -> [UInt8]? {
  guard let view = UnsafeRawBufferPointer(reading: bytes), range.contains(view.count) else {
    return nil
  }
  return Array(view)
}

/// How a key is sealed to a public key, as the interface names it (`BATON_SEALING_`).
enum UnlockSealing: Equatable {
  /// ECIES on P-256 with the X9.63 key derivation and SHA-256, and AES-GCM.
  case eciesP256

  init?(number: UInt32) {
    switch number {
    case BATON_SEALING_ECIES_P256: self = .eciesP256
    default: return nil
    }
  }

  /// The bytes of a point of P-256 in the uncompressed form of ANSI X9.63: 04, X and Y.
  static let pointBytes = 65

  /// Whether `key` can be a public key for this way of sealing. A point is not checked to be on
  /// the curve: the system does that when it seals to it.
  func accepts(key: [UInt8]) -> Bool {
    switch self {
    case .eciesP256: return key.count == UnlockSealing.pointBytes && key.first == 0x04
    }
  }
}

/// What `unlock_setup` is given: the password, the two devices, and the key of the other device
/// that the key of the box is sealed to (`BatonUnlockSetup`). The password is a secret, and it is
/// wiped by whoever made this, after the call.
struct UnlockSetup {
  /// A `BATON_SEALING_` number. A number that is not known is a way of sealing that this version
  /// does not have, which is said to the Rust side, and so it is kept as it came.
  let sealing: UInt32
  let password: Secret
  let target: [UInt8]
  let viewer: [UInt8]
  let viewerKey: [UInt8]

  /// Reads a setup of the interface. nil if the struct is shorter than this version of the
  /// interface knows it, or a length is out of range, or a pointer is missing: the password is
  /// from 1 to `passwordMax` bytes, the ids are `deviceIdBytes` bytes, and the key is from 1 to
  /// `keyMax` bytes.
  init?(reading setup: UnsafePointer<BatonUnlockSetup>) {
    // The size is read alone first: a struct that is shorter than this version of the interface
    // knows it is not copied, since the copy would read past its end.
    guard setup.pointee.size >= UInt32(MemoryLayout<BatonUnlockSetup>.size) else { return nil }
    let c = setup.pointee
    let ids = UnlockLimits.deviceIdBytes...UnlockLimits.deviceIdBytes
    guard let password = Secret(reading: c.password, within: 1...UnlockLimits.passwordMax),
      let target = copyBytes(c.target, within: ids), let viewer = copyBytes(c.viewer, within: ids),
      let viewerKey = copyBytes(c.viewer_key, within: 1...UnlockLimits.keyMax)
    else { return nil }
    self.sealing = c.sealing
    self.password = password
    self.target = target
    self.viewer = viewer
    self.viewerKey = viewerKey
  }
}

/// The attempt to unlock that `unlock` is given (`BatonUnlock`): the key of the box that the other
/// device opened with the face of its person, the box, the key that the new key is sealed to, and
/// the ids that the box is bound to. The key of the box is a secret.
struct UnlockAttempt {
  let sealing: UnlockSealing
  let boxKey: Secret
  let boxed: [UInt8]
  let viewerKey: [UInt8]
  let target: [UInt8]
  let viewer: [UInt8]

  /// Reads an attempt of the interface. nil if the struct is shorter than this version of the
  /// interface knows it, a pointer is missing, the key of the box is not `boxKeyBytes` bytes, an id
  /// is not `deviceIdBytes` bytes, the box is empty or larger than `boxMax`, the way of sealing is
  /// not known, or the key to seal to is not one that the way of sealing takes. Nothing is opened
  /// for such an attempt.
  init?(reading unlock: UnsafePointer<BatonUnlock>) {
    guard unlock.pointee.size >= UInt32(MemoryLayout<BatonUnlock>.size) else { return nil }
    let c = unlock.pointee
    let ids = UnlockLimits.deviceIdBytes...UnlockLimits.deviceIdBytes
    guard let sealing = UnlockSealing(number: c.sealing),
      let boxKey = Secret(
        reading: c.box_key, within: UnlockLimits.boxKeyBytes...UnlockLimits.boxKeyBytes),
      let boxed = copyBytes(c.boxed, within: 1...UnlockLimits.boxMax),
      let viewerKey = copyBytes(c.viewer_key, within: 1...UnlockLimits.keyMax),
      sealing.accepts(key: viewerKey),
      let target = copyBytes(c.target, within: ids), let viewer = copyBytes(c.viewer, within: ids)
    else { return nil }
    self.sealing = sealing
    self.boxKey = boxKey
    self.boxed = boxed
    self.viewerKey = viewerKey
    self.target = target
    self.viewer = viewer
  }
}

/// Writes the box and the sealed key that `unlock_setup` made into the room the Rust side gave, and
/// says whether it did. Nothing is written if the struct is shorter than this version of the
/// interface knows it, there is no room, or either is empty or larger than the interface allows:
/// the Rust side reads what an out argument holds only when the call succeeds.
func writeMade(boxed: [UInt8], sealed: [UInt8], to made: UnsafeMutablePointer<BatonUnlockMade>)
  -> Bool
{
  guard made.pointee.size >= UInt32(MemoryLayout<BatonUnlockMade>.size),
    let boxRoom = made.pointee.boxed, let sealedRoom = made.pointee.sealed,
    !boxed.isEmpty, boxed.count <= UnlockLimits.boxMax,
    !sealed.isEmpty, sealed.count <= UnlockLimits.sealedMax
  else { return false }
  for (at, byte) in boxed.enumerated() {
    boxRoom[at] = byte
  }
  for (at, byte) in sealed.enumerated() {
    sealedRoom[at] = byte
  }
  made.pointee.box_len = UInt32(boxed.count)
  made.pointee.sealed_len = UInt32(sealed.count)
  return true
}

// MARK: - What comes of it

/// What the system says of a password that the person typed to set an unlock up.
enum PasswordCheck: Equatable {
  /// It is the password of the account of the person who uses this Mac.
  case accepted
  /// The system does not accept it (`BATON_PASSWORD_WRONG`).
  case wrong
  /// This Mac cannot check a password, or its lock screen takes none
  /// (`BATON_PASSWORD_UNSUPPORTED`).
  case unsupported
  /// Anything else that went wrong, in a sentence for the person.
  case failed(String)
}

/// What came of putting a password in a box for the other device.
enum UnlockMade: Equatable {
  /// The password is in `boxed`, and the key of that box is sealed in `sealed`.
  case made(boxed: [UInt8], sealed: [UInt8])
  case wrong
  case unsupported
  case failed(String)
}

/// What `unlock` returns: what came of the attempt (`BATON_UNLOCK_`).
enum UnlockOutcome: Equatable {
  /// The password was typed, and the system says that the screen is unlocked.
  case unlocked
  /// The password was typed, and the screen stayed locked.
  case rejected
  /// The key did not open the box. Nothing was kept and nothing was typed.
  case stale
  /// Anything else that is this device's own. Nothing more is typed.
  case failed
  /// The screen is not locked. Nothing was typed.
  case notLocked
  /// The cover, the hold of the keys and the pointer, or the guard is not in place, or the person
  /// at the other device lifted something. Nothing was typed.
  case notPrivate

  var number: UInt32 {
    switch self {
    case .unlocked: return UInt32(BATON_UNLOCK_UNLOCKED)
    case .rejected: return UInt32(BATON_UNLOCK_REJECTED)
    case .stale: return UInt32(BATON_UNLOCK_STALE)
    case .failed: return UInt32(BATON_UNLOCK_FAILED)
    case .notLocked: return UInt32(BATON_UNLOCK_NOT_LOCKED)
    case .notPrivate: return UInt32(BATON_UNLOCK_NOT_PRIVATE)
    }
  }
}

// MARK: - The keys that are typed

/// A key to press and release at the lock screen, for one character.
struct TypedKey: Equatable {
  /// The virtual key code of the Mac (kVK_ in HIToolbox/Events.h).
  var virtualKey: UInt16
  /// Whether Shift is held while the key is pressed.
  var shift: Bool
  /// The character the key types, as a Unicode scalar value, or nil for a key that is no
  /// character (Return). It is also set on the events of the key, so that a field that reads the
  /// characters of an event and one that reads the key code both get the character.
  var scalar: UInt32?
}

/// Which key types which character, for a US keyboard, and what is done for any other character.
/// Kept in one table of its own because the first try on a real Mac may show that it has to change
/// (docs/adr/0013-unlock-from-a-phone.md, U1).
enum UnlockKeys {
  /// The virtual key code of Shift.
  static let shiftKey: UInt16 = 56

  /// Return, which ends the password.
  static let returnKey = TypedKey(virtualKey: 36, shift: false, scalar: nil)

  /// The key code of a character that has no key on the US layout. The character itself is set on
  /// the event.
  static let anyKey: UInt16 = 0

  /// The keys of the US layout (kVK_ANSI_), each with the character it types and the one it types
  /// with Shift.
  private static let rows: [(code: UInt16, plain: Unicode.Scalar, shifted: Unicode.Scalar)] = [
    (0x00, "a", "A"), (0x01, "s", "S"), (0x02, "d", "D"), (0x03, "f", "F"), (0x04, "h", "H"),
    (0x05, "g", "G"), (0x06, "z", "Z"), (0x07, "x", "X"), (0x08, "c", "C"), (0x09, "v", "V"),
    (0x0B, "b", "B"), (0x0C, "q", "Q"), (0x0D, "w", "W"), (0x0E, "e", "E"), (0x0F, "r", "R"),
    (0x10, "y", "Y"), (0x11, "t", "T"), (0x12, "1", "!"), (0x13, "2", "@"), (0x14, "3", "#"),
    (0x15, "4", "$"), (0x16, "6", "^"), (0x17, "5", "%"), (0x18, "=", "+"), (0x19, "9", "("),
    (0x1A, "7", "&"), (0x1B, "-", "_"), (0x1C, "8", "*"), (0x1D, "0", ")"), (0x1E, "]", "}"),
    (0x1F, "o", "O"), (0x20, "u", "U"), (0x21, "[", "{"), (0x22, "i", "I"), (0x23, "p", "P"),
    (0x25, "l", "L"), (0x26, "j", "J"), (0x27, "'", "\""), (0x28, "k", "K"), (0x29, ";", ":"),
    (0x2A, "\\", "|"), (0x2B, ",", "<"), (0x2C, "/", "?"), (0x2D, "n", "N"), (0x2E, "m", "M"),
    (0x2F, ".", ">"), (0x31, " ", " "), (0x32, "`", "~"),
  ]

  private static let table: [UInt32: TypedKey] = {
    var keys: [UInt32: TypedKey] = [:]
    for row in rows {
      keys[row.plain.value] = TypedKey(virtualKey: row.code, shift: false, scalar: row.plain.value)
      if row.shifted != row.plain {
        keys[row.shifted.value] = TypedKey(
          virtualKey: row.code, shift: true, scalar: row.shifted.value)
      }
    }
    return keys
  }()

  /// The key to press for a character, or nil for one that is not typed: the controls (Tab, a new
  /// line, Escape and the like are keys of their own, and are no part of a password), and what is
  /// no Unicode scalar. A printable character of ASCII has its key of the US layout, with Shift for
  /// the capitals and the symbols that need it. Any other character has no key of the US layout
  /// and is typed as key 0 with the character set on the event.
  static func key(for scalar: UInt32) -> TypedKey? {
    if let found = table[scalar] { return found }
    guard scalar > 0x9F, scalar <= 0x10FFFF, !(0xD800...0xDFFF).contains(scalar) else {
      return nil
    }
    return TypedKey(virtualKey: anyKey, shift: false, scalar: scalar)
  }
}

// MARK: - The text of a password

/// The characters of a password, from the UTF-8 that the Rust side passes.
enum PasswordText {
  /// The next scalar of well formed UTF-8, with `index` moved past it. nil, and `index` as it
  /// was, for anything that is not: a byte that cannot start a character, a character that is cut
  /// off, a byte inside it that is not a continuation, a form that is longer than it needs to be,
  /// a surrogate, and a value beyond U+10FFFF.
  static func nextScalar(in bytes: UnsafeRawBufferPointer, at index: inout Int) -> UInt32? {
    guard index >= 0, index < bytes.count else { return nil }
    let first = UInt32(bytes[index])
    if first < 0x80 {
      index += 1
      return first
    }
    let length: Int
    let least: UInt32
    var value: UInt32
    switch first {
    case 0xC2...0xDF:
      length = 2
      least = 0x80
      value = first & 0x1F
    case 0xE0...0xEF:
      length = 3
      least = 0x800
      value = first & 0x0F
    case 0xF0...0xF4:
      length = 4
      least = 0x10000
      value = first & 0x07
    default:
      return nil
    }
    guard index + length <= bytes.count else { return nil }
    for offset in 1..<length {
      let next = UInt32(bytes[index + offset])
      guard next & 0xC0 == 0x80 else { return nil }
      value = value << 6 | (next & 0x3F)
    }
    guard value >= least, value <= 0x10FFFF, !(0xD800...0xDFFF).contains(value) else { return nil }
    index += length
    return value
  }

  /// The characters of `password` as a secret of 32-bit numbers, one for each. nil if the bytes
  /// are not well formed UTF-8, or if a character is one that has no key (`UnlockKeys.key(for:)`),
  /// so that a password is typed whole or not at all, and never up to a character that cannot be
  /// typed. The password itself is not touched.
  static func scalars(of password: Secret) -> Secret? {
    guard let counted = password.withBytes(countScalars), let total = counted, total > 0 else {
      return nil
    }
    return Secret(count: total * 4) { memory in
      password.withBytes { bytes -> Bool in
        var index = 0
        var at = 0
        while index < bytes.count, at < total {
          guard let scalar = nextScalar(in: bytes, at: &index) else { return false }
          memory.storeBytes(of: scalar, toByteOffset: at * 4, as: UInt32.self)
          at += 1
        }
        return at == total && index == bytes.count
      } ?? false
    }
  }

  /// How many characters `bytes` has, or nil if they are not well formed UTF-8 or one of them has
  /// no key.
  private static func countScalars(_ bytes: UnsafeRawBufferPointer) -> Int? {
    var index = 0
    var total = 0
    while index < bytes.count {
      guard let scalar = nextScalar(in: bytes, at: &index), UnlockKeys.key(for: scalar) != nil
      else { return nil }
      total += 1
    }
    return total
  }
}

// MARK: - What the system gives

/// Whether the screen is locked, as far as the system says.
enum ScreenLock: Equatable {
  case locked
  case unlocked
  /// The session cannot be read, or is not the one at the console: neither a yes nor a no.
  case unknown
}

/// What opens and makes the box that keeps the password, and seals its key to the other device.
/// BatonUnlockMac.swift has the real one.
protocol UnlockBoxing: AnyObject {
  /// The password in `boxed`, opened with `key`: the box is bound to `target` and `viewer`, so that
  /// one taken from another record opens nothing. nil if it does not open, for whatever reason: a
  /// wrong key, a box that was changed, other ids, or a form that this version does not know.
  func open(_ boxed: [UInt8], key: Secret, target: [UInt8], viewer: [UInt8]) -> Secret?

  /// Puts `password` in a box under a new random key, bound to `target` and `viewer`. nil if it
  /// cannot. The key is the caller's to wipe.
  func make(password: Secret, target: [UInt8], viewer: [UInt8]) -> (boxed: [UInt8], key: Secret)?

  /// Seals `key` to `publicKey` the way `sealing` says. nil if it cannot.
  func seal(_ key: Secret, to publicKey: [UInt8], sealing: UnlockSealing) -> [UInt8]?
}

/// What checks a password against the account of the person who uses this Mac, the way the lock
/// screen does, and changes nothing.
protocol PasswordChecking: AnyObject {
  func check(_ password: Secret) -> PasswordCheck
}

/// Everything an unlock asks of the Mac and of the library, one call for each question.
protocol UnlockEnvironment: AnyObject {
  /// Whether the attempt goes on: it is not over, and the library says so (a call that fails says
  /// no).
  func live() -> Bool

  /// Keeps the password in a new box, sealed to the other device, in the place of the old one.
  /// true only if the library says it is kept.
  func keep(boxed: [UInt8], sealed: [UInt8]) -> Bool

  /// Whether the system says that the screen is locked.
  func screenLock() -> ScreenLock

  /// Whether the cover of the screens, the hold of the keys and the pointer and the guard are all
  /// in place, and the person at the other device lifted nothing.
  func privacyInPlace() -> Bool

  /// Puts the cover on every screen once more and says whether every screen that shows anything
  /// is dark.
  func coverIsDark() -> Bool

  /// Wakes the displays if they are off, with the cover kept up through the wake, and returns
  /// when they are on and can take keys. false if they could not be woken.
  func wake() -> Bool

  /// Presses and releases one key at the lock screen, with Shift if it needs it, and notes each
  /// key that goes down in the record of what the app has posted before it goes to the system.
  /// false if a key could not be posted.
  func type(_ key: TypedKey) -> Bool

  /// Waits for `seconds`, and the time in seconds, on a clock that only moves forward.
  func pause(_ seconds: TimeInterval)
  func now() -> TimeInterval
}

// MARK: - Setting an unlock up

/// The password is accepted, and put in a box that only the other device opens. In this order:
/// the way of sealing is one that this version has; the password is checked again, since the other
/// device took its time; the box is made under a new random key; the key is sealed to the key of
/// the other device. The password and the key are wiped before this returns, whatever it returns.
func makeUnlock(_ setup: UnlockSetup, checker: PasswordChecking, boxing: UnlockBoxing) -> UnlockMade
{
  defer { setup.password.wipe() }
  guard let sealing = UnlockSealing(number: setup.sealing) else { return .unsupported }
  guard sealing.accepts(key: setup.viewerKey) else {
    return .failed("The key of the other device is not one that this Mac can seal to.")
  }
  switch checker.check(setup.password) {
  case .accepted: break
  case .wrong: return .wrong
  case .unsupported: return .unsupported
  case .failed(let sentence): return .failed(sentence)
  }
  // A password with a character that has no key is refused now, and not at the first unlock.
  guard let scalars = PasswordText.scalars(of: setup.password) else {
    return .failed(
      "This password has a character that Baton cannot type at the lock screen. "
        + "Letters, digits, symbols and spaces can be.")
  }
  scalars.wipe()
  guard
    let made = boxing.make(password: setup.password, target: setup.target, viewer: setup.viewer)
  else { return .failed("This Mac could not put the password in a box.") }
  defer { made.key.wipe() }
  guard let sealed = boxing.seal(made.key, to: setup.viewerKey, sealing: sealing),
    !sealed.isEmpty, sealed.count <= UnlockLimits.sealedMax,
    !made.boxed.isEmpty, made.boxed.count <= UnlockLimits.boxMax
  else { return .failed("This Mac could not seal the key of the box to the other device.") }
  return .made(boxed: made.boxed, sealed: sealed)
}

// MARK: - The unlock

/// How long the Mac is waited for, in seconds.
struct UnlockTiming {
  /// How long the system has to say that the screen is unlocked after Return.
  var patience: TimeInterval = 10
  /// How often it is asked while it is waited for.
  var poll: TimeInterval = 0.1
}

/// Unlocks the screen with the password in the box, once. `attempt.boxKey` is wiped by the end.
///
/// In this order, as the header of the interface says (`unlock` of `BatonSharerVTable`), and
/// nothing is typed until the fifth step:
///
/// 1. The attempt must be live.
/// 2. The box is opened with the key. A key that does not open it is `stale`.
/// 3. The password goes into a new box under a new key, that key is sealed to the other device,
///    and both are kept. The key that was just used opens nothing from then on. If any of it fails
///    nothing is typed. The keys are wiped as soon as both are kept.
/// 4. The screen must be locked, and the privacy in place (nothing else may type at it, and
///    nobody at the Mac may see it): else `notLocked` or `notPrivate`.
/// 5. The displays are woken, and the cover is verified again, then the password is typed one key
///    at a time and Return. Before each key, Return included, the attempt must be live and the
///    screen locked, and the first key that fails either ends the attempt.
/// 6. The screen is waited for to be unlocked, ten seconds at most.
func runUnlock(
  _ attempt: UnlockAttempt, boxing: UnlockBoxing, environment env: UnlockEnvironment,
  timing: UnlockTiming = UnlockTiming()
) -> UnlockOutcome {
  defer { attempt.boxKey.wipe() }

  // 1.
  guard env.live() else { return .failed }

  // 2.
  guard
    let password = boxing.open(
      attempt.boxed, key: attempt.boxKey, target: attempt.target, viewer: attempt.viewer)
  else { return .stale }
  defer { password.wipe() }

  // 3.
  guard
    let next = boxing.make(password: password, target: attempt.target, viewer: attempt.viewer)
  else { return .failed }
  defer { next.key.wipe() }
  guard let sealed = boxing.seal(next.key, to: attempt.viewerKey, sealing: attempt.sealing),
    env.keep(boxed: next.boxed, sealed: sealed)
  else { return .failed }
  attempt.boxKey.wipe()
  next.key.wipe()

  // The characters to type are read before anything is typed, so that a password that cannot be
  // typed whole is not typed in part. The password itself is not needed any more.
  guard let scalars = PasswordText.scalars(of: password) else { return .failed }
  defer { scalars.wipe() }
  password.wipe()

  // 4.
  switch env.screenLock() {
  case .locked: break
  case .unlocked: return .notLocked
  case .unknown: return .failed
  }
  guard env.privacyInPlace(), env.coverIsDark() else { return .notPrivate }

  // 5.
  guard env.wake() else { return .failed }
  guard env.privacyInPlace(), env.coverIsDark() else { return .notPrivate }
  let total = scalars.count / 4
  for index in 0..<total {
    guard env.live(), env.screenLock() == .locked else { return .failed }
    guard let scalar = scalars.word(at: index), let key = UnlockKeys.key(for: scalar),
      env.type(key)
    else { return .failed }
  }
  // The last key of the password is posted: nothing of it is held any more.
  scalars.wipe()
  guard env.live(), env.screenLock() == .locked else { return .failed }
  guard env.type(UnlockKeys.returnKey) else { return .failed }

  // 6.
  let deadline = env.now() + timing.patience
  while true {
    if env.screenLock() == .unlocked { return .unlocked }
    if env.now() >= deadline { return .rejected }
    // An attempt that is over is not waited for: whoever asked has no use for the answer, and the
    // session goes on by itself when the system says the screen is unlocked.
    guard env.live() else { return .failed }
    env.pause(timing.poll)
  }
}

// MARK: - The library

/// The calls an unlock makes to the Rust library: whether the attempt goes on, and keeping the new
/// box. And what the platform tells it as it goes: whether the privacy of the screen is in place.
final class LibraryUnlockSession {
  private let link: UInt64

  init(link: UInt64) {
    self.link = link
  }

  /// Whether the attempt goes on (`baton_stream_unlock_live`). Anything but a clear yes is a no: a
  /// call that fails says that the attempt is over, as the interface says, and so does a session
  /// that ended and closed its link.
  func live() -> Bool {
    var live: UInt32 = 0
    return baton_stream_unlock_live(link, &live) == Int32(BATON_OK) && live == 1
  }

  /// Keeps the new box and the sealed key (`baton_stream_unlock_keep`). true only if the call
  /// worked and the library says that they are kept: every other answer, a call that fails
  /// included, means that nothing is kept.
  func keep(boxed: [UInt8], sealed: [UInt8]) -> Bool {
    var result = UInt32(BATON_KEEP_OTHER)
    let code = boxed.withUnsafeBufferPointer { boxedBytes in
      sealed.withUnsafeBufferPointer { sealedBytes -> Int32 in
        var box = BatonUnlockBox(
          size: UInt32(MemoryLayout<BatonUnlockBox>.size), reserved: 0,
          boxed: BatonBytes(ptr: boxedBytes.baseAddress, len: boxedBytes.count),
          sealed: BatonBytes(ptr: sealedBytes.baseAddress, len: sealedBytes.count))
        return baton_stream_unlock_keep(link, &box, &result)
      }
    }
    return code == Int32(BATON_OK) && result == UInt32(BATON_KEEP_KEPT)
  }

  /// Tells the library whether the privacy of the screen is in place
  /// (`baton_stream_source_private`). The answer is of no use: a session that is over, and a link
  /// that is closed, take no word.
  func privacy(inPlace: Bool) {
    _ = baton_stream_source_private(link, inPlace ? 1 : 0)
  }
}
