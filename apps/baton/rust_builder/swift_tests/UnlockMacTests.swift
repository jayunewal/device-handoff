// SPDX-License-Identifier: Apache-2.0

// Tests of BatonUnlockMac.swift with the system itself, which only a Mac has: on other systems this
// file is empty. The box is made and opened with CryptoKit, and its key is sealed with the ECIES of
// Security.framework to a key of P-256 that a test makes in software, in the place of the key in the
// Secure Enclave of the other device, and opened with the private half. What needs a person, an
// account or the lock screen of a Mac (the check of the password, the wake of a display, the keys
// that reach the lock screen) is checked by hand (docs/adr/0013-unlock-from-a-phone.md, U1 to U12).

#if os(macOS)

import CryptoKit
import Foundation
import Security

// MARK: - The box

func testABoxHoldsThePasswordWholeAndItsSizeDoesNotSayHowLongItIs() {
  let boxing = MacBoxing()
  let password = makeSecret("Ab1! pässwörd €😀")
  guard let made = boxing.make(password: password, target: deviceId(1), viewer: deviceId(2)) else {
    return check(false, "no box was made")
  }
  check(made.boxed.first == MacBoxing.form, "the form of the box")
  // The form, the nonce, the password padded to 128 bytes, and the tag.
  check(made.boxed.count == 1 + 12 + 128 + 16, "\(made.boxed.count) bytes")
  check(made.key.count == UnlockLimits.boxKeyBytes, "the key has \(made.key.count) bytes")
  let opened = boxing.open(made.boxed, key: made.key, target: deviceId(1), viewer: deviceId(2))
  check(opened.flatMap { bytesOf($0) } == bytesOf(password), "the password came back changed")

  let short = boxing.make(password: makeSecret("a"), target: deviceId(1), viewer: deviceId(2))
  check(short?.boxed.count == made.boxed.count, "the size of the box says how long the password is")
  let longest = boxing.make(
    password: makeSecret([UInt8](repeating: 65, count: UnlockLimits.passwordMax)),
    target: deviceId(1), viewer: deviceId(2))
  check(longest?.boxed.count == made.boxed.count, "the longest password makes a larger box")
  let reopened = longest.flatMap {
    boxing.open($0.boxed, key: $0.key, target: deviceId(1), viewer: deviceId(2))
  }
  check(reopened.flatMap { bytesOf($0) } == [UInt8](repeating: 65, count: 127), "the longest")

  // A password that is too long is not boxed, and neither is none.
  let tooLong = makeSecret([UInt8](repeating: 65, count: UnlockLimits.passwordMax + 1))
  check(boxing.make(password: tooLong, target: deviceId(1), viewer: deviceId(2)) == nil)
  let wiped = makeSecret("x")
  wiped.wipe()
  check(boxing.make(password: wiped, target: deviceId(1), viewer: deviceId(2)) == nil, "wiped")

  // Two boxes of one password are two boxes, under two keys.
  let again = boxing.make(password: password, target: deviceId(1), viewer: deviceId(2))
  check(again != nil && again?.boxed != made.boxed, "the same box twice")
  check(again.flatMap { bytesOf($0.key) } != bytesOf(made.key), "the same key twice")
}

func testABoxOpensNothingWithAnotherKeyOtherIdsAnotherFormOrAnyChange() {
  let boxing = MacBoxing()
  guard let made = boxing.make(
    password: makeSecret("hunter2"), target: deviceId(1), viewer: deviceId(2))
  else { return check(false, "no box was made") }
  func opens(
    _ boxed: [UInt8], key: Secret? = nil, target: [UInt8] = deviceId(1),
    viewer: [UInt8] = deviceId(2)
  ) -> Bool {
    boxing.open(boxed, key: key ?? made.key, target: target, viewer: viewer) != nil
  }
  check(opens(made.boxed), "the box does not open with its key")

  let other = boxing.make(password: makeSecret("hunter2"), target: deviceId(1), viewer: deviceId(2))
  check(other.map { !opens(made.boxed, key: $0.key) } == true, "another key opened the box")
  check(!opens(made.boxed, target: deviceId(3)), "another device that is unlocked")
  check(!opens(made.boxed, viewer: deviceId(3)), "another device that unlocks")
  check(!opens(made.boxed, target: deviceId(2), viewer: deviceId(1)), "the ids the wrong way round")
  check(!opens(made.boxed, target: Array(deviceId(1).dropLast())), "a short id")
  check(!opens(made.boxed, target: []), "no id")

  // A form that this version does not know opens nothing, and neither does any byte that changed.
  var form = made.boxed
  form[0] = MacBoxing.form + 1
  check(!opens(form), "another form")
  for at in made.boxed.indices {
    var changed = made.boxed
    changed[at] ^= 0x01
    check(!opens(changed), "a box with byte \(at) changed")
  }
  check(!opens([]), "an empty box")
  check(!opens([MacBoxing.form]), "a box of its form byte")
  check(!opens(Array(made.boxed.dropLast())), "a box without its last byte")
  check(!opens(Array(made.boxed.dropFirst())), "a box without its first byte")
  check(!opens(made.boxed + [0]), "a box with a byte more")

  let wiped = makeSecret(goodBoxKey)
  wiped.wipe()
  check(!opens(made.boxed, key: wiped), "a key that was wiped")
  check(!opens(made.boxed, key: makeSecret([UInt8](repeating: 1, count: 31))), "a short key")
  check(!opens(made.boxed, key: makeSecret([UInt8](repeating: 1, count: 33))), "a long key")
}

// MARK: - Sealing the key

/// A key of P-256 made in software, in the place of the one in the Secure Enclave of the phone.
private func softwareKey() -> (private: SecKey, public: [UInt8])? {
  let attributes: [String: Any] = [
    kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
    kSecAttrKeySizeInBits as String: 256,
  ]
  var failure: Unmanaged<CFError>?
  guard let made = SecKeyCreateRandomKey(attributes as CFDictionary, &failure) else {
    failure?.release()
    return nil
  }
  guard let publicKey = SecKeyCopyPublicKey(made),
    let external = SecKeyCopyExternalRepresentation(publicKey, &failure)
  else {
    failure?.release()
    return nil
  }
  return (made, Array(external as Data))
}

func testTheKeyOfABoxIsSealedToAKeyOfP256AndOnlyThatKeyOpensIt() {
  guard let phone = softwareKey(), let stranger = softwareKey() else {
    return check(false, "no software key could be made")
  }
  check(phone.public.count == UnlockSealing.pointBytes && phone.public.first == 0x04, "the point")
  let boxing = MacBoxing()
  let key = makeSecret(goodBoxKey)
  guard let sealed = boxing.seal(key, to: phone.public, sealing: .eciesP256) else {
    return check(false, "the key was not sealed")
  }
  check(!sealed.isEmpty && sealed.count <= UnlockLimits.sealedMax, "\(sealed.count) bytes")
  check(bytesOf(key) == goodBoxKey, "sealing touched the key")

  let algorithm = SecKeyAlgorithm.eciesEncryptionStandardVariableIVX963SHA256AESGCM
  var failure: Unmanaged<CFError>?
  let opened = SecKeyCreateDecryptedData(phone.private, algorithm, Data(sealed) as CFData, &failure)
  failure?.release()
  check(opened.map { Array($0 as Data) } == goodBoxKey, "the phone's key did not open it")
  failure = nil
  let notOpened = SecKeyCreateDecryptedData(
    stranger.private, algorithm, Data(sealed) as CFData, &failure)
  failure?.release()
  check(notOpened == nil, "another key opened it")

  // Sealed twice, it is two different things: the sealing is not deterministic.
  let again = boxing.seal(key, to: phone.public, sealing: .eciesP256)
  check(again != nil && again != sealed, "the same sealed key twice")

  // Nothing to seal to, or nothing to seal.
  check(boxing.seal(key, to: [], sealing: .eciesP256) == nil, "no key")
  check(boxing.seal(key, to: Array(phone.public.dropLast()), sealing: .eciesP256) == nil, "short")
  check(boxing.seal(key, to: [0x02] + phone.public.dropFirst(), sealing: .eciesP256) == nil, "form")
  let wiped = makeSecret(goodBoxKey)
  wiped.wipe()
  check(boxing.seal(wiped, to: phone.public, sealing: .eciesP256) == nil, "a key that was wiped")
}

func testAnUnlockIsSetUpAndOpenedWithNothingButThePhonesKeyAndTheBoxKeyItSealed() {
  // The two halves, as the Mac does them, with no platform in the way: the box is made and its key
  // sealed (setup), the phone opens the sealed key, and the Mac opens the box with it.
  guard let phone = softwareKey() else { return check(false, "no software key") }
  let boxing = MacBoxing()
  let password = makeSecret("Correct horse 7!")
  guard let made = boxing.make(password: password, target: deviceId(1), viewer: deviceId(2)),
    let sealed = boxing.seal(made.key, to: phone.public, sealing: .eciesP256)
  else { return check(false, "no box") }
  var failure: Unmanaged<CFError>?
  let algorithm = SecKeyAlgorithm.eciesEncryptionStandardVariableIVX963SHA256AESGCM
  guard let key = SecKeyCreateDecryptedData(phone.private, algorithm, Data(sealed) as CFData, &failure)
  else {
    failure?.release()
    return check(false, "the phone could not open the sealed key")
  }
  let theirs = makeSecret(Array(key as Data))
  let opened = boxing.open(made.boxed, key: theirs, target: deviceId(1), viewer: deviceId(2))
  check(opened.flatMap { bytesOf($0) } == bytesOf(password), "the password did not come back")
}

// MARK: - The keys

/// A poster that writes down what it is given, and fails where a test says.
final class RecordingPoster: EventPoster {
  var posted: [SyntheticEvent] = []
  var failing: Set<Int> = []

  func post(_ event: SyntheticEvent) -> Bool {
    let number = posted.count
    posted.append(event)
    return !failing.contains(number)
  }
}

private func typingEnvironment(_ poster: EventPoster) -> MacUnlockEnvironment {
  let privacy = PrivateSession(
    curtain: FakeCurtain(), guardian: FakeGuard(), end: { _ in }, sleep: { _ in },
    now: { ProcessInfo.processInfo.systemUptime })
  return MacUnlockEnvironment(
    session: LibraryUnlockSession(link: 1), privacy: privacy, poster: poster)
}

private func keyEvent(
  _ code: UInt16, down: Bool, shifted: Bool = false, text: [UInt16]? = nil
) -> SyntheticEvent {
  .key(
    SyntheticKey(
      virtualKey: code, down: down, autorepeat: false, modifiers: shifted ? [.shift] : [],
      text: text))
}

func testAKeyIsPressedAndReleasedWithShiftIfItNeedsItAndTheCharacterIsSetOnTheEvents() {
  let poster = RecordingPoster()
  let env = typingEnvironment(poster)
  check(env.type(keyFor("a")))
  check(
    poster.posted == [
      keyEvent(0x00, down: true, text: [0x61]), keyEvent(0x00, down: false, text: [0x61]),
    ], "\(poster.posted)")

  poster.posted = []
  check(env.type(keyFor("A")))
  check(
    poster.posted == [
      keyEvent(56, down: true, shifted: true),
      keyEvent(0x00, down: true, shifted: true, text: [0x41]),
      keyEvent(0x00, down: false, shifted: true, text: [0x41]),
      keyEvent(56, down: false),
    ], "\(poster.posted)")

  // A character of two UTF-16 units goes in one event, and Return has no character.
  poster.posted = []
  check(env.type(keyFor("😀")))
  check(
    poster.posted == [
      keyEvent(UnlockKeys.anyKey, down: true, text: [0xD83D, 0xDE00]),
      keyEvent(UnlockKeys.anyKey, down: false, text: [0xD83D, 0xDE00]),
    ], "\(poster.posted)")
  poster.posted = []
  check(env.type(UnlockKeys.returnKey))
  check(poster.posted == [keyEvent(36, down: true), keyEvent(36, down: false)], "\(poster.posted)")

  // A scalar that is no character is not typed at all.
  poster.posted = []
  check(!env.type(TypedKey(virtualKey: 0, shift: false, scalar: 0xD800)))
  check(poster.posted.isEmpty, "\(poster.posted)")
}

func testAKeyThatCannotBePostedIsReportedAndShiftIsReleasedWhateverHappened() {
  // The key goes down but the post fails: nothing comes up that did not go down, Shift comes up.
  let down = RecordingPoster()
  down.failing = [1]
  check(!typingEnvironment(down).type(keyFor("A")))
  check(
    down.posted == [
      keyEvent(56, down: true, shifted: true),
      keyEvent(0x00, down: true, shifted: true, text: [0x41]), keyEvent(56, down: false),
    ], "\(down.posted)")

  // The key comes up wrong: Shift comes up all the same.
  let up = RecordingPoster()
  up.failing = [2]
  check(!typingEnvironment(up).type(keyFor("A")))
  check(up.posted.count == 4 && up.posted.last == keyEvent(56, down: false), "\(up.posted)")

  // Shift does not go down: nothing else is posted.
  let shift = RecordingPoster()
  shift.failing = [0]
  check(!typingEnvironment(shift).type(keyFor("A")))
  check(shift.posted == [keyEvent(56, down: true, shifted: true)], "\(shift.posted)")

  // Without Shift there is nothing to release.
  let plain = RecordingPoster()
  plain.failing = [0]
  check(!typingEnvironment(plain).type(keyFor("a")))
  check(plain.posted == [keyEvent(0x00, down: true, text: [0x61])], "\(plain.posted)")
}

// MARK: - The password

func testAPasswordThatIsNotWellFormedUtf8IsNotCheckedAtAll() {
  let checker = MacPasswordChecker()
  for bytes: [UInt8] in [[0xFF, 0xFE], [0x61, 0xC3], [0xED, 0xA0, 0x80]] {
    let secret = makeSecret(bytes)
    if case .failed(let sentence) = checker.check(secret) {
      check(!sentence.isEmpty)
    } else {
      check(false, "a password that is not text was checked: \(bytes)")
    }
  }
}

// MARK: - The sharer and the screen

func testTheMacCanBeUnlockedAndSetsAnUnlockUpOnlyForAWayOfSealingAndAKeyItHas() {
  guard #available(macOS 14.0, *) else { return }
  let sharer = MacSharer()
  check(sharer.canUnlock, "a Mac that cannot be unlocked")
  // A way of sealing that this version does not have, and a key it cannot seal to, are refused
  // before the password is looked at.
  let unknown = makeSetup(password: Array("Ab1!".utf8), sealing: 9)
  check(sharer.unlockSetup(unknown) == .unsupported, "another sealing")
  check(unknown.password.isWiped, "the password was not wiped")
  let short = makeSetup(password: Array("Ab1!".utf8), viewerKey: [0x04, 1, 2])
  if case .failed = sharer.unlockSetup(short) {} else { check(false, "a short key") }
  check(short.password.isWiped, "the password was not wiped")
}

func testAScreenThatNeverStartedIsNotUnlockedAndItsKeyIsWiped() {
  guard #available(macOS 14.0, *) else { return }
  let sharer = MacSharer()
  do {
    let given = try sharer.screen(privacy: .private)
    let attempt = makeAttempt()
    check(given.window.unlock(attempt) == .notPrivate, "a screen with no privacy raised")
    check(attempt.boxKey.isWiped, "the key of the box was not wiped")
    given.window.released()
  } catch let refusal as ScreenRefusal {
    check(
      refusal == .needsPermission || refusal == .needsInputPermission || refusal == .unavailable,
      "\(refusal)")
  } catch let failure as SharerFailure {
    check(!failure.sentence.isEmpty, "a failure with no sentence")
  } catch {
    check(false, "an error that is not told: \(error)")
  }
}

#endif
