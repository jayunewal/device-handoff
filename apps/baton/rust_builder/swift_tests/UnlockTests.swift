// SPDX-License-Identifier: Apache-2.0

// Tests of BatonUnlock.swift: the secrets and when they are wiped, the reading of what the Rust
// side passes, the keys that are typed for a character, the setting up of an unlock, and the six
// steps of an unlock and the order they come in. The box, the check of the password, the keys and
// the cover are played by fakes that write down what they were asked, so that the order can be
// read; what the real ones do is checked on a Mac (docs/adr/0013-unlock-from-a-phone.md, U1 to
// U12) and in UnlockMacTests.swift.

import Foundation

// MARK: - Things to unlock with

/// Bytes as a secret, for a test.
func makeSecret(_ bytes: [UInt8]) -> Secret {
  guard let made = bytes.withUnsafeBytes({ Secret(copying: $0) }) else {
    fatalError("a test made a secret of no bytes")
  }
  return made
}

func makeSecret(_ text: String) -> Secret {
  makeSecret(Array(text.utf8))
}

/// The bytes a secret holds, or nil if it was wiped.
func bytesOf(_ secret: Secret) -> [UInt8]? {
  secret.withBytes { Array($0) }
}

/// An id of a device: 32 bytes of `fill`.
func deviceId(_ fill: UInt8) -> [UInt8] {
  [UInt8](repeating: fill, count: UnlockLimits.deviceIdBytes)
}

/// A point of P-256 in the form of ANSI X9.63, as far as the interface checks it.
let aPublicKey: [UInt8] = [0x04] + [UInt8](repeating: 0x42, count: 64)

let goodBoxKey = [UInt8](repeating: 0x11, count: UnlockLimits.boxKeyBytes)

/// The key for a character.
func keyFor(_ character: Character) -> TypedKey {
  guard let scalar = character.unicodeScalars.first, let key = UnlockKeys.key(for: scalar.value)
  else { fatalError("a test asked for a character that has no key") }
  return key
}

extension Arena {
  /// A setup of the interface, the way the Rust side builds it.
  func unlockSetup(
    password: [UInt8], sealing: UInt32 = UInt32(BATON_SEALING_ECIES_P256),
    target: [UInt8] = deviceId(1), viewer: [UInt8] = deviceId(2), viewerKey: [UInt8] = aPublicKey
  ) -> UnsafeMutablePointer<BatonUnlockSetup> {
    let setup = shortStruct(BatonUnlockSetup.self, size: MemoryLayout<BatonUnlockSetup>.size)
    setup.pointee.sealing = sealing
    setup.pointee.password = batonBytes(password)
    setup.pointee.target = batonBytes(target)
    setup.pointee.viewer = batonBytes(viewer)
    setup.pointee.viewer_key = batonBytes(viewerKey)
    return setup
  }

  /// An attempt of the interface, the way the Rust side builds it.
  func unlockAttempt(
    boxKey: [UInt8] = goodBoxKey, boxed: [UInt8] = [1, 2, 3, 4],
    sealing: UInt32 = UInt32(BATON_SEALING_ECIES_P256), viewerKey: [UInt8] = aPublicKey,
    target: [UInt8] = deviceId(1), viewer: [UInt8] = deviceId(2)
  ) -> UnsafeMutablePointer<BatonUnlock> {
    let attempt = shortStruct(BatonUnlock.self, size: MemoryLayout<BatonUnlock>.size)
    attempt.pointee.sealing = sealing
    attempt.pointee.box_key = batonBytes(boxKey)
    attempt.pointee.boxed = batonBytes(boxed)
    attempt.pointee.viewer_key = batonBytes(viewerKey)
    attempt.pointee.target = batonBytes(target)
    attempt.pointee.viewer = batonBytes(viewer)
    return attempt
  }
}

/// A setup that was read from the struct, for the code that is not about the struct.
func makeSetup(
  password: [UInt8], sealing: UInt32 = UInt32(BATON_SEALING_ECIES_P256),
  viewerKey: [UInt8] = aPublicKey
) -> UnlockSetup {
  let arena = Arena()
  guard
    let setup = UnlockSetup(
      reading: arena.unlockSetup(password: password, sealing: sealing, viewerKey: viewerKey))
  else { fatalError("a test made a setup that cannot be read") }
  return setup
}

/// An attempt that was read from the struct, for the code that is not about the struct.
func makeAttempt(boxKey: [UInt8] = goodBoxKey) -> UnlockAttempt {
  let arena = Arena()
  guard let attempt = UnlockAttempt(reading: arena.unlockAttempt(boxKey: boxKey)) else {
    fatalError("a test made an attempt that cannot be read")
  }
  return attempt
}

// MARK: - Secrets

func testASecretIsWipedWhenItIsToldToAndWhenItIsLetGoOf() {
  var seenAfterAWipe: [UInt8]?
  do {
    let memory = UnsafeMutableRawBufferPointer.allocate(byteCount: 8, alignment: 8)
    memory.copyBytes(from: [1, 2, 3, 4, 5, 6, 7, 8] as [UInt8])
    let secret = Secret(adopting: memory) { released in
      seenAfterAWipe = Array(released)
      released.deallocate()
    }
    withExtendedLifetime(secret) {
      check(secret.count == 8)
      check(!secret.isWiped)
      check(bytesOf(secret) == [1, 2, 3, 4, 5, 6, 7, 8])
      check(secret.data() == Data([1, 2, 3, 4, 5, 6, 7, 8]))
      secret.wipe()
      check(secret.isWiped)
      check(bytesOf(secret) == nil, "a wiped secret gave its bytes")
      check(secret.data() == nil, "a wiped secret gave its data")
      check(secret.word(at: 0) == nil, "a wiped secret gave a word")
      check(secret.count == 8, "the count changed with the wipe")
      secret.wipe()
      check(seenAfterAWipe == nil, "the memory was released while the secret was held")
    }
  }
  check(
    seenAfterAWipe == [UInt8](repeating: 0, count: 8),
    "the memory was released with \(String(describing: seenAfterAWipe)) in it")

  // A secret that nobody wiped is wiped when it is let go of, before the memory is released.
  var seenAtRelease: [UInt8]?
  do {
    let memory = UnsafeMutableRawBufferPointer.allocate(byteCount: 4, alignment: 8)
    memory.copyBytes(from: [9, 9, 9, 9] as [UInt8])
    let secret = Secret(adopting: memory) { released in
      seenAtRelease = Array(released)
      released.deallocate()
    }
    withExtendedLifetime(secret) { check(!secret.isWiped) }
  }
  check(
    seenAtRelease == [0, 0, 0, 0], "the memory was released with \(String(describing: seenAtRelease))"
  )
}

func testASecretIsMadeOnlyOfASizeThatIsAllowedAndOfAFillThatWorks() {
  check(Secret(count: 0, fill: { _ in true }) == nil, "a secret of no bytes")
  check(Secret(count: Secret.mostBytes + 1, fill: { _ in true }) == nil, "a secret that is too big")
  check(Secret(count: Secret.mostBytes, fill: { _ in true })?.count == Secret.mostBytes)
  var startedWithZeros = false
  let made = Secret(count: 16) { memory in
    startedWithZeros = Array(memory) == [UInt8](repeating: 0, count: 16)
    memory.storeBytes(of: 0xAB, toByteOffset: 3, as: UInt8.self)
    return true
  }
  check(startedWithZeros, "the memory was not zero when the fill began")
  var expected = [UInt8](repeating: 0, count: 16)
  expected[3] = 0xAB
  check(made.flatMap { bytesOf($0) } == expected, "\(String(describing: made))")
  check(Secret(count: 8, fill: { _ in false }) == nil, "a fill that failed made a secret")
}

func testASecretIsACopyOfWhatItWasMadeFromAndSaysNothingOfItsBytes() {
  var source: [UInt8] = [1, 2, 3, 4]
  let copy = source.withUnsafeBytes { Secret(copying: $0) }
  source[0] = 99
  check(copy.flatMap { bytesOf($0) } == [1, 2, 3, 4], "the secret changed with its source")
  check(([] as [UInt8]).withUnsafeBytes { Secret(copying: $0) } == nil, "an empty secret")
  let big = [UInt8](repeating: 1, count: Secret.mostBytes + 1)
  check(big.withUnsafeBytes { Secret(copying: $0) } == nil, "a secret that is too big")
  if let copy {
    check("\(copy)" == "a secret of 4 bytes", "\(copy)")
    check(!String(describing: copy).contains("1"), "the secret printed its bytes")
  }
}

func testASecretIsReadFromWhatTheRustSidePassesAndOnlyWithinTheRange() {
  let arena = Arena()
  check(Secret(reading: arena.batonBytes([1, 2, 3]), within: 1...5) != nil)
  check(Secret(reading: arena.batonBytes([]), within: 1...5) == nil, "no bytes")
  check(Secret(reading: arena.batonBytes([1, 2, 3, 4, 5, 6]), within: 1...5) == nil, "too many")
  check(Secret(reading: BatonBytes(ptr: nil, len: 3), within: 1...5) == nil, "no pointer")
  var negative = arena.batonBytes([1])
  negative.len = -1
  check(Secret(reading: negative, within: 1...5) == nil, "a negative length")
  check(copyBytes(arena.batonBytes([7, 8]), within: 2...2) == [7, 8])
  check(copyBytes(arena.batonBytes([7, 8]), within: 3...3) == nil, "too few")
  check(copyBytes(BatonBytes(ptr: nil, len: 3), within: 1...5) == nil, "no pointer")
  check(copyBytes(negative, within: 0...5) == nil, "a negative length")
  check(copyBytes(arena.batonBytes([]), within: 0...5) == [], "no bytes where none are allowed")
}

func testAWordOfASecretIsReadOnlyWhereThereIsOne() {
  let secret = Secret(count: 8) { memory in
    memory.storeBytes(of: 0x0102_0304, toByteOffset: 0, as: UInt32.self)
    memory.storeBytes(of: 0xA0B0_C0D0, toByteOffset: 4, as: UInt32.self)
    return true
  }
  check(secret?.word(at: 0) == 0x0102_0304)
  check(secret?.word(at: 1) == 0xA0B0_C0D0)
  check(secret?.word(at: 2) == nil, "a word past the end")
  check(secret?.word(at: -1) == nil, "a word before the start")
  // Seven bytes hold one whole word.
  let odd = Secret(count: 7) { _ in true }
  check(odd?.word(at: 0) == 0)
  check(odd?.word(at: 1) == nil, "half a word")
}

// MARK: - What the Rust side passes

func testASetupIsReadWithItsPasswordInMemoryThatIsWiped() {
  let arena = Arena()
  let pointer = arena.unlockSetup(password: Array("Ab1!".utf8), sealing: 99)
  guard let setup = UnlockSetup(reading: pointer) else { return check(false, "a setup") }
  check(bytesOf(setup.password) == Array("Ab1!".utf8))
  check(setup.sealing == 99, "a way of sealing that is not known is kept as it came")
  check(setup.target == deviceId(1) && setup.viewer == deviceId(2), "the ids")
  check(setup.viewerKey == aPublicKey, "the key")
  setup.password.wipe()
  check(bytesOf(setup.password) == nil)
}

func testASetupThatIsNotWholeIsNotRead() {
  let arena = Arena()
  func read(_ pointer: UnsafeMutablePointer<BatonUnlockSetup>) -> Bool {
    UnlockSetup(reading: pointer) != nil
  }
  check(read(arena.unlockSetup(password: [1])), "the smallest password")
  check(
    read(arena.unlockSetup(password: [UInt8](repeating: 65, count: UnlockLimits.passwordMax))),
    "the largest password")
  check(
    !read(arena.unlockSetup(password: [UInt8](repeating: 65, count: UnlockLimits.passwordMax + 1))),
    "a password that is too long")
  check(!read(arena.unlockSetup(password: [])), "no password")
  check(!read(arena.unlockSetup(password: [1], target: deviceId(1) + [1])), "a long id")
  check(!read(arena.unlockSetup(password: [1], target: Array(deviceId(1).dropLast()))), "a short id")
  check(!read(arena.unlockSetup(password: [1], viewer: [])), "no id")
  check(!read(arena.unlockSetup(password: [1], viewerKey: [])), "no key")
  check(
    !read(arena.unlockSetup(password: [1], viewerKey: [UInt8](repeating: 1, count: 257))),
    "a key that is too long")
  // No pointer for a length.
  let broken = arena.unlockSetup(password: [1])
  broken.pointee.password = BatonBytes(ptr: nil, len: 4)
  check(!read(broken), "a password with a length and no pointer")
  // A struct that ends before the fields that this version knows is not read past its end.
  for size in [4, 8, 16, MemoryLayout<BatonUnlockSetup>.size - 8] {
    let short = arena.shortStruct(BatonUnlockSetup.self, size: size)
    check(!read(short), "a struct of \(size) bytes")
  }
}

func testAnAttemptIsReadWithItsKeyInMemoryThatIsWiped() {
  let arena = Arena()
  let pointer = arena.unlockAttempt(boxed: [5, 6, 7])
  guard let attempt = UnlockAttempt(reading: pointer) else { return check(false, "an attempt") }
  check(bytesOf(attempt.boxKey) == goodBoxKey)
  check(attempt.sealing == .eciesP256)
  check(attempt.boxed == [5, 6, 7])
  check(attempt.target == deviceId(1) && attempt.viewer == deviceId(2), "the ids")
  check(attempt.viewerKey == aPublicKey, "the key")
  attempt.boxKey.wipe()
  check(bytesOf(attempt.boxKey) == nil)
}

func testAnAttemptThatIsNotWholeIsNotOpened() {
  let arena = Arena()
  func read(_ pointer: UnsafeMutablePointer<BatonUnlock>) -> Bool {
    UnlockAttempt(reading: pointer) != nil
  }
  check(read(arena.unlockAttempt()))
  check(read(arena.unlockAttempt(boxed: [UInt8](repeating: 1, count: UnlockLimits.boxMax))))
  check(!read(arena.unlockAttempt(boxed: [UInt8](repeating: 1, count: UnlockLimits.boxMax + 1))), "box")
  check(!read(arena.unlockAttempt(boxed: [])), "no box")
  check(!read(arena.unlockAttempt(boxKey: Array(goodBoxKey.dropLast()))), "a short box key")
  check(!read(arena.unlockAttempt(boxKey: goodBoxKey + [1])), "a long box key")
  check(!read(arena.unlockAttempt(boxKey: [])), "no box key")
  check(!read(arena.unlockAttempt(sealing: 0)), "a way of sealing that is not known")
  check(!read(arena.unlockAttempt(sealing: 99)), "a way of sealing that is not known")
  check(!read(arena.unlockAttempt(viewerKey: [])), "no key to seal to")
  check(!read(arena.unlockAttempt(viewerKey: [0x03] + [UInt8](repeating: 1, count: 64))), "form")
  check(!read(arena.unlockAttempt(viewerKey: Array(aPublicKey.dropLast()))), "a short point")
  check(!read(arena.unlockAttempt(target: deviceId(1) + [1])), "a long id")
  check(!read(arena.unlockAttempt(viewer: [])), "no id")
  let broken = arena.unlockAttempt()
  broken.pointee.box_key = BatonBytes(ptr: nil, len: 32)
  check(!read(broken), "a box key with a length and no pointer")
  for size in [4, 8, 24, MemoryLayout<BatonUnlock>.size - 8] {
    let short = arena.shortStruct(BatonUnlock.self, size: size)
    check(!read(short), "a struct of \(size) bytes")
  }
}

func testTheWayOfSealingIsKnownByItsNumber() {
  check(UnlockSealing(number: UInt32(BATON_SEALING_ECIES_P256)) == .eciesP256)
  check(UnlockSealing(number: 0) == nil)
  check(UnlockSealing(number: 2) == nil)
  check(UnlockSealing(number: UInt32.max) == nil)
  check(UnlockSealing.eciesP256.accepts(key: aPublicKey))
  check(!UnlockSealing.eciesP256.accepts(key: []))
  check(!UnlockSealing.eciesP256.accepts(key: [0x04]))
  check(!UnlockSealing.eciesP256.accepts(key: [0x02] + [UInt8](repeating: 1, count: 64)))
  check(!UnlockSealing.eciesP256.accepts(key: aPublicKey + [1]))
}

func testWhatSetUpMadeIsWrittenOnlyToRoomThatIsThereAndIsEnough() {
  let arena = Arena()
  let boxRoom = UnsafeMutablePointer<UInt8>.allocate(capacity: UnlockLimits.boxMax)
  let sealedRoom = UnsafeMutablePointer<UInt8>.allocate(capacity: UnlockLimits.sealedMax)
  defer {
    boxRoom.deallocate()
    sealedRoom.deallocate()
  }
  func made(size: Int = MemoryLayout<BatonUnlockMade>.size, rooms: Bool = true)
    -> UnsafeMutablePointer<BatonUnlockMade>
  {
    let made = arena.shortStruct(BatonUnlockMade.self, size: size)
    boxRoom.initialize(repeating: 0, count: UnlockLimits.boxMax)
    sealedRoom.initialize(repeating: 0, count: UnlockLimits.sealedMax)
    if rooms && size >= MemoryLayout<BatonUnlockMade>.size {
      made.pointee.boxed = boxRoom
      made.pointee.sealed = sealedRoom
    }
    return made
  }
  let good = made()
  check(writeMade(boxed: [1, 2, 3], sealed: [4, 5], to: good))
  check(good.pointee.box_len == 3 && good.pointee.sealed_len == 2, "the lengths")
  check(Array(UnsafeBufferPointer(start: boxRoom, count: 3)) == [1, 2, 3])
  check(Array(UnsafeBufferPointer(start: sealedRoom, count: 2)) == [4, 5])

  let largest = made()
  check(
    writeMade(
      boxed: [UInt8](repeating: 1, count: UnlockLimits.boxMax),
      sealed: [UInt8](repeating: 2, count: UnlockLimits.sealedMax), to: largest))
  check(!writeMade(boxed: [], sealed: [1], to: made()), "no box")
  check(!writeMade(boxed: [1], sealed: [], to: made()), "no sealed key")
  check(
    !writeMade(boxed: [UInt8](repeating: 1, count: UnlockLimits.boxMax + 1), sealed: [1], to: made()),
    "a box that is too big")
  check(
    !writeMade(
      boxed: [1], sealed: [UInt8](repeating: 1, count: UnlockLimits.sealedMax + 1), to: made()),
    "a sealed key that is too big")
  let none = made(rooms: false)
  check(!writeMade(boxed: [1], sealed: [1], to: none), "no room")
  check(none.pointee.box_len == 0, "the length was set though nothing was written")
  let short = made(size: 8)
  check(!writeMade(boxed: [1], sealed: [1], to: short), "a struct that ends early")
}

// MARK: - The keys that are typed

func testEveryPrintableCharacterOfAsciiHasItsOwnKeyOfTheUsLayout() {
  var seen = Set<String>()
  for value in UInt32(0x20)...UInt32(0x7E) {
    guard let key = UnlockKeys.key(for: value) else {
      check(false, "no key for \(value)")
      continue
    }
    check(key.scalar == value, "the key for \(value) types \(String(describing: key.scalar))")
    check(seen.insert("\(key.virtualKey)/\(key.shift)").inserted, "two characters on one key: \(value)")
  }
  check(seen.count == 95, "\(seen.count) keys")
}

func testTheKeysOfLettersDigitsAndSymbolsAreThoseOfTheUsLayout() {
  // The letters and the digits agree with the table of the usages of the keyboard that the viewers
  // send (BatonInput.swift), which was made apart from this one.
  for (offset, letter) in "abcdefghijklmnopqrstuvwxyz".unicodeScalars.enumerated() {
    let plain = UnlockKeys.key(for: letter.value)
    check(plain?.virtualKey == macKey(forUsage: UInt32(0x04 + offset)), "the key of \(letter)")
    check(plain?.shift == false, "\(letter) needs Shift")
    let capital = UnlockKeys.key(for: letter.value - 32)
    check(capital?.virtualKey == plain?.virtualKey && capital?.shift == true, "the capital \(letter)")
  }
  for (offset, digit) in "1234567890".unicodeScalars.enumerated() {
    let plain = UnlockKeys.key(for: digit.value)
    check(plain?.virtualKey == macKey(forUsage: UInt32(0x1E + offset)), "the key of \(digit)")
    check(plain?.shift == false)
  }
  for (offset, symbol) in "!@#$%^&*()".unicodeScalars.enumerated() {
    let shifted = UnlockKeys.key(for: symbol.value)
    check(shifted?.virtualKey == macKey(forUsage: UInt32(0x1E + offset)), "the key of \(symbol)")
    check(shifted?.shift == true, "\(symbol) needs no Shift")
  }
  let pairs: [(plain: Unicode.Scalar, shifted: Unicode.Scalar, usage: UInt32)] = [
    ("-", "_", 0x2D), ("=", "+", 0x2E), ("[", "{", 0x2F), ("]", "}", 0x30), ("\\", "|", 0x31),
    (";", ":", 0x33), ("'", "\"", 0x34), ("`", "~", 0x35), (",", "<", 0x36), (".", ">", 0x37),
    ("/", "?", 0x38),
  ]
  for pair in pairs {
    let plain = UnlockKeys.key(for: pair.plain.value)
    let shifted = UnlockKeys.key(for: pair.shifted.value)
    check(plain?.virtualKey == macKey(forUsage: pair.usage), "the key of \(pair.plain)")
    check(plain?.shift == false, "\(pair.plain) needs Shift")
    check(shifted?.virtualKey == plain?.virtualKey && shifted?.shift == true, "\(pair.shifted)")
  }
  let space = UnlockKeys.key(for: 0x20)
  check(space == TypedKey(virtualKey: 0x31, shift: false, scalar: 0x20), "\(String(describing: space))")
  check(space?.virtualKey == macKey(forUsage: 0x2C))
  // Return is a key that types no character.
  check(UnlockKeys.returnKey == TypedKey(virtualKey: 36, shift: false, scalar: nil))
  check(UnlockKeys.shiftKey == 56)
}

func testCharactersThatAreNotPartOfAPasswordHaveNoKeyAndOthersAreTypedWithTheirCharacter() {
  for value: UInt32 in [0x00, 0x08, 0x09, 0x0A, 0x0D, 0x1B, 0x7F, 0x80, 0x85, 0x9F] {
    check(UnlockKeys.key(for: value) == nil, "a control: \(value)")
  }
  for value: UInt32 in [0xD800, 0xDBFF, 0xDC00, 0xDFFF] {
    check(UnlockKeys.key(for: value) == nil, "a surrogate: \(value)")
  }
  for value: UInt32 in [0x11_0000, 0x7FFF_FFFF, UInt32.max] {
    check(UnlockKeys.key(for: value) == nil, "beyond Unicode: \(value)")
  }
  for value: UInt32 in [0xA0, 0xE9, 0x20AC, 0x1F600, 0x10_FFFF] {
    check(
      UnlockKeys.key(for: value)
        == TypedKey(virtualKey: UnlockKeys.anyKey, shift: false, scalar: value),
      "a character with no key: \(value)")
  }
}

// MARK: - The text of a password

/// What the decoder makes of `bytes`, and where it stopped.
private func decoded(_ bytes: [UInt8], at start: Int = 0) -> (value: UInt32?, index: Int) {
  var index = start
  let value = bytes.withUnsafeBytes { PasswordText.nextScalar(in: $0, at: &index) }
  return (value, index)
}

func testWellFormedUtf8IsDecodedAndTheDecoderMovesPastWhatItRead() {
  check(decoded([0x41]) == (0x41, 1))
  check(decoded([0xC3, 0xA9]) == (0xE9, 2))
  check(decoded([0xC2, 0x80]) == (0x80, 2))
  check(decoded([0xDF, 0xBF]) == (0x7FF, 2))
  check(decoded([0xE2, 0x82, 0xAC]) == (0x20AC, 3))
  check(decoded([0xE0, 0xA0, 0x80]) == (0x800, 3))
  check(decoded([0xEF, 0xBF, 0xBF]) == (0xFFFF, 3))
  check(decoded([0xF0, 0x9F, 0x98, 0x80]) == (0x1F600, 4))
  check(decoded([0xF0, 0x90, 0x80, 0x80]) == (0x10000, 4))
  check(decoded([0xF4, 0x8F, 0xBF, 0xBF]) == (0x10_FFFF, 4))
  // The text goes on after a character.
  check(decoded([0x41, 0xC3, 0xA9], at: 1) == (0xE9, 3))
  // A NUL is a character of ASCII to the decoder; the table of keys is what turns it away.
  check(decoded([0x00]) == (0, 1))
}

func testUtf8ThatIsNotWellFormedIsNotDecodedAndTheDecoderStaysWhereItWas() {
  let broken: [[UInt8]] = [
    [],
    [0x80], [0xBF],  // a continuation with nothing to continue
    [0xC0, 0x80], [0xC1, 0xBF],  // two bytes for a character of one
    [0xE0, 0x80, 0x80], [0xE0, 0x9F, 0xBF],  // three bytes for a character of fewer
    [0xF0, 0x80, 0x80, 0x80], [0xF0, 0x8F, 0xBF, 0xBF],  // four bytes for a character of fewer
    [0xED, 0xA0, 0x80], [0xED, 0xBF, 0xBF],  // surrogates
    [0xF4, 0x90, 0x80, 0x80],  // beyond U+10FFFF
    [0xF5, 0x80, 0x80, 0x80], [0xF8, 0x88, 0x80, 0x80, 0x80], [0xFF], [0xFE],  // no such start
    [0xC3], [0xE2, 0x82], [0xF0, 0x9F, 0x98],  // cut off
    [0xC3, 0x41], [0xE2, 0x82, 0x41], [0xE2, 0x41, 0xAC], [0xF0, 0x9F, 0x41, 0x80],  // not a continuation
  ]
  for bytes in broken {
    let result = decoded(bytes)
    check(result.value == nil && result.index == 0, "\(bytes) decoded to \(result)")
  }
  check(decoded([0x41], at: 1).value == nil, "an index at the end")
  check(decoded([0x41], at: 7).value == nil, "an index past the end")
  check(decoded([0x41], at: -1).value == nil, "an index before the start")
  check(decoded([0x41], at: 7).index == 7 && decoded([0x41], at: -1).index == -1, "the index moved")
}

func testAPasswordIsReadAsCharactersWholeOrNotAtAll() {
  func words(_ bytes: [UInt8]) -> [UInt32]? {
    guard let scalars = PasswordText.scalars(of: makeSecret(bytes)) else { return nil }
    return (0..<(scalars.count / 4)).compactMap { scalars.word(at: $0) }
  }
  check(words(Array("Ab1!".utf8)) == [0x41, 0x62, 0x31, 0x21])
  check(words(Array(" a b ".utf8)) == [0x20, 0x61, 0x20, 0x62, 0x20], "spaces")
  let text = "päss€😀"
  check(words(Array(text.utf8)) == text.unicodeScalars.map { $0.value }, "accents and the like")
  check(words([0x61, 0x09, 0x62]) == nil, "a tab")
  check(words([0x61, 0x0A]) == nil, "a new line")
  check(words([0x61, 0x00]) == nil, "a NUL")
  check(words([0x7F]) == nil, "delete")
  check(words([0xC2, 0x85]) == nil, "a control of Latin-1")
  check(words([0x61, 0xFF]) == nil, "bytes that are not UTF-8")
  check(words([0x61, 0xC3]) == nil, "a character that is cut off")
  check(words([0xED, 0xA0, 0x80]) == nil, "a surrogate")
  check(words([0xE2, 0x82, 0xAC]) == [0x20AC])
  let secret = makeSecret("abc")
  let scalars = PasswordText.scalars(of: secret)
  check(bytesOf(secret) == Array("abc".utf8), "the password itself was touched")
  scalars?.wipe()
  check(scalars?.isWiped == true)
}

// MARK: - Fakes of the box, the check and the Mac

/// A box that opens with one key, and makes boxes that are fakes. It writes down what it is asked.
final class FakeBoxing: UnlockBoxing {
  let journal: Journal
  /// The key that opens the box, and the password in it.
  var goodKey = goodBoxKey
  let password: [UInt8]
  var makeWorks = true
  var sealWorks = true
  var boxed: [UInt8] = [0xB0, 0x0B]
  var sealed: [UInt8] = [9, 9, 9, 9]
  /// What it handed out, to see whether it was wiped.
  private(set) var opened: Secret?
  private(set) var madeKeys: [Secret] = []
  /// What it was given, as it was when it was given.
  private(set) var openedWith: [[UInt8]?] = []
  private(set) var boxedPasswords: [[UInt8]?] = []
  private(set) var sealedKeys: [[UInt8]?] = []
  private(set) var sealedTo: [[UInt8]] = []

  init(journal: Journal, password: [UInt8]) {
    self.journal = journal
    self.password = password
  }

  func open(_ boxed: [UInt8], key: Secret, target: [UInt8], viewer: [UInt8]) -> Secret? {
    journal.add("open")
    openedWith.append(bytesOf(key))
    guard bytesOf(key) == goodKey else { return nil }
    let secret = makeSecret(password)
    opened = secret
    return secret
  }

  func make(password: Secret, target: [UInt8], viewer: [UInt8]) -> (boxed: [UInt8], key: Secret)? {
    journal.add("make")
    boxedPasswords.append(bytesOf(password))
    guard makeWorks else { return nil }
    let key = makeSecret([UInt8](repeating: 0x5A, count: UnlockLimits.boxKeyBytes))
    madeKeys.append(key)
    return (boxed, key)
  }

  func seal(_ key: Secret, to publicKey: [UInt8], sealing: UnlockSealing) -> [UInt8]? {
    journal.add("seal")
    sealedKeys.append(bytesOf(key))
    sealedTo.append(publicKey)
    return sealWorks ? sealed : nil
  }
}

final class FakeChecker: PasswordChecking {
  let journal: Journal
  var answer = PasswordCheck.accepted
  private(set) var seen: [[UInt8]?] = []

  init(journal: Journal) {
    self.journal = journal
  }

  func check(_ password: Secret) -> PasswordCheck {
    journal.add("check")
    seen.append(bytesOf(password))
    return answer
  }
}

/// The Mac, as an unlock sees it. Each answer is a function of how often that question was asked
/// before (the first time is 0), so that a test can change one of them at the Nth ask.
final class FakeEnvironment: UnlockEnvironment {
  let journal: Journal
  var liveAt: (Int) -> Bool = { _ in true }
  var keeps = true
  var lockAt: (Int) -> ScreenLock = { _ in .locked }
  var privateAt: (Int) -> Bool = { _ in true }
  var darkAt: (Int) -> Bool = { _ in true }
  var wakes = true
  var typesAt: (Int) -> Bool = { _ in true }
  /// Done when a key is typed, with the number of keys typed before it.
  var whenTyped: ((Int) -> Void)?
  private(set) var typed: [TypedKey] = []
  private(set) var kept: [(boxed: [UInt8], sealed: [UInt8])] = []
  private(set) var pauses: [TimeInterval] = []
  private var time: TimeInterval = 1000
  private var counts: [String: Int] = [:]

  init(journal: Journal) {
    self.journal = journal
  }

  private func asked(_ line: String) -> Int {
    let before = counts[line, default: 0]
    counts[line] = before + 1
    journal.add(line)
    return before
  }

  func live() -> Bool { liveAt(asked("live")) }

  func keep(boxed: [UInt8], sealed: [UInt8]) -> Bool {
    _ = asked("keep")
    kept.append((boxed, sealed))
    return keeps
  }

  func screenLock() -> ScreenLock { lockAt(asked("lock?")) }
  func privacyInPlace() -> Bool { privateAt(asked("private?")) }
  func coverIsDark() -> Bool { darkAt(asked("dark?")) }

  func wake() -> Bool {
    _ = asked("wake")
    return wakes
  }

  func type(_ key: TypedKey) -> Bool {
    let before = typed.count
    _ = asked("type")
    typed.append(key)
    whenTyped?(before)
    return typesAt(before)
  }

  func pause(_ seconds: TimeInterval) {
    _ = asked("pause")
    pauses.append(seconds)
    time += seconds
  }

  func now() -> TimeInterval { time }
}

/// An attempt, the box and the Mac to run it against.
final class UnlockRig {
  let journal = Journal()
  let boxing: FakeBoxing
  let env: FakeEnvironment
  let attempt: UnlockAttempt

  init(password: [UInt8] = Array("Ab1!".utf8), boxKey: [UInt8] = goodBoxKey) {
    boxing = FakeBoxing(journal: journal, password: password)
    env = FakeEnvironment(journal: journal)
    attempt = makeAttempt(boxKey: boxKey)
  }

  func run(timing: UnlockTiming = UnlockTiming()) -> UnlockOutcome {
    runUnlock(attempt, boxing: boxing, environment: env, timing: timing)
  }

  var lines: [String] { journal.all }

  /// Whatever the outcome: nothing of the key is left.
  func checkNothingIsLeft(_ what: String) {
    check(attempt.boxKey.isWiped, "\(what): the key of the box was not wiped")
    check(boxing.opened?.isWiped ?? true, "\(what): the password was not wiped")
    for key in boxing.madeKeys {
      check(key.isWiped, "\(what): a new key was not wiped")
    }
  }
}

// MARK: - Setting an unlock up

private func setUp(
  _ setup: UnlockSetup, answer: PasswordCheck = .accepted, configure: (FakeBoxing) -> Void = { _ in }
) -> (made: UnlockMade, lines: [String], boxing: FakeBoxing, checker: FakeChecker) {
  let journal = Journal()
  let boxing = FakeBoxing(journal: journal, password: [])
  let checker = FakeChecker(journal: journal)
  checker.answer = answer
  configure(boxing)
  let made = makeUnlock(setup, checker: checker, boxing: boxing)
  return (made, journal.all, boxing, checker)
}

func testAnUnlockIsSetUpByCheckingThePasswordBoxingItAndSealingTheKey() {
  let setup = makeSetup(password: Array("Ab1!".utf8))
  let done = setUp(setup)
  check(done.made == .made(boxed: [0xB0, 0x0B], sealed: [9, 9, 9, 9]), "\(done.made)")
  check(done.lines == ["check", "make", "seal"], "\(done.lines)")
  check(done.checker.seen.count == 1 && done.checker.seen[0] == Array("Ab1!".utf8), "checked")
  check(done.boxing.boxedPasswords == [Array("Ab1!".utf8)], "the password was boxed whole")
  check(done.boxing.sealedKeys == [[UInt8](repeating: 0x5A, count: 32)], "the key was sealed whole")
  check(done.boxing.sealedTo == [aPublicKey], "sealed to the key of the other device")
  check(setup.password.isWiped, "the password was not wiped")
  check(done.boxing.madeKeys.allSatisfy { $0.isWiped }, "the key of the box was not wiped")
}

func testAnUnlockIsNotSetUpWithAWayOfSealingOrAKeyThatThisMacDoesNotHave() {
  let unknown = makeSetup(password: [65], sealing: 7)
  let first = setUp(unknown)
  check(first.made == .unsupported, "\(first.made)")
  check(first.lines.isEmpty, "the password was checked or boxed: \(first.lines)")
  check(unknown.password.isWiped)

  let short = makeSetup(password: [65], viewerKey: [0x04, 1, 2, 3])
  let second = setUp(short)
  if case .failed(let sentence) = second.made {
    check(!sentence.isEmpty)
  } else {
    check(false, "\(second.made)")
  }
  check(second.lines.isEmpty, "the password was checked or boxed: \(second.lines)")
  check(short.password.isWiped)
}

func testWhatTheSystemSaysOfThePasswordIsWhatTheSetupSaysAndNothingIsBoxedForAPasswordItRefuses() {
  for (answer, expected) in [
    (PasswordCheck.wrong, UnlockMade.wrong), (.unsupported, .unsupported),
    (.failed("The system is busy."), .failed("The system is busy.")),
  ] {
    let setup = makeSetup(password: Array("Ab1!".utf8))
    let done = setUp(setup, answer: answer)
    check(done.made == expected, "\(done.made)")
    check(done.lines == ["check"], "something was boxed: \(done.lines)")
    check(setup.password.isWiped, "the password was not wiped")
  }
}

func testAPasswordThatCannotBeTypedIsRefusedAtTheSetupAndNotAtTheFirstUnlock() {
  for password: [UInt8] in [[0x61, 0x09, 0x62], [0x61, 0xFF], [0x61, 0x00]] {
    let setup = makeSetup(password: password)
    let done = setUp(setup)
    if case .failed(let sentence) = done.made {
      check(sentence.contains("cannot type"), sentence)
    } else {
      check(false, "\(done.made)")
    }
    check(done.lines == ["check"], "something was boxed: \(done.lines)")
    check(setup.password.isWiped)
  }
}

func testASetupThatCannotBeBoxedOrSealedFailsAndWipesWhatItMade() {
  let noBox = setUp(makeSetup(password: [65])) { $0.makeWorks = false }
  if case .failed = noBox.made {} else { check(false, "\(noBox.made)") }
  check(noBox.lines == ["check", "make"], "\(noBox.lines)")

  let noSeal = setUp(makeSetup(password: [65])) { $0.sealWorks = false }
  if case .failed = noSeal.made {} else { check(false, "\(noSeal.made)") }
  check(noSeal.lines == ["check", "make", "seal"], "\(noSeal.lines)")
  check(noSeal.boxing.madeKeys.allSatisfy { $0.isWiped }, "the key was not wiped")

  let tooBig = setUp(makeSetup(password: [65])) { $0.sealed = [UInt8](repeating: 1, count: 1025) }
  if case .failed = tooBig.made {} else { check(false, "\(tooBig.made)") }
  let empty = setUp(makeSetup(password: [65])) { $0.sealed = [] }
  if case .failed = empty.made {} else { check(false, "\(empty.made)") }
  let noBytes = setUp(makeSetup(password: [65])) { $0.boxed = [] }
  if case .failed = noBytes.made {} else { check(false, "\(noBytes.made)") }
  let hugeBox = setUp(makeSetup(password: [65])) { $0.boxed = [UInt8](repeating: 1, count: 1025) }
  if case .failed = hugeBox.made {} else { check(false, "\(hugeBox.made)") }
}

// MARK: - The unlock

/// The Mac is unlocked from the first look after Return, which is the first look of the wait.
private func unlockedAfterReturn(keys: Int) -> (Int) -> ScreenLock {
  // The looks come once for the screen, once before each key, once before Return, and then the wait.
  { $0 >= keys + 2 ? .unlocked : .locked }
}

func testAnUnlockDoesItsStepsInOrderAndTypesTheWholePasswordThenReturn() {
  let rig = UnlockRig(password: Array("Ab1!".utf8))
  rig.env.lockAt = unlockedAfterReturn(keys: 4)
  var wipedAtTheFirstKey: [Bool] = []
  rig.env.whenTyped = { before in
    guard before == 0 else { return }
    wipedAtTheFirstKey = [
      rig.attempt.boxKey.isWiped, rig.boxing.opened?.isWiped ?? false,
      rig.boxing.madeKeys.allSatisfy { $0.isWiped },
    ]
  }
  let outcome = rig.run()
  check(outcome == .unlocked, "\(outcome)")
  var expected = [
    "live", "open", "make", "seal", "keep", "lock?", "private?", "dark?", "wake", "private?",
    "dark?",
  ]
  for _ in 0..<5 { expected += ["live", "lock?", "type"] }
  expected += ["lock?"]
  check(rig.lines == expected, "\(rig.lines)")
  check(
    rig.env.typed == [keyFor("A"), keyFor("b"), keyFor("1"), keyFor("!"), UnlockKeys.returnKey],
    "\(rig.env.typed)")
  check(rig.env.kept.count == 1, "the new box was kept \(rig.env.kept.count) times")
  check(rig.env.kept.first?.boxed == [0xB0, 0x0B] && rig.env.kept.first?.sealed == [9, 9, 9, 9])
  check(rig.boxing.openedWith == [goodBoxKey], "the box was opened with the key")
  check(
    rig.boxing.boxedPasswords == [Array("Ab1!".utf8)],
    "the new box holds the password that was in the old one")
  check(rig.boxing.sealedTo == [aPublicKey], "the new key is sealed to the key of the other device")
  check(rig.env.pauses.isEmpty, "waited though the screen was unlocked")
  check(
    wipedAtTheFirstKey == [true, true, true],
    "the key of the box, the password and the new key were not all wiped before the first key: "
      + "\(wipedAtTheFirstKey)")
  rig.checkNothingIsLeft("an unlock")
}

func testAnAttemptThatIsOverTypesNothingAndKeepsNothing() {
  let rig = UnlockRig()
  rig.env.liveAt = { _ in false }
  check(rig.run() == .failed)
  check(rig.lines == ["live"], "\(rig.lines)")
  rig.checkNothingIsLeft("an attempt that is over")
}

func testAKeyThatDoesNotOpenTheBoxIsStaleAndNothingIsKeptOrTyped() {
  let rig = UnlockRig(boxKey: [UInt8](repeating: 0x22, count: 32))
  check(rig.run() == .stale)
  check(rig.lines == ["live", "open"], "\(rig.lines)")
  check(rig.env.kept.isEmpty)
  rig.checkNothingIsLeft("a stale key")
}

func testAnUnlockThatCannotMakeSealOrKeepTheNewBoxTypesNothing() {
  let noBox = UnlockRig()
  noBox.boxing.makeWorks = false
  check(noBox.run() == .failed)
  check(noBox.lines == ["live", "open", "make"], "\(noBox.lines)")
  noBox.checkNothingIsLeft("no new box")

  let noSeal = UnlockRig()
  noSeal.boxing.sealWorks = false
  check(noSeal.run() == .failed)
  check(noSeal.lines == ["live", "open", "make", "seal"], "\(noSeal.lines)")
  noSeal.checkNothingIsLeft("no sealed key")

  let noKeep = UnlockRig()
  noKeep.env.keeps = false
  check(noKeep.run() == .failed)
  check(noKeep.lines == ["live", "open", "make", "seal", "keep"], "\(noKeep.lines)")
  check(noKeep.env.typed.isEmpty, "keys were typed though the new box was not kept")
  noKeep.checkNothingIsLeft("a box that was not kept")
}

func testAnUnlockOfAPasswordThatCannotBeTypedWholeTypesNothingAtAll() {
  for password: [UInt8] in [[0x61, 0x09, 0x62], [0x61, 0xFF, 0x62], [0x61, 0x00]] {
    let rig = UnlockRig(password: password)
    check(rig.run() == .failed)
    check(rig.env.typed.isEmpty, "keys were typed for \(password)")
    check(!rig.lines.contains("lock?"), "the screen was looked at: \(rig.lines)")
    rig.checkNothingIsLeft("a password with no keys")
  }
}

func testAScreenThatIsNotLockedIsNotTypedAt() {
  let unlocked = UnlockRig()
  unlocked.env.lockAt = { _ in .unlocked }
  check(unlocked.run() == .notLocked)
  check(unlocked.env.typed.isEmpty && !unlocked.lines.contains("wake"), "\(unlocked.lines)")
  unlocked.checkNothingIsLeft("a screen that is unlocked")

  let unknown = UnlockRig()
  unknown.env.lockAt = { _ in .unknown }
  check(unknown.run() == .failed)
  check(unknown.env.typed.isEmpty && !unknown.lines.contains("wake"), "\(unknown.lines)")
  unknown.checkNothingIsLeft("a session that cannot be read")
}

func testAScreenWhosePrivacyIsNotInPlaceIsNotWokenOrTypedAt() {
  let open = UnlockRig()
  open.env.privateAt = { _ in false }
  check(open.run() == .notPrivate)
  check(open.env.typed.isEmpty && !open.lines.contains("wake"), "\(open.lines)")
  check(!open.lines.contains("dark?"), "the cover was put on though nothing is private")
  open.checkNothingIsLeft("a privacy that is not in place")

  let lit = UnlockRig()
  lit.env.darkAt = { _ in false }
  check(lit.run() == .notPrivate)
  check(lit.env.typed.isEmpty && !lit.lines.contains("wake"), "\(lit.lines)")
  lit.checkNothingIsLeft("a screen that is not dark")
}

func testTheCoverIsVerifiedAgainAfterTheWakeAndNothingIsTypedIfItIsLost() {
  let privacyLost = UnlockRig()
  privacyLost.env.privateAt = { $0 == 0 }
  check(privacyLost.run() == .notPrivate)
  check(privacyLost.lines.contains("wake") && privacyLost.env.typed.isEmpty, "\(privacyLost.lines)")
  privacyLost.checkNothingIsLeft("a privacy lost in the wake")

  let lightsUp = UnlockRig()
  lightsUp.env.darkAt = { $0 == 0 }
  check(lightsUp.run() == .notPrivate)
  check(lightsUp.lines.contains("wake") && lightsUp.env.typed.isEmpty, "\(lightsUp.lines)")
  lightsUp.checkNothingIsLeft("a screen that lit up in the wake")

  let noWake = UnlockRig()
  noWake.env.wakes = false
  check(noWake.run() == .failed)
  check(noWake.env.typed.isEmpty, "\(noWake.lines)")
  noWake.checkNothingIsLeft("displays that did not wake")
}

func testAnUnlockStopsBeforeTheFirstKeyThatIsNotForALockedScreenWhateverKeyThatIs() {
  // "ab1" is three keys and Return: four looks at the screen before a key, counted after the one
  // before the wake, and four asks whether the attempt goes on, counted after the first.
  let password = Array("ab1".utf8)
  for stop in 0...3 {
    let unlocked = UnlockRig(password: password)
    unlocked.env.lockAt = { $0 == 1 + stop ? .unlocked : .locked }
    check(unlocked.run() == .failed, "stopped by a screen that unlocked before key \(stop)")
    check(unlocked.env.typed.count == stop, "typed \(unlocked.env.typed.count) keys, not \(stop)")
    unlocked.checkNothingIsLeft("a screen that unlocked before key \(stop)")

    let unknown = UnlockRig(password: password)
    unknown.env.lockAt = { $0 == 1 + stop ? .unknown : .locked }
    check(unknown.run() == .failed, "stopped by a session that left the console before \(stop)")
    check(unknown.env.typed.count == stop, "typed \(unknown.env.typed.count) keys, not \(stop)")

    let over = UnlockRig(password: password)
    over.env.liveAt = { $0 == 1 + stop ? false : true }
    check(over.run() == .failed, "stopped by an attempt that was over before key \(stop)")
    check(over.env.typed.count == stop, "typed \(over.env.typed.count) keys, not \(stop)")
    over.checkNothingIsLeft("an attempt that was over before key \(stop)")
  }
}

func testAKeyThatCannotBePostedEndsTheAttemptAndNoKeyFollowsIt() {
  let password = Array("ab1".utf8)
  for failing in 0...3 {
    let rig = UnlockRig(password: password)
    rig.env.typesAt = { $0 != failing }
    check(rig.run() == .failed, "key \(failing)")
    check(rig.env.typed.count == failing + 1, "typed \(rig.env.typed.count) keys")
    rig.checkNothingIsLeft("a key that failed")
  }
}

func testTheScreenIsWaitedForAndTheAnswerIsWhatItProvesToBe() {
  let password = Array("ab".utf8)
  // Unlocked at the fourth look of the wait.
  let slow = UnlockRig(password: password)
  slow.env.lockAt = { $0 >= 3 + 2 + 2 ? .unlocked : .locked }
  check(slow.run() == .unlocked)
  check(slow.env.pauses == [0.1, 0.1, 0.1], "\(slow.env.pauses)")

  // Locked for good: rejected once ten seconds have passed, and not a second sooner.
  let rejected = UnlockRig(password: password)
  check(rejected.run() == .rejected)
  let waited = rejected.env.pauses.reduce(0, +)
  check(waited >= 9.9 && waited <= 10.3, "waited \(waited) seconds")
  check(rejected.env.typed.count == 3, "typed \(rejected.env.typed.count) keys")
  rejected.checkNothingIsLeft("a password that was rejected")

  // Another patience and another poll.
  let impatient = UnlockRig(password: password)
  check(impatient.run(timing: UnlockTiming(patience: 1, poll: 0.25)) == .rejected)
  check(impatient.env.pauses == [0.25, 0.25, 0.25, 0.25], "\(impatient.env.pauses)")

  // The attempt ends while it is waited for.
  let over = UnlockRig(password: password)
  over.env.liveAt = { $0 >= 1 + 3 ? false : true }
  check(over.run() == .failed)
  check(over.env.pauses.isEmpty, "waited for an attempt that is over")
  over.checkNothingIsLeft("an attempt that ended in the wait")

  // The screen is found unlocked at once, even for an attempt that is over.
  let done = UnlockRig(password: password)
  done.env.lockAt = unlockedAfterReturn(keys: 2)
  done.env.liveAt = { $0 >= 1 + 3 ? false : true }
  check(done.run() == .unlocked)
}

func testTheLastKeyIsReturnAndNothingIsTypedAfterIt() {
  let rig = UnlockRig(password: Array("ab".utf8))
  check(rig.run() == .rejected)
  check(rig.env.typed.last == UnlockKeys.returnKey, "\(rig.env.typed)")
  check(rig.env.typed.dropLast().allSatisfy { $0.scalar != nil }, "\(rig.env.typed)")
  check(rig.env.typed.count == 3)
  // The lines after the last key are looks at the screen and pauses, and nothing else.
  let lastType = rig.lines.lastIndex(of: "type") ?? 0
  check(
    rig.lines[(lastType + 1)...].allSatisfy { $0 == "lock?" || $0 == "pause" || $0 == "live" },
    "\(rig.lines[(lastType + 1)...])")
}

// MARK: - The calls to the library

func testTheLibraryIsAskedWhetherTheAttemptGoesOnAndAnythingButAYesIsANo() {
  let session = LibraryUnlockSession(link: 41)
  resetUnlockStubs()
  check(session.live(), "a yes")
  stubLock.lock()
  unlockAnswers.live = 0
  stubLock.unlock()
  check(!session.live(), "a no")
  stubLock.lock()
  unlockAnswers.live = 2
  stubLock.unlock()
  check(!session.live(), "a number that is not a yes")
  stubLock.lock()
  unlockAnswers.live = 1
  unlockAnswers.liveCode = Int32(BATON_E_UNKNOWN_LINK)
  stubLock.unlock()
  check(!session.live(), "a call that fails says that the attempt is over")
  stubLock.lock()
  let calls = unlockCalls
  stubLock.unlock()
  check(calls == [.live(41), .live(41), .live(41), .live(41)], "\(calls)")
  resetUnlockStubs()
}

func testTheNewBoxIsKeptOnlyWhenTheLibraryClearlySaysItIs() {
  let session = LibraryUnlockSession(link: 42)
  resetUnlockStubs()
  check(session.keep(boxed: [1, 2, 3], sealed: [4, 5]), "kept")
  stubLock.lock()
  let calls = unlockCalls
  stubLock.unlock()
  check(
    calls == [.keep(42, boxed: [1, 2, 3], sealed: [4, 5], size: UInt32(MemoryLayout<BatonUnlockBox>.size))],
    "\(calls)")

  for result in [
    BATON_KEEP_OVER, BATON_KEEP_TWICE, BATON_KEEP_INVALID, BATON_KEEP_CHANGED, BATON_KEEP_UNWRITABLE,
    BATON_KEEP_OTHER, 77,
  ] {
    stubLock.lock()
    unlockAnswers.keepResult = UInt32(result)
    stubLock.unlock()
    check(!session.keep(boxed: [1], sealed: [2]), "result \(result) was taken for a yes")
  }
  stubLock.lock()
  unlockAnswers.keepResult = UInt32(BATON_KEEP_KEPT)
  unlockAnswers.keepCode = Int32(BATON_E_UNKNOWN_LINK)
  stubLock.unlock()
  check(!session.keep(boxed: [1], sealed: [2]), "a call that fails kept something")
  stubLock.lock()
  unlockAnswers.keepCode = 0
  unlockAnswers.keepWritesResult = false
  stubLock.unlock()
  check(!session.keep(boxed: [1], sealed: [2]), "a library that said nothing kept something")
  resetUnlockStubs()
}

func testTheLibraryIsToldWhetherThePrivacyIsInPlace() {
  let session = LibraryUnlockSession(link: 43)
  resetUnlockStubs()
  session.privacy(inPlace: true)
  session.privacy(inPlace: false)
  stubLock.lock()
  let calls = unlockCalls
  stubLock.unlock()
  check(calls == [.place(43, 1), .place(43, 0)], "\(calls)")
  resetUnlockStubs()
}
