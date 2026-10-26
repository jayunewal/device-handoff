// SPDX-License-Identifier: Apache-2.0

// Tests of the decoder of AAC (BatonAacDecoder.swift) and of the part of the output
// (BatonEngineOutput.swift) that copies samples to the buffers of the device. They need
// AudioToolbox and AVFoundation, so they run on a Mac only. The packets that the decoder is given
// are made here with the encoder of the system, used as plainly as can be: the Mac that shares its
// screen makes them with its own.

#if canImport(AudioToolbox)

import AudioToolbox
import Foundation

private struct ToneInput {
  var samples: UnsafePointer<Float>?
  var frames: UInt32
  var channels: UInt32
  var supplied: Bool
}

/// What the input function answers when it has nothing more: no error, the packet is kept.
private let toneNoMoreInput: OSStatus = 1

private func toneInput(
  _ converter: AudioConverterRef, _ packets: UnsafeMutablePointer<UInt32>,
  _ data: UnsafeMutablePointer<AudioBufferList>,
  _ descriptions: UnsafeMutablePointer<UnsafeMutablePointer<AudioStreamPacketDescription>?>?,
  _ user: UnsafeMutableRawPointer?
) -> OSStatus {
  guard let user else {
    packets.pointee = 0
    return toneNoMoreInput
  }
  let input = user.assumingMemoryBound(to: ToneInput.self)
  guard !input.pointee.supplied, let samples = input.pointee.samples else {
    packets.pointee = 0
    return toneNoMoreInput
  }
  input.pointee.supplied = true
  data.pointee.mNumberBuffers = 1
  data.pointee.mBuffers.mNumberChannels = input.pointee.channels
  data.pointee.mBuffers.mDataByteSize = input.pointee.frames * input.pointee.channels * 4
  data.pointee.mBuffers.mData = UnsafeMutableRawPointer(mutating: samples)
  packets.pointee = input.pointee.frames
  return noErr
}

/// Up to `count` packets of AAC-LC of a sine wave of amplitude one half, one for each 1024 frames
/// of it, or nil if the system would not make them.
func tonePackets(frequency: Double, rate: UInt32, channels: UInt32, count: Int) -> [Data]? {
  var input = AudioStreamBasicDescription(
    mSampleRate: Float64(rate), mFormatID: kAudioFormatLinearPCM,
    mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
    mBytesPerPacket: 4 * channels, mFramesPerPacket: 1, mBytesPerFrame: 4 * channels,
    mChannelsPerFrame: channels, mBitsPerChannel: 32, mReserved: 0)
  var output = AudioStreamBasicDescription(
    mSampleRate: Float64(rate), mFormatID: kAudioFormatMPEG4AAC, mFormatFlags: 2,
    mBytesPerPacket: 0, mFramesPerPacket: 1024, mBytesPerFrame: 0, mChannelsPerFrame: channels,
    mBitsPerChannel: 0, mReserved: 0)
  var made: AudioConverterRef?
  guard AudioConverterNew(&input, &output, &made) == noErr, let converter = made else {
    return nil
  }
  defer { AudioConverterDispose(converter) }

  var room = [UInt8](repeating: 0, count: 8192)
  var packets: [Data] = []
  var phase = 0.0
  let step = 2.0 * Double.pi * frequency / Double(rate)
  let width = Int(channels)
  for _ in 0..<count {
    var samples = [Float](repeating: 0, count: 1024 * width)
    for frame in 0..<1024 {
      let value = Float(0.5 * sin(phase))
      phase += step
      for channel in 0..<width { samples[frame * width + channel] = value }
    }
    var produced: UInt32 = 1
    var description = AudioStreamPacketDescription()
    var size = 0
    let status: OSStatus = samples.withUnsafeBufferPointer { pointer in
      var state = ToneInput(
        samples: pointer.baseAddress, frames: 1024, channels: channels, supplied: false)
      return room.withUnsafeMutableBytes { bytes in
        var list = AudioBufferList(
          mNumberBuffers: 1,
          mBuffers: AudioBuffer(
            mNumberChannels: channels, mDataByteSize: UInt32(bytes.count), mData: bytes.baseAddress)
        )
        let status = withUnsafeMutablePointer(to: &state) { statePointer in
          AudioConverterFillComplexBuffer(
            converter, toneInput, statePointer, &produced, &list, &description)
        }
        size = Int(list.mBuffers.mDataByteSize)
        return status
      }
    }
    if status != noErr && status != toneNoMoreInput { return nil }
    if produced > 0 && size > 0 && size <= room.count { packets.append(Data(room[0..<size])) }
  }
  return packets
}

/// How loud `frequency` is in the first channel of `samples`, as the amplitude of a sine wave of
/// that frequency, by one bin of a Fourier transform. It is right, up to the loss of the codec, for
/// a number of frames that holds a whole number of waves.
func loudness(of samples: [Float], channels: Int, frequency: Double, rate: Double) -> Double {
  let frames = samples.count / channels
  guard frames > 0 else { return 0 }
  let turn = 2.0 * Double.pi * frequency / rate
  var real = 0.0
  var imaginary = 0.0
  for frame in 0..<frames {
    let value = Double(samples[frame * channels])
    real += value * cos(turn * Double(frame))
    imaginary += value * sin(turn * Double(frame))
  }
  return (real * real + imaginary * imaginary).squareRoot() * 2.0 / Double(frames)
}

/// Decodes `packets` and gives the samples of all but the first four, which hold the beginning of
/// the tone that the encoder and the decoder lead in with.
private func decodedTone(_ packets: [Data], form: SoundForm) -> [Float]? {
  guard let decoder = try? AacDecoder(form: form) else {
    check(false, "no decoder for \(form)")
    return nil
  }
  defer { decoder.invalidate() }
  var all: [Float] = []
  let width = Int(form.channels)
  for (at, packet) in packets.enumerated() {
    do {
      guard let samples = try decoder.decode(packet) else {
        check(false, "packet \(at) gave no samples")
        continue
      }
      check(samples.count == 1024 * width, "packet \(at) gave \(samples.count) samples")
      if at >= 4 { all += samples }
    } catch {
      check(false, "packet \(at) was not decoded: \(error)")
    }
  }
  return all
}

func testTheDecoderMakesTheToneBackThatTheEncoderOfTheSystemMadeOfIt() {
  let form = SoundForm(rate: 48_000, channels: 2, samples: 1024)
  guard let packets = tonePackets(frequency: 1500, rate: 48_000, channels: 2, count: 16),
    packets.count >= 12
  else { return check(false, "the encoder of the system made too few packets") }
  guard let tone = decodedTone(packets, form: form) else { return }
  // 1500 Hz is 32 waves in a packet of 1024 frames at 48 kHz.
  let wanted = loudness(of: tone, channels: 2, frequency: 1500, rate: 48_000)
  let other = loudness(of: tone, channels: 2, frequency: 6000, rate: 48_000)
  check(wanted > 0.42 && wanted < 0.58, "the tone is \(wanted) loud, and was made 0.5")
  check(other < 0.02, "a tone of 6000 Hz that was not there is \(other) loud")
  // Both channels hold the same tone.
  let left = stride(from: 0, to: tone.count, by: 2).map { tone[$0] }
  let right = stride(from: 1, to: tone.count, by: 2).map { tone[$0] }
  check(right.count == left.count)
  check(
    zip(left, right).allSatisfy { abs($0 - $1) < 0.05 },
    "the channels of a tone that is the same in both differ")
}

func testTheDecoderMakesAToneOfOneChannelAtTheRateOfCdsToo() {
  let form = SoundForm(rate: 44_100, channels: 1, samples: 1024)
  // 44100 / 32 Hz is 32 waves in a packet of 1024 frames.
  let frequency = 44_100.0 / 32.0
  guard let packets = tonePackets(frequency: frequency, rate: 44_100, channels: 1, count: 16),
    packets.count >= 12
  else { return check(false, "the encoder of the system made too few packets") }
  guard let tone = decodedTone(packets, form: form) else { return }
  let wanted = loudness(of: tone, channels: 1, frequency: frequency, rate: 44_100)
  check(wanted > 0.42 && wanted < 0.58, "the tone is \(wanted) loud, and was made 0.5")
}

func testTheDecoderGoesOnAfterAPacketThatIsNotAudioAndAReset() {
  let form = SoundForm(rate: 48_000, channels: 2, samples: 1024)
  guard let packets = tonePackets(frequency: 1500, rate: 48_000, channels: 2, count: 12),
    packets.count >= 10, let decoder = try? AacDecoder(form: form)
  else { return check(false, "no packets or no decoder") }
  for packet in packets.prefix(4) { _ = try? decoder.decode(packet) }
  // Bytes that are not a packet are refused, or decoded to noise: either is fine, a trap is not.
  _ = try? decoder.decode(Data([0xFF, 0x12, 0x34, 0x56, 0x78, 0x9A]))
  decoder.reset()
  var decoded = 0
  for packet in packets.suffix(4) {
    if let samples = try? decoder.decode(packet), samples.count == 2048 { decoded += 1 }
  }
  check(decoded == 4, "\(decoded) of 4 packets after the reset were decoded")

  do {
    _ = try decoder.decode(Data())
    check(false, "an empty packet was decoded")
  } catch {}
  do {
    _ = try decoder.decode(Data(count: 9000))
    check(false, "a packet that is larger than the protocol allows was decoded")
  } catch {}

  decoder.invalidate()
  decoder.invalidate()
  decoder.reset()
  do {
    _ = try decoder.decode(packets[0])
    check(false, "a decoder that was let go of decoded a packet")
  } catch {}
}

func testTheDecoderIsNotMadeForAFormThatIsNotAacLcOfOneOrTwoChannels() {
  for form in [
    SoundForm(rate: 48_000, channels: 0, samples: 1024),
    SoundForm(rate: 48_000, channels: 3, samples: 1024),
    SoundForm(rate: 48_000, channels: 2, samples: 480),
    SoundForm(rate: 48_000, channels: 2, samples: 2048),
    SoundForm(rate: 7_999, channels: 2, samples: 1024),
    SoundForm(rate: 96_001, channels: 2, samples: 1024),
  ] {
    do {
      _ = try AacDecoder(form: form)
      check(false, "a decoder was made for \(form)")
    } catch let failure as SoundFailure {
      check(!failure.step.isEmpty, "\(failure)")
    } catch {
      check(false, "an error that is not a failure of the sound: \(error)")
    }
  }
  check((try? AacDecoder(form: SoundForm(rate: 44_100, channels: 2, samples: 1024))) != nil)
  // The variant of 960 samples is set up with a description of the stream. Nothing here makes such
  // packets, so all that is looked at is that asking for it does not trap.
  _ = try? AacDecoder(form: SoundForm(rate: 44_100, channels: 1, samples: 960))
}

func testTheDescriptionOfTheStreamIsWhatTheStandardWritesAndNothingForWhatIsNotAacLc() {
  // Object type 2, the number of the rate, the configuration of the channels, the length of a
  // packet and two bits for what is not used: two bytes for the rates that have a number.
  check(aacSpecificConfig(rate: 48_000, channels: 2, samples: 1024) == [0x11, 0x90])
  check(aacSpecificConfig(rate: 44_100, channels: 2, samples: 1024) == [0x12, 0x10])
  check(aacSpecificConfig(rate: 44_100, channels: 1, samples: 960) == [0x12, 0x0C])
  // A rate with no number of its own is written out, in 24 bits.
  check(aacSpecificConfig(rate: 40_000, channels: 2, samples: 1024) == [0x17, 0x80, 0x4E, 0x20, 0x10])
  check(aacSpecificConfig(rate: 48_000, channels: 3, samples: 1024) == nil)
  check(aacSpecificConfig(rate: 48_000, channels: 2, samples: 512) == nil)
}

#endif

#if canImport(AVFoundation)

import AVFoundation

func testTheSamplesOfAnOutputGoToBuffersOfOneChannelEachAndToOneWithTheChannelsInterleaved() {
  // Four frames of two channels, interleaved, the way the ring gives them.
  let samples: [Float] = [1, 10, 2, 20, 3, 30, 4, 40]

  // The layout that the node is made for: a buffer for each channel. The copy begins at frame 1.
  let planar = AudioBufferList.allocate(maximumBuffers: 2)
  defer { free(planar.unsafeMutablePointer) }
  let left = UnsafeMutablePointer<Float>.allocate(capacity: 6)
  let right = UnsafeMutablePointer<Float>.allocate(capacity: 6)
  defer {
    left.deallocate()
    right.deallocate()
  }
  left.initialize(repeating: 0, count: 6)
  right.initialize(repeating: 0, count: 6)
  planar[0] = AudioBuffer(mNumberChannels: 1, mDataByteSize: 24, mData: left)
  planar[1] = AudioBuffer(mNumberChannels: 1, mDataByteSize: 24, mData: right)
  samples.withUnsafeBufferPointer { pointer in
    if let base = pointer.baseAddress {
      spreadSamples(base, frames: 4, channels: 2, offset: 1, into: planar)
    }
  }
  check(Array(UnsafeBufferPointer(start: left, count: 6)) == [0, 1, 2, 3, 4, 0])
  check(Array(UnsafeBufferPointer(start: right, count: 6)) == [0, 10, 20, 30, 40, 0])

  // One buffer with both channels in it.
  let woven = AudioBufferList.allocate(maximumBuffers: 1)
  defer { free(woven.unsafeMutablePointer) }
  let both = UnsafeMutablePointer<Float>.allocate(capacity: 12)
  defer { both.deallocate() }
  both.initialize(repeating: 0, count: 12)
  woven[0] = AudioBuffer(mNumberChannels: 2, mDataByteSize: 48, mData: both)
  samples.withUnsafeBufferPointer { pointer in
    if let base = pointer.baseAddress {
      spreadSamples(base, frames: 4, channels: 2, offset: 1, into: woven)
    }
  }
  check(
    Array(UnsafeBufferPointer(start: both, count: 12)) == [0, 0, 1, 10, 2, 20, 3, 30, 4, 40, 0, 0],
    "\(Array(UnsafeBufferPointer(start: both, count: 12)))")

  // One channel, in the one buffer that there is.
  let mono = AudioBufferList.allocate(maximumBuffers: 1)
  defer { free(mono.unsafeMutablePointer) }
  let single = UnsafeMutablePointer<Float>.allocate(capacity: 5)
  defer { single.deallocate() }
  single.initialize(repeating: 0, count: 5)
  mono[0] = AudioBuffer(mNumberChannels: 1, mDataByteSize: 20, mData: single)
  [Float(7), 8, 9].withUnsafeBufferPointer { pointer in
    if let base = pointer.baseAddress {
      spreadSamples(base, frames: 3, channels: 1, offset: 2, into: mono)
    }
  }
  check(Array(UnsafeBufferPointer(start: single, count: 5)) == [0, 0, 7, 8, 9])
}

#endif
