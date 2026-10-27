// SPDX-License-Identifier: Apache-2.0

// The three functions of the C interface that an unlock of the Mac calls, and that the Rust library
// provides, played by the tests: what the Swift code gave them is kept, and they answer as a test
// says. They are the same for every platform, since BatonUnlock.swift is.

import Foundation

/// A call that an unlock, or a private screen, made to the library.
enum UnlockCall: Equatable {
  case live(UInt64)
  case keep(UInt64, boxed: [UInt8], sealed: [UInt8], size: UInt32)
  case place(UInt64, UInt32)
}

/// What the stubs of those calls answer.
struct UnlockAnswers {
  var liveCode: Int32 = 0
  var live: UInt32 = 1
  var keepCode: Int32 = 0
  var keepResult: UInt32 = UInt32(BATON_KEEP_KEPT)
  /// Whether a yes to `keep` comes with the result; a library that forgets it breaks the contract.
  var keepWritesResult = true
  var placeCode: Int32 = 0
}

var unlockCalls: [UnlockCall] = []
var unlockAnswers = UnlockAnswers()

func resetUnlockStubs() {
  stubLock.lock()
  unlockCalls = []
  unlockAnswers = UnlockAnswers()
  stubLock.unlock()
}

@_cdecl("baton_stream_source_private")
func stubSourcePrivate(_ link: UInt64, _ inPlace: UInt32) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  unlockCalls.append(.place(link, inPlace))
  return unlockAnswers.placeCode
}

@_cdecl("baton_stream_unlock_live")
func stubUnlockLive(_ link: UInt64, _ live: UnsafeMutablePointer<UInt32>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  unlockCalls.append(.live(link))
  if unlockAnswers.liveCode == 0 { live?.pointee = unlockAnswers.live }
  return unlockAnswers.liveCode
}

@_cdecl("baton_stream_unlock_keep")
func stubUnlockKeep(
  _ link: UInt64, _ box: UnsafePointer<BatonUnlockBox>?, _ result: UnsafeMutablePointer<UInt32>?
) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  func bytes(_ bytes: BatonBytes) -> [UInt8] {
    guard bytes.len > 0, let start = bytes.ptr else { return [] }
    return Array(UnsafeBufferPointer(start: start, count: bytes.len))
  }
  if let box {
    let c = box.pointee
    unlockCalls.append(
      .keep(link, boxed: bytes(c.boxed), sealed: bytes(c.sealed), size: c.size))
  }
  if unlockAnswers.keepCode == 0 && unlockAnswers.keepWritesResult {
    result?.pointee = unlockAnswers.keepResult
  }
  return unlockAnswers.keepCode
}
