// SPDX-License-Identifier: Apache-2.0

// The Mac's side of reading the sound of a screen that is shared: a Core Audio process tap hands the
// app the sound of every program but the app itself and, while it is read, keeps that sound from the
// speakers (docs/adr/0012-sound.md, and docs/research/sound for what was measured). The tap and the
// private aggregate device that carries it belong to the app: they go with the process, so a Mac
// whose app is quit, killed or crashed plays again.
//
// What decides when the tap is made, and what becomes of what it reads, is in BatonSound.swift,
// which does not depend on any of this. An iPhone or an iPad cannot be tapped, so there is nothing
// here for them.

#if os(macOS)

import AudioToolbox
import CoreAudio
import Foundation

/// The tap that reads the sound of the Mac. It is used from one queue at a time.
@available(macOS 14.2, *)
final class MacSoundTap: SoundTap {
  private let queue = DispatchQueue(
    label: "io.github.jayunewal.baton.sound.tap", qos: .userInitiated)
  private var tap = AudioObjectID(kAudioObjectUnknown)
  private var device = AudioObjectID(kAudioObjectUnknown)
  private var procedure: AudioDeviceIOProcID?
  private var started = false

  deinit {
    stop()
  }

  func start(muted: Bool, deliver: @escaping ([Float]) -> Void) throws -> Double {
    guard !started else { throw SoundFailure(step: "start the tap twice", code: -1) }
    // Everything but this app: a sound that the app makes is not sent back.
    let description = CATapDescription(stereoGlobalTapButExcludeProcesses: ownProcessObjects())
    description.name = "Baton sound"
    description.isPrivate = true
    description.muteBehavior = muted ? .muted : .unmuted

    var status = AudioHardwareCreateProcessTap(description, &tap)
    guard status == noErr else { throw failed("make the tap", status) }

    var formatAddress = AudioObjectPropertyAddress(
      mSelector: kAudioTapPropertyFormat, mScope: kAudioObjectPropertyScopeGlobal,
      mElement: kAudioObjectPropertyElementMain)
    var format = AudioStreamBasicDescription()
    var formatSize = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    status = AudioObjectGetPropertyData(tap, &formatAddress, 0, nil, &formatSize, &format)
    guard status == noErr else { throw failed("ask for the format of the tap", status) }
    // Two channels of 32-bit floating point samples is what the tap gives for a stereo mixdown.
    // Anything else is not read, since the encoder takes this and nothing else.
    guard format.mFormatID == kAudioFormatLinearPCM, format.mChannelsPerFrame == 2,
      format.mBitsPerChannel == 32, format.mFormatFlags & kAudioFormatFlagIsFloat != 0
    else { throw failed("read a format of the tap that is not two channels of floats", -2) }
    let interleaved = format.mFormatFlags & kAudioFormatFlagIsNonInterleaved == 0

    let aggregate: [String: Any] = [
      kAudioAggregateDeviceNameKey: "Baton sound",
      kAudioAggregateDeviceUIDKey: UUID().uuidString,
      kAudioAggregateDeviceIsPrivateKey: true,
      kAudioAggregateDeviceTapAutoStartKey: true,
      kAudioAggregateDeviceTapListKey: [
        [
          kAudioSubTapUIDKey: description.uuid.uuidString,
          kAudioSubTapDriftCompensationKey: true,
        ]
      ],
    ]
    status = AudioHardwareCreateAggregateDevice(aggregate as CFDictionary, &device)
    guard status == noErr else { throw failed("make the device of the tap", status) }

    status = AudioDeviceCreateIOProcIDWithBlock(&procedure, device, queue) { _, input, _, _, _ in
      deliver(MacSoundTap.samples(of: input, interleaved: interleaved))
    }
    guard status == noErr, procedure != nil else { throw failed("read the tap", status) }

    status = AudioDeviceStart(device, procedure)
    guard status == noErr else { throw failed("start the tap", status) }
    started = true
    return format.mSampleRate
  }

  func stop() {
    if started, let procedure { AudioDeviceStop(device, procedure) }
    started = false
    // Nothing is delivered after this: the block has run to its end.
    queue.sync {}
    tearDown()
  }

  /// Whether some program other than this app is playing sound now: the system lists the programs
  /// that use audio and says which of them have output running.
  func someoneIsPlaying() -> Bool {
    let mine = Set(ownProcessObjects())
    for process in processObjects() where !mine.contains(process) {
      var address = AudioObjectPropertyAddress(
        mSelector: kAudioProcessPropertyIsRunningOutput, mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
      var running: UInt32 = 0
      var size = UInt32(MemoryLayout<UInt32>.size)
      if AudioObjectGetPropertyData(process, &address, 0, nil, &size, &running) == noErr,
        running != 0
      {
        return true
      }
    }
    return false
  }

  // MARK: - What the tap and its device are made of

  /// Lets go of what was made, last to first, whatever of it there is.
  private func tearDown() {
    if let old = procedure, device != AudioObjectID(kAudioObjectUnknown) {
      AudioDeviceDestroyIOProcID(device, old)
    }
    procedure = nil
    if device != AudioObjectID(kAudioObjectUnknown) {
      AudioHardwareDestroyAggregateDevice(device)
      device = AudioObjectID(kAudioObjectUnknown)
    }
    if tap != AudioObjectID(kAudioObjectUnknown) {
      AudioHardwareDestroyProcessTap(tap)
      tap = AudioObjectID(kAudioObjectUnknown)
    }
  }

  /// What a step that did not work leaves behind is let go of, and the failure is the answer.
  private func failed(_ step: String, _ status: OSStatus) -> SoundFailure {
    tearDown()
    return SoundFailure(step: step, code: status)
  }

  /// The audio object of this process, if the system knows it as one that uses audio: the tap leaves
  /// it out. A process that has never used audio is not one, and there is nothing to leave out.
  private func ownProcessObjects() -> [AudioObjectID] {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject,
      mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var pid = getpid()
    var object = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.size)
    let status = AudioObjectGetPropertyData(
      AudioObjectID(kAudioObjectSystemObject), &address, UInt32(MemoryLayout<pid_t>.size), &pid,
      &size, &object)
    guard status == noErr, object != AudioObjectID(kAudioObjectUnknown) else { return [] }
    return [object]
  }

  /// The audio objects of the processes that use audio.
  private func processObjects() -> [AudioObjectID] {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioHardwarePropertyProcessObjectList, mScope: kAudioObjectPropertyScopeGlobal,
      mElement: kAudioObjectPropertyElementMain)
    var size: UInt32 = 0
    let system = AudioObjectID(kAudioObjectSystemObject)
    guard AudioObjectGetPropertyDataSize(system, &address, 0, nil, &size) == noErr, size > 0
    else { return [] }
    var objects = [AudioObjectID](
      repeating: AudioObjectID(kAudioObjectUnknown),
      count: Int(size) / MemoryLayout<AudioObjectID>.size)
    guard AudioObjectGetPropertyData(system, &address, 0, nil, &size, &objects) == noErr
    else { return [] }
    return objects
  }

  /// The samples of one block, two channels interleaved. A tap that gives each channel a buffer of
  /// its own is interleaved here.
  private static func samples(of list: UnsafePointer<AudioBufferList>, interleaved: Bool)
    -> [Float]
  {
    let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: list))
    let width = MemoryLayout<Float>.size
    if interleaved {
      guard let buffer = buffers.first, let data = buffer.mData else { return [] }
      let count = Int(buffer.mDataByteSize) / width
      let floats = data.assumingMemoryBound(to: Float.self)
      return Array(UnsafeBufferPointer(start: floats, count: count - count % 2))
    }
    guard buffers.count >= 2, let left = buffers[0].mData, let right = buffers[1].mData else {
      return []
    }
    let frames = Int(min(buffers[0].mDataByteSize, buffers[1].mDataByteSize)) / width
    let one = left.assumingMemoryBound(to: Float.self)
    let other = right.assumingMemoryBound(to: Float.self)
    var samples = [Float](repeating: 0, count: frames * 2)
    for frame in 0..<frames {
      samples[frame * 2] = one[frame]
      samples[frame * 2 + 1] = other[frame]
    }
    return samples
  }
}

#endif
