// SPDX-License-Identifier: Apache-2.0

// The two functions of the C interface that the Rust library provides and the Swift code calls,
// played by the tests: the table that the Swift code registers is kept, so that a test can call
// its functions as the Rust side does, and the failures that a decoder reports are counted.

import Foundation

let stubLock = NSLock()
/// The table that the Swift code registered, if the stub took it.
var registeredTable: BatonViewerVTable?
/// What the stub answers to a registration.
var registrationCode: Int32 = 0
/// The links that were reported as failed, in order.
var reportedLinks: [UInt64] = []

@_cdecl("baton_stream_register_viewer")
func stubRegisterViewer(_ table: UnsafePointer<BatonViewerVTable>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  if registrationCode == 0, let table {
    registeredTable = table.pointee
  }
  return registrationCode
}

@_cdecl("baton_stream_link_decode_failed")
func stubLinkDecodeFailed(_ link: UInt64) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  reportedLinks.append(link)
  return 0
}

@_cdecl("baton_stream_abi_version")
func stubAbiVersion() -> UInt32 {
  return BATON_STREAM_ABI_VERSION
}
