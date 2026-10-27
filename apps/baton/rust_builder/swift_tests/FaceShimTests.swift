// SPDX-License-Identifier: Apache-2.0

// Tests of the three entries of the table that are about the face (unlock_key, unlock_open and
// unlock_cancel), of the screen that holds the asker, and of the registration. They play the Rust
// side: the functions of the registered table are called with the C structs that it builds, and
// with the ones that it should never build.

import Foundation

private let badArgument = Int32(BATON_E_ARGUMENT)
private let platformFailure = Int32(BATON_E_PLATFORM)

// MARK: - The table

func testAPlatformThatCanUnlockGivesTheThreeEntriesAndOneThatCannotGivesNone() {
  guard let table = registered(FacePlatform()) else { return check(false, "no table") }
  check(table.unlock_key != nil, "unlock_key")
  check(table.unlock_open != nil, "unlock_open")
  check(table.unlock_cancel != nil, "unlock_cancel")
  check(table.size == UInt32(MemoryLayout<BatonViewerVTable>.size), "size \(table.size)")
  check(table.size == 128)
  check(MemoryLayout<BatonViewerVTable>.offset(of: \.unlock_key) == 104)
  check(MemoryLayout<BatonViewerVTable>.offset(of: \.unlock_open) == 112)
  check(MemoryLayout<BatonViewerVTable>.offset(of: \.unlock_cancel) == 120)
  check(table.sound == nil && table.play == nil && table.stopped == nil, "the unlock gave sound")

  // A table that gives some of the three reads as none, so a platform gives all or none.
  guard let cannot = registered(FacePlatform(canUnlock: false)) else {
    return check(false, "no table")
  }
  check(cannot.unlock_key == nil, "unlock_key of a platform that cannot")
  check(cannot.unlock_open == nil, "unlock_open of a platform that cannot")
  check(cannot.unlock_cancel == nil, "unlock_cancel of a platform that cannot")
  check(cannot.size == 128, "the table has its whole size even when the entries are empty")
  check(cannot.limits != nil && cannot.open != nil && cannot.release != nil)

  // A platform that says nothing about it, like every platform before this one, cannot.
  guard let plain = registered(FakePlatform()) else { return check(false, "no table") }
  check(plain.unlock_key == nil && plain.unlock_open == nil && plain.unlock_cancel == nil)
}

func testAPlatformThatPlaysSoundAndCanUnlockGivesAllSixEntries() {
  let platform = FacePlatform()
  platform.playsSound = true
  guard let table = registered(platform) else { return check(false, "no table") }
  check(table.sound != nil && table.play != nil && table.stopped != nil)
  check(table.unlock_key != nil && table.unlock_open != nil && table.unlock_cancel != nil)
}

func testAPlatformAndAScreenThatSayNothingOfTheFaceCannotUnlock() {
  final class Plain: ViewerPlatform {
    let limits = ViewerLimits(maxWidth: 1, maxHeight: 1, maxFps: 1, codecs: 1)
    func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen {
      throw ScreenFailure(sentence: "No.")
    }
  }
  let plain = Plain()
  check(!plain.canUnlock, "a platform that says nothing can unlock")
  let request = FaceKeyRequest(device: FaceDevice(id: faceId(), name: "Studio"), renew: false)
  do {
    _ = try plain.unlockKey(request)
    check(false, "a key was given by a platform that cannot")
  } catch {
    check(error as? FaceKeyFailure == .noEnclave, "\(error)")
  }
  // A screen that cannot ask for a face takes no question and takes the cancel as nothing.
  let screen = FakeScreen()
  check(!screen.unlockOpen(faceQuestion()), "a screen with no asker took a question")
  screen.unlockCancel()
}

// MARK: - unlock_key

private func callKey(
  _ table: BatonViewerVTable, context: UnsafeMutableRawPointer?,
  source: UnsafePointer<BatonUnlockSource>?, key: UnsafeMutablePointer<BatonUnlockKey>?,
  message: UnsafeMutablePointer<BatonMessage>?
) -> Int32 {
  table.unlock_key?(context, source, key, message) ?? 99
}

func testUnlockKeyWritesTheKeyAndPassesTheRequestToThePlatform() {
  let platform = FacePlatform(outcome: .success(FacePublicKey(point: facePoint(8))))
  guard let table = registered(platform) else { return check(false, "no table") }
  let arena = Arena()
  let room = FaceKeyRoom()
  let message = FaceMessageRoom()

  var plain = faceSource(arena, id: faceId(6), name: Array("Maya's Mac".utf8), renew: 0)
  var code = callKey(
    table, context: table.context, source: &plain, key: &room.key, message: &message.message)
  check(code == 0, "code \(code)")
  check(room.key.len == 65 && room.key.sealing == BATON_SEALING_ECIES_P256, "\(room.key)")
  check(room.written == facePoint(8), "the key")
  check(message.sentence == "", "a sentence was written for a success: \(message.sentence)")

  // renew goes through as it was said, and only 1 is a yes.
  var renew = faceSource(arena, id: faceId(6), name: Array("Maya's Mac".utf8), renew: 1)
  code = callKey(
    table, context: table.context, source: &renew, key: &room.key, message: &message.message)
  check(code == 0, "code \(code)")
  let device = FaceDevice(id: faceId(6), name: "Maya's Mac")
  check(
    platform.requests == [
      FaceKeyRequest(device: device, renew: false), FaceKeyRequest(device: device, renew: true),
    ], "\(platform.requests)")
}

func testUnlockKeyMapsEachFailureToItsNumberAndWritesItsSentence() {
  struct Odd: Error {}
  let refused = FaceKeyFailure.refused("The person said no.")
  let blocked = FaceKeyFailure.blocked("The settings do not allow it.")
  let unsupported = FaceKeyFailure.unsupported("No key can be kept.")
  let failed = FaceKeyFailure.failed("It went wrong.")
  let failures: [(Error, Int32, String)] = [
    (refused, Int32(BATON_KEY_REFUSED), refused.sentence),
    (blocked, Int32(BATON_KEY_BLOCKED), blocked.sentence),
    (unsupported, Int32(BATON_KEY_UNSUPPORTED), unsupported.sentence),
    (failed, platformFailure, failed.sentence),
    (FaceKeyFailure.noEnclave, Int32(BATON_KEY_UNSUPPORTED), FaceKeyFailure.noEnclave.sentence),
    (FaceKeyFailure.noFace, Int32(BATON_KEY_UNSUPPORTED), FaceKeyFailure.noFace.sentence),
    (FaceKeyFailure.deviceLocked, platformFailure, FaceKeyFailure.deviceLocked.sentence),
    (Odd(), platformFailure, "The key for the unlock could not be made."),
  ]
  for (error, expected, sentence) in failures {
    let platform = FacePlatform(outcome: .failure(error))
    guard let table = registered(platform) else { return check(false, "no table") }
    let arena = Arena()
    let room = FaceKeyRoom()
    let message = FaceMessageRoom()
    let before = room.contents
    var source = faceSource(arena)
    let code = callKey(
      table, context: table.context, source: &source, key: &room.key, message: &message.message)
    check(code == expected, "\(error): code \(code), not \(expected)")
    check(message.sentence == sentence, "\(error): \(message.sentence)")
    check(room.contents == before && room.key.len == 0 && room.key.sealing == 0, "a failed key")
  }
}

func testASentenceOfAFailedKeyIsCutBetweenTwoCharactersWhereItDoesNotFit() {
  let sentence = "Ça ne marche pas: étonnant"
  let platform = FacePlatform(outcome: .failure(FaceKeyFailure.failed(sentence)))
  guard let table = registered(platform) else { return check(false, "no table") }
  let arena = Arena()
  var source = faceSource(arena)
  let room = FaceKeyRoom()
  for size in 0...30 {
    let message = FaceMessageRoom(room: size)
    let code = callKey(
      table, context: table.context, source: &source, key: &room.key, message: &message.message)
    check(code == platformFailure)
    check(message.sentence.utf8.count <= size, "room \(size): \(message.sentence)")
    check(sentence.hasPrefix(message.sentence), "room \(size): \(message.sentence)")
    check(!message.sentence.contains("\u{FFFD}"), "room \(size): a character was cut in two")
  }
  // No room for a sentence at all, or a struct that is too short to hold one: the number is the
  // answer and nothing is written.
  let noMessage = callKey(
    table, context: table.context, source: &source, key: &room.key, message: nil)
  check(noMessage == platformFailure, "no message: code \(noMessage)")
  let short = arena.shortStruct(BatonMessage.self, size: 8)
  let shortMessage = callKey(
    table, context: table.context, source: &source, key: &room.key, message: short)
  check(shortMessage == platformFailure, "a message of 8 bytes: code \(shortMessage)")
}

func testUnlockKeyNeverTrapsOnWhatItIsGivenAndNeverAsksThePlatformForIt() {
  let platform = FacePlatform()
  guard let table = registered(platform) else { return check(false, "no table") }
  let arena = Arena()
  let room = FaceKeyRoom()
  let message = FaceMessageRoom()
  let before = room.contents
  let context = table.context

  func refused(_ source: UnsafePointer<BatonUnlockSource>?, _ why: String) {
    let code = callKey(
      table, context: context, source: source, key: &room.key, message: &message.message)
    check(code == badArgument, "\(why): code \(code)")
  }
  var good = faceSource(arena)
  refused(nil, "no source")
  let noContext = callKey(
    table, context: nil, source: &good, key: &room.key, message: &message.message)
  check(noContext == badArgument, "no context: code \(noContext)")
  let noKey = callKey(table, context: context, source: &good, key: nil, message: &message.message)
  check(noKey == badArgument, "no key: code \(noKey)")

  for renew: UInt32 in [2, 3, 0x1_0000, UInt32.max] {
    var source = faceSource(arena, renew: renew)
    refused(&source, "renew \(renew)")
  }
  for count in [0, 1, 16, 31, 33, 64, 4096] {
    var source = faceSource(arena, id: [UInt8](repeating: 1, count: count))
    refused(&source, "an id of \(count) bytes")
  }
  var idWithNoPointer = faceSource(arena)
  idWithNoPointer.id.ptr = nil
  refused(&idWithNoPointer, "an id with a length and no pointer")
  var idBelowZero = faceSource(arena)
  idBelowZero.id.len = -32
  refused(&idBelowZero, "an id with a length below zero")
  var longName = faceSource(arena, name: [UInt8](repeating: 0x61, count: 257))
  refused(&longName, "a name that is too long")
  var nameWithNoPointer = faceSource(arena)
  nameWithNoPointer.name.ptr = nil
  refused(&nameWithNoPointer, "a name with a length and no pointer")
  var nameBelowZero = faceSource(arena)
  nameBelowZero.name.len = -1
  refused(&nameBelowZero, "a name with a length below zero")

  // Structs that are shorter than this version of the interface knows: the memory is exactly as
  // long as the struct says, so a read past it is found by the address sanitizer.
  for size in [0, 4, 8, 24, MemoryLayout<BatonUnlockSource>.size - 1] {
    let short = arena.shortStruct(BatonUnlockSource.self, size: max(size, 4))
    short.pointee.size = UInt32(size)
    refused(short, "a source of \(size) bytes")
  }
  for size in [4, 8, 16, MemoryLayout<BatonUnlockKey>.size - 1] {
    let short = arena.shortStruct(BatonUnlockKey.self, size: size)
    let code = callKey(
      table, context: context, source: &good, key: short, message: &message.message)
    check(code == badArgument, "a key struct of \(size) bytes: code \(code)")
  }
  var noRoom = BatonUnlockKey(
    size: UInt32(MemoryLayout<BatonUnlockKey>.size), sealing: 0, len: 0, reserved: 0, key: nil)
  let noRoomCode = callKey(
    table, context: context, source: &good, key: &noRoom, message: &message.message)
  check(noRoomCode == badArgument, "a key with no room: code \(noRoomCode)")

  // Nothing of this reached the platform, which could have replaced a key for a call that could
  // not take the answer, and nothing was written.
  check(platform.requests.isEmpty, "the platform was asked: \(platform.requests)")
  check(room.contents == before && room.key.len == 0, "something was written")
  check(message.sentence == "")

  // A whole call goes through, with no message too.
  var whole = faceSource(arena)
  check(callKey(table, context: context, source: &whole, key: &room.key, message: nil) == 0)
  check(platform.requests.count == 1)
  check(room.key.len == 65)
}

func testAKeyThatDoesNotFitTheRoomOfTheRustSideIsRefusedWithoutBeingWritten() {
  // The platform made a key that is not one the interface can carry.
  for bytes in [[UInt8](repeating: 4, count: 300), []] {
    let platform = FacePlatform(outcome: .success(FacePublicKey(sealing: 1, bytes: bytes)))
    guard let table = registered(platform) else { return check(false, "no table") }
    let arena = Arena()
    var source = faceSource(arena)
    let room = FaceKeyRoom()
    let message = FaceMessageRoom()
    let before = room.contents
    let code = callKey(
      table, context: table.context, source: &source, key: &room.key, message: &message.message)
    check(code == platformFailure, "a key of \(bytes.count) bytes: code \(code)")
    check(room.contents == before && room.key.len == 0 && room.key.sealing == 0)
    check(message.sentence == "The key for the unlock could not be handed over.", message.sentence)
  }
}

// MARK: - unlock_open and unlock_cancel

private func openedScreenOf(
  _ table: BatonViewerVTable
) -> UnsafeMutableRawPointer? {
  open(table, window(Arena()), link: 41).context
}

func testUnlockOpenGivesTheScreenACopyOfTheQuestionAndSaysWhetherItWasTaken() {
  let platform = FacePlatform()
  guard let table = registered(platform), let unlockOpen = table.unlock_open,
    let context = openedScreenOf(table)
  else { return check(false, "no screen") }
  let sealed = (0..<113).map { UInt8($0) }
  let memory = FaceQuestionMemory(
    number: 31, sealed: sealed, id: faceId(9), name: Array("Maya's Mac".utf8))
  let code = unlockOpen(context, &memory.question)
  // The memory of the question is the Rust side's and is gone when the call returns.
  memory.scrub()
  check(code == 0, "code \(code)")
  let expected = FaceQuestion(
    number: 31, sealed: sealed, device: FaceDevice(id: faceId(9), name: "Maya's Mac"))
  check(platform.screen.questions == [expected], "\(platform.screen.questions)")

  platform.screen.takes = false
  let again = FaceQuestionMemory(number: 32, sealed: sealed, id: faceId(9), name: [])
  check(unlockOpen(context, &again.question) == platformFailure, "a question that was not taken")
  check(platform.screen.questions.count == 2)
  table.release?(context)
}

func testUnlockOpenNeverTrapsOnWhatItIsGivenAndTheScreenNeverHearsOfIt() {
  let platform = FacePlatform()
  guard let table = registered(platform), let unlockOpen = table.unlock_open,
    let context = openedScreenOf(table)
  else { return check(false, "no screen") }
  let arena = Arena()
  func refused(_ question: BatonUnlockQuestion, _ why: String) {
    var question = question
    let code = unlockOpen(context, &question)
    check(code == badArgument, "\(why): code \(code)")
  }

  var good = faceQuestionStruct(arena)
  check(unlockOpen(nil, &good) == badArgument, "no screen")
  check(unlockOpen(context, nil) == badArgument, "no question")
  for count in [0, 1, 32, 80, 1025, 4096] {
    refused(
      faceQuestionStruct(arena, sealed: [UInt8](repeating: 1, count: count)),
      "a sealed key of \(count) bytes")
  }
  var sealedWithNoPointer = faceQuestionStruct(arena)
  sealedWithNoPointer.sealed.ptr = nil
  refused(sealedWithNoPointer, "a sealed key with a length and no pointer")
  var sealedBelowZero = faceQuestionStruct(arena)
  sealedBelowZero.sealed.len = -113
  refused(sealedBelowZero, "a sealed key with a length below zero")
  for count in [0, 1, 31, 33, 64] {
    refused(
      faceQuestionStruct(arena, id: [UInt8](repeating: 1, count: count)),
      "an id of \(count) bytes")
  }
  var idWithNoPointer = faceQuestionStruct(arena)
  idWithNoPointer.id.ptr = nil
  refused(idWithNoPointer, "an id with a length and no pointer")
  refused(
    faceQuestionStruct(arena, name: [UInt8](repeating: 0x61, count: 257)),
    "a name that is too long")
  var nameWithNoPointer = faceQuestionStruct(arena)
  nameWithNoPointer.name.ptr = nil
  refused(nameWithNoPointer, "a name with a length and no pointer")
  var nameBelowZero = faceQuestionStruct(arena)
  nameBelowZero.name.len = -1
  refused(nameBelowZero, "a name with a length below zero")

  // Structs that are shorter than this version of the interface knows.
  for size in [0, 4, 8, 16, 48, MemoryLayout<BatonUnlockQuestion>.size - 1] {
    let short = arena.shortStruct(BatonUnlockQuestion.self, size: max(size, 4))
    short.pointee.size = UInt32(size)
    check(unlockOpen(context, short) == badArgument, "a question of \(size) bytes")
  }
  check(platform.screen.questions.isEmpty, "the screen heard of \(platform.screen.questions)")

  // A whole question after all of that is taken as if nothing had happened.
  check(unlockOpen(context, &good) == 0)
  check(platform.screen.questions.count == 1)
  table.release?(context)
}

func testUnlockCancelReachesTheScreenAndAMissingScreenIsNothing() {
  let platform = FacePlatform()
  guard let table = registered(platform), let unlockCancel = table.unlock_cancel,
    let context = openedScreenOf(table)
  else { return check(false, "no screen") }
  unlockCancel(nil)
  check(platform.screen.cancelCalls == 0)
  unlockCancel(context)
  unlockCancel(context)
  check(platform.screen.cancelCalls == 2, "\(platform.screen.cancelCalls)")
  table.release?(context)
}

func testAScreenThatCannotAskForAFaceRefusesTheQuestionSoThatTheLibraryDeclinesIt() {
  let platform = FacePlatform()
  platform.otherScreen = FakeScreen()
  guard let table = registered(platform), let unlockOpen = table.unlock_open,
    let unlockCancel = table.unlock_cancel, let context = openedScreenOf(table)
  else { return check(false, "no screen") }
  let arena = Arena()
  var question = faceQuestionStruct(arena)
  check(unlockOpen(context, &question) == platformFailure)
  withExtendedLifetime(arena) {}
  unlockCancel(context)
  table.release?(context)
}

// MARK: - The screen

/// A window over the fake decoder with the asker of a rig, and what the asker answers.
private func screenWith(_ asker: FaceAsker?) -> WindowScreen<FakeDecoder> {
  WindowScreen(
    decoder: FakeDecoder(), present: { _ in }, failed: {}, thermal: { 0 }, onRelease: {},
    asker: asker)
}

func testAWindowScreenHandsTheQuestionToItsAskerAndTakesItBackOnCancel() {
  let rig = FaceRig()
  rig.keys.script = { _, attempt in attempt.waits = true }
  let screen = screenWith(rig.asker)
  check(screen.unlockOpen(faceQuestion(1)), "the question was refused")
  guard let first = rig.keys.attempts.first, first.began.wait(timeout: .now() + 5) == .success
  else { return check(false, "the work never reached the prompt") }
  check(!screen.unlockOpen(faceQuestion(2)), "a second question while one is open")
  screen.unlockCancel()
  check(first.cancelCalls == 1, "the prompt was not taken down")
  rig.asker.idle()
  check(rig.sink.events.isEmpty)
  // After a cancel the screen takes the next question.
  check(screen.unlockOpen(faceQuestion(3)), "the question after a cancel was refused")
  screen.unlockCancel()
  rig.asker.idle()
  check(rig.sink.events.isEmpty)
}

func testTheEndAndTheReleaseOfAWindowScreenTakeTheQuestionBackWithoutACancel() {
  for ending in ["ended", "released", "ended and released"] {
    let rig = FaceRig()
    rig.keys.script = { _, attempt in attempt.waits = true }
    let screen = screenWith(rig.asker)
    check(screen.unlockOpen(faceQuestion(1)), "\(ending): the question was refused")
    guard let attempt = rig.keys.attempts.first, attempt.began.wait(timeout: .now() + 5) == .success
    else { return check(false, "the work never reached the prompt") }
    switch ending {
    case "ended": screen.ended()
    case "released": screen.released()
    default:
      screen.ended()
      screen.released()
    }
    check(attempt.cancelCalls == 1, "\(ending): taken down \(attempt.cancelCalls) times")
    rig.asker.idle()
    check(rig.sink.events.isEmpty, "\(ending): \(rig.sink.events)")
    check(!screen.unlockOpen(faceQuestion(2)), "\(ending): a question was taken after the end")
    screen.unlockCancel()
    check(rig.keys.attempts.count == 1)
  }
}

func testAWindowScreenWithoutAnAskerTakesNoQuestion() {
  let screen = screenWith(nil)
  check(!screen.unlockOpen(faceQuestion(1)))
  screen.unlockCancel()
  screen.ended()
  screen.released()
  check(!screen.unlockOpen(faceQuestion(2)))
}

// MARK: - All of it, as the device wires it

func testAQuestionGoesThroughTheTableToTheKeysAndTheAnswerComesFromAnotherThread() {
  resetFaceStubs()
  let keys = FaceFakeKeys()
  keys.script = { _, attempt in
    attempt.waits = true
    attempt.result = .opened(faceKey(0x77))
  }
  let platform = FaceUnlockPlatform(keys: keys)
  guard let table = registered(platform), let unlockOpen = table.unlock_open,
    let context = open(table, window(Arena()), link: 41).context
  else { return check(false, "no screen") }

  // The call returns while the person has not decided, and the memory is the Rust side's.
  let sealed = (0..<113).map { UInt8(200 &- $0) }
  let memory = FaceQuestionMemory(
    number: 21, sealed: sealed, id: faceId(2), name: Array("Studio".utf8))
  let code = FaceShared<Int32>(-1)
  check(
    faceReturnsWithin(2) { code.set(unlockOpen(context, &memory.question)) },
    "unlock_open waited for the person")
  memory.scrub()
  check(code.get() == 0, "code \(code.get())")
  guard let attempt = keys.attempts.first, attempt.began.wait(timeout: .now() + 5) == .success
  else { return check(false, "the work never reached the prompt") }
  check(attempt.id == faceId(2), "the key of another device would have been tried")
  check(attempt.sealed == sealed, "the sealed key")
  check(attempt.reason == "Unlock Studio", attempt.reason)
  check(faceAnswersSoFar().isEmpty, "answered before the person did")

  // The person is recognised: the key goes to the library with the link of the screen.
  attempt.letGo()
  check(eventually { faceAnswersSoFar().count == 1 }, "the answer never came")
  platform.askers.first?.idle()
  let answers = faceAnswersSoFar()
  let secret = [UInt8](repeating: 0x77, count: 32)
  check(
    answers.map { $0.answer } == [.opened(link: 41, number: 21, key: secret)],
    "\(answers.map { $0.answer })")
  check(answers.first?.thread !== Thread.current, "answered from the thread of unlock_open")
  table.release?(context)
  resetFaceStubs()
}

func testACancelThroughTheTableTakesThePromptDownAndNothingIsAnswered() {
  resetFaceStubs()
  let keys = FaceFakeKeys()
  keys.script = { _, attempt in
    attempt.waits = true
    attempt.result = .opened(faceKey(0x78))
  }
  let platform = FaceUnlockPlatform(keys: keys)
  guard let table = registered(platform), let unlockOpen = table.unlock_open,
    let unlockCancel = table.unlock_cancel,
    let context = open(table, window(Arena()), link: 42).context
  else { return check(false, "no screen") }
  let memory = FaceQuestionMemory(
    number: 1, sealed: [UInt8](repeating: 7, count: 113), id: faceId(), name: [])
  check(unlockOpen(context, &memory.question) == 0)
  guard let attempt = keys.attempts.first, attempt.began.wait(timeout: .now() + 5) == .success
  else { return check(false, "the work never reached the prompt") }
  check(attempt.reason == "Unlock your Mac", "a Mac with no name: \(attempt.reason)")
  unlockCancel(context)
  check(attempt.cancelCalls == 1, "the prompt was not taken down")
  platform.askers.first?.idle()
  check(faceAnswersSoFar().isEmpty, "\(faceAnswersSoFar().map { $0.answer })")

  // The next question on the same screen is taken, and the end of the screen takes it back with
  // no cancel before it.
  check(unlockOpen(context, &memory.question) == 0, "the next question was refused")
  guard keys.attempts.count == 2, keys.attempts[1].began.wait(timeout: .now() + 5) == .success
  else { return check(false, "the second question never reached the prompt") }
  table.ended?(context)
  check(keys.attempts[1].cancelCalls == 1, "the end did not take the prompt down")
  check(unlockOpen(context, &memory.question) == platformFailure, "a question after the end")
  platform.askers.first?.idle()
  check(faceAnswersSoFar().isEmpty, "\(faceAnswersSoFar().map { $0.answer })")
  table.release?(context)
  check(keys.attempts[1].cancelCalls == 1, "the release took the prompt down again")
  resetFaceStubs()
}
