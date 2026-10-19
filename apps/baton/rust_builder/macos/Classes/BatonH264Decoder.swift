// SPDX-License-Identifier: Apache-2.0

// Decoding of H.264 with VideoToolbox: the hardware decoder where the device has one, and Apple's
// software decoder where it has not. It imports no more than the frameworks of the system, so
// that a Mac can build and test it without Flutter (apps/baton/rust_builder/swift_tests).

#if canImport(VideoToolbox)

import CoreMedia
import CoreVideo
import Foundation
import VideoToolbox

final class H264Decoder: PictureDecoder {
  typealias Picture = CVPixelBuffer

  /// The format of the pictures that come out. BGRA is what Flutter draws as it is; a test asks
  /// for the planes that the decoder makes, to compare them with those of another decoder.
  private let pixelFormat: OSType

  private let lock = NSLock()
  private var format: CMVideoFormatDescription?
  private var session: VTDecompressionSession?

  init(pixelFormat: OSType = kCVPixelFormatType_32BGRA) {
    self.pixelFormat = pixelFormat
  }

  deinit {
    stop()
  }

  func prepare(_ sets: ParameterSets) -> Bool {
    guard let description = H264Decoder.describe(sets) else { return false }
    lock.lock()
    let old = session
    let oldFormat = format
    lock.unlock()
    // A session goes on when the pictures are the same size and it can take the new format, for
    // example for a new set of parameters after the encoder started again. Any other change,
    // above all a window that was resized, gets a new session, made when the next frame comes:
    // the buffers of the old one are the size of the old pictures.
    var keep = false
    if let old, let oldFormat, H264Decoder.sameSize(oldFormat, description) {
      keep = VTDecompressionSessionCanAcceptFormatDescription(old, formatDescription: description)
    }
    lock.lock()
    format = description
    if !keep { session = nil }
    lock.unlock()
    if !keep, let old { VTDecompressionSessionInvalidate(old) }
    return true
  }

  func decode(
    _ sample: UnsafeRawBufferPointer, number: UInt64,
    then finish: @escaping (Result<CVPixelBuffer, DecodeFailure>) -> Void
  ) -> Bool {
    guard let bytes = sample.baseAddress, !sample.isEmpty,
      let (format, session) = current()
    else { return false }

    // The decoder reads the sample after this call returns, so it gets a copy.
    var blockOut: CMBlockBuffer?
    guard
      CMBlockBufferCreateWithMemoryBlock(
        allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: sample.count,
        blockAllocator: kCFAllocatorDefault, customBlockSource: nil, offsetToData: 0,
        dataLength: sample.count, flags: kCMBlockBufferAssureMemoryNowFlag,
        blockBufferOut: &blockOut) == noErr,
      let block = blockOut,
      CMBlockBufferReplaceDataBytes(
        with: bytes, blockBuffer: block, offsetIntoDestination: 0, dataLength: sample.count)
        == noErr
    else { return false }

    var size = sample.count
    var bufferOut: CMSampleBuffer?
    guard
      CMSampleBufferCreateReady(
        allocator: kCFAllocatorDefault, dataBuffer: block, formatDescription: format,
        sampleCount: 1, sampleTimingEntryCount: 0, sampleTimingArray: nil,
        sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &bufferOut) == noErr,
      let buffer = bufferOut
    else { return false }

    // kVTDecodeFrame_EnableAsynchronousDecompression: the call returns when the frame is queued.
    let asynchronous = VTDecodeFrameFlags(rawValue: 1 << 0)
    let queued = VTDecompressionSessionDecodeFrame(
      session, sampleBuffer: buffer, flags: asynchronous, infoFlagsOut: nil
    ) { [weak self] status, _, image, _, _ in
      if status != noErr {
        // A decoder that the system took away, or that broke, is not asked again: the next key
        // frame makes a new one.
        if status == kVTInvalidSessionErr || status == kVTVideoDecoderMalfunctionErr {
          self?.forget(session)
        }
        finish(.failure(.status(status)))
      } else if let image {
        finish(.success(image))
      } else {
        finish(.failure(.dropped))
      }
    }
    if queued == noErr { return true }
    // The system takes a decoder away from an app that goes to the background. The next key
    // frame makes a new one.
    if queued == kVTInvalidSessionErr { forget(session) }
    return false
  }

  func stop() {
    lock.lock()
    let old = session
    session = nil
    format = nil
    lock.unlock()
    if let old { VTDecompressionSessionInvalidate(old) }
  }

  /// The format of the pictures that come, and the session that decodes them: made when the first
  /// frame comes, so that `prepare` does not wait for the hardware.
  private func current() -> (CMVideoFormatDescription, VTDecompressionSession)? {
    lock.lock()
    let format = self.format
    let session = self.session
    lock.unlock()
    guard let format else { return nil }
    if let session { return (format, session) }
    guard let made = makeSession(for: format) else { return nil }
    lock.lock()
    // `stop` may have come while the session was made.
    let stopped = self.format == nil
    if !stopped { self.session = made }
    lock.unlock()
    if stopped {
      VTDecompressionSessionInvalidate(made)
      return nil
    }
    return (format, made)
  }

  private func forget(_ gone: VTDecompressionSession) {
    lock.lock()
    let same = session === gone
    if same { session = nil }
    lock.unlock()
    if same { VTDecompressionSessionInvalidate(gone) }
  }

  private func makeSession(for format: CMVideoFormatDescription) -> VTDecompressionSession? {
    let attributes: [String: Any] = [
      kCVPixelBufferPixelFormatTypeKey as String: pixelFormat,
      // Flutter draws a picture with Metal, from the memory the decoder wrote it to.
      kCVPixelBufferMetalCompatibilityKey as String: true,
      kCVPixelBufferIOSurfacePropertiesKey as String: [String: Any](),
    ]
    var sessionOut: VTDecompressionSession?
    let status = VTDecompressionSessionCreate(
      allocator: kCFAllocatorDefault, formatDescription: format, decoderSpecification: nil,
      imageBufferAttributes: attributes as CFDictionary, outputCallback: nil,
      decompressionSessionOut: &sessionOut)
    guard status == noErr, let made = sessionOut else { return nil }
    // Pictures are wanted as soon as they are made, not at the best use of the hardware.
    _ = VTSessionSetProperty(made, key: kVTDecompressionPropertyKey_RealTime, value: kCFBooleanTrue)
    return made
  }

  private static func sameSize(_ a: CMVideoFormatDescription, _ b: CMVideoFormatDescription) -> Bool {
    let first = CMVideoFormatDescriptionGetDimensions(a)
    let second = CMVideoFormatDescriptionGetDimensions(b)
    return first.width == second.width && first.height == second.height
  }

  /// The description of the format that these parameter sets give. nil if VideoToolbox does not
  /// take them, for example because they are not an H.264 profile that it knows.
  static func describe(_ sets: ParameterSets) -> CMVideoFormatDescription? {
    guard !sets.sequence.isEmpty, !sets.picture.isEmpty else { return nil }
    let all = sets.all
    var storage: [UInt8] = []
    var starts: [Int] = []
    var sizes: [Int] = []
    for set in all {
      starts.append(storage.count)
      sizes.append(set.count)
      storage.append(contentsOf: set)
    }
    var descriptionOut: CMFormatDescription?
    let status = storage.withUnsafeBufferPointer { bytes -> OSStatus in
      guard let base = bytes.baseAddress else { return -1 }
      let pointers = starts.map { base + $0 }
      return pointers.withUnsafeBufferPointer { pointerBuffer in
        sizes.withUnsafeBufferPointer { sizeBuffer in
          guard let pointerBase = pointerBuffer.baseAddress, let sizeBase = sizeBuffer.baseAddress
          else { return -1 }
          return CMVideoFormatDescriptionCreateFromH264ParameterSets(
            allocator: kCFAllocatorDefault, parameterSetCount: all.count,
            parameterSetPointers: pointerBase, parameterSetSizes: sizeBase,
            nalUnitHeaderLength: 4, formatDescriptionOut: &descriptionOut)
        }
      }
    }
    return status == noErr ? descriptionOut : nil
  }
}

#endif
