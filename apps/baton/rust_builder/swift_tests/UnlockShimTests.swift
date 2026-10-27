// SPDX-License-Identifier: Apache-2.0

// Tests of the three functions of the sharer's table that are for an unlock (BatonSource.swift):
// they play the Rust side, which calls them with the C structs it builds, and a platform and a
// window that write down what they were given. What matters here: the entries are in the table all
// together or not at all, a call that cannot be read is refused and reaches nothing, the secrets are
// wiped when the call is over whatever it answered, and every outcome is the number that the Rust
// side expects.

import Foundation

/// A platform that gives its screen and can be unlocked, and the table that it registers.
private func unlockingPlatform() -> (platform: FakeSharerPlatform, table: BatonSharerVTable)? {
  let platform = FakeSharerPlatform()
  platform.canGiveScreen = true
  platform.canUnlock = true
  guard let table = registeredSharing(platform) else {
    check(false, "no table was registered")
    return nil
  }
  return (platform, table)
}

// MARK: - The table

func testTheTableHasTheThreeEntriesToUnlockTogetherOrNotAtAll() {
  let neither = FakeSharerPlatform()
  neither.canGiveScreen = true
  guard let without = registeredSharing(neither) else { return check(false) }
  check(without.unlock_check == nil, "an entry to check for a platform that cannot unlock")
  check(without.unlock_setup == nil, "an entry to set up for a platform that cannot unlock")
  check(without.unlock == nil, "an entry to unlock for a platform that cannot")
  check(without.size == UInt32(MemoryLayout<BatonSharerVTable>.size), "size \(without.size)")

  // An unlock goes with the privacy of a screen that is given private, so a platform that gives no
  // screen has none, whatever else it says.
  let noScreen = FakeSharerPlatform()
  noScreen.canUnlock = true
  guard let alone = registeredSharing(noScreen) else { return check(false) }
  check(
    alone.unlock_check == nil && alone.unlock_setup == nil && alone.unlock == nil,
    "an entry to unlock for a platform with no screen to give")

  guard let both = unlockingPlatform()?.table else { return }
  check(both.unlock_check != nil && both.unlock_setup != nil && both.unlock != nil, "the entries")
  check(both.size == 144, "size \(both.size)")
  check(both.leave == nil && both.hear == nil, "the other entries are the platform's own to give")

  // A platform from before there was an unlock has no entry, whatever its screen can do.
  check(registeredSharing(PlatformWithoutAScreen())?.unlock == nil)
}

// MARK: - Checking the password

private func callCheck(
  _ table: BatonSharerVTable, password: [UInt8], messageSize: Int? = nil
) -> (code: Int32, sentence: String) {
  let arena = Arena()
  var bytes = arena.batonBytes(password)
  guard let call = table.unlock_check else { return (-99, "") }
  return withMessage(size: messageSize) { message in
    withUnsafePointer(to: &bytes) { call(table.context, $0, message) }
  }
}

func testThePasswordIsCheckedByThePlatformAndWhatItSaysIsWhatTheRustSideHears() {
  guard let (platform, table) = unlockingPlatform() else { return }
  let password = Array("Ab1! pässwörd".utf8)
  let accepted = callCheck(table, password: password)
  check(accepted.code == 0, "accepted: \(accepted)")
  check(platform.checkedPasswords == [password], "the platform was given \(platform.checkedPasswords)")
  check(platform.checkedSecrets.allSatisfy { $0.isWiped }, "the password was not wiped")

  platform.passwordCheck = .wrong
  check(callCheck(table, password: password).code == Int32(BATON_PASSWORD_WRONG), "wrong")
  platform.passwordCheck = .unsupported
  check(callCheck(table, password: password).code == Int32(BATON_PASSWORD_UNSUPPORTED), "unsupported")
  platform.passwordCheck = .failed("This Mac is busy.")
  let failed = callCheck(table, password: password)
  check(failed.code == Int32(BATON_E_PLATFORM), "failed: \(failed)")
  check(failed.sentence == "This Mac is busy.", failed.sentence)
  check(platform.checkedPasswords.count == 4, "\(platform.checkedPasswords.count) calls")
  check(platform.checkedSecrets.allSatisfy { $0.isWiped }, "a password was not wiped")
}

func testACheckThatCannotBeReadIsRefusedAndTheNumberOfBytesIsWhatTheInterfaceAllows() {
  guard let (platform, table) = unlockingPlatform(), let call = table.unlock_check else { return }
  let arena = Arena()
  let refused = Int32(BATON_E_ARGUMENT)
  check(callCheck(table, password: []).code == refused, "no password")
  check(
    callCheck(table, password: [UInt8](repeating: 65, count: UnlockLimits.passwordMax + 1)).code
      == refused, "a password that is too long")
  check(
    callCheck(table, password: [UInt8](repeating: 65, count: UnlockLimits.passwordMax)).code == 0,
    "the longest password")
  check(callCheck(table, password: [65], messageSize: 4).code == refused, "a message that is too short")

  var bytes = arena.batonBytes([65])
  _ = withMessage { message in
    check(call(table.context, nil, message) == refused, "no password")
    check(call(nil, &bytes, message) == refused, "no context")
    return 0
  }
  check(withUnsafePointer(to: &bytes) { call(table.context, $0, nil) } == refused, "no message")
  var noPointer = BatonBytes(ptr: nil, len: 3)
  _ = withMessage { message in
    check(call(table.context, &noPointer, message) == refused, "a length and no pointer")
    return 0
  }
  // Only the one call that could be read reached the platform.
  check(platform.checkedPasswords.count == 1, "\(platform.checkedPasswords.count) calls")
}

// MARK: - Setting up

private func setUp(
  _ table: BatonSharerVTable, _ arena: Arena, password: [UInt8] = Array("Ab1!".utf8),
  boxRoom: Int = UnlockLimits.boxMax, sealedRoom: Int = UnlockLimits.sealedMax,
  madeSize: Int = MemoryLayout<BatonUnlockMade>.size, messageSize: Int? = nil,
  setup: UnsafeMutablePointer<BatonUnlockSetup>? = nil
) -> (code: Int32, sentence: String, boxed: [UInt8], sealed: [UInt8], lengths: [UInt32]) {
  guard let call = table.unlock_setup else { return (-99, "", [], [], []) }
  let boxMemory = UnsafeMutablePointer<UInt8>.allocate(capacity: max(boxRoom, 1))
  let sealedMemory = UnsafeMutablePointer<UInt8>.allocate(capacity: max(sealedRoom, 1))
  defer {
    boxMemory.deallocate()
    sealedMemory.deallocate()
  }
  boxMemory.initialize(repeating: 0, count: max(boxRoom, 1))
  sealedMemory.initialize(repeating: 0, count: max(sealedRoom, 1))
  let made = arena.shortStruct(BatonUnlockMade.self, size: madeSize)
  if madeSize >= MemoryLayout<BatonUnlockMade>.size {
    made.pointee.boxed = boxMemory
    made.pointee.sealed = sealedMemory
  }
  let request = setup ?? arena.unlockSetup(password: password)
  let (code, sentence) = withMessage(size: messageSize) { message in
    call(table.context, request, made, message)
  }
  let lengths: [UInt32] =
    madeSize >= MemoryLayout<BatonUnlockMade>.size
    ? [made.pointee.box_len, made.pointee.sealed_len] : []
  let boxed = lengths.isEmpty ? [] : Array(UnsafeBufferPointer(start: boxMemory, count: Int(lengths[0])))
  let sealed =
    lengths.isEmpty ? [] : Array(UnsafeBufferPointer(start: sealedMemory, count: Int(lengths[1])))
  return (code, sentence, boxed, sealed, lengths)
}

func testAnUnlockIsSetUpByThePlatformAndWhatItMadeIsWrittenForTheRustSide() {
  guard let (platform, table) = unlockingPlatform() else { return }
  let arena = Arena()
  platform.unlockMade = .made(boxed: [1, 2, 3], sealed: [4, 5])
  let done = setUp(table, arena, password: Array("Ab1!".utf8))
  check(done.code == 0, "\(done)")
  check(done.boxed == [1, 2, 3] && done.sealed == [4, 5], "\(done.boxed) \(done.sealed)")
  check(done.lengths == [3, 2], "\(done.lengths)")
  check(platform.setups.count == 1, "\(platform.setups.count) calls")
  check(platform.setupPasswords == [Array("Ab1!".utf8)], "\(platform.setupPasswords)")
  check(platform.setups.allSatisfy { $0.password.isWiped }, "the password was not wiped")
  check(
    platform.setups.first.map { $0.target == deviceId(1) && $0.viewer == deviceId(2) } == true,
    "the ids")
  check(platform.setups.first?.viewerKey == aPublicKey, "the key")

  // The largest box and sealed key fit the room that the interface gives.
  platform.unlockMade = .made(
    boxed: [UInt8](repeating: 7, count: UnlockLimits.boxMax),
    sealed: [UInt8](repeating: 8, count: UnlockLimits.sealedMax))
  let largest = setUp(table, arena)
  check(largest.code == 0 && largest.boxed.count == UnlockLimits.boxMax, "\(largest.code)")
}

func testAnUnlockThatTheSystemRefusesOrCannotMakeIsToldAsTheInterfaceSays() {
  guard let (platform, table) = unlockingPlatform() else { return }
  let arena = Arena()
  platform.unlockMade = .wrong
  check(setUp(table, arena).code == Int32(BATON_PASSWORD_WRONG), "wrong")
  platform.unlockMade = .unsupported
  check(setUp(table, arena).code == Int32(BATON_PASSWORD_UNSUPPORTED), "unsupported")
  platform.unlockMade = .failed("This Mac could not put the password in a box.")
  let failed = setUp(table, arena)
  check(failed.code == Int32(BATON_E_PLATFORM), "failed: \(failed.code)")
  check(failed.sentence == "This Mac could not put the password in a box.", failed.sentence)
  check(failed.lengths == [0, 0], "the lengths of a failure: \(failed.lengths)")
  // A box that does not fit what the interface allows is told as a failure, and writes nothing.
  platform.unlockMade = .made(boxed: [], sealed: [1])
  let empty = setUp(table, arena)
  check(empty.code == Int32(BATON_E_PLATFORM), "an empty box: \(empty.code)")
  check(!empty.sentence.isEmpty && empty.lengths == [0, 0], "\(empty.sentence) \(empty.lengths)")
  platform.unlockMade = .made(
    boxed: [1], sealed: [UInt8](repeating: 1, count: UnlockLimits.sealedMax + 1))
  check(setUp(table, arena).code == Int32(BATON_E_PLATFORM), "a sealed key that is too big")
  check(platform.setups.allSatisfy { $0.password.isWiped }, "a password was not wiped")
}

func testASetupThatCannotBeReadIsRefusedAndReachesNothing() {
  guard let (platform, table) = unlockingPlatform(), let call = table.unlock_setup else { return }
  let arena = Arena()
  let refused = Int32(BATON_E_ARGUMENT)
  check(setUp(table, arena, password: []).code == refused, "no password")
  check(
    setUp(table, arena, password: [UInt8](repeating: 1, count: UnlockLimits.passwordMax + 1)).code
      == refused, "a password that is too long")
  check(setUp(table, arena, madeSize: 8).code == refused, "room that ends early")
  check(setUp(table, arena, messageSize: 4).code == refused, "a message that is too short")
  let short = arena.shortStruct(BatonUnlockSetup.self, size: 16)
  check(setUp(table, arena, setup: short).code == refused, "a setup that ends early")
  let noPointer = arena.unlockSetup(password: [1])
  noPointer.pointee.viewer_key = BatonBytes(ptr: nil, len: 65)
  check(setUp(table, arena, setup: noPointer).code == refused, "a key with a length and no pointer")
  _ = withMessage { message in
    let made = arena.shortStruct(BatonUnlockMade.self, size: MemoryLayout<BatonUnlockMade>.size)
    let request = arena.unlockSetup(password: [1])
    check(call(table.context, nil, made, message) == refused, "no setup")
    check(call(table.context, request, nil, message) == refused, "no room")
    check(call(nil, request, made, message) == refused, "no context")
    check(call(table.context, request, made, nil) == refused, "no message")
    return 0
  }
  // A made struct with no room for the box.
  let bare = arena.shortStruct(BatonUnlockMade.self, size: MemoryLayout<BatonUnlockMade>.size)
  _ = withMessage { message in
    let code = call(table.context, arena.unlockSetup(password: [1]), bare, message)
    check(code == Int32(BATON_E_PLATFORM), "no place to write to: \(code)")
    return 0
  }
  // Only the call with no place to write to reached the platform, and its password was wiped.
  check(platform.setups.count == 1, "\(platform.setups.count) calls")
  check(platform.setups.allSatisfy { $0.password.isWiped }, "a password was not wiped")
}

// MARK: - Unlocking

private func unlockCall(
  _ table: BatonSharerVTable, _ picked: UnsafeMutableRawPointer?,
  _ attempt: UnsafePointer<BatonUnlock>?
) -> UInt32 {
  guard let call = table.unlock else { return UInt32.max }
  return call(picked, attempt)
}

func testEachOutcomeOfAnUnlockIsTheNumberThatTheRustSideExpects() {
  guard let (_, table) = unlockingPlatform(), let release = table.release else { return }
  let window = FakeSharedWindow()
  guard let picked = chosen(window) else { return check(false) }
  let arena = Arena()
  let expected: [(UnlockOutcome, UInt32)] = [
    (.unlocked, UInt32(BATON_UNLOCK_UNLOCKED)), (.rejected, UInt32(BATON_UNLOCK_REJECTED)),
    (.stale, UInt32(BATON_UNLOCK_STALE)), (.failed, UInt32(BATON_UNLOCK_FAILED)),
    (.notLocked, UInt32(BATON_UNLOCK_NOT_LOCKED)), (.notPrivate, UInt32(BATON_UNLOCK_NOT_PRIVATE)),
  ]
  for (outcome, number) in expected {
    window.unlockOutcome = outcome
    let got = unlockCall(table, picked, arena.unlockAttempt(boxed: [9, 8, 7]))
    check(got == number, "\(outcome) came back as \(got)")
    check(outcome.number == number, "\(outcome).number is \(outcome.number)")
  }
  check(window.unlocks.count == 6, "\(window.unlocks.count) attempts")
  check(window.unlockKeys.allSatisfy { $0 == goodBoxKey }, "the window saw another key")
  check(window.unlocks.allSatisfy { $0.boxKey.isWiped }, "the key of the box was not wiped")
  check(window.unlocks.first?.boxed == [9, 8, 7], "the box")
  check(window.unlocks.first?.target == deviceId(1) && window.unlocks.first?.viewer == deviceId(2))
  check(window.unlocks.first?.viewerKey == aPublicKey, "the key")
  release(picked)
}

func testAnAttemptThatCannotBeReadOrHasNoWindowIsAFailureThatReachesNothing() {
  guard let (_, table) = unlockingPlatform(), let release = table.release else { return }
  let window = FakeSharedWindow()
  window.unlockOutcome = .unlocked
  guard let picked = chosen(window) else { return check(false) }
  let arena = Arena()
  let failed = UInt32(BATON_UNLOCK_FAILED)
  check(unlockCall(table, picked, nil) == failed, "no attempt")
  check(unlockCall(table, nil, arena.unlockAttempt()) == failed, "no window")
  check(unlockCall(table, picked, arena.unlockAttempt(sealing: 5)) == failed, "another sealing")
  check(unlockCall(table, picked, arena.unlockAttempt(boxKey: [1, 2, 3])) == failed, "a short key")
  check(unlockCall(table, picked, arena.unlockAttempt(boxed: [])) == failed, "no box")
  check(
    unlockCall(table, picked, arena.shortStruct(BatonUnlock.self, size: 16)) == failed,
    "an attempt that ends early")
  check(window.unlocks.isEmpty, "the window was asked about an attempt that cannot be read")
  // And one that can be read gets through.
  check(unlockCall(table, picked, arena.unlockAttempt()) == UInt32(BATON_UNLOCK_UNLOCKED))
  check(window.unlocks.count == 1)
  release(picked)
}

func testAWindowThatCannotUnlockSaysItFailedAndStillWipesTheKey() {
  guard let (_, table) = unlockingPlatform(), let release = table.release else { return }
  // A window from before unlocking: the default says that it failed.
  let window = WindowThatOnlyEnds()
  guard let picked = chosen(window) else { return check(false) }
  let arena = Arena()
  check(unlockCall(table, picked, arena.unlockAttempt()) == UInt32(BATON_UNLOCK_FAILED))
  withExtendedLifetime(arena) {}
  release(picked)
}
