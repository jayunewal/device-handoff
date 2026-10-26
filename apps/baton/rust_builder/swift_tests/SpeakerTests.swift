// SPDX-License-Identifier: Apache-2.0

// Tests of the sound of a screen on the side that plays it: the ring (BatonSpeaker.swift), the
// speaker that makes a run of sound out of the calls, and the three calls about sound in the table
// of the viewer (BatonViewer.swift). What decodes and what plays are fakes here; the decoder of the
// system is tested in AacDecoderTests.swift.

import Foundation

// MARK: - Helpers

/// `frames` frames of `channels` channels in which the left channel counts up from `first`, and
/// every other channel holds that and a half more.
func counting(_ frames: Int, from first: Float, channels: Int = 2) -> [Float] {
  var samples: [Float] = []
  samples.reserveCapacity(frames * channels)
  for frame in 0..<frames {
    for channel in 0..<channels {
      samples.append(first + Float(frame) + Float(channel) * 0.5)
    }
  }
  return samples
}

/// The numbers of a range as the samples that `counting` makes of them.
func values(_ range: Range<Int>) -> [Float] {
  range.map { Float($0) }
}

/// The left channel of the packets of a test, which have ten frames of the byte they start with.
func framesOf(_ bytes: [UInt8]) -> [Float] {
  bytes.flatMap { [Float](repeating: Float($0), count: 10) }
}

extension SoundRing {
  func put(_ samples: [Float], gap: Int = 0) {
    samples.withUnsafeBufferPointer { write($0, gap: gap) }
  }

  /// Takes `frames` frames: the left channel of them, and how many of them were sound and not
  /// silence.
  func take(_ frames: Int) -> (left: [Float], real: Int) {
    var out = [Float](repeating: -1, count: frames * channels)
    var real = 0
    out.withUnsafeMutableBufferPointer { buffer in
      if let base = buffer.baseAddress { real = read(into: base, frames: frames) }
    }
    let stride = channels
    return ((0..<frames).map { out[$0 * stride] }, real)
  }
}

// MARK: - The ring

func testTheRingPlaysNothingUntilItHoldsTheStartLevelAndThenPlaysInOrder() {
  // At 1000 Hz a millisecond is a frame: the start level is 80 frames and the most is 250.
  let ring = SoundRing(rate: 1000, channels: 2)
  ring.put(counting(50, from: 0))
  check(!ring.isPlaying && ring.frames == 50)
  let early = ring.take(10)
  check(early.real == 0 && early.left.allSatisfy { $0 == 0 }, "silence while it fills")
  check(ring.frames == 50, "nothing was taken while it filled")
  ring.put(counting(30, from: 50))
  check(ring.isPlaying, "80 frames is the start level")
  let played = ring.take(80)
  check(played.real == 80, "real \(played.real)")
  check(played.left == values(0..<80), "\(played.left)")
  check(ring.frames == 0)
}

func testTheRingPlaysSilenceAndFillsAgainWhenItRunsDry() {
  let ring = SoundRing(rate: 1000, channels: 2)
  ring.put(counting(100, from: 0))
  check(ring.take(60).real == 60)
  let dry = ring.take(60)
  check(dry.real == 40, "real \(dry.real)")
  check(Array(dry.left[0..<40]) == values(60..<100))
  check(dry.left[40...].allSatisfy { $0 == 0 }, "silence for what it did not have")
  check(!ring.isPlaying && ring.counts.ranDry == 1)
  ring.put(counting(50, from: 100))
  check(!ring.isPlaying, "50 frames is below the start level")
  check(ring.take(10).real == 0)
  ring.put(counting(30, from: 150))
  check(ring.isPlaying)
  let again = ring.take(80)
  check(again.left == values(100..<180), "\(again.left)")
}

func testTheRingDropsTheOldestSoundWhenItWouldHoldMoreThanTheMost() {
  let ring = SoundRing(rate: 1000, channels: 2)
  ring.put(counting(200, from: 0))
  check(ring.frames == 200 && ring.counts.droppedFrames == 0)
  ring.put(counting(100, from: 200))
  check(ring.frames == 80, "frames \(ring.frames): it is cut back to the start level")
  check(ring.counts.droppedFrames == 220, "dropped \(ring.counts.droppedFrames)")
  check(ring.isPlaying)
  check(ring.take(80).left == values(220..<300))
}

func testTheRingLetsGoOfSoundThatItDidNotComeDownToInASecondOfPlaying() {
  // At 1000 Hz a millisecond is a frame: the start level is 80 frames, the most 250, the slack 40
  // and a second 1000 frames. A burst of 170 frames, as the newest eight packets that a source kept
  // through a stall, is under the most, and nothing else brings it down.
  let ring = SoundRing(rate: 1000, channels: 2)
  var next = 0
  func put(_ frames: Int) {
    ring.put(counting(frames, from: Float(next)))
    next += frames
  }
  put(170)
  check(ring.isPlaying && ring.frames == 170)
  var heard: [Float] = []
  // Ten frames are taken and ten put back, so the level is 160 after every read.
  for step in 0..<99 {
    let got = ring.take(10)
    check(got.real == 10, "step \(step)")
    heard += got.left
    put(10)
  }
  check(ring.frames == 170 && ring.counts == SoundCounts(), "nothing yet: \(ring.counts)")
  // The hundredth read ends the second. The level was never under 160, which is 80 above the start
  // level, and the oldest 80 frames go.
  let last = ring.take(10)
  heard += last.left
  check(last.real == 10)
  check(ring.counts.caughtUp == 1 && ring.counts.droppedFrames == 80, "\(ring.counts)")
  check(ring.frames == 80, "frames \(ring.frames)")
  put(10)
  check(heard == values(0..<1000), "what was played came in order, and none of it was let go of")
  check(ring.take(90).left == values(1080..<1170), "the newest sound is what is left")
}

func testTheRingKeepsWhatTheLevelCameDownToWithinTheSlackAndLetsGoInTheNextSecond() {
  let ring = SoundRing(rate: 1000, channels: 2)
  var next = 0
  func put(_ frames: Int) {
    ring.put(counting(frames, from: Float(next)))
    next += frames
  }
  put(170)
  // Ten frames at a time, except at one step where sixty are taken. That leaves 110, which is 30
  // above the start level, under the slack of 40.
  var played = 0
  var step = 0
  while played < 1000 {
    let size = step == 40 ? 60 : 10
    check(ring.take(size).real == size, "step \(step)")
    put(size)
    played += size
    step += 1
  }
  check(ring.frames == 170 && ring.counts == SoundCounts(), "the first second: \(ring.counts)")
  // The next second has no such step, and the level was never under 160.
  for _ in 0..<100 {
    check(ring.take(10).real == 10)
    put(10)
  }
  check(ring.counts.caughtUp == 1 && ring.counts.droppedFrames == 80, "\(ring.counts)")
  check(ring.frames == 90, "frames \(ring.frames)")
}

func testTheRingBeginsTheSecondAgainWhenItRunsDry() {
  let ring = SoundRing(rate: 1000, channels: 2)
  var next = 0
  func put(_ frames: Int) {
    ring.put(counting(frames, from: Float(next)))
    next += frames
  }
  put(170)
  // 900 frames are played with the level at 160 and above.
  for _ in 0..<90 {
    check(ring.take(10).real == 10)
    put(10)
  }
  // It runs dry and fills again: what was seen before says nothing about what follows.
  check(ring.take(200).real == 170)
  check(!ring.isPlaying && ring.counts.ranDry == 1)
  put(170)
  check(ring.isPlaying)
  for _ in 0..<11 {
    check(ring.take(10).real == 10)
    put(10)
  }
  check(ring.counts.droppedFrames == 0 && ring.counts.caughtUp == 0, "\(ring.counts)")
}

func testTheRingKeepsTheOrderOfSoundThatWrapsAroundItsStorage() {
  let ring = SoundRing(rate: 1000, channels: 2)
  var next = 0
  var expected = 0
  ring.put(counting(100, from: Float(next)))
  next += 100
  for step in 0..<400 {
    let size = 37 + step % 23
    ring.put(counting(size, from: Float(next)))
    next += size
    let got = ring.take(size)
    check(got.real == size, "step \(step)")
    check(got.left == values(expected..<expected + size), "step \(step)")
    expected += size
  }
  check(ring.counts == SoundCounts(), "the ring neither dropped nor ran dry")
}

func testAGapIsCoveredWithSilenceOnlyWhileTheRingPlaysWithSoundInIt() {
  // Before it plays there is nothing to keep time with.
  let waiting = SoundRing(rate: 1000, channels: 2)
  waiting.put(counting(50, from: 0), gap: 40)
  check(waiting.frames == 50 && waiting.counts.coveredFrames == 0)

  // While it plays, what is missing is silence in its place.
  let playing = SoundRing(rate: 1000, channels: 2)
  playing.put(counting(100, from: 0))
  check(playing.take(30).real == 30)
  playing.put(counting(10, from: 100), gap: 20)
  check(playing.frames == 100 && playing.counts.coveredFrames == 20)
  let heard = playing.take(100)
  check(Array(heard.left[0..<70]) == values(30..<100))
  check(heard.left[70..<90].allSatisfy { $0 == 0 }, "the gap is silence")
  check(Array(heard.left[90..<100]) == values(100..<110))

  // Never more silence than the ring waits for before it plays.
  let long = SoundRing(rate: 1000, channels: 2)
  long.put(counting(100, from: 0))
  long.put(counting(10, from: 100), gap: 500)
  check(long.counts.coveredFrames == 80, "covered \(long.counts.coveredFrames)")
  check(long.frames == 190, "frames \(long.frames)")

  // After it ran dry the silence that it played stands for the gap.
  let dry = SoundRing(rate: 1000, channels: 2)
  dry.put(counting(100, from: 0))
  _ = dry.take(120)
  dry.put(counting(10, from: 100), gap: 30)
  check(dry.counts.coveredFrames == 0 && dry.frames == 10)
}

func testTheRingTakesMonoAndIgnoresAPartOfAFrameAtTheEnd() {
  let mono = SoundRing(rate: 1000, channels: 1)
  mono.put(counting(90, from: 0, channels: 1))
  check(mono.frames == 90 && mono.isPlaying)
  check(mono.take(90).left == values(0..<90))

  let stereo = SoundRing(rate: 1000, channels: 2)
  stereo.put(counting(90, from: 0) + [9_999])
  check(stereo.frames == 90, "frames \(stereo.frames)")
  stereo.put([])
  stereo.put([1])
  check(stereo.frames == 90, "a write of less than a frame changes nothing")
  check(stereo.take(0).real == 0, "a read of no frames reads none")
}

func testARingThatOneThreadWritesAndAnotherReadsKeepsTheOrderAndLosesNothingUnaccountedFor() {
  let ring = SoundRing(rate: 48_000, channels: 2)
  let packets = 300
  let size = 1024
  let finished = DispatchSemaphore(value: 0)
  DispatchQueue.global().async {
    for packet in 0..<packets {
      ring.put(counting(size, from: Float(packet * size + 1)))
      if packet % 4 == 0 { Thread.sleep(forTimeInterval: 0.001) }
    }
    finished.signal()
  }
  var heard: [Float] = []
  var real = 0
  let end = Date().addingTimeInterval(20)
  var over = false
  while !over && Date() < end {
    over = finished.wait(timeout: .now()) == .success
    let got = ring.take(480)
    real += got.real
    heard.append(contentsOf: got.left.filter { $0 != 0 })
    if !over { Thread.sleep(forTimeInterval: 0.0005) }
  }
  check(over, "the writer did not finish")
  // What is left is read to the end.
  while true {
    let got = ring.take(480)
    real += got.real
    heard.append(contentsOf: got.left.filter { $0 != 0 })
    if got.real == 0 { break }
  }
  check(
    zip(heard, heard.dropFirst()).allSatisfy { $0 < $1 },
    "the sound did not come out in the order it went in")
  check(real == heard.count, "real \(real), heard \(heard.count)")
  let kept = ring.frames
  let dropped = ring.counts.droppedFrames
  check(
    real + dropped + kept == packets * size,
    "written \(packets * size), heard \(real), dropped \(dropped), kept \(kept)")
}

// MARK: - A speaker with a decoder and an output that are fakes

/// A decoder that gives the packets of a test: every sample of a packet holds the first byte of it.
final class FakeSoundDecoder: SoundDecoder {
  let form: SoundForm
  private let lock = NSLock()
  private var events: [String] = []
  private var gone = 0
  /// The packets that start with these bytes cannot be decoded, or give no samples.
  var failing: Set<UInt8> = []
  var empty: Set<UInt8> = []
  /// While the packet that starts with `byte` is decoded, the decoder says `entered` and waits for
  /// `open`.
  var gate: (byte: UInt8, entered: DispatchSemaphore, open: DispatchSemaphore)?

  init(form: SoundForm) {
    self.form = form
  }

  func decode(_ packet: Data) throws -> [Float]? {
    guard let byte = packet.first else { throw SoundFailure(step: "decode", code: -1) }
    lock.lock()
    events.append("decode \(byte)")
    lock.unlock()
    if let gate, gate.byte == byte {
      gate.entered.signal()
      gate.open.wait()
    }
    if failing.contains(byte) { throw SoundFailure(step: "decode", code: -9) }
    if empty.contains(byte) { return nil }
    return [Float](repeating: Float(byte), count: Int(form.samples) * Int(form.channels))
  }

  func reset() {
    lock.lock()
    events.append("reset")
    lock.unlock()
  }

  func invalidate() {
    lock.lock()
    gone += 1
    lock.unlock()
  }

  var log: [String] {
    lock.lock()
    defer { lock.unlock() }
    return events
  }

  var invalidated: Int {
    lock.lock()
    defer { lock.unlock() }
    return gone
  }
}

final class FakeSoundOutput: SoundOutput {
  let ring: SoundRing
  private let lock = NSLock()
  private var started = 0
  private var stopped = 0
  var failStart = false
  /// While the output starts, it says `entered` and waits for `open`.
  var gate: (entered: DispatchSemaphore, open: DispatchSemaphore)?

  init(ring: SoundRing) {
    self.ring = ring
  }

  func start() throws {
    lock.lock()
    started += 1
    lock.unlock()
    if let gate {
      gate.entered.signal()
      gate.open.wait()
    }
    if failStart { throw SoundFailure(step: "start", code: -5) }
  }

  func stop() {
    lock.lock()
    stopped += 1
    lock.unlock()
  }

  var starts: Int {
    lock.lock()
    defer { lock.unlock() }
    return started
  }

  var stops: Int {
    lock.lock()
    defer { lock.unlock() }
    return stopped
  }
}

/// A speaker over fakes, with the packets of a form in which a packet is ten frames and the ring
/// starts to play at four packets and holds ten at the most.
final class SpeakerRig {
  let form: SoundForm
  let arena = Arena()
  var decoderFails = false
  var outputFails = false
  var startFails = false
  var startGate: (entered: DispatchSemaphore, open: DispatchSemaphore)?

  private let lock = NSLock()
  private var madeDecoders: [FakeSoundDecoder] = []
  private var madeOutputs: [FakeSoundOutput] = []
  private var number: UInt64 = 0
  private var time = 0.0

  /// The time on the clock of the speaker, in seconds. It stands where a test puts it.
  var now: Double {
    get {
      lock.lock()
      defer { lock.unlock() }
      return time
    }
    set {
      lock.lock()
      time = newValue
      lock.unlock()
    }
  }

  init(form: SoundForm = SoundForm(rate: 1000, channels: 2, samples: 10)) {
    self.form = form
  }

  lazy var speaker = ScreenSpeaker(
    levels: SoundLevels(startMillis: 40, mostMillis: 100),
    clock: { [self] in now },
    makeDecoder: { [self] form in
      if decoderFails { throw SoundFailure(step: "make the decoder", code: -7) }
      let decoder = FakeSoundDecoder(form: form)
      lock.lock()
      madeDecoders.append(decoder)
      lock.unlock()
      return decoder
    },
    makeOutput: { [self] _, ring in
      if outputFails { throw SoundFailure(step: "make the output", code: -8) }
      let output = FakeSoundOutput(ring: ring)
      output.failStart = startFails
      output.gate = startGate
      lock.lock()
      madeOutputs.append(output)
      lock.unlock()
      return output
    })

  var decoders: [FakeSoundDecoder] {
    lock.lock()
    defer { lock.unlock() }
    return madeDecoders
  }

  var outputs: [FakeSoundOutput] {
    lock.lock()
    defer { lock.unlock() }
    return madeOutputs
  }

  func begin() {
    speaker.begin(form)
  }

  /// Plays a packet whose bytes are `byte`.
  func send(_ byte: UInt8, first: Bool = false, lost: UInt64 = 0) {
    number += 1
    speaker.play(
      PlayInput(
        flags: first ? UInt32(BATON_PLAY_FIRST) : 0, number: number, captureTime: number * 1000,
        lostBefore: lost, packet: arena.bytes([byte])))
  }

  /// Waits until the speaker has done what it was handed.
  func settle() {
    speaker.idle()
  }
}

func testARunBeginsWithADecoderAnOutputAndARingOfItsForm() {
  let rig = SpeakerRig(form: SoundForm(rate: 44_100, channels: 1, samples: 960))
  rig.begin()
  rig.settle()
  check(rig.decoders.count == 1 && rig.outputs.count == 1)
  check(rig.decoders.first?.form == SoundForm(rate: 44_100, channels: 1, samples: 960))
  check(rig.outputs.first?.starts == 1, "the output is started when the run begins")
  check(rig.outputs.first?.ring.channels == 1)
  check(rig.outputs.first?.stops == 0)
}

func testPacketsAreDecodedInOrderAndPlayedOnceTheRingHoldsTheStartLevel() {
  let rig = SpeakerRig()
  rig.begin()
  rig.send(1, first: true)
  rig.send(2)
  rig.send(3)
  rig.settle()
  guard let ring = rig.outputs.first?.ring else { return check(false, "no output") }
  check(ring.frames == 30 && !ring.isPlaying, "frames \(ring.frames)")
  rig.send(4)
  rig.settle()
  check(ring.isPlaying, "four packets of ten frames are the start level of 40")
  let heard = ring.take(40)
  check(heard.left == framesOf([1, 2, 3, 4]), "\(heard.left)")
}

func testTheFirstPacketOfARunAndThePacketAfterAGapResetTheDecoder() {
  let rig = SpeakerRig()
  rig.begin()
  rig.send(1, first: true)
  rig.send(2)
  rig.send(3, lost: 2)
  rig.send(4)
  rig.settle()
  check(
    rig.decoders.first?.log == ["reset", "decode 1", "decode 2", "reset", "decode 3", "decode 4"],
    "\(String(describing: rig.decoders.first?.log))")
}

func testASpeakerCoversAGapWithSilenceWhileItPlays() {
  let rig = SpeakerRig()
  rig.begin()
  rig.send(1, first: true)
  for byte in 2...5 { rig.send(UInt8(byte)) }
  rig.send(6, lost: 2)
  rig.settle()
  guard let ring = rig.outputs.first?.ring else { return check(false, "no output") }
  // Five packets are 50 frames, the two that did not come 20 frames of silence, then the sixth.
  check(ring.frames == 80, "frames \(ring.frames)")
  check(ring.counts.coveredFrames == 20)
  let heard = ring.take(80).left
  check(Array(heard[0..<50]) == framesOf([1, 2, 3, 4, 5]))
  check(heard[50..<70].allSatisfy { $0 == 0 })
  check(Array(heard[70..<80]) == framesOf([6]))
}

func testAPacketThatCannotBeDecodedIsDroppedAndCountsAsMissing() {
  let rig = SpeakerRig()
  rig.begin()
  rig.settle()
  rig.decoders.first?.failing = [5]
  rig.send(1, first: true)
  for byte in 2...4 { rig.send(UInt8(byte)) }
  rig.send(5)
  rig.send(6)
  rig.settle()
  check(
    rig.decoders.first?.log == [
      "reset", "decode 1", "decode 2", "decode 3", "decode 4", "decode 5", "reset", "decode 6",
    ], "\(String(describing: rig.decoders.first?.log))")
  guard let ring = rig.outputs.first?.ring else { return check(false, "no output") }
  // Four packets, ten frames of silence for the fifth, and the sixth.
  check(ring.frames == 60, "frames \(ring.frames)")
  let heard = ring.take(60).left
  check(Array(heard[0..<40]) == framesOf([1, 2, 3, 4]))
  check(heard[40..<50].allSatisfy { $0 == 0 }, "the packet that did not decode is silence")
  check(Array(heard[50..<60]) == framesOf([6]))
}

func testAPacketThatGivesNoSamplesChangesNothing() {
  let rig = SpeakerRig()
  rig.begin()
  rig.settle()
  rig.decoders.first?.empty = [2]
  rig.send(1, first: true)
  rig.send(2)
  rig.send(3)
  rig.settle()
  let frames = rig.outputs.first?.ring.frames
  check(frames == 20, "frames \(String(describing: frames))")
}

func testAPacketBeforeARunBeginsAndAfterItEndedIsDropped() {
  let rig = SpeakerRig()
  rig.send(1, first: true)
  rig.settle()
  check(rig.decoders.isEmpty && rig.outputs.isEmpty, "a packet made a decoder")

  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  check(rig.decoders.first?.log == ["reset", "decode 1"])
  rig.speaker.stop()
  rig.send(2)
  rig.settle()
  check(rig.decoders.first?.log == ["reset", "decode 1"], "a packet after the run ended was decoded")
  check(rig.decoders.count == 1)
}

func testStoppedEndsTheRunLetsGoOfTheDecoderAndStopsTheOutputOnce() {
  let rig = SpeakerRig()
  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  rig.speaker.stop()
  rig.settle()
  check(rig.outputs.first?.stops == 1 && rig.decoders.first?.invalidated == 1)
  rig.speaker.stop()
  rig.settle()
  check(
    rig.outputs.first?.stops == 1 && rig.decoders.first?.invalidated == 1,
    "a second stop let go of them again")
  rig.begin()
  rig.send(2, first: true)
  rig.settle()
  check(rig.decoders.count == 2 && rig.outputs.count == 2, "a run after the one that ended")
  check(rig.outputs.last?.starts == 1 && rig.outputs.last?.stops == 0)
}

func testANewRunEndsTheOneThatWentOn() {
  let rig = SpeakerRig()
  rig.begin()
  rig.send(1, first: true)
  rig.begin()
  rig.send(2, first: true)
  rig.settle()
  check(rig.decoders.count == 2 && rig.outputs.count == 2)
  check(rig.outputs.first?.stops == 1 && rig.decoders.first?.invalidated == 1, "the first run")
  check(rig.outputs.last?.stops == 0 && rig.decoders.last?.log == ["reset", "decode 2"])
  check(rig.outputs.first?.ring !== rig.outputs.last?.ring, "a run has a ring of its own")
}

func testClosingEndsTheRunAndLaterCallsDoNothing() {
  let rig = SpeakerRig()
  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  rig.speaker.close()
  rig.settle()
  check(rig.outputs.first?.stops == 1 && rig.decoders.first?.invalidated == 1)
  rig.begin()
  rig.send(2, first: true)
  rig.speaker.stop()
  rig.speaker.close()
  rig.settle()
  check(rig.decoders.count == 1 && rig.outputs.count == 1, "a closed speaker began a run")
  check(rig.outputs.first?.stops == 1 && rig.decoders.first?.invalidated == 1)
}

func testADecoderOrAnOutputThatCannotBeMadeEndsTheRunQuietly() {
  let rig = SpeakerRig()
  rig.decoderFails = true
  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  check(rig.decoders.isEmpty && rig.outputs.isEmpty, "an output was made without a decoder")

  rig.decoderFails = false
  rig.outputFails = true
  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  check(rig.decoders.count == 1 && rig.decoders.first?.invalidated == 1, "the decoder was kept")
  check(rig.decoders.first?.log.isEmpty == true, "a packet was decoded for a run with no output")

  rig.outputFails = false
  rig.startFails = true
  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  check(rig.decoders.count == 2 && rig.outputs.count == 1)
  check(rig.outputs.first?.stops == 1 && rig.decoders.last?.invalidated == 1, "the failed start")
  check(rig.decoders.last?.log.isEmpty == true)

  rig.startFails = false
  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  check(rig.decoders.last?.log == ["reset", "decode 1"], "a run after the ones that failed")
  check(rig.outputs.last?.stops == 0)
}

func testARunThatCouldNotBeMadeIsTriedAgainOnAPacketAfterASecond() {
  let rig = SpeakerRig()
  rig.outputFails = true
  rig.begin()
  rig.send(1, first: true)
  rig.settle()
  check(rig.decoders.count == 1 && rig.outputs.isEmpty, "the first try")

  // The call is over, or the device is back.
  rig.outputFails = false
  rig.now = 0.5
  rig.send(2)
  rig.settle()
  check(rig.decoders.count == 1 && rig.outputs.isEmpty, "a try before the second was over")

  rig.now = 1.0
  rig.send(3)
  rig.send(4)
  rig.settle()
  check(rig.decoders.count == 2 && rig.outputs.count == 1, "the try after the second")
  check(
    rig.decoders.last?.log == ["decode 3", "decode 4"],
    "\(String(describing: rig.decoders.last?.log))")
  check(rig.outputs.last?.starts == 1 && rig.outputs.last?.stops == 0)
  check(rig.outputs.last?.ring.frames == 20, "the packets after the try are played")
  rig.send(5)
  rig.now = 9
  rig.send(6)
  rig.settle()
  check(rig.decoders.count == 2 && rig.outputs.count == 1, "a run that goes is not made again")
}

func testARunThatKeepsFailingIsTriedAgainOncePerSecond() {
  let rig = SpeakerRig()
  rig.startFails = true
  rig.begin()
  rig.settle()
  check(rig.decoders.count == 1 && rig.outputs.count == 1, "the first try")
  var byte: UInt8 = 0
  for time in [0.25, 0.5, 0.75, 0.99] {
    rig.now = time
    byte += 1
    rig.send(byte)
  }
  rig.settle()
  check(rig.decoders.count == 1, "a try inside the second: \(rig.decoders.count)")
  rig.now = 1.0
  rig.send(9)
  rig.settle()
  check(rig.decoders.count == 2 && rig.outputs.count == 2, "the second try")
  rig.now = 1.5
  rig.send(10)
  rig.settle()
  check(rig.decoders.count == 2, "a second is counted from the last try")
  rig.now = 2.0
  rig.send(11)
  rig.settle()
  check(rig.decoders.count == 3 && rig.outputs.count == 3, "the third try")
  check(rig.decoders.allSatisfy { $0.invalidated == 1 }, "a try that failed let go of its decoder")
  check(rig.outputs.allSatisfy { $0.stops == 1 }, "a try that failed stopped its output")
  check(rig.decoders.allSatisfy { $0.log.isEmpty }, "a packet was decoded without a run")
}

func testARunThatWasStoppedBeforeItWasMadeIsNotTriedAgainAndANewOneIsTriedInItsForm() {
  let rig = SpeakerRig()
  rig.outputFails = true
  rig.begin()
  rig.speaker.stop()
  rig.settle()
  rig.outputFails = false
  rig.now = 5
  rig.send(1)
  rig.settle()
  check(rig.decoders.count == 1 && rig.outputs.isEmpty, "a run that was stopped was made")

  // A run that begins while another waits takes its place: it is the form of the last that is made.
  let other = SoundForm(rate: 1000, channels: 1, samples: 20)
  rig.outputFails = true
  rig.begin()
  rig.settle()
  rig.speaker.begin(other)
  rig.settle()
  rig.outputFails = false
  rig.now = 6
  rig.send(2)
  rig.settle()
  check(rig.decoders.count == 4 && rig.outputs.count == 1, "the run that waited")
  check(rig.decoders.last?.form == other, "a run was made in the form of the one before")
  check(rig.decoders.last?.log == ["decode 2"] && rig.outputs.last?.ring.frames == 20)
}

func testThePacketIsCopiedBeforePlayReturns() {
  let rig = SpeakerRig()
  rig.begin()
  rig.settle()
  guard let decoder = rig.decoders.first else { return check(false, "no decoder") }
  let entered = DispatchSemaphore(value: 0)
  let open = DispatchSemaphore(value: 0)
  decoder.gate = (byte: 1, entered: entered, open: open)
  rig.send(1, first: true)
  check(entered.wait(timeout: .now() + 5) == .success, "the decoder never got the first packet")
  // The decoder is busy with the first packet, so the second waits in the queue while the memory
  // it came in is given back.
  let memory = UnsafeMutableRawPointer.allocate(byteCount: 4, alignment: 1)
  memory.initializeMemory(as: UInt8.self, repeating: 2, count: 4)
  rig.speaker.play(
    PlayInput(
      flags: 0, number: 2, captureTime: 0, lostBefore: 0,
      packet: UnsafeRawBufferPointer(start: memory, count: 4)))
  memory.initializeMemory(as: UInt8.self, repeating: 0xFF, count: 4)
  memory.deallocate()
  open.signal()
  rig.settle()
  check(decoder.log == ["reset", "decode 1", "decode 2"], "\(decoder.log)")
}

func testClosingDoesNotWaitForTheOutputToStartAndStopsItAfterwards() {
  let rig = SpeakerRig()
  let entered = DispatchSemaphore(value: 0)
  let open = DispatchSemaphore(value: 0)
  rig.startGate = (entered: entered, open: open)
  rig.begin()
  check(entered.wait(timeout: .now() + 5) == .success, "the output never started")
  let closed = DispatchSemaphore(value: 0)
  DispatchQueue.global().async {
    rig.speaker.close()
    closed.signal()
  }
  let returned = closed.wait(timeout: .now() + 5) == .success
  open.signal()
  check(returned, "close waited for the output to start")
  rig.settle()
  check(rig.outputs.first?.stops == 1, "the output is stopped once its start is over")
  check(rig.decoders.first?.invalidated == 1)
}

// MARK: - A screen with a speaker

func testAWindowScreenHandsTheSoundToItsSpeakerAndClosesItWhenItEnds() {
  let rig = SpeakerRig()
  let screen = WindowScreen(
    decoder: FakeDecoder(), present: { _ in }, failed: {}, thermal: { 0 }, onRelease: {},
    speaker: rig.speaker)
  screen.sound(rig.form)
  screen.play(PlayInput(flags: 1, number: 1, captureTime: 0, lostBefore: 0, packet: rig.arena.bytes([7])))
  rig.settle()
  check(rig.decoders.first?.log == ["reset", "decode 7"], "\(String(describing: rig.decoders.first?.log))")

  screen.stopped(UInt32(BATON_SILENT_OFF))
  rig.settle()
  check(rig.outputs.first?.stops == 1, "stopped ends the run")

  screen.sound(rig.form)
  rig.settle()
  check(rig.outputs.count == 2 && rig.outputs.last?.stops == 0)
  screen.ended()
  rig.settle()
  check(rig.outputs.last?.stops == 1, "the end of the screen ends the run")
  screen.released()
  screen.sound(rig.form)
  rig.settle()
  check(rig.outputs.count == 2, "a screen that is over began a run")
}

func testAScreenWithNoSpeakerTakesTheCallsAboutSoundAndDoesNothing() {
  let screen = WindowScreen(
    decoder: FakeDecoder(), present: { _ in }, failed: {}, thermal: { 0 }, onRelease: {})
  let arena = Arena()
  screen.sound(SoundForm(rate: 48_000, channels: 2, samples: 1024))
  screen.play(PlayInput(flags: 1, number: 1, captureTime: 0, lostBefore: 0, packet: arena.bytes([1])))
  screen.stopped(UInt32(BATON_SILENT_WINDOW))
  screen.ended()
  screen.released()
  check(true)
}

// MARK: - The table

private func soundForm(
  codec: UInt32 = BATON_SOUND_CODEC_AAC, rate: UInt32 = 48_000, channels: UInt32 = 2,
  samples: UInt32 = 1024
) -> BatonSound {
  BatonSound(
    size: UInt32(MemoryLayout<BatonSound>.size), codec: codec, rate: rate, channels: channels,
    samples: samples, reserved: 0)
}

private func playedPacket(
  _ arena: Arena, flags: UInt32 = 0, number: UInt64 = 1, captureTime: UInt64 = 0,
  lostBefore: UInt64 = 0, bytes: [UInt8] = [1, 2, 3]
) -> BatonPlay {
  BatonPlay(
    size: UInt32(MemoryLayout<BatonPlay>.size), flags: flags, number: number,
    capture_time: captureTime, lost_before: lostBefore, packet: arena.batonBytes(bytes))
}

func testAPlatformThatPlaysSoundGivesTheThreeEntriesAndTheScreenIsToldInOrder() {
  let platform = FakePlatform()
  platform.playsSound = true
  guard let table = registered(platform), let sound = table.sound, let play = table.play,
    let stopped = table.stopped
  else { return check(false, "the entries of the sound were not registered") }
  check(table.size == UInt32(MemoryLayout<BatonViewerVTable>.size))
  let opened = open(table, window(Arena()))
  guard let context = opened.context, let screen = platform.made.first else { return check(false) }

  let arena = Arena()
  var form = soundForm(rate: 44_100, channels: 1, samples: 960)
  sound(context, &form)
  var first = playedPacket(
    arena, flags: UInt32(BATON_PLAY_FIRST), number: 5, captureTime: 1_000, bytes: [9, 8, 7])
  play(context, &first)
  var later = playedPacket(arena, number: 8, captureTime: 2_000, lostBefore: 2, bytes: [4])
  play(context, &later)
  var empty = playedPacket(arena, number: 9, bytes: [])
  play(context, &empty)
  stopped(context, UInt32(BATON_SILENT_NO_PERMISSION))
  stopped(context, UInt32(BATON_SILENT_OFF))
  check(
    screen.soundCalls == [
      .sound(SoundForm(rate: 44_100, channels: 1, samples: 960)),
      .play(
        SeenPlay(
          flags: UInt32(BATON_PLAY_FIRST), number: 5, captureTime: 1_000, lostBefore: 0,
          bytes: [9, 8, 7])),
      .play(SeenPlay(flags: 0, number: 8, captureTime: 2_000, lostBefore: 2, bytes: [4])),
      .play(SeenPlay(flags: 0, number: 9, captureTime: 0, lostBefore: 0, bytes: [])),
      .stopped(UInt32(BATON_SILENT_NO_PERMISSION)),
      .stopped(UInt32(BATON_SILENT_OFF)),
    ], "\(screen.soundCalls)")
  table.release?(context)
}

func testSoundCallsWithStructsThatAreNotWholeNeverReachTheScreen() {
  let platform = FakePlatform()
  platform.playsSound = true
  guard let table = registered(platform), let sound = table.sound, let play = table.play,
    let stopped = table.stopped
  else { return check(false) }
  let opened = open(table, window(Arena()))
  guard let context = opened.context, let screen = platform.made.first else { return check(false) }
  let arena = Arena()

  let shortForm = arena.shortStruct(BatonSound.self, size: 12)
  shortForm.pointee.codec = BATON_SOUND_CODEC_AAC
  sound(context, shortForm)
  var otherCodec = soundForm(codec: 7)
  sound(context, &otherCodec)
  var whole = soundForm()
  sound(context, nil)
  sound(nil, &whole)

  let shortPacket = arena.shortStruct(BatonPlay.self, size: 40)
  shortPacket.pointee.number = 3
  play(context, shortPacket)
  var noBytes = playedPacket(arena)
  noBytes.packet.ptr = nil
  play(context, &noBytes)
  var negative = playedPacket(arena)
  negative.packet.len = -1
  play(context, &negative)
  var fine = playedPacket(arena, number: 11)
  play(context, nil)
  play(nil, &fine)
  stopped(nil, 1)
  check(screen.soundCalls.isEmpty, "\(screen.soundCalls)")

  sound(context, &whole)
  play(context, &fine)
  check(screen.soundCalls.count == 2, "a whole struct reaches the screen after the others did not")
  table.release?(context)
}

func testAScreenThatPlaysNoSoundIsNotGivenCallsAboutIt() {
  // The platform plays none: the entries are not there, so the Rust side cannot call them, and the
  // calls that a screen has by default do nothing.
  let platform = FakePlatform()
  guard let table = registered(platform) else { return check(false) }
  check(table.sound == nil && table.play == nil && table.stopped == nil)

  final class Silent: ViewerScreen {
    func decode(_ frame: FrameInput) -> Bool { true }
    func show(_ number: UInt64) {}
    func source(_ message: SourceInput) {}
    func state() -> DecoderReport { DecoderReport(decodeMicros: 0, thermal: 0) }
    func ended() {}
    func released() {}
  }
  let quiet = Silent()
  let arena = Arena()
  quiet.sound(SoundForm(rate: 1, channels: 1, samples: 1))
  quiet.play(PlayInput(flags: 0, number: 0, captureTime: 0, lostBefore: 0, packet: arena.bytes([])))
  quiet.stopped(0)
  check(true)
}
