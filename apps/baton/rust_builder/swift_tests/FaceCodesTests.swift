// SPDX-License-Identifier: Apache-2.0

// Holds the numbers that the sorting of BatonFace.swift knows to the constants of the SDK. A wrong
// number there would only show on a device, as an error that is sorted wrongly, so this test runs
// wherever LocalAuthentication and Security are, which is on a Mac. FaceTests.swift holds the same
// numbers to what was written down, on every computer.

#if canImport(LocalAuthentication) && canImport(Security)
  import Foundation
  import LocalAuthentication
  import Security

  func testTheNumbersOfTheSystemThatTheSortingKnowsAreThoseOfItsSdk() {
    check(FaceSystemCodes.localAuthentication == LAError.errorDomain, LAError.errorDomain)
    check(FaceSystemCodes.osStatus == NSOSStatusErrorDomain, NSOSStatusErrorDomain)

    check(FaceSystemCodes.authenticationFailed == LAError.Code.authenticationFailed.rawValue)
    check(FaceSystemCodes.userCancel == LAError.Code.userCancel.rawValue)
    check(FaceSystemCodes.userFallback == LAError.Code.userFallback.rawValue)
    check(FaceSystemCodes.systemCancel == LAError.Code.systemCancel.rawValue)
    check(FaceSystemCodes.passcodeNotSet == LAError.Code.passcodeNotSet.rawValue)
    check(FaceSystemCodes.biometryNotAvailable == LAError.Code.biometryNotAvailable.rawValue)
    check(FaceSystemCodes.biometryNotEnrolled == LAError.Code.biometryNotEnrolled.rawValue)
    check(FaceSystemCodes.biometryLockout == LAError.Code.biometryLockout.rawValue)
    check(FaceSystemCodes.appCancel == LAError.Code.appCancel.rawValue)
    check(FaceSystemCodes.invalidContext == LAError.Code.invalidContext.rawValue)
    check(FaceSystemCodes.notInteractive == LAError.Code.notInteractive.rawValue)

    check(FaceSystemCodes.unimplemented == Int(errSecUnimplemented))
    check(FaceSystemCodes.param == Int(errSecParam))
    check(FaceSystemCodes.userCanceled == Int(errSecUserCanceled))
    check(FaceSystemCodes.authFailed == Int(errSecAuthFailed))
    check(FaceSystemCodes.itemNotFound == Int(errSecItemNotFound))
    check(FaceSystemCodes.interactionNotAllowed == Int(errSecInteractionNotAllowed))

    // An error that the system makes, read as the sorting reads it.
    let cancelled = NSError(domain: LAError.errorDomain, code: LAError.Code.userCancel.rawValue)
    check(FaceDecline(system: FaceSystemError.chain(of: cancelled)) == .declined)
    let gone = NSError(domain: NSOSStatusErrorDomain, code: Int(errSecItemNotFound))
    check(FaceDecline(system: FaceSystemError.chain(of: gone)) == .keyLost)
  }
#endif
