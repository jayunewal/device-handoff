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

// MARK: - The functions for sharing a window

/// The table that the Swift code registered to share windows, if the stub took it.
var registeredSharer: BatonSharerVTable?
/// What the stub answers to a registration of the sharer.
var sharerRegistrationCode: Int32 = 0

/// What the Swift code gave as the answer to a pick, as the Rust side reads it.
struct PickDone {
  var request: UInt64
  /// The window the person chose, if there was one.
  var chosen: Chosen?
  /// The sentence of a picker that failed, if there was one.
  var failure: String?

  struct Chosen {
    var size: UInt32
    var width: UInt32
    var height: UInt32
    var kind: UInt32
    var context: UnsafeMutableRawPointer?
    var title: String
    var app: String
  }
}

/// The answers to picks, in order.
var pickDones: [PickDone] = []
/// What the stub answers to an answer.
var pickDoneCode: Int32 = 0

@_cdecl("baton_stream_register_sharer")
func stubRegisterSharer(_ table: UnsafePointer<BatonSharerVTable>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  if sharerRegistrationCode == 0, let table {
    registeredSharer = table.pointee
  }
  return sharerRegistrationCode
}

@_cdecl("baton_stream_pick_done")
func stubPickDone(
  _ request: UInt64, _ picked: UnsafePointer<BatonPicked>?, _ failure: UnsafePointer<BatonBytes>?
) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  func text(_ bytes: BatonBytes) -> String {
    guard bytes.len > 0, let start = bytes.ptr else { return "" }
    return String(decoding: UnsafeBufferPointer(start: start, count: bytes.len), as: UTF8.self)
  }
  pickDones.append(
    PickDone(
      request: request,
      chosen: picked.map {
        PickDone.Chosen(
          size: $0.pointee.size, width: $0.pointee.width, height: $0.pointee.height,
          kind: $0.pointee.kind, context: $0.pointee.context, title: text($0.pointee.title), app: text($0.pointee.app))
      },
      failure: failure.map { text($0.pointee) }))
  return pickDoneCode
}

/// What the Swift code handed to `baton_stream_source_sent`, as the Rust side reads it.
struct SentRecord: Equatable {
  var size: UInt32
  var flags: UInt32
  var captureTime: UInt64
  var lengthSize: UInt32
  var sample: [UInt8]
  var sets: [[UInt8]]
  /// nil for a geometry that is all zero, which says nothing.
  var geometry: SentGeometry? = nil
}

/// The geometry of a picture, as the Rust side reads it.
struct SentGeometry: Equatable {
  var windowWidth: Double
  var windowHeight: Double
  var pictureWidth: UInt32
  var pictureHeight: UInt32
  var content: [Double]
}

/// What the Swift code gave `baton_stream_source_sound_begin`, as the Rust side reads it.
struct SoundRecord: Equatable {
  var size: UInt32
  var codec: UInt32
  var rate: UInt32
  var channels: UInt32
  var samples: UInt32
}

/// What the Swift code gave `baton_stream_source_sound`, as the Rust side reads it.
struct SoundPacketRecord: Equatable {
  var size: UInt32
  var captureTime: UInt64
  var bytes: [UInt8]
}

/// A call about a window that is shared.
enum SourceCall: Equatable {
  case clock(UInt64)
  case begin(UInt64)
  case cancel(UInt64)
  case recoveryPending(UInt64)
  case sent(UInt64, SentRecord)
  case changed(UInt64, UInt64)
  case pause(UInt64, UInt32)
  case resume(UInt64)
  /// nil is a NULL pointer for the sentence.
  case end(UInt64, String?)
  case soundBegin(UInt64, SoundRecord)
  case soundSilent(UInt64, UInt32)
  case sound(UInt64, SoundPacketRecord)
}

/// What the stubs of the calls about a window answer.
struct SourceAnswers {
  var clockCode: Int32 = 0
  var micros: UInt64 = 777
  var beginCode: Int32 = 0
  var beginKind: UInt32 = BATON_BEGIN_DELTA
  /// Whether a yes to `begin` comes with the kind; a library that forgets it breaks the contract.
  var beginWritesKind = true
  var cancelCode: Int32 = 0
  var pendingCode: Int32 = 0
  var pending: UInt32 = 0
  var sentCode: Int32 = 0
  var outcome: UInt32 = BATON_SENT
  var number: UInt64 = 5
  var changedCode: Int32 = 0
  var pauseCode: Int32 = 0
  var resumeCode: Int32 = 0
  var endCode: Int32 = 0
  var soundBeginCode: Int32 = 0
  var soundSilentCode: Int32 = 0
  var soundCode: Int32 = 0
  var soundOutcome: UInt32 = BATON_SOUND_SENT
  var soundNumber: UInt64 = 7
}

var sourceCalls: [SourceCall] = []
var sourceAnswers = SourceAnswers()

func resetSourceStubs() {
  stubLock.lock()
  sourceCalls = []
  sourceAnswers = SourceAnswers()
  pickDones = []
  pickDoneCode = 0
  stubLock.unlock()
}

@_cdecl("baton_stream_source_clock")
func stubSourceClock(_ link: UInt64, _ micros: UnsafeMutablePointer<UInt64>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.clock(link))
  if sourceAnswers.clockCode == 0 { micros?.pointee = sourceAnswers.micros }
  return sourceAnswers.clockCode
}

@_cdecl("baton_stream_source_begin")
func stubSourceBegin(_ link: UInt64, _ kind: UnsafeMutablePointer<UInt32>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.begin(link))
  if sourceAnswers.beginCode == 0 && sourceAnswers.beginWritesKind {
    kind?.pointee = sourceAnswers.beginKind
  }
  return sourceAnswers.beginCode
}

@_cdecl("baton_stream_source_cancel")
func stubSourceCancel(_ link: UInt64) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.cancel(link))
  return sourceAnswers.cancelCode
}

@_cdecl("baton_stream_source_recovery_pending")
func stubSourceRecoveryPending(_ link: UInt64, _ pending: UnsafeMutablePointer<UInt32>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.recoveryPending(link))
  if sourceAnswers.pendingCode == 0 { pending?.pointee = sourceAnswers.pending }
  return sourceAnswers.pendingCode
}

@_cdecl("baton_stream_source_sent")
func stubSourceSent(
  _ link: UInt64, _ picture: UnsafePointer<BatonPicture>?, _ outcome: UnsafeMutablePointer<UInt32>?,
  _ number: UnsafeMutablePointer<UInt64>?
) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  func bytes(_ bytes: BatonBytes) -> [UInt8] {
    guard bytes.len > 0, let start = bytes.ptr else { return [] }
    return Array(UnsafeBufferPointer(start: start, count: bytes.len))
  }
  if let picture {
    let c = picture.pointee
    let sets: [[UInt8]] =
      c.set_count == 0 || c.sets == nil
      ? [] : UnsafeBufferPointer(start: c.sets, count: c.set_count).map { bytes($0) }
    let g = c.geometry
    let silent = g.window_width == 0 && g.window_height == 0 && g.picture_width == 0
      && g.picture_height == 0 && g.content.x == 0 && g.content.y == 0 && g.content.width == 0
      && g.content.height == 0
    sourceCalls.append(
      .sent(
        link,
        SentRecord(
          size: c.size, flags: c.flags, captureTime: c.capture_time, lengthSize: c.length_size,
          sample: bytes(c.sample), sets: sets,
          geometry: silent
            ? nil
            : SentGeometry(
              windowWidth: g.window_width, windowHeight: g.window_height,
              pictureWidth: g.picture_width, pictureHeight: g.picture_height,
              content: [g.content.x, g.content.y, g.content.width, g.content.height]))))
  }
  if sourceAnswers.sentCode == 0 {
    outcome?.pointee = sourceAnswers.outcome
    number?.pointee = sourceAnswers.number
  }
  return sourceAnswers.sentCode
}

@_cdecl("baton_stream_source_end")
func stubSourceEnd(_ link: UInt64, _ sentence: UnsafePointer<BatonBytes>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  var text: String?
  if let sentence {
    let c = sentence.pointee
    text =
      c.len > 0 && c.ptr != nil
      ? String(decoding: UnsafeBufferPointer(start: c.ptr, count: c.len), as: UTF8.self) : ""
  }
  sourceCalls.append(.end(link, text))
  return sourceAnswers.endCode
}

@_cdecl("baton_stream_source_changed")
func stubSourceChanged(_ link: UInt64, _ captureTime: UInt64) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.changed(link, captureTime))
  return sourceAnswers.changedCode
}

@_cdecl("baton_stream_source_pause")
func stubSourcePause(_ link: UInt64, _ reason: UInt32) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.pause(link, reason))
  return sourceAnswers.pauseCode
}

@_cdecl("baton_stream_source_resume")
func stubSourceResume(_ link: UInt64) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.resume(link))
  return sourceAnswers.resumeCode
}

@_cdecl("baton_stream_source_sound_begin")
func stubSourceSoundBegin(_ link: UInt64, _ sound: UnsafePointer<BatonSound>?) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  if let sound {
    let c = sound.pointee
    sourceCalls.append(
      .soundBegin(
        link,
        SoundRecord(
          size: c.size, codec: c.codec, rate: c.rate, channels: c.channels, samples: c.samples)))
  }
  return sourceAnswers.soundBeginCode
}

@_cdecl("baton_stream_source_sound_silent")
func stubSourceSoundSilent(_ link: UInt64, _ reason: UInt32) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  sourceCalls.append(.soundSilent(link, reason))
  return sourceAnswers.soundSilentCode
}

@_cdecl("baton_stream_source_sound")
func stubSourceSound(
  _ link: UInt64, _ packet: UnsafePointer<BatonSoundPacket>?,
  _ outcome: UnsafeMutablePointer<UInt32>?, _ number: UnsafeMutablePointer<UInt64>?
) -> Int32 {
  stubLock.lock()
  defer { stubLock.unlock() }
  if let packet {
    let c = packet.pointee
    let bytes: [UInt8] =
      c.bytes.len > 0 && c.bytes.ptr != nil
      ? Array(UnsafeBufferPointer(start: c.bytes.ptr, count: c.bytes.len)) : []
    sourceCalls.append(
      .sound(link, SoundPacketRecord(size: c.size, captureTime: c.capture_time, bytes: bytes)))
  }
  if sourceAnswers.soundCode == 0 {
    outcome?.pointee = sourceAnswers.soundOutcome
    number?.pointee = sourceAnswers.soundNumber
  }
  return sourceAnswers.soundCode
}
