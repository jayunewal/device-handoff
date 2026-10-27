// SPDX-License-Identifier: Apache-2.0

// The Mac side of being unlocked from a phone or a tablet (docs/adr/0013-unlock-from-a-phone.md):
// what BatonUnlock.swift asks of the Mac, done with the calls of the system. The box that keeps
// the password is AES-256-GCM of CryptoKit, and the key of the box is sealed to the Secure Enclave
// key of the other device with the ECIES of Security.framework. The password is checked with Open
// Directory, the way the lock screen asks it. The password is typed as keys posted to the system
// (BatonControl.swift), after the displays are woken with the cover kept up.
//
// This file talks to the system, so it is built on a Mac and nowhere else, and what it does there
// is checked on one (the checks U1 to U12 of the record). The parts of it that the system does not
// decide are in BatonUnlock.swift, which any computer with a Swift compiler can test.
//
// The password, the key of the box and every key that is typed are secrets. Nothing here logs one,
// nor how many there are of them, only that a step was taken or not. A copy of a secret in a
// `Data`, an array or a `String` cannot be wiped; the copies here are made where the system takes
// nothing else, for as short a time as it can be, and what can be wiped is wiped.

#if os(macOS)

import CoreGraphics
import CryptoKit
import Foundation
import IOKit.pwr_mgt
import OpenDirectory
import Security
import os

/// The log of the system. The lines are public so that `log show` has them whole.
private let unlockLog = Logger(subsystem: "io.github.jayunewal.baton", category: "unlock")

/// Says in the log what came of an unlock: the name of the outcome and nothing else.
func noteUnlock(_ outcome: UnlockOutcome) {
  let name = String(describing: outcome)
  unlockLog.notice("Baton: an unlock came to \(name, privacy: .public).")
}

// MARK: - The box

/// The box that keeps the password, and the sealing of its key.
///
/// A box is the byte `form`, and then the 12-byte nonce, the encrypted password and the 16-byte tag
/// of AES-256-GCM. What is encrypted is 128 bytes: one that says how long the password is, the
/// password, and bytes that are random, so that the size of the box does not say how long the
/// password is. What is authenticated with it is `baton-unlock-box-v1`, a zero, and the ids of
/// the two devices (the one that is unlocked and the one that unlocks), so that a box that is
/// taken from one record opens nothing in another.
final class MacBoxing: UnlockBoxing {
  /// The form of a box, which is its first byte. A later version that changes what follows changes
  /// this, and an older one finds a box that it cannot open, and not a wrong one.
  static let form: UInt8 = 1

  private static let context = Array("baton-unlock-box-v1".utf8)
  private static let paddedBytes = 128
  private static let keyBytes = UnlockLimits.boxKeyBytes

  func open(_ boxed: [UInt8], key: Secret, target: [UInt8], viewer: [UInt8]) -> Secret? {
    guard boxed.count > 1, boxed.first == MacBoxing.form else { return nil }
    let opened = key.withBytes { keyBytes -> Data? in
      guard keyBytes.count == MacBoxing.keyBytes,
        let sealed = try? AES.GCM.SealedBox(combined: Data(boxed.dropFirst()))
      else { return nil }
      return try? AES.GCM.open(
        sealed, using: SymmetricKey(data: keyBytes),
        authenticating: MacBoxing.bound(to: target, viewer))
    }
    guard var padded = opened ?? nil else { return nil }
    defer { padded.resetBytes(in: padded.startIndex..<padded.endIndex) }
    guard padded.count == MacBoxing.paddedBytes else { return nil }
    let length = Int(padded[padded.startIndex])
    guard length >= 1, length < MacBoxing.paddedBytes else { return nil }
    return padded.withUnsafeBytes { all in
      Secret(copying: UnsafeRawBufferPointer(rebasing: all[1..<(1 + length)]))
    }
  }

  func make(password: Secret, target: [UInt8], viewer: [UInt8]) -> (boxed: [UInt8], key: Secret)? {
    let made = Secret(count: MacBoxing.keyBytes) { memory in
      guard let base = memory.baseAddress else { return false }
      return SecRandomCopyBytes(kSecRandomDefault, memory.count, base) == errSecSuccess
    }
    guard let key = made else { return nil }
    let boxed = key.withBytes {
      MacBoxing.encrypt(password, under: $0, target: target, viewer: viewer)
    }
    guard let boxed = boxed ?? nil else {
      key.wipe()
      return nil
    }
    return (boxed, key)
  }

  func seal(_ key: Secret, to publicKey: [UInt8], sealing: UnlockSealing) -> [UInt8]? {
    switch sealing {
    case .eciesP256:
      return sealWithEcies(key, to: publicKey)
    }
  }

  /// The password, padded, encrypted under the key. nil if anything fails.
  private static func encrypt(
    _ password: Secret, under key: UnsafeRawBufferPointer, target: [UInt8], viewer: [UInt8]
  ) -> [UInt8]? {
    var padded = [UInt8](repeating: 0, count: paddedBytes)
    defer {
      padded.withUnsafeMutableBytes {
        if let base = $0.baseAddress { wipeMemory(base, $0.count) }
      }
    }
    let used = password.withBytes { bytes -> Int? in
      guard bytes.count >= 1, bytes.count < paddedBytes else { return nil }
      padded[0] = UInt8(bytes.count)
      for (at, byte) in bytes.enumerated() {
        padded[1 + at] = byte
      }
      return 1 + bytes.count
    }
    guard let used = used ?? nil, key.count == keyBytes else { return nil }
    let filled = padded.withUnsafeMutableBytes { all -> Bool in
      guard let base = all.baseAddress else { return false }
      let rest = all.count - used
      return rest == 0 || SecRandomCopyBytes(kSecRandomDefault, rest, base + used) == errSecSuccess
    }
    guard filled,
      let sealed = try? AES.GCM.seal(
        padded, using: SymmetricKey(data: key), authenticating: bound(to: target, viewer)),
      let combined = sealed.combined
    else { return nil }
    return [form] + [UInt8](combined)
  }

  private static func bound(to target: [UInt8], _ viewer: [UInt8]) -> [UInt8] {
    context + [0] + target + viewer
  }

  /// Seals the key to a point of P-256, with ECIES of the standard form with a variable IV, the key
  /// derivation of X9.63 with SHA-256, and AES-GCM: the form that the key of the Secure Enclave
  /// opens (U5, U6). The key goes to the system in a `Data`, whose bytes are zeroed afterwards as far
  /// as the system left them where they were.
  private func sealWithEcies(_ key: Secret, to publicKey: [UInt8]) -> [UInt8]? {
    guard UnlockSealing.eciesP256.accepts(key: publicKey) else { return nil }
    let attributes: [String: Any] = [
      kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeyClass as String: kSecAttrKeyClassPublic,
      kSecAttrKeySizeInBits as String: 256,
    ]
    var failure: Unmanaged<CFError>?
    guard
      let target = SecKeyCreateWithData(
        Data(publicKey) as CFData, attributes as CFDictionary, &failure)
    else {
      failure?.release()
      return nil
    }
    let algorithm = SecKeyAlgorithm.eciesEncryptionStandardVariableIVX963SHA256AESGCM
    guard SecKeyIsAlgorithmSupported(target, .encrypt, algorithm) else { return nil }
    let sealed = key.withBytes { bytes -> [UInt8]? in
      var plain = Data(bytes)
      defer { plain.resetBytes(in: plain.startIndex..<plain.endIndex) }
      var failure: Unmanaged<CFError>?
      guard
        let encrypted = SecKeyCreateEncryptedData(
          target, algorithm, plain as CFData, &failure)
      else {
        failure?.release()
        return nil
      }
      return Array(encrypted as Data)
    }
    return sealed ?? nil
  }
}

// MARK: - The password

/// Checks a password against the account of the person who uses this Mac, with Open Directory, the
/// way the lock screen does. It changes nothing: the keychain and the sign-in are not touched, and
/// nothing is kept.
final class MacPasswordChecker: PasswordChecking {
  /// The error domain of Open Directory, and the code for a password that the account does not
  /// have (kODErrorCredentialsInvalid). The codes that are not this are not told apart here: the
  /// log has them, for the check of what a Mac says in each case (U7).
  private static let domain = "com.apple.OpenDirectory"
  private static let invalid = 5000

  func check(_ password: Secret) -> PasswordCheck {
    // The text is made as late as it can be, from UTF-8 that is well formed and from nothing else.
    let made: String? = password.withBytes { String(bytes: $0, encoding: .utf8) } ?? nil
    guard let text = made, !text.isEmpty else {
      return .failed("This password cannot be checked.")
    }
    do {
      let node = try ODNode(
        session: ODSession.default(), type: ODNodeType(kODNodeTypeAuthentication))
      let record = try node.record(
        withRecordType: kODRecordTypeUsers, name: NSUserName(), attributes: nil)
      try record.verifyPassword(text)
      return .accepted
    } catch {
      let failure = error as NSError
      if failure.domain == MacPasswordChecker.domain, failure.code == MacPasswordChecker.invalid {
        return .wrong
      }
      let domain = failure.domain
      let code = failure.code
      unlockLog.error(
        "Baton: the password could not be checked (\(domain, privacy: .public), \(code)).")
      return .failed("This Mac could not check the password.")
    }
  }
}

// MARK: - What an unlock asks of this Mac

/// Everything an unlock asks of this Mac, for one private screen. It is used by the one thread
/// that runs the unlock.
final class MacUnlockEnvironment: UnlockEnvironment {
  /// How long between the events of one key, in seconds: the program that has the keyboard takes
  /// each in turn.
  static let keyGap: TimeInterval = 0.012
  /// How long a display may take to wake, in seconds.
  static let wakeLimit: TimeInterval = 5
  /// How long the cover is kept up and looked at before the password is typed, after the displays
  /// are on and a key was sent to the lock screen, in seconds.
  static let settle: TimeInterval = 0.5
  /// How often the cover is put on while the displays wake, in seconds.
  static let coverBeat: TimeInterval = 0.02

  private let session: LibraryUnlockSession
  private let privacy: PrivateSession
  private let poster: EventPoster

  /// `poster` is the one that keeps its keys in the record of what the app has posted, so that the
  /// tripwire (BatonPrivacy.swift) stays quiet through the password.
  init(session: LibraryUnlockSession, privacy: PrivateSession, poster: EventPoster) {
    self.session = session
    self.privacy = privacy
    self.poster = poster
  }

  func live() -> Bool {
    session.live()
  }

  func keep(boxed: [UInt8], sealed: [UInt8]) -> Bool {
    session.keep(boxed: boxed, sealed: sealed)
  }

  /// Locked only if the session says so and is the one at the console: a session that was switched
  /// away is not the one that keys reach, and a session that cannot be read says nothing.
  func screenLock() -> ScreenLock {
    guard let dictionary = CGSessionCopyCurrentDictionary() as? [String: Any] else {
      return .unknown
    }
    if (dictionary[kCGSessionOnConsoleKey as String] as? NSNumber)?.boolValue == false {
      return .unknown
    }
    // The key for the lock screen is not documented, and has been there as long as the lock screen
    // has (BatonLock.swift).
    let locked = (dictionary["CGSSessionScreenIsLocked"] as? NSNumber)?.boolValue == true
    return locked ? .locked : .unlocked
  }

  func privacyInPlace() -> Bool {
    privacy.inPlace
  }

  func coverIsDark() -> Bool {
    privacy.darken()
  }

  func wake() -> Bool {
    if anyDisplayAsleep() {
      var assertion = IOPMAssertionID(0)
      let declared = IOPMAssertionDeclareUserActivity(
        "Baton unlocks this Mac for the person who uses it." as CFString, kIOPMUserActiveLocal,
        &assertion)
      guard declared == kIOReturnSuccess else {
        unlockLog.error("Baton: the displays could not be woken.")
        return false
      }
      defer { _ = IOPMAssertionRelease(assertion) }
      // The cover goes on as each display lights, and not only at the next look of the session.
      let limit = now() + MacUnlockEnvironment.wakeLimit
      while anyDisplayAsleep() {
        _ = privacy.darken()
        guard now() < limit else {
          unlockLog.error("Baton: the displays did not wake in time.")
          return false
        }
        pause(MacUnlockEnvironment.coverBeat)
      }
    }
    // A key that types nothing, for a lock screen that takes the first key to light up or to end
    // the screen saver and is not heard from again until the next one.
    guard tapShift() else { return false }
    let until = now() + MacUnlockEnvironment.settle
    while now() < until {
      _ = privacy.darken()
      pause(MacUnlockEnvironment.coverBeat)
    }
    unlockLog.notice("Baton: the displays are on and the cover is up.")
    return true
  }

  func type(_ key: TypedKey) -> Bool {
    var text: [UInt16]?
    if let scalar = key.scalar {
      guard let character = Unicode.Scalar(scalar) else { return false }
      text = Array(String(Character(character)).utf16)
    }
    var ok = true
    var shiftDown = false
    if key.shift {
      shiftDown = poster.post(.key(shiftKey(down: true)))
      ok = shiftDown
      pause(MacUnlockEnvironment.keyGap)
    }
    var keyDown = false
    if ok {
      keyDown = poster.post(.key(characterKey(key, down: true, text: text)))
      ok = keyDown
      pause(MacUnlockEnvironment.keyGap)
    }
    // A key that went down comes up, and so does Shift, whatever else failed.
    if keyDown {
      ok = poster.post(.key(characterKey(key, down: false, text: text))) && ok
      pause(MacUnlockEnvironment.keyGap)
    }
    if shiftDown {
      ok = poster.post(.key(shiftKey(down: false))) && ok
      pause(MacUnlockEnvironment.keyGap)
    }
    return ok
  }

  func pause(_ seconds: TimeInterval) {
    Thread.sleep(forTimeInterval: seconds)
  }

  func now() -> TimeInterval {
    ProcessInfo.processInfo.systemUptime
  }

  // MARK: Helpers

  private func anyDisplayAsleep() -> Bool {
    // A list that cannot be read is no proof that the displays are awake.
    guard let displays = ScreenCover.online() else { return true }
    return displays.contains { CGDisplayIsAsleep($0) != 0 }
  }

  /// Shift down and up, which types no character.
  private func tapShift() -> Bool {
    let down = poster.post(.key(shiftKey(down: true)))
    pause(MacUnlockEnvironment.keyGap)
    let up = poster.post(.key(shiftKey(down: false)))
    pause(MacUnlockEnvironment.keyGap)
    return down && up
  }

  private func shiftKey(down: Bool) -> SyntheticKey {
    SyntheticKey(
      virtualKey: UnlockKeys.shiftKey, down: down, autorepeat: false,
      modifiers: down ? [.shift] : [], text: nil)
  }

  private func characterKey(_ key: TypedKey, down: Bool, text: [UInt16]?) -> SyntheticKey {
    SyntheticKey(
      virtualKey: key.virtualKey, down: down, autorepeat: false,
      modifiers: key.shift ? [.shift] : [], text: text)
  }
}

// MARK: - The sharer

@available(macOS 14.0, *)
extension MacSharer {
  /// This Mac can check a password against the account of its person, put it in a box for another
  /// device, and type it at the lock screen of a private screen.
  var canUnlock: Bool { true }

  func unlockCheck(_ password: Secret) -> PasswordCheck {
    MacPasswordChecker().check(password)
  }

  func unlockSetup(_ setup: UnlockSetup) -> UnlockMade {
    makeUnlock(setup, checker: MacPasswordChecker(), boxing: MacBoxing())
  }
}

#endif
