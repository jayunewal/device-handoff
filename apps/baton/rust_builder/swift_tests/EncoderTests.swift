// SPDX-License-Identifier: Apache-2.0

// Tests of BatonH264Encoder.swift with VideoToolbox itself, which only a Mac or an iPhone has: on
// other systems this file is empty. They check what the C interface is promised of a picture (the
// units behind lengths of four bytes, the parameter sets with a key frame and with no other frame,
// a key frame whenever one is asked for) and that the decoder of the viewer takes what the encoder
// makes, so that the two halves of the app are tried against each other on the machine that has
// them both.

#if canImport(VideoToolbox)

import CoreMedia
import CoreVideo
import Foundation
import VideoToolbox

private let width = 320
private let height = 240

/// An image of the kind that the capture of a window gives: two planes, with a pattern in them that
/// changes with `seed`, so that two images of different seeds differ.
private func image(seed: Int) -> CVPixelBuffer? {
  var made: CVPixelBuffer?
  guard
    CVPixelBufferCreate(
      kCFAllocatorDefault, width, height, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, nil,
      &made) == kCVReturnSuccess,
    let buffer = made, CVPixelBufferLockBaseAddress(buffer, []) == kCVReturnSuccess
  else { return nil }
  defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
  for plane in 0..<2 {
    guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane) else { return nil }
    let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane)
    let columns = CVPixelBufferGetWidthOfPlane(buffer, plane) * (plane == 0 ? 1 : 2)
    for y in 0..<CVPixelBufferGetHeightOfPlane(buffer, plane) {
      let row = (base + y * stride).assumingMemoryBound(to: UInt8.self)
      for x in 0..<columns {
        row[x] = UInt8(truncatingIfNeeded: x * 3 + y * 5 + seed * 29 + plane * 64)
      }
    }
  }
  return buffer
}

/// The types of the units of a sample that has them behind lengths of `lengthSize` bytes, or nil if
/// the lengths do not add up to the sample.
private func unitTypes(of sample: Data, lengthSize: Int) -> [UInt8]? {
  let bytes = [UInt8](sample)
  var types: [UInt8] = []
  var at = 0
  while at < bytes.count {
    guard bytes.count - at >= lengthSize else { return nil }
    var length = 0
    for byte in bytes[at..<(at + lengthSize)] { length = length << 8 | Int(byte) }
    at += lengthSize
    guard length >= 1, bytes.count - at >= length else { return nil }
    types.append(bytes[at] & 0x1f)
    at += length
  }
  return types
}

private func makeEncoder(_ picture: PictureFormat = PictureFormat(width: 320, height: 240, fps: 30))
  -> H264Encoder?
{
  do {
    return try H264Encoder(format: picture)
  } catch {
    check(false, "VideoToolbox makes no encoder for \(picture): \(error)")
    return nil
  }
}

/// Checks that `picture` is a key frame as the C interface wants it: a slice of an IDR picture,
/// behind lengths of four bytes, and both kinds of parameter set.
private func checkKeyFrame(_ picture: EncodedPicture, _ what: String, line: UInt = #line) {
  check(picture.lengthSize == 4, "\(what): the lengths are of \(picture.lengthSize) bytes", line: line)
  let types = unitTypes(of: picture.sample, lengthSize: Int(picture.lengthSize))
  check(types != nil, "\(what): the lengths do not add up to the picture", line: line)
  check(types?.contains(5) == true, "\(what): no slice of an IDR picture in \(String(describing: types))", line: line)
  let sets = picture.sets.map { $0.first.map { $0 & 0x1f } }
  check(sets == [7, 8], "\(what): the parameter sets are \(sets)", line: line)
}

/// Checks that `picture` follows another: no IDR slice and no parameter sets.
private func checkDeltaFrame(_ picture: EncodedPicture, _ what: String, line: UInt = #line) {
  check(picture.lengthSize == 4, "\(what): the lengths are of \(picture.lengthSize) bytes", line: line)
  let types = unitTypes(of: picture.sample, lengthSize: Int(picture.lengthSize))
  check(types != nil, "\(what): the lengths do not add up to the picture", line: line)
  check(types?.contains(5) == false, "\(what): a slice of an IDR picture in \(String(describing: types))", line: line)
  check(picture.sets.isEmpty, "\(what): it has \(picture.sets.count) parameter sets", line: line)
}

func testTheEncoderMakesAKeyFrameWithItsParameterSetsAndTheFramesAfterItWithoutThem() {
  guard let encoder = makeEncoder(), let first = image(seed: 0) else {
    check(false, "no encoder or no image")
    return
  }
  defer { encoder.invalidate() }
  do {
    let key = try encoder.encode(first, key: true, captureTime: 1_000)
    checkKeyFrame(key, "the first picture")
    check(key.captureTime == 1_000, "the capture time is \(key.captureTime)")
    let delta = try encoder.encode(first, key: false, captureTime: 34_000)
    checkDeltaFrame(delta, "the picture after it")
    check(delta.captureTime == 34_000, "the capture time is \(delta.captureTime)")
    check(!delta.sample.isEmpty && !key.sample.isEmpty)
  } catch {
    check(false, "the encoder failed: \(error)")
  }
}

func testTheEncoderMakesAKeyFrameWheneverOneIsAskedFor() {
  guard let encoder = makeEncoder(), let one = image(seed: 0), let changed = image(seed: 3) else {
    check(false, "no encoder or no image")
    return
  }
  defer { encoder.invalidate() }
  do {
    checkKeyFrame(try encoder.encode(one, key: true, captureTime: 1_000), "the first key frame")
    checkDeltaFrame(try encoder.encode(one, key: false, captureTime: 2_000), "a delta frame")
    checkDeltaFrame(try encoder.encode(changed, key: false, captureTime: 3_000), "another")
    // The same image again, as when the viewer lost a picture and the window did not change.
    checkKeyFrame(try encoder.encode(changed, key: true, captureTime: 3_000), "the second key frame")
    checkDeltaFrame(try encoder.encode(changed, key: false, captureTime: 4_000), "one after that")
  } catch {
    check(false, "the encoder failed: \(error)")
  }
}

func testTwoPicturesOfTheSameMomentAreBothMade() {
  // A window that did not change is made into a key frame again with the capture time of the
  // image, so the second has the time of the first. The encoder wants every time to be later.
  guard let encoder = makeEncoder(), let one = image(seed: 0) else {
    check(false, "no encoder or no image")
    return
  }
  defer { encoder.invalidate() }
  do {
    let first = try encoder.encode(one, key: true, captureTime: 5_000)
    let second = try encoder.encode(one, key: true, captureTime: 5_000)
    checkKeyFrame(first, "the first")
    checkKeyFrame(second, "the second")
    check(first.captureTime == 5_000 && second.captureTime == 5_000, "the capture times are kept")
    // And a time that is earlier than the one before.
    let third = try encoder.encode(one, key: false, captureTime: 1_000)
    checkDeltaFrame(third, "the third")
    check(third.captureTime == 1_000)
  } catch {
    check(false, "the encoder failed: \(error)")
  }
}

func testAnEncoderThatIsStartedAnewMakesAKeyFrameAndOneLetGoOfRefusesToEncode() {
  guard let encoder = makeEncoder(), let one = image(seed: 0) else {
    check(false, "no encoder or no image")
    return
  }
  do {
    checkKeyFrame(try encoder.encode(one, key: true, captureTime: 1_000), "before")
    try encoder.reset()
    checkKeyFrame(try encoder.encode(one, key: true, captureTime: 2_000), "after it started anew")
    encoder.invalidate()
    do {
      _ = try encoder.encode(one, key: true, captureTime: 3_000)
      check(false, "an encoder that was let go of made a picture")
    } catch EncodeFailure.stopped {
      check(true)
    } catch {
      check(false, "an encoder that was let go of failed with \(error), not with .stopped")
    }
    // Letting go again is harmless.
    encoder.invalidate()
  } catch {
    check(false, "the encoder failed: \(error)")
  }
}

func testAnEncoderIsNotMadeForASizeThatTheProtocolDoesNotAllow() {
  for size in [(1, 1), (0, 240), (320, 1), (3842, 240), (320, 3842)] {
    let format = PictureFormat(width: UInt32(size.0), height: UInt32(size.1), fps: 30)
    check((try? H264Encoder(format: format)) == nil, "an encoder was made for \(size)")
  }
}

/// What the decoder of the viewer made of the picture, waited for.
private func decodedBy(_ decoder: H264Decoder, _ picture: EncodedPicture, number: UInt64)
  -> Result<CVPixelBuffer, DecodeFailure>?
{
  let answered = DispatchSemaphore(value: 0)
  var answer: Result<CVPixelBuffer, DecodeFailure>?
  let taken = picture.sample.withUnsafeBytes { bytes in
    decoder.decode(bytes, number: number) { result in
      answer = result
      answered.signal()
    }
  }
  guard taken, answered.wait(timeout: .now() + 10) == .success else { return nil }
  return answer
}

func testTheDecoderOfTheViewerTakesWhatTheEncoderMakes() {
  guard let encoder = makeEncoder(), let one = image(seed: 0), let changed = image(seed: 5) else {
    check(false, "no encoder or no image")
    return
  }
  defer { encoder.invalidate() }
  let decoder = H264Decoder(pixelFormat: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
  do {
    let key = try encoder.encode(one, key: true, captureTime: 1_000)
    guard key.sets.count == 2 else {
      check(false, "the key frame has \(key.sets.count) parameter sets")
      return
    }
    check(
      decoder.prepare(ParameterSets(sequence: [key.sets[0]], picture: [key.sets[1]])),
      "the decoder takes no format from the parameter sets of the encoder")
    for (number, picture) in [
      (UInt64(0), key),
      (1, try encoder.encode(one, key: false, captureTime: 2_000)),
      (2, try encoder.encode(changed, key: false, captureTime: 3_000)),
    ] {
      switch decodedBy(decoder, picture, number: number) {
      case .success(let buffer)?:
        let size = (CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer))
        check(size == (width, height), "picture \(number) is \(size), not \((width, height))")
      case let unexpected:
        check(false, "picture \(number) did not come out: \(String(describing: unexpected))")
      }
    }
  } catch {
    check(false, "the encoder failed: \(error)")
  }
}

func testThePumpMakesPicturesOfImagesWithTheEncoderOfTheSystem() {
  // The whole of the platform's side but the capture and the library: the pump asks the session
  // what to make, the encoder makes it, and the session is given a picture it can send.
  guard let encoder = makeEncoder(), let one = image(seed: 0), let changed = image(seed: 2) else {
    check(false, "no encoder or no image")
    return
  }
  let session = FakeSession()
  session.begins = [.key, .delta, .recovery]
  let pump = FramePump(session: session, encoder: encoder)
  for (at, picture) in [one, changed, changed].enumerated() {
    session.micros = UInt64(1_000 * (at + 1))
    pump.captured(picture)
  }
  check(session.sent.count == 3, "\(session.sent.count) pictures were sent")
  if session.sent.count == 3 {
    checkKeyFrame(session.sent[0], "the key frame")
    checkDeltaFrame(session.sent[1], "the delta frame")
    // A recovery frame is made as a key frame, since nothing is marked as a long-term reference.
    checkKeyFrame(session.sent[2], "the recovery frame")
    check(session.sent.map { $0.captureTime } == [1_000, 2_000, 3_000])
  }
  check(session.violations.isEmpty, "\(session.violations)")
  check(session.ended.isEmpty && !pump.over)
  pump.stop()
}

#endif
