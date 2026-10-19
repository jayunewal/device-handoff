// SPDX-License-Identifier: Apache-2.0

// Tests of BatonH264Decoder.swift with VideoToolbox itself, which only a Mac or an iPhone has: on
// other systems this file is empty. The video is the one of PcmVideo.swift, whose decoded samples
// are known, so a picture that comes out different shows in the first sample that differs.

#if canImport(VideoToolbox)

import CoreMedia
import CoreVideo
import Foundation
import IOSurface
import VideoToolbox

/// The two planes of a picture, row by row without the padding at the end of each row.
private func planes(of buffer: CVPixelBuffer) -> (luma: [[UInt8]], chroma: [[UInt8]])? {
  guard CVPixelBufferGetPlaneCount(buffer) == 2,
    CVPixelBufferLockBaseAddress(buffer, .readOnly) == kCVReturnSuccess
  else { return nil }
  defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }

  func rows(of plane: Int, samplesPerPixel: Int) -> [[UInt8]]? {
    guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, plane) else { return nil }
    let stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, plane)
    let count = CVPixelBufferGetWidthOfPlane(buffer, plane) * samplesPerPixel
    return (0..<CVPixelBufferGetHeightOfPlane(buffer, plane)).map { y in
      let row = (base + y * stride).assumingMemoryBound(to: UInt8.self)
      return Array(UnsafeBufferPointer(start: row, count: count))
    }
  }
  guard let luma = rows(of: 0, samplesPerPixel: 1), let chroma = rows(of: 1, samplesPerPixel: 2)
  else { return nil }
  return (luma, chroma)
}

/// Where two sets of rows first differ, for the message of a check.
private func firstDifference(_ seen: [[UInt8]], _ expected: [[UInt8]]) -> String {
  if seen.count != expected.count { return "\(seen.count) rows, not \(expected.count)" }
  for (y, (a, b)) in zip(seen, expected).enumerated() {
    if a.count != b.count { return "row \(y) has \(a.count) samples, not \(b.count)" }
    if let x = zip(a, b).enumerated().first(where: { $0.element.0 != $0.element.1 })?.offset {
      return "row \(y), sample \(x): \(a[x]), not \(b[x])"
    }
  }
  return "none"
}

/// What the decoder made of `sample`, waited for. nil if the decoder did not take the sample or
/// did not answer in ten seconds.
private func decoded(_ decoder: H264Decoder, _ unit: [UInt8], number: UInt64 = 0)
  -> Result<CVPixelBuffer, DecodeFailure>?
{
  let answered = DispatchSemaphore(value: 0)
  var answer: Result<CVPixelBuffer, DecodeFailure>?
  let taken = PcmVideo.sample(unit).withUnsafeBytes { bytes in
    decoder.decode(bytes, number: number) { result in
      answer = result
      answered.signal()
    }
  }
  guard taken, answered.wait(timeout: .now() + 10) == .success else { return nil }
  return answer
}

private func parameterSets(of video: PcmVideo) -> ParameterSets {
  ParameterSets(
    sequence: [Data(video.sequenceParameterSet)], picture: [Data(video.pictureParameterSet)])
}

/// A decoder that two planes come out of, so that the samples can be compared as they are.
private func planarDecoder(for video: PcmVideo) -> H264Decoder? {
  let decoder = H264Decoder(pixelFormat: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
  return decoder.prepare(parameterSets(of: video)) ? decoder : nil
}

/// Checks that `result` is a picture of the video, with the samples of picture `number`.
private func checkPicture(
  _ result: Result<CVPixelBuffer, DecodeFailure>?, of video: PcmVideo, number: Int, _ what: String,
  line: UInt = #line
) {
  guard case .success(let buffer)? = result else {
    check(false, "\(what) did not come out: \(String(describing: result))", line: line)
    return
  }
  let size = (CVPixelBufferGetWidth(buffer), CVPixelBufferGetHeight(buffer))
  check(
    size == (video.width, video.height),
    "\(what) is \(size.0) by \(size.1) samples, not \(video.width) by \(video.height)", line: line)
  guard let seen = planes(of: buffer) else {
    check(false, "\(what) has no two planes", line: line)
    return
  }
  let expected = video.picture(number).rows(width: video.width, height: video.height)
  check(
    seen.luma == expected.luma, "luma of \(what) differs at \(firstDifference(seen.luma, expected.luma))",
    line: line)
  check(
    seen.chroma == expected.chroma,
    "chroma of \(what) differs at \(firstDifference(seen.chroma, expected.chroma))", line: line)
}

func testVideoToolboxGivesBackTheSamplesThatWereCoded() {
  // A size that is not a multiple of 16, so that the cut at the edge is part of the test.
  let video = PcmVideo(macroblocksWide: 3, macroblocksHigh: 2, cropRight: 2, cropBottom: 1)
  guard let decoder = planarDecoder(for: video) else {
    check(false, "VideoToolbox takes no format from the parameter sets")
    return
  }
  let frames = [
    ("the first key frame", video.keyFrame(0), 0),
    ("the frame that repeats it", video.repeatedFrame(1), 0),
    ("the second key frame", video.keyFrame(1), 1),
    ("the frame that repeats that", video.repeatedFrame(1), 1),
  ]
  for (at, frame) in frames.enumerated() {
    checkPicture(
      decoded(decoder, frame.1, number: UInt64(at)), of: video, number: frame.2, frame.0)
  }
}

func testVideoToolboxGoesOnWhenTheSameParameterSetsComeAgain() {
  // A key frame brings its parameter sets every time. A decoder that started again for each would
  // lose the picture that the next frame repeats.
  let video = PcmVideo(macroblocksWide: 2, macroblocksHigh: 2)
  guard let decoder = planarDecoder(for: video) else {
    check(false, "VideoToolbox takes no format from the parameter sets")
    return
  }
  checkPicture(decoded(decoder, video.keyFrame(3)), of: video, number: 3, "the key frame")
  check(decoder.prepare(parameterSets(of: video)), "the same parameter sets are refused")
  checkPicture(
    decoded(decoder, video.repeatedFrame(1)), of: video, number: 3,
    "the frame that repeats the picture before the parameter sets came again")
}

func testVideoToolboxStartsAgainForAnotherSize() {
  let small = PcmVideo(macroblocksWide: 2, macroblocksHigh: 2)
  let large = PcmVideo(macroblocksWide: 5, macroblocksHigh: 3, cropRight: 3, cropBottom: 2)
  guard let decoder = planarDecoder(for: small) else {
    check(false, "VideoToolbox takes no format from the parameter sets")
    return
  }
  checkPicture(decoded(decoder, small.keyFrame(0)), of: small, number: 0, "the small picture")
  check(decoder.prepare(parameterSets(of: large)), "the parameter sets of the large size are refused")
  checkPicture(decoded(decoder, large.keyFrame(2)), of: large, number: 2, "the large picture")
  checkPicture(
    decoded(decoder, large.repeatedFrame(1)), of: large, number: 2, "the frame that repeats it")
  check(decoder.prepare(parameterSets(of: small)), "the parameter sets of the small size are refused")
  checkPicture(decoded(decoder, small.keyFrame(1)), of: small, number: 1, "the small picture again")
}

func testVideoToolboxMakesTheFormatThatFlutterDraws() {
  // The decoder the app makes, with the format it asks for: BGRA, in memory that the graphics
  // hardware reads as it is.
  let video = PcmVideo(macroblocksWide: 3, macroblocksHigh: 2)
  let decoder = H264Decoder()
  check(decoder.prepare(parameterSets(of: video)), "the parameter sets are refused")
  guard case .success(let buffer)? = decoded(decoder, video.keyFrame(0)) else {
    check(false, "the key frame did not come out")
    return
  }
  check(
    CVPixelBufferGetPixelFormatType(buffer) == kCVPixelFormatType_32BGRA,
    "the format is \(CVPixelBufferGetPixelFormatType(buffer))")
  check(CVPixelBufferGetWidth(buffer) == video.width, "width \(CVPixelBufferGetWidth(buffer))")
  check(CVPixelBufferGetHeight(buffer) == video.height, "height \(CVPixelBufferGetHeight(buffer))")
  check(CVPixelBufferGetIOSurface(buffer) != nil, "the picture is not in an IOSurface")

  // Every pixel is opaque, and the colors are in the order of the format: blue first. The picture
  // is red at the bottom left (Cr is 199 and Cb is 47 there) and blue at the top right (Cb is 193
  // and Cr is 87), whichever of the usual conversions from Y, Cb and Cr is used.
  guard CVPixelBufferLockBaseAddress(buffer, .readOnly) == kCVReturnSuccess,
    let base = CVPixelBufferGetBaseAddress(buffer)
  else {
    check(false, "the pixels cannot be read")
    return
  }
  defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
  let stride = CVPixelBufferGetBytesPerRow(buffer)
  func pixel(_ x: Int, _ y: Int) -> [UInt8] {
    let at = (base + y * stride + x * 4).assumingMemoryBound(to: UInt8.self)
    return Array(UnsafeBufferPointer(start: at, count: 4))
  }
  var everyPixelIsOpaque = true
  for y in 0..<video.height {
    for x in 0..<video.width where pixel(x, y)[3] != 255 { everyPixelIsOpaque = false }
  }
  check(everyPixelIsOpaque, "a pixel is not opaque")
  let red = pixel(0, video.height - 2)
  check(Int(red[2]) > Int(red[0]) + 100, "the bottom left pixel (blue, green, red, alpha) is \(red)")
  let blue = pixel(video.width - 2, 0)
  check(Int(blue[0]) > Int(blue[2]) + 100, "the top right pixel (blue, green, red, alpha) is \(blue)")
}

func testVideoToolboxAnswersEveryFrameThatItTook() {
  let video = PcmVideo(macroblocksWide: 4, macroblocksHigh: 3)
  guard let decoder = planarDecoder(for: video) else {
    check(false, "VideoToolbox takes no format from the parameter sets")
    return
  }
  let count = 90
  let lock = NSLock()
  var order: [UInt64] = []
  var failures: [DecodeFailure] = []
  let answered = DispatchSemaphore(value: 0)
  // The frames are given one after the other without waiting for any, as the Rust side does.
  var units = [video.keyFrame(0)]
  for at in 1..<count { units.append(video.repeatedFrame(at % 16)) }
  var taken = 0
  for (at, unit) in units.enumerated() {
    let yes = PcmVideo.sample(unit).withUnsafeBytes { bytes in
      decoder.decode(bytes, number: UInt64(at)) { result in
        lock.lock()
        order.append(UInt64(at))
        if case .failure(let failure) = result { failures.append(failure) }
        lock.unlock()
        answered.signal()
      }
    }
    if yes { taken += 1 }
  }
  check(taken == count, "the decoder took \(taken) of \(count) frames")
  var answers = 0
  while answers < taken && answered.wait(timeout: .now() + 10) == .success { answers += 1 }
  check(answers == taken, "\(answers) of \(taken) frames were answered")
  lock.lock()
  check(failures.isEmpty, "failures: \(failures)")
  check(order.sorted() == Array(0..<UInt64(order.count)), "the frames answered are \(order)")
  lock.unlock()
}

func testVideoToolboxRefusesWhatItCannotUse() {
  let video = PcmVideo(macroblocksWide: 2, macroblocksHigh: 2)
  let decoder = H264Decoder()
  let sample = PcmVideo.sample(video.keyFrame(0))
  check(
    !sample.withUnsafeBytes { decoder.decode($0, number: 0) { _ in } },
    "a frame is taken before the parameter sets came")

  check(
    !decoder.prepare(ParameterSets(sequence: [Data([1, 2, 3])], picture: [Data([4, 5])])),
    "parameter sets that are not H.264 are taken")
  check(
    !decoder.prepare(ParameterSets(sequence: [Data(video.sequenceParameterSet)], picture: [])),
    "parameter sets with no picture parameter set are taken")
  check(
    !decoder.prepare(ParameterSets(sequence: [], picture: [Data(video.pictureParameterSet)])),
    "parameter sets with no sequence parameter set are taken")
  check(
    !sample.withUnsafeBytes { decoder.decode($0, number: 0) { _ in } },
    "a frame is taken after parameter sets were refused")

  check(decoder.prepare(parameterSets(of: video)), "the parameter sets are refused")
  check(
    !decoder.decode(UnsafeRawBufferPointer(start: nil, count: 0), number: 0, then: { _ in }),
    "an empty frame is taken")
  decoder.stop()
  check(
    !sample.withUnsafeBytes { decoder.decode($0, number: 0) { _ in } },
    "a frame is taken after the decoder was stopped")
}

func testVideoToolboxPicturesReachTheScreenThroughTheWholeChain() {
  // The decoder the app makes, behind the screen the Rust side drives: what a window does from
  // the frame that the Rust side gives to the picture that is shown.
  let video = PcmVideo(macroblocksWide: 4, macroblocksHigh: 2, cropRight: 1)
  let arena = Arena()
  let pictures = PictureLog()
  let screen = WindowScreen(
    decoder: H264Decoder(), present: { pictures.add($0) }, failed: {}, thermal: { 0 }, onRelease: {}
  )

  let key = FrameInput(
    flags: UInt32(BATON_FRAME_KEY), number: 1, dependsOn: 0, captureTime: 0, chain: 0, epoch: 0,
    sample: arena.bytes(PcmVideo.sample(video.keyFrame(0))),
    sets: [arena.bytes(video.sequenceParameterSet), arena.bytes(video.pictureParameterSet)],
    sequenceSets: 1)
  check(screen.decode(key), "the key frame is refused")
  screen.show(1)
  check(eventually { pictures.count == 1 }, "the key frame is not shown")

  let next = FrameInput(
    flags: 0, number: 2, dependsOn: 1, captureTime: 0, chain: 0, epoch: 0,
    sample: arena.bytes(PcmVideo.sample(video.repeatedFrame(1))), sets: [], sequenceSets: 0)
  check(screen.decode(next), "the frame after it is refused")
  screen.show(2)
  check(eventually { pictures.count == 2 }, "the frame after it is not shown")

  for (at, buffer) in pictures.all.enumerated() {
    guard let buffer else { continue }
    check(
      CVPixelBufferGetWidth(buffer) == video.width && CVPixelBufferGetHeight(buffer) == video.height,
      "picture \(at) is \(CVPixelBufferGetWidth(buffer)) by \(CVPixelBufferGetHeight(buffer))")
  }
  screen.ended()
  check(pictures.all.last.map { $0 == nil } ?? false, "the screen still shows a picture after the end")
  screen.released()
}

func testAPictureWithNothingInItIsOneThatFlutterDraws() {
  // A texture that has no picture is given this one: Flutter on an iPhone goes on drawing the
  // last picture of a texture that answers with none.
  guard let blank = makeBlankPicture() else { return check(false, "no blank picture was made") }
  check(
    CVPixelBufferGetPixelFormatType(blank) == kCVPixelFormatType_32BGRA,
    "the format that the decoder makes, which Flutter draws as it is")
  check(CVPixelBufferGetWidth(blank) > 0 && CVPixelBufferGetHeight(blank) > 0)
  check(
    CVPixelBufferGetIOSurface(blank) != nil,
    "Flutter draws a picture with Metal, from memory that the GPU shares")
  guard CVPixelBufferLockBaseAddress(blank, .readOnly) == kCVReturnSuccess,
    let base = CVPixelBufferGetBaseAddress(blank)
  else { return check(false, "the picture cannot be read") }
  defer { CVPixelBufferUnlockBaseAddress(blank, .readOnly) }
  let bytes = UnsafeBufferPointer(
    start: base.assumingMemoryBound(to: UInt8.self),
    count: CVPixelBufferGetBytesPerRow(blank) * CVPixelBufferGetHeight(blank))
  check(bytes.allSatisfy { $0 == 0 }, "a picture with nothing in it has nothing in it")
}

/// The pictures that a screen was asked to show, from any thread.
private final class PictureLog {
  private let lock = NSLock()
  private var pictures: [CVPixelBuffer?] = []

  func add(_ picture: CVPixelBuffer?) {
    lock.lock()
    pictures.append(picture)
    lock.unlock()
  }

  var all: [CVPixelBuffer?] {
    lock.lock()
    defer { lock.unlock() }
    return pictures
  }

  var count: Int { all.count }
}

#endif
