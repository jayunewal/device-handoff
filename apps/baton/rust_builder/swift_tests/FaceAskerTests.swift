// SPDX-License-Identifier: Apache-2.0

// Tests of the asker of BatonFace.swift: the one question that a screen has open, who answers it,
// and what is wiped. The keys and the prompt are fakes (FaceFakes.swift). The answers go to a sink,
// or, where a test is about what the Rust side hears, to the stubs of StubsUnlockPhone.swift.

import Foundation

/// Waits until the work of the asker has reached the prompt for the face.
private func reachedThePrompt(_ attempt: FaceFakeAttempt) -> Bool {
  attempt.began.wait(timeout: .now() + 5) == .success
}

private func secretOf(_ byte: UInt8) -> [UInt8] {
  [UInt8](repeating: byte, count: Int(BATON_BOX_KEY_BYTES))
}

// MARK: - One question

func testAQuestionIsWorkedOnByAQueueOfItsOwnAndAnsweredOnceWithItsNumber() {
  let rig = FaceRig()
  let key = faceKey(0x42)
  rig.keys.script = { _, attempt in attempt.result = .opened(key) }
  check(rig.asker.ask(faceQuestion(7)), "the question was taken")
  rig.asker.idle()
  let attempts = rig.keys.attempts
  check(attempts.count == 1, "\(attempts.count) attempts")
  check(attempts.first?.openCalls == 1, "asked for \(attempts.first?.openCalls ?? 0) times")
  check(attempts.first?.openThread !== Thread.current, "asked for on the thread of the caller")
  check(rig.sink.events == [.opened(number: 7, key: secretOf(0x42))], "\(rig.sink.events)")
  check(rig.sink.answerThreads.first !== Thread.current, "answered on the thread of the caller")
  check(key.isWiped, "the key was not wiped once it was sent")
  check(rig.sink.openedKeys.first?.isWiped == true)
  check(attempts.first?.cancelCalls == 0, "a prompt that was done with was taken down")
}

func testAQuestionThatWaitsForThePersonDoesNotHoldTheCaller() {
  let rig = FaceRig()
  rig.keys.script = { _, attempt in
    attempt.waits = true
    attempt.result = .opened(faceKey(0x11))
  }
  let taken = FaceShared(false)
  let returned = faceReturnsWithin(2) { taken.set(rig.asker.ask(faceQuestion(1))) }
  check(returned, "ask waited for the person")
  check(taken.get(), "the question was taken")
  guard let attempt = rig.keys.attempts.first, reachedThePrompt(attempt) else {
    return check(false, "the work never reached the prompt")
  }
  check(rig.sink.events.isEmpty, "answered before the person did")
  // The person is done.
  attempt.letGo()
  check(eventually { rig.sink.events.count == 1 }, "the answer never came")
  rig.asker.idle()
  check(rig.sink.events == [.opened(number: 1, key: secretOf(0x11))], "\(rig.sink.events)")
}

func testASecondQuestionWhileOneIsOpenIsRefusedAndAsksNothing() {
  let rig = FaceRig()
  rig.keys.script = { _, attempt in
    attempt.waits = true
    attempt.result = .opened(faceKey(0x11))
  }
  check(rig.asker.ask(faceQuestion(1)))
  guard let first = rig.keys.attempts.first, reachedThePrompt(first) else {
    return check(false, "the work never reached the prompt")
  }
  check(!rig.asker.ask(faceQuestion(2)), "a second question was taken while one was open")
  check(!rig.asker.ask(faceQuestion(1)), "the same question was taken twice")
  check(rig.keys.attempts.count == 1, "the person was asked for their face a second time")
  first.letGo()
  rig.asker.idle()
  check(rig.sink.events == [.opened(number: 1, key: secretOf(0x11))], "\(rig.sink.events)")
}

func testAQuestionIsAnsweredOnceAndTheNextIsTakenAfterIt() {
  let rig = FaceRig()
  rig.keys.script = { index, attempt in
    attempt.result = .opened(faceKey(UInt8(0x10 + index)))
  }
  check(rig.asker.ask(faceQuestion(5)))
  rig.asker.idle()
  // A cancel that comes after the answer changes nothing: the prompt is gone already.
  rig.asker.cancel()
  check(rig.asker.ask(faceQuestion(6)), "the next question was refused")
  rig.asker.idle()
  check(
    rig.sink.events == [
      .opened(number: 5, key: secretOf(0x10)), .opened(number: 6, key: secretOf(0x11)),
    ], "\(rig.sink.events)")
  check(rig.keys.attempts.map { $0.cancelCalls } == [0, 0], "a cancel took a finished prompt down")
  check(rig.keys.attempts.map { $0.openCalls } == [1, 1], "each prompt is asked for once")
}

func testADeclineCarriesItsReasonAndNothingElseIsAnswered() {
  for reason in [FaceDecline.declined, .keyLost, .failed] {
    let rig = FaceRig()
    rig.keys.script = { _, attempt in attempt.result = .declined(reason) }
    check(rig.asker.ask(faceQuestion(3)))
    rig.asker.idle()
    let expected: [FaceAnswerSink.Event] = [.declined(number: 3, reason: reason)]
    check(rig.sink.events == expected, "\(reason): \(rig.sink.events)")
    check(rig.sink.openedKeys.isEmpty)
    check(rig.asker.ask(faceQuestion(4)), "a question after a decline was refused")
    rig.asker.idle()
    check(rig.sink.events.count == 2)
  }
}

func testTheKeysAreAskedForOneDeviceAndThePromptNamesIt() {
  let rig = FaceRig()
  let question = faceQuestion(3, name: "Maya's Mac\nmini", seed: 9, sealedBytes: 200)
  check(rig.asker.ask(question))
  rig.asker.idle()
  check(rig.keys.attempts.count == 1, "\(rig.keys.attempts.count) attempts")
  let attempt = rig.keys.attempts.first
  check(attempt?.id == faceId(9), "the key of another device would have been tried")
  check(attempt?.sealed == question.sealed, "the sealed key")
  check(attempt?.reason == "Unlock Maya's Mac mini", "\(attempt?.reason ?? "no reason")")
  // A sealed key does not say which key it was sealed to, so one pair is used and no other.
  check(rig.keys.keyRequests.isEmpty, "a question made, found or tried keys")
}

// MARK: - Taking a question back

func testACancelBeforeTheWorkBeginsAsksNothingAndAnswersNothing() {
  let rig = FaceRig()
  rig.queue.suspend()
  check(rig.asker.ask(faceQuestion(1)))
  rig.asker.cancel()
  rig.queue.resume()
  rig.asker.idle()
  let attempt = rig.keys.attempts.first
  check(attempt?.openCalls == 0, "the person was asked for their face for a question taken back")
  check(attempt?.cancelCalls == 1, "the attempt was not told")
  check(rig.sink.events.isEmpty, "\(rig.sink.events)")
  check(rig.asker.ask(faceQuestion(2)), "the next question was refused")
  rig.asker.idle()
  check(rig.sink.events.count == 1)
}

func testACancelWhileTheFaceIsAskedTakesThePromptDownAndAnswersNothing() {
  let rig = FaceRig()
  rig.keys.script = { index, attempt in
    // Only the first prompt waits for the person; the next one ends at once.
    attempt.waits = index == 0
    attempt.result = .declined(.declined)
  }
  check(rig.asker.ask(faceQuestion(1)))
  guard let attempt = rig.keys.attempts.first, reachedThePrompt(attempt) else {
    return check(false, "the work never reached the prompt")
  }
  check(faceReturnsWithin(2) { rig.asker.cancel() }, "a cancel waited for the person")
  check(attempt.cancelCalls == 1, "the prompt was not taken down")
  rig.asker.idle()
  check(rig.sink.events.isEmpty, "something was answered for a question that was taken back")
  // Taking it back twice takes the prompt down once.
  rig.asker.cancel()
  check(attempt.cancelCalls == 1, "a prompt that was down was taken down again")
  // And the next question is taken.
  check(rig.asker.ask(faceQuestion(2)), "the next question was refused")
  rig.asker.idle()
  check(rig.sink.events.count == 1)
}

func testAPromptThatStaysAfterACancelDoesNotHoldBackTheNextQuestion() {
  let rig = FaceRig()
  rig.keys.script = { index, attempt in
    if index == 0 {
      // The system has not taken the prompt down yet: open does not end by itself.
      attempt.waits = true
      attempt.endsOnCancel = false
      attempt.result = .opened(faceKey(0x01))
    } else {
      attempt.result = .opened(faceKey(0x02))
    }
  }
  check(rig.asker.ask(faceQuestion(1)))
  guard let stuck = rig.keys.attempts.first, reachedThePrompt(stuck) else {
    return check(false, "the work never reached the prompt")
  }
  rig.asker.cancel()
  check(stuck.cancelCalls == 1)
  check(rig.asker.ask(faceQuestion(2)), "a stuck prompt held back the next question")
  check(eventually { rig.sink.events.count == 1 }, "the next question was never worked on")
  check(rig.sink.events == [.opened(number: 2, key: secretOf(0x02))], "\(rig.sink.events)")
  // The stuck prompt ends at last, with a key: it is not sent, and it is wiped.
  let firstKey = stuck.result
  stuck.letGo()
  rig.asker.idle()
  check(rig.sink.events.count == 1, "a question that was taken back was answered when it ended")
  if case .opened(let key) = firstKey { check(key.isWiped, "the key of a question taken back") }
}

func testClosingTakesTheQuestionBackAndNoOtherIsTaken() {
  let rig = FaceRig()
  rig.keys.script = { _, attempt in attempt.waits = true }
  check(rig.asker.ask(faceQuestion(1)))
  guard let attempt = rig.keys.attempts.first, reachedThePrompt(attempt) else {
    return check(false, "the work never reached the prompt")
  }
  check(faceReturnsWithin(2) { rig.asker.close() }, "a close waited for the person")
  check(attempt.cancelCalls == 1, "the prompt was not taken down")
  rig.asker.idle()
  check(rig.sink.events.isEmpty)
  check(!rig.asker.ask(faceQuestion(2)), "a question was taken after the end")
  check(rig.keys.attempts.count == 1, "a prompt was made for a screen that had ended")
  rig.asker.close()
  rig.asker.cancel()
  check(attempt.cancelCalls == 1, "a prompt was taken down twice")

  // A screen that ends with no question open takes none afterwards either.
  let quiet = FaceRig()
  quiet.asker.close()
  check(!quiet.asker.ask(faceQuestion(1)))
  check(quiet.keys.attempts.isEmpty)
  quiet.asker.cancel()
  check(quiet.sink.events.isEmpty)
}

func testAKeyThatWasOpenedAndNotSentIsWiped() {
  // The key is open when the question is taken back: the person was recognised, and the Rust side
  // does not want the answer any more. A key that stays open is good for one unlock without a face.
  for ending in ["cancel", "close"] {
    let rig = FaceRig()
    let key = faceKey(0x33)
    rig.keys.script = { _, attempt in
      attempt.waits = true
      attempt.endsOnCancel = false
      attempt.result = .opened(key)
    }
    check(rig.asker.ask(faceQuestion(1)))
    guard let attempt = rig.keys.attempts.first, reachedThePrompt(attempt) else {
      return check(false, "the work never reached the prompt")
    }
    if ending == "cancel" { rig.asker.cancel() } else { rig.asker.close() }
    attempt.letGo()
    rig.asker.idle()
    check(key.isWiped, "\(ending): a key that was not sent was left open")
    check(rig.sink.events.isEmpty, "\(ending): \(rig.sink.events)")
  }
}

func testACancelDoesNotWaitForAnAnswerThatIsOnItsWay() {
  // The Rust side holds the lock of the session while it calls unlock_cancel, and takes it when it
  // is answered. An answer that waits for that lock must not hold the cancel up, or the two wait
  // for each other.
  let rig = FaceRig()
  let entered = DispatchSemaphore(value: 0)
  let gate = DispatchSemaphore(value: 0)
  rig.sink.hook = {
    entered.signal()
    gate.wait()
  }
  let key = faceKey(0x44)
  rig.keys.script = { _, attempt in attempt.result = .opened(key) }
  check(rig.asker.ask(faceQuestion(1)))
  check(entered.wait(timeout: .now() + 5) == .success, "the answer never reached the Rust side")
  check(faceReturnsWithin(2) { rig.asker.cancel() }, "a cancel waited for an answer on its way")
  check(faceReturnsWithin(2) { rig.asker.close() }, "a close waited for an answer on its way")
  check(faceReturnsWithin(2) { _ = rig.asker.ask(faceQuestion(2)) }, "an ask waited for it")
  gate.signal()
  rig.asker.idle()
  // The answer was taken before the cancel, so it goes out once; the Rust side refuses it if the
  // question is not open any more.
  check(rig.sink.events == [.opened(number: 1, key: secretOf(0x44))], "\(rig.sink.events)")
  check(key.isWiped)
  check(rig.keys.attempts.first?.cancelCalls == 0, "a prompt that was answered was taken down")
}

func testACancelAndAnAnswerThatComeTogetherAnswerAtMostOnce() {
  var answered = 0
  var cancelledFirst = 0
  for round in 0..<300 {
    let rig = FaceRig()
    let key = faceKey(0x55)
    rig.keys.script = { _, attempt in attempt.result = .opened(key) }
    check(rig.asker.ask(faceQuestion(UInt64(round))))
    // This races with the work on the queue: either may come first.
    if round % 3 != 0 { Thread.sleep(forTimeInterval: Double(round % 3) * 0.0002) }
    rig.asker.cancel()
    rig.asker.idle()
    let events = rig.sink.events
    check(events.count <= 1, "round \(round): \(events.count) answers")
    if let first = events.first {
      answered += 1
      let expected = FaceAnswerSink.Event.opened(number: UInt64(round), key: secretOf(0x55))
      check(first == expected, "round \(round): \(first)")
    } else {
      cancelledFirst += 1
    }
    let opened = rig.keys.attempts.first?.openCalls ?? 0
    check(key.isWiped || opened == 0, "round \(round): a key was left open")
    check((rig.keys.attempts.first?.openCalls ?? 0) <= 1, "round \(round): asked twice")
  }
  check(answered + cancelledFirst == 300)
}

// MARK: - The key of the box

func testAKeyOfTheWrongLengthIsNotSentAndIsWiped() {
  for count in [0, 1, 16, 31, 33, 64, 256] {
    let rig = FaceRig()
    let key = faceKey(0x66, count: count)
    rig.keys.script = { _, attempt in attempt.result = .opened(key) }
    check(rig.asker.ask(faceQuestion(8)))
    rig.asker.idle()
    check(
      rig.sink.events == [.declined(number: 8, reason: .failed)],
      "a key of \(count) bytes: \(rig.sink.events)")
    check(key.isWiped, "a key of \(count) bytes was left open")
    check(rig.sink.openedKeys.isEmpty, "a key of \(count) bytes was sent")
  }
}

// MARK: - The answers to the Rust side

func testTheAnswerGoesToTheLibraryWithTheLinkTheNumberAndTheKey() {
  resetFaceStubs()
  let keys = FaceFakeKeys()
  keys.script = { _, attempt in attempt.result = .opened(faceKey(0x42)) }
  let asker = FaceUnlock(keys: keys).asker(link: 77)
  check(asker.ask(faceQuestion(12)))
  asker.idle()
  let answers = faceAnswersSoFar()
  check(
    answers.map { $0.answer } == [.opened(link: 77, number: 12, key: secretOf(0x42))],
    "\(answers.map { $0.answer })")
  check(answers.first?.thread !== Thread.current, "answered from the thread of the caller")

  keys.script = { _, attempt in attempt.result = .declined(.keyLost) }
  check(asker.ask(faceQuestion(13)))
  asker.idle()
  keys.script = { _, attempt in attempt.result = .declined(.declined) }
  check(asker.ask(faceQuestion(14)))
  asker.idle()
  keys.script = { _, attempt in attempt.result = .declined(.failed) }
  check(asker.ask(faceQuestion(15)))
  asker.idle()
  check(
    faceAnswersSoFar().map { $0.answer } == [
      .opened(link: 77, number: 12, key: secretOf(0x42)),
      .declined(link: 77, number: 13, reason: BATON_FACE_KEY_LOST),
      .declined(link: 77, number: 14, reason: BATON_FACE_DECLINED),
      .declined(link: 77, number: 15, reason: BATON_FACE_FAILED),
    ], "\(faceAnswersSoFar().map { $0.answer })")
  resetFaceStubs()
}

func testAnAnswerThatTheLibraryRefusesIsNotAnErrorAndTheNextQuestionIsTaken() {
  resetFaceStubs()
  let keys = FaceFakeKeys()
  let asker = FaceUnlock(keys: keys).asker(link: 5)
  for code in [Int32(BATON_E_UNKNOWN_LINK), Int32(BATON_E_ARGUMENT), Int32(BATON_E_PLATFORM)] {
    setFaceAnswerCode(code)
    let before = faceAnswersSoFar().count
    keys.script = { _, attempt in attempt.result = .opened(faceKey(0x42)) }
    check(asker.ask(faceQuestion(UInt64(before + 1))), "refused after the library said \(code)")
    asker.idle()
    check(faceAnswersSoFar().count == before + 1, "the answer was made again or not at all")
  }
  setFaceAnswerCode(0)
  resetFaceStubs()
}

func testAKeyThatWasWipedBeforeItWasSentIsDeclinedAsFailed() {
  resetFaceStubs()
  let key = faceKey(0x42)
  key.wipe()
  FaceAnswers.toRust(link: 5).opened(9, key)
  let expected: [UnlockAnswer] = [.declined(link: 5, number: 9, reason: BATON_FACE_FAILED)]
  check(
    faceAnswersSoFar().map { $0.answer } == expected,
    "\(faceAnswersSoFar().map { $0.answer })")
  resetFaceStubs()
}

func testTheAnswerIsMadeWithNoLockHeldSoTheLibraryMayWaitForItsOwn() {
  // The stub waits inside the call, as the library waits for the lock of the session. Taking the
  // question back meanwhile, as the library does with that lock held, must not wait for the answer.
  resetFaceStubs()
  let entered = DispatchSemaphore(value: 0)
  let gate = DispatchSemaphore(value: 0)
  setFaceAnswerHook {
    entered.signal()
    gate.wait()
  }
  let keys = FaceFakeKeys()
  keys.script = { _, attempt in attempt.result = .opened(faceKey(0x42)) }
  let asker = FaceUnlock(keys: keys).asker(link: 6)
  check(asker.ask(faceQuestion(1)))
  check(entered.wait(timeout: .now() + 5) == .success, "the answer never reached the library")
  check(faceReturnsWithin(2) { asker.cancel() }, "a cancel waited for the library")
  gate.signal()
  asker.idle()
  setFaceAnswerHook(nil)
  check(faceAnswersSoFar().count == 1)
  resetFaceStubs()
}

// MARK: - The keys of a device

private func outcome(
  _ unlock: FaceUnlock, _ request: FaceKeyRequest
) -> Result<FacePublicKey, Error> {
  Result { try unlock.key(for: request) }
}

func testAKeyIsGivenForTheDeviceAndRenewedWhenAsked() {
  let keys = FaceFakeKeys()
  keys.point = facePoint(3)
  let unlock = FaceUnlock(keys: keys)
  let device = FaceDevice(id: faceId(4), name: "Studio")
  let plain = try? unlock.key(for: FaceKeyRequest(device: device, renew: false))
  check(plain == FacePublicKey(point: facePoint(3)), "\(String(describing: plain))")
  check(plain?.sealing == BATON_SEALING_ECIES_P256)
  let renewed = try? unlock.key(for: FaceKeyRequest(device: device, renew: true))
  check(renewed == FacePublicKey(point: facePoint(3)))
  check(
    keys.keyRequests == [
      FaceKeyRequestSeen(id: faceId(4), renew: false),
      FaceKeyRequestSeen(id: faceId(4), renew: true),
    ], "\(keys.keyRequests)")
  check(keys.attempts.isEmpty, "a face was asked for to give a key")
}

func testAKeyThatIsNotAPointOfP256IsNotGivenOn() {
  let device = FaceDevice(id: faceId(), name: "Studio")
  let request = FaceKeyRequest(device: device, renew: false)
  var bad: [[UInt8]] = [[], [0x04], Array(facePoint().dropLast()), facePoint() + [0]]
  var compressed = [UInt8](repeating: 1, count: 33)
  compressed[0] = 0x02
  bad.append(compressed)
  var wrongFirst = facePoint()
  wrongFirst[0] = 0x00
  bad.append(wrongFirst)
  for point in bad {
    let keys = FaceFakeKeys()
    keys.point = point
    switch outcome(FaceUnlock(keys: keys), request) {
    case .success: check(false, "a key of \(point.count) bytes was given")
    case .failure(let error):
      check(error as? FaceKeyFailure == .notAPoint, "a key of \(point.count) bytes: \(error)")
    }
  }
}

func testAFailureToMakeAKeyReachesTheCallerAsItWas() {
  let device = FaceDevice(id: faceId(), name: "Studio")
  let request = FaceKeyRequest(device: device, renew: true)
  let keys = FaceFakeKeys()
  keys.failure = FaceKeyFailure.noFace
  switch outcome(FaceUnlock(keys: keys), request) {
  case .success: check(false, "a key was given")
  case .failure(let error): check(error as? FaceKeyFailure == .noFace, "\(error)")
  }
  struct Odd: Error {}
  keys.failure = Odd()
  switch outcome(FaceUnlock(keys: keys), request) {
  case .success: check(false, "a key was given")
  case .failure(let error): check(error is Odd, "\(error)")
  }
  keys.failure = nil
  if case .failure(let error) = outcome(FaceUnlock(keys: keys), request) {
    check(false, "a key was refused after the failure was gone: \(error)")
  }
}

func testTwoCallsForAKeyNeverRunAtTheSameTime() {
  // A pair is found by its tag, and two pairs under one tag could not be told apart, so a pair is
  // never made or replaced while another call is at it.
  let keys = FaceFakeKeys()
  keys.keyDelay = 0.003
  let unlock = FaceUnlock(keys: keys)
  let group = DispatchGroup()
  for thread in 0..<6 {
    group.enter()
    Thread.detachNewThread {
      for _ in 0..<4 {
        let device = FaceDevice(id: faceId(UInt8(thread)), name: "x")
        _ = try? unlock.key(for: FaceKeyRequest(device: device, renew: thread % 2 == 0))
      }
      group.leave()
    }
  }
  check(group.wait(timeout: .now() + 60) == .success, "the threads finished")
  check(keys.keyRequests.count == 24, "\(keys.keyRequests.count) calls")
  check(keys.mostAtOnce == 1, "\(keys.mostAtOnce) calls ran at the same moment")
}

func testEachScreenGetsAnAskerOfItsOwnThatAnswersWithItsLink() {
  resetFaceStubs()
  let keys = FaceFakeKeys()
  keys.script = { _, attempt in attempt.result = .opened(faceKey(0x42)) }
  let unlock = FaceUnlock(keys: keys)
  let first = unlock.asker(link: 100)
  let second = unlock.asker(link: 200)
  check(first !== second)
  check(first.ask(faceQuestion(1)) && second.ask(faceQuestion(1)), "one question per screen")
  first.idle()
  second.idle()
  let answers = faceAnswersSoFar().map { $0.answer }
  check(answers.count == 2, "\(answers)")
  check(answers.contains(.opened(link: 100, number: 1, key: secretOf(0x42))), "\(answers)")
  check(answers.contains(.opened(link: 200, number: 1, key: secretOf(0x42))), "\(answers)")
  resetFaceStubs()
}
