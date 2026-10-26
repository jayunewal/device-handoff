// SPDX-License-Identifier: Apache-2.0

// Encoding of AAC-LC with AudioToolbox: the encoder that the system has, on a Mac as on a phone. It
// makes the packets of the sound of a screen that this device shares, in the form that the C
// interface takes them (BatonSound.swift): one access unit with no ADTS header and no other
// framing, 1024 samples to a packet. It imports no more than the frameworks of the system, so that
// a Mac can build and test it without Flutter (apps/baton/rust_builder/swift_tests).

#if canImport(AudioToolbox)

import AudioToolbox
import Foundation

/// The object type of AAC-LC in the description of a stream (kMPEG4Object_AAC_LC).
private let aacLowComplexity: UInt32 = 2

/// The bit rate of the packets: 128 kbit/s in two channels is clear to the ear and about 340 bytes
/// to a packet of 21 ms at 48 kHz.
private let aacBitRate: UInt32 = 128_000

/// What the input function answers when it has nothing more for this call. It is no error: the
/// packet that was made from what it gave is kept.
private let aacNoMoreInput: OSStatus = 1

/// The samples that one call of the encoder is given, handed to the converter by `aacInput`.
private struct AacInput {
  var samples: UnsafePointer<Float>?
  var frames: UInt32
  var channels: UInt32
  var supplied: Bool
}

/// The function with which the converter asks for samples. It gets the packet of samples once for
/// each call of `encode` and then says that it has no more.
private func aacInput(
  _ converter: AudioConverterRef, _ packets: UnsafeMutablePointer<UInt32>,
  _ data: UnsafeMutablePointer<AudioBufferList>,
  _ descriptions: UnsafeMutablePointer<UnsafeMutablePointer<AudioStreamPacketDescription>?>?,
  _ user: UnsafeMutableRawPointer?
) -> OSStatus {
  guard let user else {
    packets.pointee = 0
    return aacNoMoreInput
  }
  let input = user.assumingMemoryBound(to: AacInput.self)
  guard !input.pointee.supplied, let samples = input.pointee.samples else {
    packets.pointee = 0
    return aacNoMoreInput
  }
  input.pointee.supplied = true
  data.pointee.mNumberBuffers = 1
  data.pointee.mBuffers.mNumberChannels = input.pointee.channels
  data.pointee.mBuffers.mDataByteSize =
    input.pointee.frames * input.pointee.channels * UInt32(MemoryLayout<Float>.size)
  data.pointee.mBuffers.mData = UnsafeMutableRawPointer(mutating: samples)
  packets.pointee = input.pointee.frames
  return noErr
}

/// Makes AAC-LC packets of 32-bit floating point samples, interleaved. It is used from one queue at
/// a time.
final class AacEncoder: SoundEncoder {
  private let form: SoundForm
  private var converter: AudioConverterRef?
  /// Room for the largest packet that the converter can make.
  private var room: [UInt8]

  init(form: SoundForm) throws {
    guard form.channels == 1 || form.channels == 2, form.samples == 1024 else {
      throw SoundFailure(step: "choose the form of the encoder", code: -1)
    }
    self.form = form
    var input = AudioStreamBasicDescription(
      mSampleRate: Float64(form.rate), mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
      mBytesPerPacket: 4 * form.channels, mFramesPerPacket: 1, mBytesPerFrame: 4 * form.channels,
      mChannelsPerFrame: form.channels, mBitsPerChannel: 32, mReserved: 0)
    var output = AudioStreamBasicDescription(
      mSampleRate: Float64(form.rate), mFormatID: kAudioFormatMPEG4AAC,
      mFormatFlags: aacLowComplexity, mBytesPerPacket: 0, mFramesPerPacket: form.samples,
      mBytesPerFrame: 0, mChannelsPerFrame: form.channels, mBitsPerChannel: 0, mReserved: 0)
    var made: AudioConverterRef?
    let status = AudioConverterNew(&input, &output, &made)
    guard status == noErr, let made else {
      throw SoundFailure(step: "make the encoder", code: status)
    }
    var rate = aacBitRate * form.channels / 2
    let set = AudioConverterSetProperty(
      made, kAudioConverterEncodeBitRate, UInt32(MemoryLayout<UInt32>.size), &rate)
    guard set == noErr else {
      AudioConverterDispose(made)
      throw SoundFailure(step: "set the bit rate of the encoder", code: set)
    }
    // The converter says how large a packet can be, and 8 KiB, the most the protocol takes, is
    // the least that is kept ready.
    var largest: UInt32 = 0
    var size = UInt32(MemoryLayout<UInt32>.size)
    let asked = AudioConverterGetProperty(
      made, kAudioConverterPropertyMaximumOutputPacketSize, &size, &largest)
    converter = made
    room = [UInt8](repeating: 0, count: asked == noErr ? Int(max(largest, 8192)) : 8192)
  }

  deinit {
    invalidate()
  }

  func encode(_ samples: [Float]) throws -> Data? {
    guard let converter else { throw SoundFailure(step: "use the encoder", code: -2) }
    guard samples.count == Int(form.samples) * Int(form.channels) else {
      throw SoundFailure(step: "give the encoder one packet of samples", code: -3)
    }
    var produced: UInt32 = 1
    var description = AudioStreamPacketDescription()
    var size = 0
    let frames = form.samples
    let channels = form.channels
    let status: OSStatus = samples.withUnsafeBufferPointer { pointer in
      var input = AacInput(
        samples: pointer.baseAddress, frames: frames, channels: channels, supplied: false)
      return room.withUnsafeMutableBytes { bytes in
        var list = AudioBufferList(
          mNumberBuffers: 1,
          mBuffers: AudioBuffer(
            mNumberChannels: channels, mDataByteSize: UInt32(clamping: bytes.count),
            mData: bytes.baseAddress))
        let status = withUnsafeMutablePointer(to: &input) { state in
          AudioConverterFillComplexBuffer(converter, aacInput, state, &produced, &list, &description)
        }
        size = Int(list.mBuffers.mDataByteSize)
        return status
      }
    }
    guard status == noErr || status == aacNoMoreInput else {
      throw SoundFailure(step: "encode a packet", code: status)
    }
    guard produced > 0, size > 0, size <= room.count else { return nil }
    return Data(room[0..<size])
  }

  func invalidate() {
    guard let old = converter else { return }
    converter = nil
    AudioConverterDispose(old)
  }
}

#endif
