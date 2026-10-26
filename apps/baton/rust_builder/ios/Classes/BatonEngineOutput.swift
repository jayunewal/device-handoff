// SPDX-License-Identifier: Apache-2.0

// The output of the sound of a screen: an audio engine of the system that reads the ring
// (BatonSpeaker.swift) from the thread of the device and plays it where the person has chosen to
// hear their sound, which is the speaker, the headphones or whatever else is connected. It imports
// no more than the frameworks of the system, so that a Mac can build it and test the part that is
// plain without Flutter (apps/baton/rust_builder/swift_tests). Nothing here starts an engine in a
// test, since a computer with no sound device does not take one.

#if canImport(AVFoundation)

import AVFoundation
import Foundation

/// How many frames the thread of the device is served at once. A request for more is served in
/// pieces.
private let renderFrames = 4096

/// The memory that the thread of the device works in, made before it runs: it must not allocate.
private final class RenderRoom {
  let samples: UnsafeMutablePointer<Float>

  init(channels: Int) {
    let count = renderFrames * max(channels, 1)
    samples = .allocate(capacity: count)
    samples.initialize(repeating: 0, count: count)
  }

  deinit {
    samples.deallocate()
  }
}

/// Copies `frames` frames of interleaved samples to the buffers that the device gave, from frame
/// `offset` on. The device asks for the layout that the node was made for, one buffer to a channel,
/// and a layout of one buffer with the channels interleaved is served as well.
func spreadSamples(
  _ samples: UnsafePointer<Float>, frames: Int, channels: Int, offset: Int,
  into buffers: UnsafeMutableAudioBufferListPointer
) {
  guard frames > 0, channels > 0, offset >= 0 else { return }
  if buffers.count == 1 && channels > 1 && buffers[0].mNumberChannels == UInt32(channels) {
    guard let data = buffers[0].mData?.assumingMemoryBound(to: Float.self) else { return }
    (data + offset * channels).update(from: samples, count: frames * channels)
    return
  }
  for channel in 0..<min(buffers.count, channels) {
    guard let data = buffers[channel].mData?.assumingMemoryBound(to: Float.self) else { continue }
    for frame in 0..<frames {
      data[offset + frame] = samples[frame * channels + channel]
    }
  }
}

/// The code that the system gave for an error, for `SoundFailure`.
private func code(of error: Error) -> Int32 {
  Int32(truncatingIfNeeded: (error as NSError).code)
}

/// Plays a ring with an engine of the system.
final class EngineOutput: SoundOutput {
  private let engine: AVAudioEngine
  private let node: AVAudioSourceNode
  private let lock = NSLock()
  private var running = false
  /// Whether this output holds the audio session of the app, which a phone has one of.
  private var holdsDevice = false
  private var observers: [NSObjectProtocol] = []

  /// Throws `SoundFailure` when the device has nothing to play on. The engine is made here and
  /// started by `start`.
  init(form: SoundForm, ring: SoundRing) throws {
    // The session before the engine looks at the device: what the device is depends on it.
    try EngineOutput.takeTheDevice()
    var made = false
    defer { if !made { EngineOutput.letGoOfTheDevice() } }

    let engine = AVAudioEngine()
    // An engine that is started on a computer with no sound device stops the app with an
    // exception, so the device is asked first.
    let hardware = engine.outputNode.outputFormat(forBus: 0)
    guard hardware.sampleRate > 0, hardware.channelCount > 0 else {
      throw SoundFailure(step: "find a device to play on", code: -1)
    }
    guard
      let format = AVAudioFormat(
        standardFormatWithSampleRate: Double(form.rate),
        channels: AVAudioChannelCount(form.channels))
    else {
      throw SoundFailure(step: "describe the sound to the engine", code: -2)
    }
    let channels = Int(form.channels)
    let room = RenderRoom(channels: channels)
    let node = AVAudioSourceNode(format: format) { _, _, frameCount, audioBufferList -> OSStatus in
      let buffers = UnsafeMutableAudioBufferListPointer(audioBufferList)
      let total = Int(frameCount)
      var done = 0
      while done < total {
        let piece = min(total - done, renderFrames)
        ring.read(into: room.samples, frames: piece)
        spreadSamples(room.samples, frames: piece, channels: channels, offset: done, into: buffers)
        done += piece
      }
      return noErr
    }
    engine.attach(node)
    engine.connect(node, to: engine.mainMixerNode, format: format)
    self.engine = engine
    self.node = node
    holdsDevice = true
    made = true
  }

  func start() throws {
    lock.lock()
    defer { lock.unlock() }
    guard !running else { return }
    do {
      try engine.start()
    } catch {
      throw SoundFailure(step: "start the engine", code: code(of: error))
    }
    running = true
    watch()
  }

  func stop() {
    lock.lock()
    running = false
    let watching = observers
    observers = []
    let release = holdsDevice
    holdsDevice = false
    lock.unlock()
    for observer in watching { NotificationCenter.default.removeObserver(observer) }
    engine.stop()
    if release { EngineOutput.letGoOfTheDevice() }
  }

  /// A phone has one audio session, and the app plays on it: as media, so that the sound is heard
  /// also when the ringer is off, and other apps that play are paused.
  private static func takeTheDevice() throws {
    #if os(iOS)
    let session = AVAudioSession.sharedInstance()
    do {
      try session.setCategory(.playback, mode: .default, options: [])
      try session.setActive(true)
    } catch {
      throw SoundFailure(step: "take the speaker", code: code(of: error))
    }
    #endif
  }

  /// The app gives the session back, and an app that played before is told that it may play again.
  private static func letGoOfTheDevice() {
    #if os(iOS)
    try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    #endif
  }

  /// A change of the device (headphones plugged in or taken out, another output chosen) stops the
  /// engine, and so does a call or an alarm on a phone. The sound goes on when it is over.
  private func watch() {
    let center = NotificationCenter.default
    observers.append(
      center.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: nil) {
        [weak self] _ in
        self?.resume()
      })
    #if os(iOS)
    observers.append(
      center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: nil) {
        [weak self] note in
        let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt
        if raw == AVAudioSession.InterruptionType.ended.rawValue { self?.resume() }
      })
    #endif
  }

  private func resume() {
    lock.lock()
    defer { lock.unlock() }
    guard running, !engine.isRunning else { return }
    #if os(iOS)
    try? AVAudioSession.sharedInstance().setActive(true)
    #endif
    do {
      try engine.start()
    } catch {
      NSLog("Baton: the sound of a screen could not go on after the device changed.")
    }
  }
}

#endif
