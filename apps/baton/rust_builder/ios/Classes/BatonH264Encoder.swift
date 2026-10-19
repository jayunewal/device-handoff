// SPDX-License-Identifier: Apache-2.0

// Encoding of H.264 with VideoToolbox: the hardware encoder where the device has one, and Apple's
// software encoder where it has not. It makes the pictures of a window that this device shares,
// in the form that the C interface takes them (BatonSource.swift): each picture behind lengths of
// four bytes, and the parameter sets of a key frame beside it. It imports no more than the
// frameworks of the system, so that a Mac can build and test it without Flutter
// (apps/baton/rust_builder/swift_tests).

#if canImport(VideoToolbox)

import CoreMedia
import CoreVideo
import Foundation
import VideoToolbox

/// Why a picture could not be made.
enum EncodeFailure: Error, Equatable {
  /// The encoder was let go of.
  case stopped
  /// VideoToolbox said no.
  case status(OSStatus)
  /// VideoToolbox did not give a picture back: it dropped the frame, or did not answer in time.
  case nothingCame
  /// A key frame was asked for and another kind came back.
  case notAKeyFrame
  /// What came back is not a picture that can be passed on.
  case malformed
}

final class H264Encoder: PictureEncoder {
  typealias Image = CVPixelBuffer

  /// How long to wait for VideoToolbox to give a picture back. A picture of a window is a fraction
  /// of a second of work for the slowest encoder; this is for an encoder that was taken away.
  private static let patience: DispatchTimeInterval = .seconds(2)

  /// The key frames and the delta frames are all of this size and rate.
  private let format: PictureFormat

  private let lock = NSLock()
  private var session: VTCompressionSession?
  /// The timestamp of the picture before, so that the next one is later: a window that did not
  /// change is made into a key frame again with the capture time of the picture it was made from.
  private var lastTimestamp: Int64 = -1

  init(format: PictureFormat) throws {
    self.format = format
    session = try H264Encoder.makeSession(for: format)
  }

  deinit {
    invalidate()
  }

  func encode(_ image: CVPixelBuffer, key: Bool, captureTime: UInt64) throws -> EncodedPicture {
    lock.lock()
    let session = self.session
    // A timestamp is in microseconds, and each one is later than the one before.
    let timestamp = max(Int64(clamping: captureTime), lastTimestamp + 1)
    lastTimestamp = timestamp
    lock.unlock()
    guard let session else { throw EncodeFailure.stopped }

    let time = CMTime(value: timestamp, timescale: 1_000_000)
    let properties: CFDictionary? =
      key ? [kVTEncodeFrameOptionKey_ForceKeyFrame as String: true] as CFDictionary : nil
    let waiting = Waiting()
    let queued = VTCompressionSessionEncodeFrame(
      session, imageBuffer: image, presentationTimeStamp: time, duration: .invalid,
      frameProperties: properties, infoFlagsOut: nil
    ) { status, _, sample in
      waiting.finish(status: status, sample: sample)
    }
    guard queued == noErr else { throw EncodeFailure.status(queued) }
    // The encoder is told that no picture follows soon, so that it gives this one back at once.
    let completed = VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: time)
    guard completed == noErr else { throw EncodeFailure.status(completed) }
    guard waiting.wait(H264Encoder.patience) else { throw EncodeFailure.nothingCame }

    let sample = try waiting.sample()
    return try H264Encoder.picture(from: sample, key: key, captureTime: captureTime)
  }

  func reset() throws {
    lock.lock()
    let old = session
    session = nil
    lock.unlock()
    if let old { VTCompressionSessionInvalidate(old) }
    let made = try H264Encoder.makeSession(for: format)
    lock.lock()
    session = made
    lock.unlock()
  }

  func invalidate() {
    lock.lock()
    let old = session
    session = nil
    lock.unlock()
    if let old { VTCompressionSessionInvalidate(old) }
  }

  // MARK: - The session of VideoToolbox

  private static func makeSession(for format: PictureFormat) throws -> VTCompressionSession {
    guard format.width >= 2, format.height >= 2, format.width <= 3840, format.height <= 3840 else {
      throw EncodeFailure.status(kVTParameterErr)
    }
    var specification: [String: Any] = [:]
    #if os(macOS)
    // The hardware encoder where there is one; the software encoder if there is not.
    specification[kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder as String] =
      true
    #endif
    var made: VTCompressionSession?
    let status = VTCompressionSessionCreate(
      allocator: kCFAllocatorDefault, width: Int32(format.width), height: Int32(format.height),
      codecType: kCMVideoCodecType_H264, encoderSpecification: specification as CFDictionary,
      imageBufferAttributes: nil, compressedDataAllocator: nil, outputCallback: nil,
      refcon: nil, compressionSessionOut: &made)
    guard status == noErr, let session = made else { throw EncodeFailure.status(status) }

    let bitrate = targetBitrate(for: format)
    // At most twice the average over any second: a key frame is many times larger than a delta.
    let limits: [Int] = [bitrate / 8 * 2, 1]
    let properties: [(CFString, CFTypeRef)] = [
      // Pictures are wanted as soon as they are made, not at the best use of the hardware.
      (kVTCompressionPropertyKey_RealTime, kCFBooleanTrue),
      (kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel),
      // The pictures come out in the order they went in: the viewer shows what it decodes.
      (kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse),
      (kVTCompressionPropertyKey_AverageBitRate, NSNumber(value: bitrate)),
      (kVTCompressionPropertyKey_DataRateLimits, limits as CFArray),
      (kVTCompressionPropertyKey_ExpectedFrameRate, NSNumber(value: format.fps)),
      // A key frame is made when the viewer needs one and not before: the library asks for it.
      (kVTCompressionPropertyKey_MaxKeyFrameInterval, NSNumber(value: Int32.max)),
      (kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, NSNumber(value: Int32.max)),
    ]
    for (key, value) in properties {
      // A property that this encoder does not know is not a reason to go without it.
      _ = VTSessionSetProperty(session, key: key, value: value)
    }
    _ = VTCompressionSessionPrepareToEncodeFrames(session)
    return session
  }

  // MARK: - What the encoder gives back

  /// The answer of VideoToolbox to one picture, which arrives on a queue of its own.
  private final class Waiting {
    private let arrived = DispatchSemaphore(value: 0)
    private var status: OSStatus = noErr
    private var buffer: CMSampleBuffer?

    func finish(status: OSStatus, sample: CMSampleBuffer?) {
      self.status = status
      self.buffer = sample
      arrived.signal()
    }

    /// Whether the answer arrived within `time`.
    func wait(_ time: DispatchTimeInterval) -> Bool {
      arrived.wait(timeout: .now() + time) == .success
    }

    func sample() throws -> CMSampleBuffer {
      if status != noErr { throw EncodeFailure.status(status) }
      guard let buffer else { throw EncodeFailure.nothingCame }
      return buffer
    }
  }

  /// The picture that a sample of the encoder is, in the form of the C interface.
  static func picture(from sample: CMSampleBuffer, key: Bool, captureTime: UInt64) throws
    -> EncodedPicture
  {
    guard CMSampleBufferDataIsReady(sample), let block = CMSampleBufferGetDataBuffer(sample),
      let description = CMSampleBufferGetFormatDescription(sample)
    else { throw EncodeFailure.malformed }

    let isKey = isKeyFrame(sample)
    if key && !isKey { throw EncodeFailure.notAKeyFrame }

    let length = CMBlockBufferGetDataLength(block)
    var bytes = Data(count: length)
    let copied = bytes.withUnsafeMutableBytes { raw -> OSStatus in
      guard let base = raw.baseAddress else { return kVTParameterErr }
      return CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: base)
    }
    guard length > 0, copied == noErr else { throw EncodeFailure.malformed }

    let (sets, lengthSize) = try parameterSets(of: description)
    // Only a key frame carries its parameter sets: the others are decoded with the ones before.
    return EncodedPicture(
      sample: bytes, lengthSize: lengthSize, sets: isKey ? sets : [], captureTime: captureTime)
  }

  /// Whether the encoder says that the picture can be decoded on its own: it is a picture that is
  /// not marked as depending on another.
  private static func isKeyFrame(_ sample: CMSampleBuffer) -> Bool {
    guard
      let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false)
        as? [[CFString: Any]],
      let first = attachments.first
    else { return true }
    return (first[kCMSampleAttachmentKey_NotSync] as? Bool) != true
  }

  /// The sequence and picture parameter sets of the encoder's format, in this order, each without
  /// a start code, and the number of bytes of each length in the sample.
  private static func parameterSets(of description: CMFormatDescription) throws
    -> ([Data], UInt32)
  {
    var count = 0
    var headerLength: Int32 = 0
    var first: UnsafePointer<UInt8>?
    var size = 0
    let status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
      description, parameterSetIndex: 0, parameterSetPointerOut: &first,
      parameterSetSizeOut: &size, parameterSetCountOut: &count,
      nalUnitHeaderLengthOut: &headerLength)
    guard status == noErr, count > 0, [1, 2, 4].contains(headerLength) else {
      throw EncodeFailure.malformed
    }
    var sets: [Data] = []
    for index in 0..<count {
      var pointer: UnsafePointer<UInt8>?
      var setSize = 0
      let got = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        description, parameterSetIndex: index, parameterSetPointerOut: &pointer,
        parameterSetSizeOut: &setSize, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil)
      guard got == noErr, let pointer, setSize > 0 else { throw EncodeFailure.malformed }
      sets.append(Data(bytes: pointer, count: setSize))
    }
    return (sets, UInt32(headerLength))
  }
}

#endif
