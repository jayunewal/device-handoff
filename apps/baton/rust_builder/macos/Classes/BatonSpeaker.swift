// SPDX-License-Identifier: Apache-2.0

// The sound of a screen that another device shares, on this device: the packets are decoded, kept
// in a buffer that evens out the time they take to arrive, and played. docs/adr/0012-sound.md says
// what the sound is and why the buffer is as large as it is, and
// crates/baton-stream-abi/include/baton_stream_abi.h what the Rust side promises when it calls
// sound, play and stopped.
//
// This file holds what does not depend on the frameworks of an Apple device: the buffer, and the
// speaker that makes a run of sound out of the calls. The decoder (BatonAacDecoder.swift) and the
// output (BatonEngineOutput.swift) are given to it as protocols, so that it can be built and tested
// on any computer that has a Swift compiler (apps/baton/rust_builder/swift_tests); nothing here may
// import more than Foundation.
//
// A call from Rust must never end in a trap, because a trap ends the whole app: nothing here
// force-unwraps, indexes without looking at the numbers, or asserts on what Rust passed.

import Foundation

// MARK: - What decodes the packets and what plays them

/// Why a tap, an encoder, a decoder or an output could not be made, started or used: the step, and
/// the code that the system gave. It holds no sound and nothing that names a program.
struct SoundFailure: Error, Equatable {
  var step: String
  var code: Int32
}

/// What turns the packets of one run into samples. It is used from one queue at a time.
protocol SoundDecoder: AnyObject {
  /// The samples of one packet: 32-bit floating point, interleaved, one packet of the form that the
  /// decoder was made for. nil when nothing came out of the packet. Throws `SoundFailure` when the
  /// packet cannot be decoded.
  func decode(_ packet: Data) throws -> [Float]?

  /// What the packets before left in the decoder is forgotten: the packet that comes next does not
  /// go on from them.
  func reset()

  /// Lets go of the decoder.
  func invalidate()
}

/// What plays the samples of a ring: on a device the speaker, or the headphones, or whatever the
/// person has chosen.
protocol SoundOutput: AnyObject {
  /// Starts to take sound from the ring. Throws `SoundFailure`, and leaves nothing behind when it
  /// does.
  func start() throws

  /// Stops, and lets go of the device: what plays on it is heard again, and nothing is taken from
  /// the ring after this returns.
  func stop()
}

// MARK: - The buffer

/// How much sound is kept before it is played, in milliseconds. docs/adr/0012-sound.md says that
/// these are starting values, and that the hardware tries settle them.
struct SoundLevels: Equatable {
  /// How much is waited for before the sound is played, at the beginning and again after the
  /// buffer ran dry. It is also what is left when there is too much.
  var startMillis: Int
  /// The most that is kept. When a packet would make it more, the oldest sound is dropped, down to
  /// `startMillis`: the clock of the other device and the clock of this one differ a little, and
  /// this is where a difference that has built up goes.
  var mostMillis: Int
  /// How much more than `startMillis` the buffer has to have held, all through `windowMillis` of
  /// playing, before it lets go of that much: sound that stayed in the buffer so long without being
  /// needed only makes the sound late. This is where the sound that a stall held back goes when it
  /// came in under `mostMillis`.
  var slackMillis = 40
  /// How much playing the lowest level is watched for, in milliseconds of sound.
  var windowMillis = 1000

  static let standard = SoundLevels(startMillis: 80, mostMillis: 250)
}

/// What the buffer has done so far.
struct SoundCounts: Equatable {
  /// How many times the sound was played to the end and the device asked for more.
  var ranDry = 0
  /// How many frames were dropped because there was too much.
  var droppedFrames = 0
  /// How many frames of silence were put in to cover packets that did not come.
  var coveredFrames = 0
  /// How many times it let go of sound that had stayed in it for a whole window without being
  /// needed. The frames are among those dropped.
  var caughtUp = 0
}

/// The sound that is decoded and not played yet. One thread writes the packets as they are decoded
/// and another, the one of the device, reads what it plays next; the critical sections are a few
/// copies of a few thousand samples, and so short that neither waits for the other for long. The
/// samples are 32-bit floating point and interleaved.
///
/// The ring plays nothing until it holds `startMillis` of sound. It then gives sound until it has
/// none left, and plays silence, and waits for `startMillis` again. It never holds more than
/// `mostMillis`, and it does not keep for long what it does not need: when its level has not come
/// down to within `slackMillis` of `startMillis` for a whole window of playing, it lets go of the
/// difference, the oldest sound first.
final class SoundRing {
  let channels: Int

  private let lock = NSLock()
  private let storage: UnsafeMutablePointer<Float>
  /// All in frames, a frame being a sample of each channel.
  private let capacity: Int
  private let start: Int
  private let most: Int
  private let slack: Int
  private let window: Int
  /// Where the oldest frame is, and how many frames there are.
  private var head = 0
  private var count = 0
  private var playing = false
  private var tally = SoundCounts()
  /// How many frames have been played since the window began, and the lowest the level has been
  /// since, as it was left by a read.
  private var watched = 0
  private var lowest = Int.max

  /// `rate` is the sampling rate in hertz.
  init(rate: Int, channels: Int, levels: SoundLevels = .standard) {
    let channels = max(channels, 1)
    let rate = max(rate, 1)
    let start = max(rate * max(levels.startMillis, 0) / 1000, 1)
    let most = max(rate * max(levels.mostMillis, 0) / 1000, start)
    let slack = max(rate * max(levels.slackMillis, 0) / 1000, 1)
    let window = max(rate * max(levels.windowMillis, 0) / 1000, 1)
    // Room for the most that is kept, a gap of silence that covers lost packets and the largest
    // packet there is, so that a write does not have to drop what it has not yet counted.
    let capacity = most + start + 4096
    let storage = UnsafeMutablePointer<Float>.allocate(capacity: capacity * channels)
    storage.initialize(repeating: 0, count: capacity * channels)
    self.channels = channels
    self.capacity = capacity
    self.start = start
    self.most = most
    self.slack = slack
    self.window = window
    self.storage = storage
  }

  deinit {
    storage.deallocate()
  }

  /// How many frames are held.
  var frames: Int {
    lock.lock()
    defer { lock.unlock() }
    return count
  }

  /// Whether the ring gives sound now, and not silence while it fills.
  var isPlaying: Bool {
    lock.lock()
    defer { lock.unlock() }
    return playing
  }

  var counts: SoundCounts {
    lock.lock()
    defer { lock.unlock() }
    return tally
  }

  /// Adds the sound of a packet: `samples` holds whole frames, and a part of a frame at the end is
  /// ignored. `gap` is how many frames of the packets before this one are missing. While the ring
  /// plays and holds sound it puts that much silence in before the packet, so that what follows is
  /// heard when it was meant to be, but never more than it waits for before it plays. When the ring
  /// has run dry the silence that it played already stands for the gap, and when it has not begun
  /// there is nothing to keep time with, so nothing is put in.
  func write(_ samples: UnsafeBufferPointer<Float>, gap: Int = 0) {
    let incoming = samples.count / channels
    guard incoming > 0, incoming <= capacity - start, let source = samples.baseAddress else {
      return
    }
    lock.lock()
    defer { lock.unlock() }
    let silence = playing && count > 0 ? min(max(gap, 0), start) : 0
    let needed = silence + incoming
    if count + needed > capacity {
      drop(count + needed - capacity)
    }
    if silence > 0 {
      fill(silence)
      tally.coveredFrames += silence
    }
    copy(source, frames: incoming)
    if count > most {
      drop(count - start)
    }
    if !playing && count >= start {
      playing = true
    }
  }

  /// Takes the next `frames` frames into `out`, which has room for that many. What the ring does
  /// not have is silence. Gives how many frames of the sound there were.
  @discardableResult
  func read(into out: UnsafeMutablePointer<Float>, frames: Int) -> Int {
    guard frames > 0 else { return 0 }
    lock.lock()
    defer { lock.unlock() }
    var taken = 0
    if playing {
      taken = min(frames, count)
      var done = 0
      while done < taken {
        let run = min(taken - done, capacity - head)
        (out + done * channels).update(from: storage + head * channels, count: run * channels)
        head = (head + run) % capacity
        count -= run
        done += run
      }
      if taken < frames {
        playing = false
        tally.ranDry += 1
        beginWindow()
      } else {
        watch(taken)
      }
    }
    if taken < frames {
      (out + taken * channels).update(repeating: 0, count: (frames - taken) * channels)
    }
    return taken
  }

  /// Drops the oldest `frames` frames. The lock is held.
  private func drop(_ frames: Int) {
    let dropped = min(max(frames, 0), count)
    head = (head + dropped) % capacity
    count -= dropped
    tally.droppedFrames += dropped
  }

  /// Notes the level that a read of `frames` frames left, and when a window of playing is over lets
  /// go of what the level did not come down to in it. The lock is held.
  private func watch(_ frames: Int) {
    lowest = min(lowest, count)
    watched += frames
    guard watched >= window else { return }
    let unneeded = lowest - start
    if unneeded >= slack {
      drop(unneeded)
      tally.caughtUp += 1
    }
    beginWindow()
  }

  /// A new window begins: nothing has been played in it, and no level has been seen. The lock is
  /// held.
  private func beginWindow() {
    watched = 0
    lowest = Int.max
  }

  /// Puts `frames` frames of silence after the newest. The lock is held, and there is room.
  private func fill(_ frames: Int) {
    var done = 0
    while done < frames {
      let at = (head + count) % capacity
      let run = min(frames - done, capacity - at)
      (storage + at * channels).update(repeating: 0, count: run * channels)
      count += run
      done += run
    }
  }

  /// Puts `frames` frames after the newest. The lock is held, and there is room.
  private func copy(_ source: UnsafePointer<Float>, frames: Int) {
    var done = 0
    while done < frames {
      let at = (head + count) % capacity
      let run = min(frames - done, capacity - at)
      (storage + at * channels).update(from: source + done * channels, count: run * channels)
      count += run
      done += run
    }
  }
}

// MARK: - A speaker

/// The speaker of one screen: it makes a run of sound out of the calls `sound`, `play` and
/// `stopped`, and plays it.
///
/// The Rust side makes those calls while it holds the lock of the session, so none of them waits
/// for the device: each one hands its work to a queue of the speaker, which makes the decoder and
/// the output, decodes the packets one after the other and writes them to the ring, which the
/// device reads from a thread of its own. A call that comes when the speaker has been closed does
/// nothing.
///
/// A run that cannot be made or started is not tried again by the speaker: it says so, once, with
/// `cannotPlay`, and goes on without a run. The Rust side turns the sound off and has the other
/// device stop sending it, the person is told, and a new run comes only when they ask for the
/// sound again. A phone call that holds the audio session of the phone, or an output that is gone,
/// may be over by then, and a speaker that went on trying by itself would leave the other device
/// quiet for a sound that nobody hears.
final class ScreenSpeaker {
  private let queue = DispatchQueue(
    label: "io.github.jayunewal.baton.speaker", qos: .userInteractive)
  private let makeDecoder: (SoundForm) throws -> SoundDecoder
  private let makeOutput: (SoundForm, SoundRing) throws -> SoundOutput
  private let levels: SoundLevels
  private let cannotPlay: () -> Void

  private let lock = NSLock()
  private var closed = false

  /// The run that goes, or nil. Only the queue touches it.
  private var run: Run?

  /// What belongs to one run of sound, from a `sound` to the call that ends it.
  private final class Run {
    let form: SoundForm
    let decoder: SoundDecoder
    let ring: SoundRing
    let output: SoundOutput
    /// How many packets are missing between the last that was played and the next to come.
    var owed: UInt64 = 0
    var packets = 0
    var undecodable = 0

    init(form: SoundForm, decoder: SoundDecoder, ring: SoundRing, output: SoundOutput) {
      self.form = form
      self.decoder = decoder
      self.ring = ring
      self.output = output
    }
  }

  /// The most packets that are covered with silence for one gap. A gap is covered only up to what
  /// the ring waits for, and this keeps a number that Rust passes from being a loop of any length.
  private static let mostCovered: UInt64 = 64

  /// `makeDecoder` makes the decoder for a form, and `makeOutput` the output that plays a ring of
  /// it; either one throws when the device cannot. Both are called from the queue of the speaker.
  /// `cannotPlay` is called, from the queue too and once for each run, when a run could not be
  /// made or started; on a device it tells the Rust side (`baton_stream_link_sound_failed`), which
  /// must not be called from inside a call of the Rust side, and this is not one.
  init(
    levels: SoundLevels = .standard,
    cannotPlay: @escaping () -> Void,
    makeDecoder: @escaping (SoundForm) throws -> SoundDecoder,
    makeOutput: @escaping (SoundForm, SoundRing) throws -> SoundOutput
  ) {
    self.levels = levels
    self.cannotPlay = cannotPlay
    self.makeDecoder = makeDecoder
    self.makeOutput = makeOutput
  }

  /// A run of sound begins in this form. The run that went is over.
  func begin(_ form: SoundForm) {
    queue.async { [self] in
      if isClosed { return }
      endRun()
      startRun(form)
    }
  }

  /// Plays one packet. It is copied before this returns.
  func play(_ packet: PlayInput) {
    let bytes = Data(packet.packet)
    let first = packet.isFirst
    let lost = packet.lostBefore
    queue.async { [self] in
      if isClosed { return }
      decode(bytes, first: first, lost: lost)
    }
  }

  /// No sound is wanted or sent: the run is over, and what was queued is not played out.
  func stop() {
    queue.async { [self] in
      endRun()
    }
  }

  /// The screen is over: the run is over, and nothing is played after this. It does not wait for
  /// the device.
  func close() {
    lock.lock()
    let already = closed
    closed = true
    lock.unlock()
    if already { return }
    queue.async { [self] in
      endRun()
    }
  }

  /// Waits until the work that was handed to the speaker is done, for the tests.
  func idle() {
    queue.sync {}
  }

  private var isClosed: Bool {
    lock.lock()
    defer { lock.unlock() }
    return closed
  }

  /// Makes the run, or says that the device cannot play it.
  private func startRun(_ form: SoundForm) {
    let decoder: SoundDecoder
    do {
      decoder = try makeDecoder(form)
    } catch {
      return failed("make the decoder", error)
    }
    let ring = SoundRing(rate: Int(form.rate), channels: Int(form.channels), levels: levels)
    let output: SoundOutput
    do {
      output = try makeOutput(form, ring)
    } catch {
      decoder.invalidate()
      return failed("make the output", error)
    }
    do {
      try output.start()
    } catch {
      output.stop()
      decoder.invalidate()
      return failed("start the output", error)
    }
    run = Run(form: form, decoder: decoder, ring: ring, output: output)
  }

  /// The run could not be made: it is told in the log, and to the Rust side, unless the screen is
  /// over and nobody is left to tell.
  private func failed(_ step: String, _ error: Error) {
    report(step, error)
    if isClosed { return }
    cannotPlay()
  }

  private func decode(_ bytes: Data, first: Bool, lost: UInt64) {
    guard let run else { return }
    run.owed = first ? 0 : run.owed &+ min(lost, ScreenSpeaker.mostCovered)
    // A decoder goes on from the packet before, and these do not: the first of a run, and the one
    // after packets that did not come.
    if first || run.owed > 0 { run.decoder.reset() }
    let decoded: [Float]?
    do {
      decoded = try run.decoder.decode(bytes)
    } catch {
      // The packet is dropped and counts as missing, so that the packet after it is not played as
      // if it had followed the one before.
      run.undecodable += 1
      run.owed = run.owed &+ 1
      return
    }
    guard let samples = decoded, !samples.isEmpty else { return }
    let gap = Int(min(run.owed, ScreenSpeaker.mostCovered)) * Int(run.form.samples)
    samples.withUnsafeBufferPointer { run.ring.write($0, gap: gap) }
    run.owed = 0
    run.packets += 1
  }

  private func endRun() {
    guard let old = run else { return }
    run = nil
    // The output first, so that nothing reads the ring when the decoder is let go of.
    old.output.stop()
    old.decoder.invalidate()
    let counts = old.ring.counts
    NSLog(
      "Baton: a run of sound ended after %ld packets. The buffer ran dry %ld times, caught up %ld "
        + "times, dropped %ld frames in all and covered %ld; %ld packets could not be decoded.",
      old.packets, counts.ranDry, counts.caughtUp, counts.droppedFrames, counts.coveredFrames,
      old.undecodable)
  }

  /// Says in the log that a run of sound cannot be played, with the step and the code of the
  /// system. The person is told through `cannotPlay`, and nothing in the log names a program or a
  /// person.
  private func report(_ step: String, _ error: Error) {
    if let failure = error as? SoundFailure {
      NSLog(
        "Baton: the sound of a screen cannot be played: it did not %@ (%d, in %@).", step,
        failure.code, failure.step)
    } else {
      NSLog("Baton: the sound of a screen cannot be played: it did not %@.", step)
    }
  }
}
