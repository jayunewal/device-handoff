// SPDX-License-Identifier: Apache-2.0

// The part of unlocking a locked Mac with a face that needs the system of an iPhone or an iPad
// (docs/adr/0013-unlock-from-a-phone.md): the pairs of keys in the Secure Enclave, and the prompt
// for the face that opens a sealed key with one of them. BatonFace.swift decides everything else
// and names what it needs from this file as two small protocols (`FaceKeys` and `FaceAttempt`), so
// that nothing here decides anything that its tests could not reach.
//
// The Secure Enclave is hardware that makes a key, keeps it, and does the work with it without
// ever giving it out. A key that is made with an access rule is used only when the rule is met,
// and the rule here is a face that the system recognises, with no passcode as a way round. Only
// the Security and LocalAuthentication frameworks of an iPhone or an iPad are used, so the file is
// empty on any other system and cannot be built or run without the SDK of one. The Simulator has
// no Secure Enclave, and there the file only says so.
//
// Nothing here may trap, and nothing here writes the key of a box, a sealed key, the name of a
// device or its id to a log. A log line has the step that failed and the domains and codes of the
// errors of the system, which is what whoever tries the device needs to sort them.

#if os(iOS)

import Foundation
import LocalAuthentication
import Security

// MARK: - The pairs of keys

/// The pairs of keys that this device keeps in the Secure Enclave, one for each device that it may
/// unlock.
///
/// The private half of a pair is made by the Secure Enclave, stays in it and is used in it. Its
/// access rule is `biometryCurrentSet` and `privateKeyUsage`, and nothing else:
///
/// - `biometryCurrentSet`: every use of the key needs a face that the system recognises (Face ID,
///   or Touch ID on a device that has no Face ID), and the key stops working when a face is added
///   or removed. Without the second part, a person who knows the passcode of this device could
///   add their own face and unlock the Mac with it.
/// - Nothing in the rule lets the passcode stand in for the face, which is why the rule is not
///   `userPresence`, is not `devicePasscode`, and has no `or`.
/// - `privateKeyUsage`: the key may be used in the Secure Enclave at all. The system asks for it
///   of every key that is made there.
///
/// The key is kept only on a device that has a passcode, and only while the device is unlocked
/// (`kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly`). It is in no backup, it does not move to
/// another device, and it goes when the passcode is removed.
final class SecureEnclaveFaceKeys: FaceKeys {
  func publicKey(for id: [UInt8], renew: Bool) throws -> [UInt8] {
    guard let tag = Enclave.tag(for: id) else { throw FaceKeyFailure.cannotMake }
    guard Enclave.hasSecureEnclave else { throw FaceKeyFailure.noEnclave }
    // What can be found out without changing anything comes first, so that a key that works is
    // not replaced on a device that could not make the next one.
    try checkThatAFaceCanBeUsed()
    if !renew, let existing = try find(tag: tag) {
      return try describe(existing)
    }
    // A pair is found by its tag alone, so no pair may be left under the tag when a new one is
    // made: two could not be told apart, and a key that was meant to be gone would still open.
    try erase(tag: tag)
    let made = try make(tag: tag)
    do {
      return try describe(made)
    } catch {
      // A pair that cannot be given out is not kept. The next try makes another.
      _ = try? erase(tag: tag)
      throw error
    }
  }

  func attempt(opening sealed: [UInt8], for id: [UInt8], reason: String) -> FaceAttempt {
    SecureEnclaveAttempt(sealed: sealed, tag: Enclave.tag(for: id), reason: reason)
  }

  /// A key that needs a face can be made only on a device that has a passcode and a face that is
  /// set up, and the system says which of them is missing. Asking first gives the person that
  /// sentence, where making the key would give a number of the keychain that says little.
  private func checkThatAFaceCanBeUsed() throws {
    let context = LAContext()
    var problem: NSError?
    if context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &problem) {
      return
    }
    guard let refusal = problem else { throw FaceKeyFailure(biometry: nil) }
    throw FaceKeyFailure(step: "check the face", system: FaceSystemError.chain(of: refusal))
  }

  /// The private key of the pair under `tag`, or nil if there is none.
  private func find(tag: Data) throws -> SecKey? {
    // The lookup gives back a reference to the key and needs no face, and this device asks the
    // person nothing when it gives the key out. A context that may show nothing makes sure of
    // that: if the system wanted a face for the lookup, the lookup would fail and not ask.
    let context = LAContext()
    context.interactionNotAllowed = true
    let found = Enclave.lookUp(tag: tag, context: context)
    if found.status == errSecItemNotFound { return nil }
    guard found.status == errSecSuccess else {
      throw FaceKeyFailure(step: "find the key", system: [Enclave.status(found.status)])
    }
    guard let key = found.key else { throw FaceKeyFailure.cannotMake }
    return key
  }

  /// Removes every key under `tag`. A key that is not there is not a failure.
  private func erase(tag: Data) throws {
    let code = SecItemDelete(Enclave.keyQuery(tag: tag) as CFDictionary)
    guard code == errSecSuccess || code == errSecItemNotFound else {
      throw FaceKeyFailure(step: "remove the old key", system: [Enclave.status(code)])
    }
  }

  /// Makes the pair in the Secure Enclave and keeps it in the keychain under `tag`.
  private func make(tag: Data) throws -> SecKey {
    var ruleProblem: Unmanaged<CFError>?
    guard
      let rule = SecAccessControlCreateWithFlags(
        kCFAllocatorDefault, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly,
        [.privateKeyUsage, .biometryCurrentSet], &ruleProblem)
    else {
      throw FaceKeyFailure(step: "make the rule", system: Enclave.system(ruleProblem))
    }
    // The token goes in the attributes of the whole pair and not in those of the private key, as
    // the documentation of kSecAttrTokenID says. The tag and the rule are those of the private
    // key, which is the half that is kept.
    let attributes: [String: Any] = [
      kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeySizeInBits as String: 256,
      kSecAttrTokenID as String: kSecAttrTokenIDSecureEnclave,
      kSecPrivateKeyAttrs as String: [
        kSecAttrIsPermanent as String: true,
        kSecAttrApplicationTag as String: tag,
        kSecAttrAccessControl as String: rule,
      ] as [String: Any],
    ]
    var keyProblem: Unmanaged<CFError>?
    guard let key = SecKeyCreateRandomKey(attributes as CFDictionary, &keyProblem) else {
      throw FaceKeyFailure(step: "make the key", system: Enclave.system(keyProblem))
    }
    return key
  }

  /// The public half of the pair of `key`, as the 65 bytes of a point in the form of ANSI X9.63,
  /// once the system says that the key can open what the Mac seals. Asking now is better than
  /// finding out at the first unlock, after the person has set up all the rest.
  private func describe(_ key: SecKey) throws -> [UInt8] {
    guard SecKeyIsAlgorithmSupported(key, .decrypt, Enclave.algorithm) else {
      throw FaceKeyFailure.cannotOpen
    }
    guard let publicKey = SecKeyCopyPublicKey(key) else { throw FaceKeyFailure.cannotMake }
    var problem: Unmanaged<CFError>?
    guard let data = SecKeyCopyExternalRepresentation(publicKey, &problem) else {
      throw FaceKeyFailure(step: "export the public key", system: Enclave.system(problem))
    }
    return Enclave.bytes(of: data)
  }
}

// MARK: - One try to open a sealed key

/// One try to open a sealed key with the private half of one pair, which asks for the face.
///
/// The system asks for the face when the key is used, so `open` waits in the call that opens the
/// sealed key for as long as the person takes. A fresh `LAContext` is made for each try and given
/// to the key (`kSecUseAuthenticationContext`), which is how the prompt gets its reason and how
/// it is taken down. The context is used once: a face that was recognised on a context would open
/// the key again for as long as the context lives, so it is invalidated as soon as the try is
/// over.
private final class SecureEnclaveAttempt: FaceAttempt {
  private let sealed: [UInt8]
  /// nil if the id of the device was not one that a tag can be made of.
  private let tag: Data?
  private let reason: String

  private let lock = NSLock()
  private var cancelled = false
  /// The context of the prompt while `open` is running, so that `cancel` can take it down.
  private var held: LAContext?

  init(sealed: [UInt8], tag: Data?, reason: String) {
    self.sealed = sealed
    self.tag = tag
    self.reason = reason
  }

  func open() -> FaceResult {
    guard let tag else { return .declined(.failed) }
    let context = LAContext()
    // What the prompt says under the name of the app.
    context.localizedReason = reason
    // The prompt offers the passcode after a face was not recognised, unless this is empty. The
    // key would not open with the passcode, so the button would only mislead.
    context.localizedFallbackTitle = ""
    // A face is asked for every time. This is also the default.
    context.touchIDAuthenticationAllowableReuseDuration = 0
    guard hold(context) else { return .declined(.declined) }
    defer { drop(context) }

    let found = Enclave.lookUp(tag: tag, context: context)
    guard found.status == errSecSuccess else {
      return fail("find the key", [Enclave.status(found.status)])
    }
    guard let key = found.key else { return fail("read the key", []) }
    guard SecKeyIsAlgorithmSupported(key, .decrypt, Enclave.algorithm) else {
      return fail("use the key to open a sealed key", [])
    }
    // The prompt is not begun for a question that was taken back while the key was looked for.
    if isCancelled { return .declined(.declined) }

    var problem: Unmanaged<CFError>?
    guard
      let opened = SecKeyCreateDecryptedData(
        key, Enclave.algorithm, Data(sealed) as CFData, &problem)
    else {
      return fail("open the sealed key", Enclave.system(problem))
    }
    // The bytes are copied once, into the memory that is wiped, and the data of the system is let
    // go of. The system does not let that data be wiped, so it stays in freed memory until the
    // memory is used again.
    let count = CFDataGetLength(opened)
    guard count > 0, let start = CFDataGetBytePtr(opened),
      let boxKey = FaceBoxKey(copying: UnsafeRawBufferPointer(start: start, count: count))
    else { return fail("read the opened key", []) }
    return .opened(boxKey)
  }

  func cancel() {
    lock.lock()
    cancelled = true
    let context = held
    held = nil
    lock.unlock()
    // Outside the lock: the Rust side calls this with the lock of its session held, and it must
    // not wait for anything of ours. The documentation of `invalidate` says that the pending
    // evaluations of the context fail with the error for a cancel by the app. That the prompt of
    // a key that uses the context comes down with them is to be checked on a device.
    context?.invalidate()
  }

  private var isCancelled: Bool {
    lock.lock()
    defer { lock.unlock() }
    return cancelled
  }

  /// Keeps the context so that `cancel` finds it. false if the try was cancelled already.
  private func hold(_ context: LAContext) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    if cancelled { return false }
    held = context
    return true
  }

  private func drop(_ context: LAContext) {
    lock.lock()
    held = nil
    lock.unlock()
    context.invalidate()
  }

  /// The answer for a try that did not open the key, sorted by what the system said. The step and
  /// the codes go to the log: it is where whoever tries the device sees which error the system
  /// gives for a face that was not recognised, a cancel, and a key that a new face ended.
  private func fail(_ step: String, _ errors: [FaceSystemError]) -> FaceResult {
    NSLog(
      "Baton: the sealed key was not opened: it did not %@ (%@).", step,
      FaceSystemError.line(errors))
    return .declined(FaceDecline(system: errors))
  }
}

// MARK: - What both use

private enum Enclave {
  /// The way the Mac seals the key of the box: ECIES on P-256 with the X9.63 key derivation and
  /// SHA-256, AES-GCM, and an IV that the derivation makes. It is what the Mac's side of the
  /// unlock uses, and it is the one place to change if the Secure Enclave turns out not to do it
  /// (`SecKeyIsAlgorithmSupported` says so at the setup). The Cofactor variant of the same
  /// algorithm gives the same bytes on P-256, whose cofactor is 1.
  static var algorithm: SecKeyAlgorithm { .eciesEncryptionStandardVariableIVX963SHA256AESGCM }

  /// The Simulator has no Secure Enclave. A key that asks for one cannot be made there.
  static var hasSecureEnclave: Bool {
    #if targetEnvironment(simulator)
      return false
    #else
      return true
    #endif
  }

  /// The tag of the pair for the device `id`, as the keychain wants it.
  static func tag(for id: [UInt8]) -> Data? {
    FaceTag.make(for: id).map { Data($0.utf8) }
  }

  /// The keys of the pair under a tag: what a deletion and a lookup both start from.
  static func keyQuery(tag: Data) -> [String: Any] {
    [
      kSecClass as String: kSecClassKey,
      kSecAttrApplicationTag as String: tag,
      kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
    ]
  }

  /// Looks for the private key under `tag`, to be used with `context`. `status` is what the
  /// lookup said. `key` is there only if it succeeded and gave a key.
  static func lookUp(tag: Data, context: LAContext) -> (key: SecKey?, status: OSStatus) {
    var query = keyQuery(tag: tag)
    query[kSecAttrKeyClass as String] = kSecAttrKeyClassPrivate
    query[kSecReturnRef as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    query[kSecUseAuthenticationContext as String] = context
    var item: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &item)
    guard status == errSecSuccess else { return (nil, status) }
    return (key(from: item), status)
  }

  /// The key that a lookup gave, if what it gave is a key. An object of Core Foundation is told
  /// by its type and not by a cast, so the type is asked before the object is taken for a key.
  private static func key(from item: CFTypeRef?) -> SecKey? {
    guard let item, CFGetTypeID(item) == SecKeyGetTypeID() else { return nil }
    return unsafeBitCast(item, to: SecKey.self)
  }

  /// A status of the keychain or of Security.framework, as the error that the sorting reads.
  static func status(_ code: OSStatus) -> FaceSystemError {
    FaceSystemError(domain: FaceSystemCodes.osStatus, code: Int(code))
  }

  /// The error that a function of Security.framework gave, which the caller has to release, and
  /// the errors under it. Empty if it gave none.
  static func system(_ problem: Unmanaged<CFError>?) -> [FaceSystemError] {
    guard let problem else { return [] }
    return FaceSystemError.chain(of: problem.takeRetainedValue() as Error)
  }

  /// A copy of the bytes of `data`. For data that is no secret.
  static func bytes(of data: CFData) -> [UInt8] {
    let count = CFDataGetLength(data)
    guard count > 0, let start = CFDataGetBytePtr(data) else { return [] }
    return Array(UnsafeBufferPointer(start: start, count: count))
  }
}

#endif
