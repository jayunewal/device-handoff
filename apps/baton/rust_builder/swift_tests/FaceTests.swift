// SPDX-License-Identifier: Apache-2.0

// Tests of BatonFace.swift that are about data: the reading of what the Rust side passes, the
// numbers that go back to it, the sorting of what the system says, the prompt, and the wiping of
// the key of a box. The asker is tested in FaceAskerTests.swift and the calls of the table in
// FaceShimTests.swift.

import Foundation

// MARK: - What the Rust side passes

func testAFaceDeviceIsReadFromAnIdOfThirtyTwoBytesAndAName() {
  let arena = Arena()
  let device = FaceDevice(
    reading: arena.batonBytes(faceId(3)), name: arena.batonBytes(Array("Maya's Mac".utf8)))
  check(device?.id == faceId(3), "the id")
  check(device?.name == "Maya's Mac", "the name")
  let nameless = FaceDevice(reading: arena.batonBytes(faceId()), name: arena.batonBytes([]))
  check(nameless?.name == "", "a name may be empty")
  let longest = [UInt8](repeating: 0x61, count: FaceDevice.mostNameBytes)
  let long = FaceDevice(reading: arena.batonBytes(faceId()), name: arena.batonBytes(longest))
  check(long?.name.utf8.count == FaceDevice.mostNameBytes, "the longest name the protocol allows")
}

func testAFaceDeviceIsNotReadWhenTheIdOrTheNameIsWrong() {
  let arena = Arena()
  let name = arena.batonBytes(Array("Studio".utf8))
  for count in [0, 1, 31, 33, 64, 1024] {
    let id = arena.batonBytes([UInt8](repeating: 1, count: count))
    check(FaceDevice(reading: id, name: name) == nil, "an id of \(count) bytes")
  }
  var idWithNoPointer = arena.batonBytes(faceId())
  idWithNoPointer.ptr = nil
  check(FaceDevice(reading: idWithNoPointer, name: name) == nil, "an id with no pointer")
  var idBelowZero = arena.batonBytes(faceId())
  idBelowZero.len = -32
  check(FaceDevice(reading: idBelowZero, name: name) == nil, "an id with a length below zero")

  let id = arena.batonBytes(faceId())
  let tooLong = arena.batonBytes([UInt8](repeating: 0x61, count: FaceDevice.mostNameBytes + 1))
  check(FaceDevice(reading: id, name: tooLong) == nil, "a name that is too long")
  var nameWithNoPointer = name
  nameWithNoPointer.ptr = nil
  check(FaceDevice(reading: id, name: nameWithNoPointer) == nil, "a name with no pointer")
  var nameBelowZero = name
  nameBelowZero.len = -1
  check(FaceDevice(reading: id, name: nameBelowZero) == nil, "a name with a length below zero")
}

func testANameThatIsNotUtf8IsRepairedAndNothingOfADeviceIsPrinted() {
  let arena = Arena()
  let broken = FaceDevice(
    reading: arena.batonBytes(faceId()), name: arena.batonBytes([0x41, 0xFF, 0x42]))
  check(broken?.name == "A\u{FFFD}B", "a name that is not UTF-8 is shown, repaired")

  let device = FaceDevice(id: faceId(), name: "Maya-Secret-Name")
  check(!"\(device)".contains("Secret"), "the name is printed: \(device)")
  check(!String(describing: device).contains("Secret"))
  check(!String(reflecting: device).contains("Secret"))
  let question = FaceQuestion(
    number: 4242, sealed: [UInt8](repeating: 0xAB, count: 113), device: device)
  check(!"\(question)".contains("171"), "the sealed key is printed: \(question)")
  check(!"\(question)".contains("Secret"), "the name is printed: \(question)")
  let request = FaceKeyRequest(device: device, renew: true)
  check(!"\(request)".contains("Secret"), "the name is printed: \(request)")
}

func testARequestForAKeyIsReadWithRenewAsZeroOrOneAndNothingElse() {
  let arena = Arena()
  func read(_ source: BatonUnlockSource) -> FaceKeyRequest? {
    withUnsafePointer(to: source) { FaceKeyRequest(reading: $0) }
  }
  let device = FaceDevice(id: faceId(5), name: "Studio")
  let plain = FaceKeyRequest(device: device, renew: false)
  let renewing = FaceKeyRequest(device: device, renew: true)
  check(read(faceSource(arena, id: faceId(5), renew: 0)) == plain, "renew 0")
  check(read(faceSource(arena, id: faceId(5), renew: 1)) == renewing, "renew 1")
  // A key is replaced only when the Rust side clearly said so.
  for renew: UInt32 in [2, 3, 255, 0x8000_0000, UInt32.max] {
    check(read(faceSource(arena, renew: renew)) == nil, "renew \(renew)")
  }
  check(read(faceSource(arena, id: [UInt8](repeating: 1, count: 31))) == nil, "an id of 31 bytes")
  check(read(faceSource(arena, name: [UInt8](repeating: 0x61, count: 257))) == nil, "a long name")
}

func testARequestFromAStructThatIsTooShortIsNotRead() {
  let arena = Arena()
  let whole = MemoryLayout<BatonUnlockSource>.size
  // The memory is exactly as long as the struct says, so a read past it is found by the address
  // sanitizer.
  for size in [0, 4, 8, 24, whole - 1] {
    let short = arena.shortStruct(BatonUnlockSource.self, size: max(size, 4))
    short.pointee.size = UInt32(size)
    check(FaceKeyRequest(reading: short) == nil, "a struct of \(size) bytes")
  }
  let longer = arena.shortStruct(BatonUnlockSource.self, size: whole + 16)
  longer.pointee.size = UInt32(whole + 16)
  longer.pointee.id = arena.batonBytes(faceId())
  longer.pointee.name = arena.batonBytes(Array("Studio".utf8))
  check(
    FaceKeyRequest(reading: longer)
      == FaceKeyRequest(device: FaceDevice(id: faceId(), name: "Studio"), renew: false),
    "a struct that a later version made longer is read as far as this version knows it")
}

func testAQuestionIsReadAsACopyWithinTheSizesOfASealedKey() {
  // The memory is the Rust side's and is gone when the call returns: the question is a copy.
  func allocated(_ values: [UInt8]) -> UnsafeMutablePointer<UInt8> {
    let memory = UnsafeMutablePointer<UInt8>.allocate(capacity: max(values.count, 1))
    memory.initialize(from: values, count: values.count)
    return memory
  }
  let sealed = [UInt8](repeating: 0x33, count: 113)
  let sealedMemory = allocated(sealed)
  let idMemory = allocated(faceId(2))
  let nameMemory = allocated(Array("Studio".utf8))
  defer {
    sealedMemory.deallocate()
    idMemory.deallocate()
    nameMemory.deallocate()
  }
  let c = BatonUnlockQuestion(
    size: UInt32(MemoryLayout<BatonUnlockQuestion>.size), reserved: 0, number: 99,
    sealed: BatonBytes(ptr: sealedMemory, len: sealed.count),
    id: BatonBytes(ptr: idMemory, len: 32), name: BatonBytes(ptr: nameMemory, len: 6))
  let question = withUnsafePointer(to: c) { FaceQuestion(reading: $0) }
  sealedMemory.initialize(repeating: 0, count: sealed.count)
  idMemory.initialize(repeating: 0, count: 32)
  nameMemory.initialize(repeating: 0, count: 6)
  check(question?.number == 99)
  check(question?.sealed == sealed, "the sealed key was kept as a pointer and not copied")
  check(question?.device.id == faceId(2), "the id was kept as a pointer and not copied")
  check(question?.device.name == "Studio", "the name was kept as a pointer and not copied")

  let arena = Arena()
  let sizes = [
    FaceQuestion.leastSealedBytes, FaceQuestion.leastSealedBytes + 1, 113, 400,
    FaceQuestion.mostSealedBytes,
  ]
  for count in sizes {
    let made = faceQuestionStruct(arena, sealed: [UInt8](repeating: 1, count: count))
    let read = withUnsafePointer(to: made) { FaceQuestion(reading: $0) }
    check(read?.sealed.count == count, "a sealed key of \(count) bytes")
  }
  check(FaceQuestion.leastSealedBytes == 81 && FaceQuestion.mostSealedBytes == 1024)
}

func testAQuestionThatIsNotWellFormedIsNotRead() {
  let arena = Arena()
  func read(_ question: BatonUnlockQuestion) -> FaceQuestion? {
    withUnsafePointer(to: question) { FaceQuestion(reading: $0) }
  }
  check(read(faceQuestionStruct(arena)) != nil, "the question the others are made from")
  // A sealed key is a point of the curve, a tag, and the secret, so a shorter one cannot be opened;
  // a longer one than the interface allows is not one that the Rust side checked.
  for count in [0, 1, 32, 80, 1025, 4096] {
    let bad = faceQuestionStruct(arena, sealed: [UInt8](repeating: 1, count: count))
    check(read(bad) == nil, "a sealed key of \(count) bytes")
  }
  var noPointer = faceQuestionStruct(arena)
  noPointer.sealed.ptr = nil
  check(read(noPointer) == nil, "a sealed key with a length and no pointer")
  var belowZero = faceQuestionStruct(arena)
  belowZero.sealed.len = -113
  check(read(belowZero) == nil, "a sealed key with a length below zero")
  check(read(faceQuestionStruct(arena, id: faceId(1) + [0])) == nil, "an id of 33 bytes")
  check(read(faceQuestionStruct(arena, id: [])) == nil, "no id")
  var idWithNoPointer = faceQuestionStruct(arena)
  idWithNoPointer.id.ptr = nil
  check(read(idWithNoPointer) == nil, "an id with a length and no pointer")
  check(
    read(faceQuestionStruct(arena, name: [UInt8](repeating: 0x61, count: 257))) == nil,
    "a name that is too long")
  var nameWithNoPointer = faceQuestionStruct(arena)
  nameWithNoPointer.name.ptr = nil
  check(read(nameWithNoPointer) == nil, "a name with a length and no pointer")

  let whole = MemoryLayout<BatonUnlockQuestion>.size
  for size in [0, 4, 8, 16, 48, whole - 1] {
    let short = arena.shortStruct(BatonUnlockQuestion.self, size: max(size, 4))
    short.pointee.size = UInt32(size)
    check(FaceQuestion(reading: short) == nil, "a struct of \(size) bytes")
  }
}

// MARK: - What the Rust side is given

func testAPublicKeyIsWrittenIntoTheRoomAndNothingElseIsTouched() {
  let room = FaceKeyRoom()
  room.key.reserved = 77
  let point = facePoint(9)
  check(FacePublicKey(point: point).write(to: &room.key), "the key was written")
  check(room.key.len == 65, "len \(room.key.len)")
  check(room.key.sealing == BATON_SEALING_ECIES_P256, "sealing \(room.key.sealing)")
  check(room.written == point, "the bytes")
  check(
    Array(room.contents.dropFirst(65)) == [UInt8](repeating: 0xEE, count: 256 - 65),
    "the room after the key was touched")
  check(room.key.reserved == 77, "reserved is written by nobody")
  check(room.key.size == UInt32(MemoryLayout<BatonUnlockKey>.size))

  // The most that the room holds, and a number of sealing that is not ours is carried as it is.
  let most = FacePublicKey(
    sealing: 5, bytes: [UInt8](repeating: 1, count: Int(BATON_UNLOCK_KEY_MAX)))
  let full = FaceKeyRoom()
  check(most.write(to: &full.key), "a key as long as the room")
  check(full.key.len == BATON_UNLOCK_KEY_MAX && full.key.sealing == 5)
  check(full.written == most.bytes)
}

func testAPublicKeyThatDoesNotFitIsNotWrittenAtAll() {
  // The Rust side reads what an out argument holds only when the call succeeds, so a refusal
  // leaves it as it was.
  let room = FaceKeyRoom()
  let before = room.contents
  let tooLong = FacePublicKey(point: [UInt8](repeating: 4, count: Int(BATON_UNLOCK_KEY_MAX) + 1))
  check(!tooLong.write(to: &room.key), "a key longer than the room")
  check(room.contents == before && room.key.len == 0 && room.key.sealing == 0)
  let empty = FacePublicKey(point: [])
  check(!empty.write(to: &room.key), "an empty key")
  check(room.contents == before && room.key.len == 0 && room.key.sealing == 0)

  var noRoom = BatonUnlockKey(
    size: UInt32(MemoryLayout<BatonUnlockKey>.size), sealing: 0, len: 0, reserved: 0, key: nil)
  check(!FacePublicKey(point: facePoint()).write(to: &noRoom), "a struct with no room")
  check(noRoom.len == 0 && noRoom.sealing == 0)

  let arena = Arena()
  let whole = MemoryLayout<BatonUnlockKey>.size
  for size in [4, 8, 16, whole - 1] {
    let short = arena.shortStruct(BatonUnlockKey.self, size: size)
    check(!FacePublicKey(point: facePoint()).write(to: short), "a struct of \(size) bytes")
  }
}

func testOnlySixtyFiveBytesThatBeginWithFourCanBeAPointOfP256() {
  check(FacePublicKey.isPointOfP256(facePoint()))
  check(FacePublicKey.pointBytes == 65)
  check(!FacePublicKey.isPointOfP256([]))
  check(!FacePublicKey.isPointOfP256(Array(facePoint().dropLast())), "64 bytes")
  check(!FacePublicKey.isPointOfP256(facePoint() + [0]), "66 bytes")
  var compressed = [UInt8](repeating: 7, count: 33)
  compressed[0] = 0x02
  check(!FacePublicKey.isPointOfP256(compressed), "a compressed point")
  var wrongFirst = facePoint()
  for first: UInt8 in [0x00, 0x01, 0x02, 0x03, 0x05, 0xFF] {
    wrongFirst[0] = first
    check(!FacePublicKey.isPointOfP256(wrongFirst), "a first byte of \(first)")
  }
  check(FacePublicKey(point: facePoint()).sealing == BATON_SEALING_ECIES_P256)
}

func testEachFailureToMakeAKeyHasItsNumberAndASentence() {
  let numbered: [(FaceKeyFailure, Int32)] = [
    (.refused("a"), Int32(BATON_KEY_REFUSED)),
    (.blocked("b"), Int32(BATON_KEY_BLOCKED)),
    (.unsupported("c"), Int32(BATON_KEY_UNSUPPORTED)),
    (.failed("d"), Int32(BATON_E_PLATFORM)),
  ]
  for (failure, code) in numbered {
    check(failure.code == code, "\(failure)")
    check(failure.code != Int32(BATON_OK), "a failure that reads as success: \(failure)")
  }
  check(numbered.map { $0.0.sentence } == ["a", "b", "c", "d"])
  check(Set(numbered.map { $0.1 }).count == 4, "four failures, four numbers")
  check(Int32(BATON_KEY_REFUSED) == 1 && Int32(BATON_KEY_BLOCKED) == 2)
  check(Int32(BATON_KEY_UNSUPPORTED) == 3)

  // The ones that are made ready: what they are, and a sentence that fits the room of the Rust side
  // and reads as a sentence.
  let unsupported = Int32(BATON_KEY_UNSUPPORTED)
  let platform = Int32(BATON_E_PLATFORM)
  let ready: [(FaceKeyFailure, Int32)] = [
    (.noEnclave, unsupported), (.noPasscode, unsupported), (.noFace, unsupported),
    (.faceUnavailable, unsupported), (.faceLocked, platform), (.deviceLocked, platform),
    (.cannotMake, platform), (.cannotOpen, unsupported), (.notAPoint, platform),
  ]
  for (failure, code) in ready {
    check(failure.code == code, "\(failure.sentence)")
    let sentence = failure.sentence
    check(sentence.utf8.count > 20 && sentence.utf8.count <= Int(BATON_MESSAGE_MAX), sentence)
    check(sentence.hasSuffix("."), sentence)
    check(!sentence.contains("\u{2014}") && !sentence.contains("\n"), sentence)
  }
  check(Set(ready.map { $0.0.sentence }).count == ready.count, "two failures with one sentence")
}

func testEachReasonToDeclineHasItsNumberOfTheInterface() {
  check(FaceDecline.declined.reason == BATON_FACE_DECLINED)
  check(FaceDecline.keyLost.reason == BATON_FACE_KEY_LOST)
  check(FaceDecline.failed.reason == BATON_FACE_FAILED)
  check(Set([FaceDecline.declined, .keyLost, .failed].map { $0.reason }).count == 3)
  check(BATON_FACE_DECLINED == 0 && BATON_FACE_KEY_LOST == 1 && BATON_FACE_FAILED == 2)
}

// MARK: - What the system says when a key cannot be used

private func laError(_ code: Int) -> FaceSystemError {
  FaceSystemError(domain: FaceSystemCodes.localAuthentication, code: code)
}

private func statusError(_ code: Int) -> FaceSystemError {
  FaceSystemError(domain: FaceSystemCodes.osStatus, code: code)
}

func testTheCodesOfTheSystemAreTheNumbersOfItsHeaders() {
  // A test on a Mac holds these to the constants of the SDK (FaceCodesTests.swift); this one holds
  // them to what was written down, so that an edit that slips is found on every computer.
  check(FaceSystemCodes.localAuthentication == "com.apple.LocalAuthentication")
  check(FaceSystemCodes.osStatus == "NSOSStatusErrorDomain")
  check(FaceSystemCodes.authenticationFailed == -1 && FaceSystemCodes.userCancel == -2)
  check(FaceSystemCodes.userFallback == -3 && FaceSystemCodes.systemCancel == -4)
  check(FaceSystemCodes.passcodeNotSet == -5 && FaceSystemCodes.biometryNotAvailable == -6)
  check(FaceSystemCodes.biometryNotEnrolled == -7 && FaceSystemCodes.biometryLockout == -8)
  check(FaceSystemCodes.appCancel == -9 && FaceSystemCodes.invalidContext == -10)
  check(FaceSystemCodes.notInteractive == -1004)
  check(FaceSystemCodes.unimplemented == -4 && FaceSystemCodes.param == -50)
  check(FaceSystemCodes.userCanceled == -128 && FaceSystemCodes.authFailed == -25293)
  check(FaceSystemCodes.itemNotFound == -25300 && FaceSystemCodes.interactionNotAllowed == -25308)
}

func testWhatTheSystemSaysIsSortedIntoTheThreeReasonsToDecline() {
  let table: [([FaceSystemError], FaceDecline)] = [
    // The face was not recognised, or the person or the system or the app took the prompt down.
    ([laError(-1)], .declined), ([laError(-2)], .declined), ([laError(-3)], .declined),
    ([laError(-4)], .declined), ([laError(-8)], .declined), ([laError(-9)], .declined),
    // No passcode and no face: the key that needed them is gone with them.
    ([laError(-5)], .keyLost), ([laError(-7)], .keyLost),
    // The rest of LocalAuthentication is trouble of this device and not a no.
    ([laError(-6)], .failed), ([laError(-10)], .failed), ([laError(-1004)], .failed),
    ([laError(0)], .failed), ([laError(1)], .failed), ([laError(-11)], .failed),
    // The keychain says the same in its own numbers.
    ([statusError(-128)], .declined), ([statusError(-25293)], .declined),
    ([statusError(-25300)], .keyLost),
    ([statusError(-25308)], .failed), ([statusError(-50)], .failed), ([statusError(-4)], .failed),
    ([statusError(0)], .failed), ([statusError(-67673)], .failed),
    // A status with LocalAuthentication under it: the more specific one decides.
    ([statusError(-25293), laError(-2)], .declined),
    ([statusError(-25300), laError(-8)], .declined),
    ([statusError(-25293), laError(-7)], .keyLost),
    ([statusError(-25308), laError(-6)], .failed),
    ([laError(-2), statusError(-25300)], .declined),
    // A status that says nothing, over one that does.
    ([statusError(-25308), statusError(-128)], .declined),
    ([statusError(-50), statusError(-25300)], .keyLost),
    // Nothing known.
    ([], .failed),
  ]
  for (errors, expected) in table {
    let got = FaceDecline(system: errors)
    check(got == expected, "\(FaceSystemError.line(errors)): \(got), not \(expected)")
  }
}

func testACodeOfAnotherDomainIsNeverTakenForOneOfTheSystem() {
  let other = "com.example.Other"
  for code in [-1, -2, -3, -4, -5, -7, -8, -9, -128, -25293, -25300] {
    let errors = [FaceSystemError(domain: other, code: code)]
    check(FaceDecline(system: errors) == .failed, "\(code) of another domain")
  }
  // A number of LocalAuthentication in the domain of the keychain, and the other way round.
  check(FaceDecline(system: [statusError(-2)]) == .failed, "-2 in the keychain is no cancel")
  check(FaceDecline(system: [statusError(-9)]) == .failed)
  check(FaceDecline(system: [statusError(-7)]) == .failed)
  check(FaceDecline(system: [laError(-25300)]) == .failed, "-25300 in LocalAuthentication")
  check(FaceDecline(system: [laError(-128)]) == .failed)
  check(FaceDecline(system: [laError(-25293)]) == .failed)
}

func testTheChainOfAnErrorIsReadAsDomainsAndCodesAndNoFurtherThanEight() {
  let inner = NSError(
    domain: FaceSystemCodes.localAuthentication, code: -2,
    userInfo: [NSLocalizedDescriptionKey: "Secret person"])
  let outer = NSError(
    domain: FaceSystemCodes.osStatus, code: -25293, userInfo: [NSUnderlyingErrorKey: inner])
  let chain = FaceSystemError.chain(of: outer)
  check(chain == [statusError(-25293), laError(-2)], "\(chain)")
  let line = FaceSystemError.line(chain)
  check(line == "NSOSStatusErrorDomain -25293, under com.apple.LocalAuthentication -2", line)
  check(!line.contains("Secret"), "a line for the log holds only domains and codes")
  check(FaceDecline(system: chain) == .declined)

  check(FaceSystemError.line([]) == "")
  check(FaceSystemError.chain(of: inner) == [laError(-2)])

  // Only so many are looked at, outermost first.
  var deep = NSError(domain: "deep", code: 0, userInfo: nil)
  for level in 1..<20 {
    deep = NSError(domain: "deep", code: level, userInfo: [NSUnderlyingErrorKey: deep])
  }
  let codes = FaceSystemError.chain(of: deep).map { $0.code }
  check(codes == [19, 18, 17, 16, 15, 14, 13, 12], "\(codes)")
  check(FaceSystemError.deepest == 8)

  // An underlying value that is no error is not followed.
  let odd = NSError(
    domain: "odd", code: 1, userInfo: [NSUnderlyingErrorKey: "not an error"])
  check(FaceSystemError.chain(of: odd) == [FaceSystemError(domain: "odd", code: 1)])

  enum Oops: Error { case bad }
  let swift = FaceSystemError.chain(of: Oops.bad)
  check(swift.count == 1, "\(swift)")
  check(swift.first?.domain != FaceSystemCodes.localAuthentication)
  check(swift.first?.domain != FaceSystemCodes.osStatus)
  check(FaceDecline(system: swift) == .failed)
}

func testAFaceThatCannotBeAskedForIsToldByWhatLocalAuthenticationSays() {
  check(FaceKeyFailure(biometry: nil) == .faceUnavailable, "no reason")
  check(FaceKeyFailure(biometry: laError(-5)) == .noPasscode)
  check(FaceKeyFailure(biometry: laError(-7)) == .noFace)
  check(FaceKeyFailure(biometry: laError(-6)) == .faceUnavailable)
  check(FaceKeyFailure(biometry: laError(-8)) == .faceLocked)
  check(FaceKeyFailure(biometry: laError(-1)) == .cannotMake)
  check(FaceKeyFailure(biometry: laError(12)) == .cannotMake)
  check(
    FaceKeyFailure(biometry: FaceSystemError(domain: "com.example.Other", code: -5))
      == .faceUnavailable, "a code of another domain is not -5 of LocalAuthentication")
  // What the person is told is something they can do something about, or that they cannot.
  check(FaceKeyFailure.noPasscode.sentence.contains("passcode"))
  check(FaceKeyFailure.noFace.sentence.contains("Face ID"))
  check(FaceKeyFailure.faceLocked.sentence.contains("passcode"))
}

func testAStepOfMakingTheKeyThatDidNotWorkIsToldByWhatTheKeychainSaid() {
  check(FaceKeyFailure(step: "make the key", system: [statusError(-25308)]) == .deviceLocked)
  check(FaceKeyFailure(step: "make the key", system: [statusError(-4)]) == .noEnclave)
  check(FaceKeyFailure(step: "make the key", system: [statusError(-50)]) == .cannotMake)
  check(FaceKeyFailure(step: "make the key", system: [statusError(0)]) == .cannotMake)
  check(FaceKeyFailure(step: "make the key", system: []) == .cannotMake)
  check(
    FaceKeyFailure(step: "make the key", system: [statusError(-25308), statusError(-4)])
      == .deviceLocked)
  // LocalAuthentication is more specific than the status that wraps it.
  check(
    FaceKeyFailure(step: "make the key", system: [statusError(-50), laError(-7)]) == .noFace)
  check(
    FaceKeyFailure(step: "make the key", system: [statusError(-25308), laError(-5)])
      == .noPasscode)
}

// MARK: - Names

func testTheTagOfAKeyIsMadeOfTheIdAndNothingElse() {
  check(FaceTag.prefix == "io.github.jayunewal.baton.unlock.")
  let id = (0..<32).map { UInt8($0 * 8 + 1) }
  let tag = FaceTag.make(for: id)
  check(tag?.hasPrefix(FaceTag.prefix) == true)
  check(tag?.count == FaceTag.prefix.count + 64, "\(tag?.count ?? 0)")
  // The digits are the id in hex, in lower case, as the formatting of the system writes it.
  let expected = id.map { String(format: "%02x", Int($0)) }.joined()
  check(tag == FaceTag.prefix + expected, "\(tag ?? "no tag")")
  let zeros = [UInt8](repeating: 0, count: 32)
  check(FaceTag.make(for: zeros) == FaceTag.prefix + String(repeating: "0", count: 64))
  let ones = [UInt8](repeating: 0xFF, count: 32)
  check(FaceTag.make(for: ones) == FaceTag.prefix + String(repeating: "f", count: 64))
  let mixed = [UInt8](repeating: 0xA5, count: 32)
  check(FaceTag.make(for: mixed) == FaceTag.prefix + String(repeating: "a5", count: 32))
  check(FaceTag.make(for: faceId(1)) == FaceTag.make(for: faceId(1)), "the same id, the same tag")
  var tags = Set<String>()
  for seed in 0..<50 { tags.insert(FaceTag.make(for: faceId(UInt8(seed))) ?? "") }
  check(tags.count == 50 && !tags.contains(""), "two devices, one tag")
  // One byte makes the tag another: a pair is found by the device it was made for.
  var changed = faceId(1)
  changed[31] ^= 1
  check(FaceTag.make(for: changed) != FaceTag.make(for: faceId(1)))
  for count in [0, 1, 16, 31, 33, 64] {
    check(FaceTag.make(for: [UInt8](repeating: 1, count: count)) == nil, "\(count) bytes")
  }
}

func testThePromptNamesTheDeviceAndNothingCanChangeWhatItSays() {
  check(FacePrompt.reason(for: "Maya's MacBook Pro") == "Unlock Maya's MacBook Pro")
  check(FacePrompt.reason(for: "") == "Unlock your Mac")
  check(FacePrompt.reason(for: "   \n\t ") == "Unlock your Mac")
  check(FacePrompt.reason(for: "  Studio   Mac  ") == "Unlock Studio Mac", "one space")
  // Whatever breaks a line or changes the way text runs is taken out, so that a name cannot make
  // the prompt say two things or run backwards.
  let marks: [(String, String)] = [
    ("Studio\nMac", "Unlock Studio Mac"),
    ("Studio\r\nMac", "Unlock Studio Mac"),
    ("Studio\u{2028}Mac", "Unlock Studio Mac"),
    ("Studio\u{2029}Mac", "Unlock Studio Mac"),
    ("Studio\u{0007}Mac", "Unlock Studio Mac"),
    ("Studio\u{0000}Mac", "Unlock Studio Mac"),
    ("Studio\u{0085}Mac", "Unlock Studio Mac"),
    ("A\u{202E}B", "Unlock A B"),
    ("A\u{202D}B\u{202C}C", "Unlock A B C"),
    ("A\u{202A}B\u{202B}C", "Unlock A B C"),
    ("A\u{061C}B", "Unlock A B"),
    ("\u{200E}Studio\u{200F}", "Unlock Studio"),
    ("\u{2066}x\u{2067}y\u{2068}z\u{2069}", "Unlock x y z"),
    ("\u{202E}", "Unlock your Mac"),
    ("\n\n", "Unlock your Mac"),
  ]
  for (name, expected) in marks {
    let got = FacePrompt.reason(for: name)
    check(got == expected, "\(name.unicodeScalars.map { $0.value }): \(got)")
    check(!got.contains("\n") && !got.contains("\r"), "a prompt of two lines")
  }
  // A name that is not in the Latin alphabet, and an emoji that is made of several scalars, stay
  // as they are.
  let family = "Home \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"
  check(FacePrompt.reason(for: family) == "Unlock " + family)
  let office = "Büro \u{0645}\u{0643}\u{062A}\u{0628}"
  check(FacePrompt.reason(for: office) == "Unlock " + office)
  check(FacePrompt.unnamed == "your Mac")
}

func testALongNameIsCutInThePromptBetweenTwoCharacters() {
  let most = FacePrompt.mostCharacters
  check(most == 48)
  let exact = String(repeating: "a", count: most)
  check(FacePrompt.shown(exact) == exact, "a name of the most characters is not cut")
  let one = String(repeating: "a", count: most + 1)
  check(FacePrompt.shown(one) == exact + "\u{2026}", "one more is cut")
  let hundred = String(repeating: "a", count: 100)
  check(FacePrompt.reason(for: hundred) == "Unlock " + exact + "\u{2026}")
  check(FacePrompt.shown(hundred).count == most + 1)
  // A character that is made of several scalars is cut whole or not at all.
  let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"
  let long = String(repeating: "a", count: most - 1) + family + "bbb"
  let cut = FacePrompt.shown(long)
  check(cut == String(repeating: "a", count: most - 1) + family + "\u{2026}", cut)
  let acute = String(repeating: "e\u{0301}", count: 60)
  let cutAccents = FacePrompt.shown(acute)
  let wholeAccents = String(repeating: "e\u{0301}", count: most) + "\u{2026}"
  check(cutAccents == wholeAccents, "an accent was cut off")
  // The white space is collapsed before the name is measured.
  check(FacePrompt.shown(exact + "    ") == exact)
}

// MARK: - A secret that is wiped

func testAKeyOfABoxIsWipedWhenAskedAndWhenLetGoOf() {
  // The memory is the test's, so that it can be looked at after the wipe and, from the function
  // that releases it, just before it is freed.
  let size = Int(BATON_BOX_KEY_BYTES)
  let secret = [UInt8](repeating: 0xAB, count: size)
  let zeros = [UInt8](repeating: 0, count: size)

  func make(
    seeing release: @escaping ([UInt8]) -> Void
  ) -> (FaceBoxKey, UnsafeMutableRawBufferPointer) {
    let memory = UnsafeMutableRawBufferPointer.allocate(byteCount: size, alignment: 1)
    memory.initializeMemory(as: UInt8.self, repeating: 0xAB)
    let key = FaceBoxKey(
      adopting: memory,
      release: {
        release(Array($0))
        $0.deallocate()
      })
    return (key, memory)
  }

  // Wiped when asked: zero at once, and zero again when the memory is freed.
  let seenAfterWipe = FaceShared<[UInt8]?>(nil)
  do {
    let (key, memory) = make { seenAfterWipe.set($0) }
    // The key stays until the end of the block, since the memory is freed with it.
    withExtendedLifetime(key) {
      check(key.count == size && !key.isWiped)
      check(faceBytes(key) == secret, "the key holds what it was given")
      key.wipe()
      check(Array(memory) == zeros, "wipe left the key in memory")
      check(key.isWiped, "isWiped")
      check(faceBytes(key) == nil, "a wiped key gave its bytes")
      check(key.count == size, "count stays what it was")
      key.wipe()
      check(Array(memory) == zeros, "a second wipe")
    }
  }
  check(seenAfterWipe.get() == zeros, "the memory was not zero when it was freed")

  // Let go of without a wipe: wiped by the end of its life, before the memory is freed.
  let seenAtEnd = FaceShared<[UInt8]?>(nil)
  do {
    let (key, _) = make { seenAtEnd.set($0) }
    check(faceBytes(key) == secret)
  }
  check(seenAtEnd.get() == zeros, "a key that was let go of was not wiped before it was freed")
}

func testAKeyOfABoxIsCopiedOnlyWhenItHasBytesAndNotTooMany() {
  func copy(_ count: Int, _ byte: UInt8 = 9) -> FaceBoxKey? {
    [UInt8](repeating: byte, count: count).withUnsafeBytes { FaceBoxKey(copying: $0) }
  }
  check(copy(0) == nil, "a key of no bytes")
  check(copy(FaceBoxKey.mostBytes + 1) == nil, "a key of too many bytes")
  check(copy(1)?.count == 1)
  check(copy(32)?.count == 32)
  check(copy(FaceBoxKey.mostBytes)?.count == FaceBoxKey.mostBytes)
  let key = copy(32, 0x42)
  check(key.flatMap { faceBytes($0) } == [UInt8](repeating: 0x42, count: 32))
  // The copy is the key's own: wiping it does not touch the memory it was made from.
  let original = [UInt8](repeating: 0x42, count: 32)
  original.withUnsafeBytes { raw in
    let own = FaceBoxKey(copying: raw)
    own?.wipe()
    check(Array(raw) == original, "the memory that was copied was wiped")
  }
  check(FaceBoxKey.mostBytes == 256)
}

func testAKeyOfABoxIsPrintedAsItsSizeAndNothingOfItsBytes() {
  let key = faceKey(0xAB)
  check("\(key)" == "a key of 32 bytes", "\(key)")
  check(String(describing: key) == "a key of 32 bytes")
  check(String(reflecting: key) == "a key of 32 bytes")
  check(!"\(key)".contains("171") && !"\(key)".lowercased().contains("ab"))
  key.wipe()
  check("\(key)" == "a key of 32 bytes", "a wiped key says the same")
}

func testAKeyOfABoxCanBeReadAndWipedFromManyThreads() {
  let key = faceKey(0xAB)
  let secret = [UInt8](repeating: 0xAB, count: 32)
  let bad = FaceShared(0)
  let group = DispatchGroup()
  for _ in 0..<4 {
    group.enter()
    Thread.detachNewThread {
      for _ in 0..<3000 {
        // The bytes are read with the lock held, so a wipe never comes in the middle of a read.
        if let seen = faceBytes(key), seen != secret { bad.update { $0 += 1 } }
      }
      group.leave()
    }
  }
  Thread.sleep(forTimeInterval: 0.005)
  key.wipe()
  check(group.wait(timeout: .now() + 30) == .success, "the threads finished")
  check(bad.get() == 0, "a read saw a key that was half wiped")
  check(key.isWiped && faceBytes(key) == nil)
}
