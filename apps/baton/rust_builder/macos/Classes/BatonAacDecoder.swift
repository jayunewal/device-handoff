// SPDX-License-Identifier: Apache-2.0

// Decoding of AAC-LC with AudioToolbox: the decoder that the system has, on a phone as on a Mac. It
// takes the packets of the sound of a screen in the form that the C interface hands them over
// (BatonViewer.swift): one access unit with no ADTS header and no other framing, 1024 samples to a
// packet, or 960 in the variant that some encoders make. It imports no more than the frameworks of
// the system, so that a Mac can build and test it without Flutter
// (apps/baton/rust_builder/swift_tests).

#if canImport(AudioToolbox)

import AudioToolbox
import Foundation

/// The object type of AAC-LC in the description of a stream (kMPEG4Object_AAC_LC).
private let aacLowComplexity: UInt32 = 2

/// What the input function answers when it has nothing more for this call. It is no error: the
/// samples that were made from what it gave are kept.
private let aacNoMoreInput: OSStatus = 1

/// The packet that one call of the decoder is given, handed to the converter by `aacPacketInput`.
private struct AacPacket {
  var bytes: UnsafeRawPointer?
  var size: UInt32
  var description: UnsafeMutablePointer<AudioStreamPacketDescription>?
  var channels: UInt32
  var supplied: Bool
}

/// The function with which the converter asks for packets. It gets the packet once for each call of
/// `decode` and then says that it has no more.
private func aacPacketInput(
  _ converter: AudioConverterRef, _ packets: UnsafeMutablePointer<UInt32>,
  _ data: UnsafeMutablePointer<AudioBufferList>,
  _ descriptions: UnsafeMutablePointer<UnsafeMutablePointer<AudioStreamPacketDescription>?>?,
  _ user: UnsafeMutableRawPointer?
) -> OSStatus {
  guard let user else {
    packets.pointee = 0
    return aacNoMoreInput
  }
  let input = user.assumingMemoryBound(to: AacPacket.self)
  guard !input.pointee.supplied, let bytes = input.pointee.bytes,
    let description = input.pointee.description
  else {
    packets.pointee = 0
    return aacNoMoreInput
  }
  input.pointee.supplied = true
  // The packets of AAC have no fixed size, so the converter is told how large this one is.
  description.pointee = AudioStreamPacketDescription(
    mStartOffset: 0, mVariableFramesInPacket: 0, mDataByteSize: input.pointee.size)
  descriptions?.pointee = description
  data.pointee.mNumberBuffers = 1
  data.pointee.mBuffers.mNumberChannels = input.pointee.channels
  data.pointee.mBuffers.mDataByteSize = input.pointee.size
  data.pointee.mBuffers.mData = UnsafeMutableRawPointer(mutating: bytes)
  packets.pointee = 1
  return noErr
}

/// What the decoder reads of a stream besides its packets, as ISO 14496-3 has it (the
/// AudioSpecificConfig): the object type, the sampling rate, the channels, and whether a packet
/// has 1024 samples or 960. The decoder needs it for the variant of 960 and gets by without it for
/// 1024. nil for a form that is not AAC-LC in one or two channels.
func aacSpecificConfig(rate: UInt32, channels: UInt32, samples: UInt32) -> [UInt8]? {
  guard channels == 1 || channels == 2, samples == 1024 || samples == 960 else { return nil }
  let rates: [UInt32] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
  ]
  var bits: [Bool] = []
  func put(_ value: UInt32, _ count: Int) {
    for shift in stride(from: count - 1, through: 0, by: -1) {
      bits.append((value >> UInt32(shift)) & 1 == 1)
    }
  }
  put(aacLowComplexity, 5)
  if let index = rates.firstIndex(of: rate) {
    put(UInt32(index), 4)
  } else {
    // A rate that has no number of its own is written out.
    put(15, 4)
    put(rate, 24)
  }
  put(channels, 4)
  put(samples == 960 ? 1 : 0, 1)
  put(0, 1)  // no core coder
  put(0, 1)  // no extension
  while bits.count % 8 != 0 { bits.append(false) }
  var bytes: [UInt8] = []
  var at = 0
  while at < bits.count {
    var byte: UInt8 = 0
    for bit in bits[at..<at + 8] { byte = (byte << 1) | (bit ? 1 : 0) }
    bytes.append(byte)
    at += 8
  }
  return bytes
}

/// Makes 32-bit floating point samples, interleaved, of AAC-LC packets. It is used from one queue
/// at a time.
final class AacDecoder: SoundDecoder {
  private let form: SoundForm
  private var converter: AudioConverterRef?
  /// Room for the samples of one packet.
  private var room: [Float]

  init(form: SoundForm) throws {
    guard form.channels == 1 || form.channels == 2, form.samples == 1024 || form.samples == 960,
      form.rate >= 8_000, form.rate <= 96_000
    else {
      throw SoundFailure(step: "choose the form of the decoder", code: -1)
    }
    self.form = form
    var input = AudioStreamBasicDescription(
      mSampleRate: Float64(form.rate), mFormatID: kAudioFormatMPEG4AAC,
      mFormatFlags: aacLowComplexity, mBytesPerPacket: 0, mFramesPerPacket: form.samples,
      mBytesPerFrame: 0, mChannelsPerFrame: form.channels, mBitsPerChannel: 0, mReserved: 0)
    var output = AudioStreamBasicDescription(
      mSampleRate: Float64(form.rate), mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
      mBytesPerPacket: 4 * form.channels, mFramesPerPacket: 1, mBytesPerFrame: 4 * form.channels,
      mChannelsPerFrame: form.channels, mBitsPerChannel: 32, mReserved: 0)
    var made: AudioConverterRef?
    let status = AudioConverterNew(&input, &output, &made)
    guard status == noErr, let made else {
      throw SoundFailure(step: "make the decoder", code: status)
    }
    if form.samples != 1024 {
      guard let config = aacSpecificConfig(
        rate: form.rate, channels: form.channels, samples: form.samples)
      else {
        AudioConverterDispose(made)
        throw SoundFailure(step: "describe the stream to the decoder", code: -2)
      }
      let set = AudioConverterSetProperty(
        made, kAudioConverterDecompressionMagicCookie, UInt32(config.count), config)
      guard set == noErr else {
        AudioConverterDispose(made)
        throw SoundFailure(step: "describe the stream to the decoder", code: set)
      }
    }
    converter = made
    room = [Float](repeating: 0, count: Int(form.samples) * Int(form.channels))
  }

  deinit {
    invalidate()
  }

  func decode(_ packet: Data) throws -> [Float]? {
    guard let converter else { throw SoundFailure(step: "use the decoder", code: -3) }
    // The protocol takes no more than 8 KiB in a packet, and a packet of no bytes has no sound.
    guard !packet.isEmpty, packet.count <= 8192 else {
      throw SoundFailure(step: "give the decoder a packet", code: -4)
    }
    let channels = form.channels
    var frames = form.samples
    var description = AudioStreamPacketDescription(
      mStartOffset: 0, mVariableFramesInPacket: 0, mDataByteSize: UInt32(packet.count))
    var status: OSStatus = noErr
    packet.withUnsafeBytes { (bytes: UnsafeRawBufferPointer) in
      withUnsafeMutablePointer(to: &description) { described in
        var input = AacPacket(
          bytes: bytes.baseAddress, size: UInt32(bytes.count), description: described,
          channels: channels, supplied: false)
        room.withUnsafeMutableBytes { out in
          var list = AudioBufferList(
            mNumberBuffers: 1,
            mBuffers: AudioBuffer(
              mNumberChannels: channels, mDataByteSize: UInt32(clamping: out.count),
              mData: out.baseAddress))
          status = withUnsafeMutablePointer(to: &input) { state in
            AudioConverterFillComplexBuffer(converter, aacPacketInput, state, &frames, &list, nil)
          }
        }
      }
    }
    guard status == noErr || status == aacNoMoreInput else {
      throw SoundFailure(step: "decode a packet", code: status)
    }
    let count = Int(frames) * Int(channels)
    guard count > 0, count <= room.count else { return nil }
    return Array(room[0..<count])
  }

  func reset() {
    guard let converter else { return }
    _ = AudioConverterReset(converter)
  }

  func invalidate() {
    guard let old = converter else { return }
    converter = nil
    AudioConverterDispose(old)
  }
}

#endif
