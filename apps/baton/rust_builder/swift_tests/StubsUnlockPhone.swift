// SPDX-License-Identifier: Apache-2.0

// The two functions of the C interface that the Rust library provides for the answers to a question
// about a face, played by the tests: what the Swift code answers is kept, with the thread it
// answered from, so that a test can look at it as the Rust side would. The tests link without the
// Rust library, so these stand in for it.

import Foundation

/// What the Swift code answered to a question about a face.
enum UnlockAnswer: Equatable {
  /// `baton_stream_unlock_opened`: the key of the box that was sent.
  case opened(link: UInt64, number: UInt64, key: [UInt8])
  /// `baton_stream_unlock_declined`, with a BATON_FACE_ number.
  case declined(link: UInt64, number: UInt64, reason: UInt32)
}

struct FaceAnswerRecord {
  var answer: UnlockAnswer
  /// The thread that made the call.
  var thread: Thread
}

/// Held for a moment around the state below, never while a hook runs.
let faceStubLock = NSLock()
/// The answers, in order.
var faceAnswers: [FaceAnswerRecord] = []
/// What the stubs return: BATON_OK, or what the Rust side says to an answer that it will not take
/// (BATON_E_UNKNOWN_LINK for a question that is not open).
var faceAnswerCode: Int32 = 0
/// Called inside the stubs, with no lock held, before they return. A test makes the call wait here,
/// as the real function waits for the lock of the session.
var faceAnswerHook: (() -> Void)?

func resetFaceStubs() {
  faceStubLock.lock()
  faceAnswers = []
  faceAnswerCode = 0
  faceAnswerHook = nil
  faceStubLock.unlock()
}

func setFaceAnswerCode(_ code: Int32) {
  faceStubLock.lock()
  faceAnswerCode = code
  faceStubLock.unlock()
}

func setFaceAnswerHook(_ hook: (() -> Void)?) {
  faceStubLock.lock()
  faceAnswerHook = hook
  faceStubLock.unlock()
}

/// The answers so far.
func faceAnswersSoFar() -> [FaceAnswerRecord] {
  faceStubLock.lock()
  defer { faceStubLock.unlock() }
  return faceAnswers
}

@_cdecl("baton_stream_unlock_opened")
func stubUnlockOpened(
  _ link: UInt64, _ number: UInt64, _ key: UnsafePointer<BatonBytes>?
) -> Int32 {
  var bytes: [UInt8] = []
  if let key, key.pointee.len > 0, let start = key.pointee.ptr {
    bytes = Array(UnsafeBufferPointer(start: start, count: key.pointee.len))
  }
  faceStubLock.lock()
  faceAnswers.append(
    FaceAnswerRecord(
      answer: .opened(link: link, number: number, key: bytes), thread: Thread.current))
  let hook = faceAnswerHook
  let code = faceAnswerCode
  faceStubLock.unlock()
  hook?()
  return code
}

@_cdecl("baton_stream_unlock_declined")
func stubUnlockDeclined(_ link: UInt64, _ number: UInt64, _ reason: UInt32) -> Int32 {
  faceStubLock.lock()
  faceAnswers.append(
    FaceAnswerRecord(
      answer: .declined(link: link, number: number, reason: reason), thread: Thread.current))
  let hook = faceAnswerHook
  let code = faceAnswerCode
  faceStubLock.unlock()
  hook?()
  return code
}
