// SPDX-License-Identifier: Apache-2.0

// The Mac's side of sending the sound of a whole screen: what the library is told (the form of the
// packets, why there is none, each packet), when the Mac is kept quiet, how the samples that the
// system gives become packets, and how a Mac that has not let the app record its sound is found
// out. docs/adr/0012-sound.md says what the sound is and why the Mac is quiet while it is sent,
// and crates/baton-stream-abi/include/baton_stream_abi.h what the two sides promise each other.
//
// This file holds what does not depend on the frameworks of an Apple device. The tap that reads the
// sound and the encoder that makes the packets are given to it as protocols (BatonSoundMac.swift
// and BatonAacEncoder.swift have the ones of a Mac), so that it can be built and tested on any
// computer that has a Swift compiler (apps/baton/rust_builder/swift_tests); nothing here may import
// more than Foundation.
//
// A call from Rust must never end in a trap, because a trap ends the whole app: nothing here
// force-unwraps, indexes without looking, or asserts on what Rust passed.

import Foundation

// MARK: - What the library is told

// The form of the packets (`SoundForm`) is in BatonViewer.swift, and the failure of a tap, an
// encoder, a decoder or an output (`SoundFailure`) is in BatonSpeaker.swift: the device that plays
// the sound and the device that sends it speak of the same two.

/// Why the Mac sends no sound, as `baton_stream_source_sound_silent` has it.
enum SoundWhy: Equatable {
  /// The system has not let the app record the sound of this Mac.
  case noPermission
  /// Something went wrong where the sound is recorded or encoded.
  case failed
}

/// What the library did with the form of the packets.
enum SoundBegun: Equatable {
  /// It took the form, or the other device does not want to hear any more. Either way the packets
  /// go on being handed over, and the answer to each says what became of it.
  case taken
  /// It is not a form that the library takes.
  case refused
  /// The session is over.
  case over
}

/// What became of a packet that was handed over.
enum SoundSent: Equatable {
  /// It went out.
  case sent
  /// It is of no use: nobody wants to hear, or there was no room for it. Drop it and go on.
  case dropped
  /// It is larger than the protocol allows. Nothing went out.
  case tooLarge
  /// The session is over. Nothing went out.
  case over
  /// The library did not take it, because it is not a packet that can go on the wire.
  case refused
}

/// The calls the platform makes about the sound of a screen that it shares, as
/// `baton_stream_source_sound*` have them. Each one may be made from any thread.
protocol SoundSession: AnyObject {
  /// The microseconds since the session began, or nil once it is over.
  func clock() -> UInt64?
  /// Says what the packets are: for the other device, a run of sound begins here.
  func begin(_ form: SoundForm) -> SoundBegun
  /// Says why there is no sound, to the device that asked for it.
  func silent(_ why: SoundWhy)
  /// Hands over one packet. `captureTime` is when its first sample was recorded, on the clock of
  /// the session.
  func send(_ packet: Data, captureTime: UInt64) -> SoundSent
}

private let ok = Int32(BATON_OK)

/// The calls about the sound of a screen that is shared, made to the Rust library.
final class LibrarySoundSession: SoundSession {
  private let link: UInt64

  init(link: UInt64) {
    self.link = link
  }

  func clock() -> UInt64? {
    var micros: UInt64 = 0
    return baton_stream_source_clock(link, &micros) == ok ? micros : nil
  }

  func begin(_ form: SoundForm) -> SoundBegun {
    var c = BatonSound(
      size: UInt32(MemoryLayout<BatonSound>.size), codec: BATON_SOUND_CODEC_AAC, rate: form.rate,
      channels: form.channels, samples: form.samples, reserved: 0)
    let code = baton_stream_source_sound_begin(link, &c)
    if code == ok { return .taken }
    if code == Int32(BATON_E_UNKNOWN_LINK) { return .over }
    NSLog("Baton: the library did not take the form of the sound (%d).", code)
    return .refused
  }

  func silent(_ why: SoundWhy) {
    let reason: UInt32
    switch why {
    case .noPermission: reason = BATON_SILENT_NO_PERMISSION
    case .failed: reason = BATON_SILENT_FAILED
    }
    _ = baton_stream_source_sound_silent(link, reason)
  }

  func send(_ packet: Data, captureTime: UInt64) -> SoundSent {
    let link = self.link
    return packet.withUnsafeBytes { bytes in
      var c = BatonSoundPacket(
        size: UInt32(MemoryLayout<BatonSoundPacket>.size), reserved: 0, capture_time: captureTime,
        bytes: BatonBytes(ptr: bytes.bindMemory(to: UInt8.self).baseAddress, len: bytes.count))
      var outcome: UInt32 = 0
      var number: UInt64 = 0
      let code = baton_stream_source_sound(link, &c, &outcome, &number)
      if code == Int32(BATON_E_UNKNOWN_LINK) { return .over }
      if code != ok { return .refused }
      switch outcome {
      case BATON_SOUND_SENT: return .sent
      case BATON_SOUND_DROPPED: return .dropped
      case BATON_SOUND_TOO_LARGE: return .tooLarge
      case BATON_SOUND_OVER: return .over
      default: return .dropped
      }
    }
  }
}

// MARK: - What reads the sound and what encodes it

/// What reads the sound of the Mac: everything that every program but this app plays, as 32-bit
/// floating point samples of two channels, interleaved. While it reads, the sound does not reach the
/// speakers if it is told to keep it from them, and it comes back when the tap is stopped.
protocol SoundTap: AnyObject {
  /// Starts to read. `deliver` is given each block of samples, `frames * 2` of them, from a queue of
  /// the tap, and is not called after `stop` has returned. Returns the sampling rate in hertz.
  /// Throws `SoundFailure`, and leaves nothing behind when it does.
  func start(muted: Bool, deliver: @escaping ([Float]) -> Void) throws -> Double

  /// Stops reading and lets go of everything the tap holds: what plays is heard again. Waits until
  /// `deliver` is not being called.
  func stop()

  /// Whether some program is playing sound on the Mac now. A Mac that has not let the app record its
  /// sound gives silence, and silence is also what a Mac that plays nothing gives: this tells which.
  func someoneIsPlaying() -> Bool
}

/// What turns samples into packets.
protocol SoundEncoder: AnyObject {
  /// Makes the packet for `samples`, which is exactly one packet of two channels, interleaved. nil
  /// when the encoder has none to give for them yet. Throws `SoundFailure`.
  func encode(_ samples: [Float]) throws -> Data?

  /// Lets go of the encoder.
  func invalidate()
}

// MARK: - Changing the rate

/// Changes the sampling rate of samples of two channels by interpolating between neighbours, block
/// after block: the sound of a Mac whose speakers change their rate in the middle of a session
/// (another output is chosen) goes on in the form that the session began with. It is plain, and
/// that is enough for a rate that changes seldom; the delay it adds is one frame.
struct LinearResampler {
  /// How many frames of the input there are for each frame of the output.
  private let step: Double
  /// Where the next frame of the output lies, counted in frames of the input from `last`, which
  /// is at 0. The first frame of the first block is at 1.
  private var position = 1.0
  /// The last frame of the block before: left and right.
  private var last: (Float, Float) = (0, 0)

  /// nil unless both rates are positive.
  init?(from input: Double, to output: Double) {
    guard input.isFinite, output.isFinite, input > 0, output > 0 else { return nil }
    step = input / output
  }

  /// The block at the new rate. Two samples are a frame; an odd one at the end is ignored.
  mutating func convert(_ input: [Float]) -> [Float] {
    let frames = input.count / 2
    guard frames > 0 else { return [] }
    var output: [Float] = []
    output.reserveCapacity(Int(Double(frames) / step) * 2 + 4)
    while position < Double(frames) {
      let index = Int(position)
      let fraction = Float(position - Double(index))
      let before = frame(index, of: input)
      let after = frame(index + 1, of: input)
      output.append(before.0 + (after.0 - before.0) * fraction)
      output.append(before.1 + (after.1 - before.1) * fraction)
      position += step
    }
    position -= Double(frames)
    last = frame(frames, of: input)
    return output
  }

  /// Frame `index` of the line that begins with the last frame of the block before and goes on with
  /// `input`.
  private func frame(_ index: Int, of input: [Float]) -> (Float, Float) {
    guard index > 0 else { return last }
    let at = (index - 1) * 2
    guard at + 1 < input.count else { return last }
    return (input[at], input[at + 1])
  }
}

// MARK: - The sound of a screen that is shared

/// How many samples of each channel one packet holds. AAC-LC has 1024, and a viewer plays that.
let soundPacketSamples: UInt32 = 1024

/// The sound is sent in two channels.
let soundChannels: UInt32 = 2

/// The sampling rate in which the sound is sent for a tap that reads at `tapRate`: 44.1 kHz and
/// 48 kHz are the rates of the speakers of a Mac and of the encoder, and the sound of any other
/// rate (an output that runs at 96 kHz) is converted to 48 kHz.
func soundRate(forTap tapRate: Double) -> UInt32 {
  tapRate == 44_100 ? 44_100 : 48_000
}

/// How long the tap may read nothing but zeros while some program plays, before the Mac is taken for
/// one that has not let the app record its sound.
let soundSilenceSeconds = 2.0

/// Sends the sound of a Mac for the length of one session of a whole screen.
///
/// The other device asks to hear (`hear`) and stops asking. While it listens the tap reads, keeps
/// the sound from the speakers, and the packets go to the library; when it does not, the tap is
/// stopped and the Mac plays again. A screen that is private (`quiet`) keeps the Mac quiet for the
/// whole of the session instead: the tap is made at once (`keepQuiet`), what it reads is thrown
/// away while nobody listens, and it is stopped by `close`, which the platform calls when the Mac
/// is locked again.
///
/// Everything happens on one queue, and the calls from outside only put work on it.
final class SoundPump {
  private let queue = DispatchQueue(label: "io.github.jayunewal.baton.sound")
  private let session: SoundSession
  private let quiet: Bool
  private let makeTap: () -> SoundTap
  private let makeEncoder: (SoundForm) throws -> SoundEncoder

  // Everything below is touched on `queue` and nowhere else.

  /// The form of the session, once a run has begun: every later run has it as well.
  private var form: SoundForm?
  private var tap: SoundTap?
  /// The rate at which `tap` reads.
  private var tapRate = 0.0
  /// Which tap the samples come from. Blocks that a tap left in the queue when it was stopped are
  /// not the sound of the one that was made after it.
  private var generation = 0
  private var encoder: SoundEncoder?
  private var resampler: LinearResampler?
  /// Whether packets are handed to the library: a run has begun and has not ended.
  private var running = false
  /// The samples of the run that do not make a whole packet yet.
  private var pending: [Float] = []
  /// How many frames of silence the tap has read in a row, whether some program played when that
  /// count began (nil until the first silence), and whether the tap ever read anything else.
  private var silentFrames = 0
  private var playedAtBegin: Bool?
  private var heardSound = false
  /// How many packets of this run the library refused.
  private var refusals = 0
  private var closed = false

  init(
    session: SoundSession, quiet: Bool, makeTap: @escaping () -> SoundTap,
    makeEncoder: @escaping (SoundForm) throws -> SoundEncoder
  ) {
    self.session = session
    self.quiet = quiet
    self.makeTap = makeTap
    self.makeEncoder = makeEncoder
  }

  /// Keeps this Mac quiet from now until `close`, whether or not the other device listens. It is for
  /// a screen that is private: nobody is at the Mac to hear it, and the person at the other device
  /// has the sound or none. The Mac is not quiet if the tap cannot be made, and the log says so.
  func keepQuiet() {
    queue.sync {
      guard !closed else { return }
      _ = ensureTap()
    }
  }

  /// The other device wants to hear the sound (true), or does not any more (false). A true comes
  /// again, with no false between, when the person there asks anew; a run begins at each.
  func hear(_ on: Bool) {
    queue.async { [self] in
      guard !closed else { return }
      if on {
        beginRun()
      } else {
        endRun()
        if !quiet { stopTap() }
      }
    }
  }

  /// The session is over and the Mac may play again: stops the tap and lets go of everything. Waits
  /// for the work that is queued, and nothing is done after it.
  func close() {
    queue.sync {
      closed = true
      endRun()
      stopTap()
    }
  }

  /// Waits until the work that is queued has been done. For tests.
  func idle() {
    queue.sync {}
  }

  // MARK: - A run of sound

  private func beginRun() {
    endRun()
    guard ensureTap() else {
      tellWhy(.failed)
      return
    }
    let chosen =
      form
      ?? SoundForm(
        rate: soundRate(forTap: tapRate), channels: soundChannels, samples: soundPacketSamples)
    do {
      encoder = try makeEncoder(chosen)
    } catch {
      NSLog("Baton: the encoder of the sound could not be made (%@).", String(describing: error))
      tellWhy(.failed)
      return
    }
    // The sound of a Mac whose speakers changed their rate since the first run is brought to the
    // form of the session, which has one for the whole of it.
    resampler =
      Double(chosen.rate) == tapRate ? nil : LinearResampler(from: tapRate, to: Double(chosen.rate))
    switch session.begin(chosen) {
    case .taken:
      form = chosen
      running = true
    case .refused:
      tellWhy(.failed)
    case .over:
      finish()
    }
  }

  /// The run is over, and nothing is handed to the library until the next begins.
  private func endRun() {
    running = false
    pending.removeAll(keepingCapacity: true)
    encoder?.invalidate()
    encoder = nil
    resampler = nil
    silentFrames = 0
    playedAtBegin = nil
    refusals = 0
  }

  /// The run cannot go on: the other device is told why, and the Mac plays again unless it is kept
  /// quiet for the whole session.
  private func tellWhy(_ why: SoundWhy) {
    endRun()
    session.silent(why)
    if !quiet { stopTap() }
  }

  /// The session is over: the library says so, and nothing is told to the other device.
  private func finish() {
    endRun()
    if !quiet { stopTap() }
  }

  // MARK: - The tap

  /// Makes the tap if there is none. false if it could not be made.
  private func ensureTap() -> Bool {
    if tap != nil { return true }
    let made = makeTap()
    generation += 1
    let mine = generation
    let rate: Double
    do {
      rate = try made.start(muted: true) { [weak self] samples in
        guard let pump = self else { return }
        pump.queue.async { pump.read(samples, from: mine) }
      }
    } catch {
      NSLog("Baton: the sound of this Mac could not be read (%@).", String(describing: error))
      return false
    }
    // A rate that the protocol does not allow cannot be sent, and a rate that is not a number
    // could not be told apart from a rate to convert from.
    guard rate.isFinite, rate >= 8_000, rate <= 96_000 else {
      made.stop()
      NSLog("Baton: the sound of this Mac has a rate that cannot be sent (%f).", rate)
      return false
    }
    tap = made
    tapRate = rate
    silentFrames = 0
    playedAtBegin = nil
    return true
  }

  private func stopTap() {
    guard let old = tap else { return }
    tap = nil
    generation += 1
    old.stop()
  }

  // MARK: - The samples

  private func read(_ block: [Float], from source: Int) {
    guard !closed, running, source == generation, tap != nil else { return }
    guard block.count >= 2, let form, let encoder else { return }
    guard noticeSilence(in: block) else { return }
    var samples = block
    if var converter = resampler {
      samples = converter.convert(block)
      resampler = converter
    }
    pending.append(contentsOf: samples)
    guard let now = session.clock() else {
      finish()
      return
    }
    let channels = Int(form.channels)
    let packetSamples = Int(form.samples) * channels
    guard channels > 0, packetSamples > 0 else { return }
    // The block ends about now, so the sound that is held, the end of this block included, began
    // that long ago. Packets are cut from the front, one packet of sound after the other.
    let rate = Double(form.rate)
    let held = Double(pending.count / channels) / rate * 1_000_000
    let first = micros(Double(now) - held)
    let length = Double(form.samples) / rate * 1_000_000
    var used = 0
    var made = 0
    while pending.count - used >= packetSamples {
      let packet: Data?
      do {
        packet = try encoder.encode(Array(pending[used..<used + packetSamples]))
      } catch {
        NSLog("Baton: a packet of sound could not be made (%@).", String(describing: error))
        tellWhy(.failed)
        return
      }
      used += packetSamples
      let captured = first + micros(Double(made) * length)
      made += 1
      guard let packet else { continue }
      if !hand(packet, captured) { return }
    }
    pending.removeFirst(used)
  }

  /// Hands a packet to the library. false once the run is over.
  private func hand(_ packet: Data, _ captured: UInt64) -> Bool {
    switch session.send(packet, captureTime: captured) {
    case .sent, .dropped:
      return true
    case .tooLarge, .refused:
      refusals += 1
      if refusals == 1 { NSLog("Baton: the library did not take a packet of sound.") }
      return true
    case .over:
      finish()
      return false
    }
  }

  /// Looks at what the tap read for the sign that the system has not let the app record the sound:
  /// silence for `soundSilenceSeconds` while some program plays, a program that was there when the
  /// count of the silence began and still is at its end. A program that begins to play within the
  /// count may not have been read yet, so one that is there only at the end is not the sign, and
  /// the next count is the first in which it can be. Sound that was ever heard puts the question to
  /// rest, since a program that plays silence on purpose would otherwise be taken for a Mac that
  /// gives none. false if the sign was found, and the run has ended.
  private func noticeSilence(in samples: [Float]) -> Bool {
    if heardSound { return true }
    if samples.contains(where: { $0 != 0 }) {
      heardSound = true
      silentFrames = 0
      return true
    }
    if playedAtBegin == nil { playedAtBegin = tap?.someoneIsPlaying() == true }
    silentFrames += samples.count / 2
    guard Double(silentFrames) / tapRate >= soundSilenceSeconds else { return true }
    let played = playedAtBegin == true
    // The look that ends this count is the one that begins the next.
    let plays = tap?.someoneIsPlaying() == true
    silentFrames = 0
    playedAtBegin = plays
    guard played, plays else { return true }
    NSLog("Baton: the Mac reads only silence while a program plays: the app may not record it.")
    tellWhy(.noPermission)
    return false
  }

  /// A time in microseconds for the library: a negative one, or one that is not a number, is 0.
  private func micros(_ value: Double) -> UInt64 {
    guard value.isFinite, value > 0 else { return 0 }
    return UInt64(min(value, 1e18))
  }
}
