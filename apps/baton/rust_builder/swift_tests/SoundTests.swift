// SPDX-License-Identifier: Apache-2.0

// Tests of BatonSound.swift, which holds what the Mac does to send the sound of a screen, and of the
// encoder of AAC (BatonAacEncoder.swift) where the system has one. The pump is given a tap and an
// encoder that make what a test says, and a session that keeps the calls the library would have
// had; the calls to the library itself are played by the stubs of Stubs.swift.

import Foundation

// MARK: - What the pump is given

extension NSLock {
  fileprivate func locked<T>(_ body: () -> T) -> T {
    lock()
    defer { unlock() }
    return body()
  }
}

/// A session of sound that keeps the calls made to it, and answers as a test says.
final class FakeSoundSession: SoundSession {
  private let lock = NSLock()
  private var forms: [SoundForm] = []
  private var reasons: [SoundWhy] = []
  private var handed: [(bytes: Data, captureTime: UInt64)] = []
  private var clockAnswer: UInt64? = 10_000_000
  private var beginAnswer = SoundBegun.taken
  private var sendAnswers: [SoundSent] = []

  /// The forms the runs began with.
  var begun: [SoundForm] { lock.locked { forms } }
  /// The reasons there was no sound, as the other device was told.
  var silences: [SoundWhy] { lock.locked { reasons } }
  /// The packets that were handed over, in order.
  var packets: [(bytes: Data, captureTime: UInt64)] { lock.locked { handed } }

  /// The microseconds since the session began, or nil for a session that is over.
  func answerClock(_ micros: UInt64?) { lock.locked { clockAnswer = micros } }
  func answerBegin(_ answer: SoundBegun) { lock.locked { beginAnswer = answer } }
  /// What the first packets are answered, in order. The ones after them go out.
  func answerSends(_ answers: [SoundSent]) { lock.locked { sendAnswers = answers } }

  func clock() -> UInt64? { lock.locked { clockAnswer } }

  func begin(_ form: SoundForm) -> SoundBegun {
    lock.lock()
    defer { lock.unlock() }
    forms.append(form)
    return beginAnswer
  }

  func silent(_ why: SoundWhy) {
    lock.lock()
    defer { lock.unlock() }
    reasons.append(why)
  }

  func send(_ packet: Data, captureTime: UInt64) -> SoundSent {
    lock.lock()
    defer { lock.unlock() }
    handed.append((bytes: packet, captureTime: captureTime))
    let index = handed.count - 1
    return index < sendAnswers.count ? sendAnswers[index] : .sent
  }
}

/// A tap that reads what a test hands it, from the thread of the test.
final class FakeSoundTap: SoundTap {
  private let lock = NSLock()
  private let rate: Double
  private let failure: Error?
  private var muteds: [Bool] = []
  private var stopCount = 0
  private var lookCount = 0
  private var isPlaying: Bool
  private var deliver: (([Float]) -> Void)?
  /// What the pump gave to be called with the blocks, also after the tap was stopped: the blocks
  /// that the tap of the system had in its queue then arrive through it.
  private var lastDeliver: (([Float]) -> Void)?

  init(rate: Double, failure: Error?, playing: Bool) {
    self.rate = rate
    self.failure = failure
    self.isPlaying = playing
  }

  /// Whether each start asked for the sound to be kept from the speakers.
  var starts: [Bool] { lock.locked { muteds } }
  var stops: Int { lock.locked { stopCount } }
  /// How many times the pump asked whether some program plays.
  var looks: Int { lock.locked { lookCount } }

  func setPlaying(_ playing: Bool) { lock.locked { isPlaying = playing } }

  func start(muted: Bool, deliver: @escaping ([Float]) -> Void) throws -> Double {
    lock.lock()
    defer { lock.unlock() }
    muteds.append(muted)
    if let failure { throw failure }
    self.deliver = deliver
    lastDeliver = deliver
    return rate
  }

  func stop() {
    lock.lock()
    defer { lock.unlock() }
    stopCount += 1
    deliver = nil
  }

  func someoneIsPlaying() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    lookCount += 1
    return isPlaying
  }

  /// What the system does while the tap reads: a block is handed over from a thread of the tap.
  func read(_ samples: [Float]) {
    let call = lock.locked { deliver }
    call?(samples)
  }

  /// A block that was in the queue of the tap when it was stopped, handed over after that.
  func readLate(_ samples: [Float]) {
    let call = lock.locked { lastDeliver }
    call?(samples)
  }
}

/// Makes the taps that the pump asks for, with the rate and the failure that a test chose.
final class TapMaker {
  private let lock = NSLock()
  private var made: [FakeSoundTap] = []
  private var rateToGive = 48_000.0
  private var failureToGive: Error?
  private var playingToGive = false

  /// The rate at which the next taps read.
  var rate: Double {
    get { lock.locked { rateToGive } }
    set { lock.locked { rateToGive = newValue } }
  }

  /// What the next taps fail with when they are started, if they do.
  var failure: Error? {
    get { lock.locked { failureToGive } }
    set { lock.locked { failureToGive = newValue } }
  }

  /// Whether a program plays on the Mac, for the next taps.
  var playing: Bool {
    get { lock.locked { playingToGive } }
    set { lock.locked { playingToGive = newValue } }
  }

  var taps: [FakeSoundTap] { lock.locked { made } }
  var latest: FakeSoundTap? { lock.locked { made.last } }

  func make() -> SoundTap {
    lock.lock()
    defer { lock.unlock() }
    let tap = FakeSoundTap(rate: rateToGive, failure: failureToGive, playing: playingToGive)
    made.append(tap)
    return tap
  }
}

/// An encoder that makes a packet of two bytes for every call: the number of the call, and 170.
final class FakeSoundEncoder: SoundEncoder {
  let form: SoundForm
  private let lock = NSLock()
  private let failAt: Int?
  private let emptyAt: Set<Int>
  private var inputs: [[Float]] = []
  private var invalidations = 0

  init(form: SoundForm, failAt: Int?, emptyAt: Set<Int>) {
    self.form = form
    self.failAt = failAt
    self.emptyAt = emptyAt
  }

  /// What each call was given.
  var received: [[Float]] { lock.locked { inputs } }
  var invalidated: Int { lock.locked { invalidations } }

  func encode(_ samples: [Float]) throws -> Data? {
    lock.lock()
    defer { lock.unlock() }
    inputs.append(samples)
    let number = inputs.count
    if number == failAt { throw SoundFailure(step: "encode a packet", code: -9) }
    if emptyAt.contains(number) { return nil }
    return Data([UInt8(truncatingIfNeeded: number), 170])
  }

  func invalidate() { lock.locked { invalidations += 1 } }
}

/// Makes the encoders that the pump asks for.
final class EncoderMaker {
  private let lock = NSLock()
  private var made: [FakeSoundEncoder] = []
  private var makeFailure: Error?
  private var failAtCall: Int?
  private var emptyCalls: Set<Int> = []

  /// What making an encoder fails with, if it does.
  var failure: Error? {
    get { lock.locked { makeFailure } }
    set { lock.locked { makeFailure = newValue } }
  }

  /// The call, counted from 1 in each encoder, that the next encoders fail at.
  var failEncodeAt: Int? {
    get { lock.locked { failAtCall } }
    set { lock.locked { failAtCall = newValue } }
  }

  /// The calls, counted from 1 in each encoder, that the next encoders have no packet for.
  var emptyAt: Set<Int> {
    get { lock.locked { emptyCalls } }
    set { lock.locked { emptyCalls = newValue } }
  }

  var encoders: [FakeSoundEncoder] { lock.locked { made } }

  func make(_ form: SoundForm) throws -> SoundEncoder {
    lock.lock()
    defer { lock.unlock() }
    if let makeFailure { throw makeFailure }
    let encoder = FakeSoundEncoder(form: form, failAt: failAtCall, emptyAt: emptyCalls)
    made.append(encoder)
    return encoder
  }
}

/// A pump with a session, taps and encoders to play with.
final class SoundRig {
  let session: FakeSoundSession
  let taps: TapMaker
  let encoders: EncoderMaker
  let pump: SoundPump
  /// The next sample that `ramp` gives: they count up from 1, block after block.
  private var next: Float = 1

  init(quiet: Bool = false) {
    let session = FakeSoundSession()
    let taps = TapMaker()
    let encoders = EncoderMaker()
    self.session = session
    self.taps = taps
    self.encoders = encoders
    self.pump = SoundPump(
      session: session, quiet: quiet, makeTap: { taps.make() },
      makeEncoder: { try encoders.make($0) })
  }

  /// `frames` frames of two channels. The samples count up, so that a test can tell which of them a
  /// packet was made of.
  func ramp(_ frames: Int) -> [Float] {
    var block = [Float](repeating: 0, count: frames * 2)
    for index in block.indices {
      block[index] = next
      next += 1
    }
    return block
  }

  /// The tap reads `frames` frames of the ramp, and the pump has dealt with them when this returns.
  /// The calls that were made before are dealt with first, since a tap is made by one of them.
  func feed(_ frames: Int) {
    pump.idle()
    taps.latest?.read(ramp(frames))
    pump.idle()
  }

  /// The tap reads `blocks` blocks of 1024 frames of silence.
  func silence(blocks: Int) {
    pump.idle()
    let block = [Float](repeating: 0, count: 2048)
    for _ in 0..<blocks { taps.latest?.read(block) }
    pump.idle()
  }
}

/// The samples 1, 2, 3 and on, as many as `count`.
private func counting(_ count: Int) -> [Float] {
  (1...count).map { Float($0) }
}

private let form48 = SoundForm(rate: 48_000, channels: 2, samples: 1024)

// MARK: - A run of sound

func testListeningStartsTheTapMutedAndBeginsTheRunInTheFormOfTheTap() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.taps.taps.count == 1, "\(rig.taps.taps.count) taps")
  check(rig.taps.latest?.starts == [true], "the tap was not started muted")
  check(rig.session.begun == [form48], "\(rig.session.begun)")
  check(rig.encoders.encoders.count == 1 && rig.encoders.encoders[0].form == form48)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 0, "the tap was stopped while it is listened to")
  rig.pump.close()
  check(rig.taps.latest?.stops == 1, "the tap was stopped \(String(describing: rig.taps.latest?.stops)) times")
  check(rig.encoders.encoders[0].invalidated == 1)
}

func testSamplesBecomePacketsOfOneThousandAndTwentyFourFramesWithTheTimeTheyWereRecorded() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.pump.idle()
  // Half a packet is not one.
  rig.feed(512)
  check(rig.session.packets.isEmpty, "a packet of half the samples")
  rig.feed(512)
  var packets = rig.session.packets
  check(packets.count == 1, "\(packets.count) packets")
  // The block ends now, which is 10 s into the session, and the first sample of the packet is as
  // old as the packet is long.
  let long = 1024.0 / 48_000.0 * 1_000_000.0
  check(packets.first?.captureTime == UInt64(10_000_000.0 - long), "\(String(describing: packets.first?.captureTime))")
  // A block that holds more than a packet makes several, and what is left waits for the next.
  rig.feed(3000)
  packets = rig.session.packets
  check(packets.count == 3, "\(packets.count) packets")
  check(packets.count == 3 && packets[1].captureTime == 9_937_500, "\(packets.map { $0.captureTime })")
  if packets.count == 3 {
    let apart = Int64(packets[2].captureTime) - Int64(packets[1].captureTime)
    check(apart == 21_333 || apart == 21_334, "packets \(apart) microseconds apart")
  }
  rig.feed(100)
  packets = rig.session.packets
  check(packets.count == 4, "\(packets.count) packets")
  // The packets are made of the samples in the order in which they were read, none twice and none
  // left out, and they go out in that order.
  let inputs = rig.encoders.encoders[0].received
  check(inputs.count == 4 && inputs.allSatisfy { $0.count == 2048 }, "\(inputs.map { $0.count })")
  check(Array(inputs.joined()) == counting(4 * 2048), "the samples of the packets are not the ones read")
  check(
    packets.map { Array($0.bytes) } == [[1, 170], [2, 170], [3, 170], [4, 170]],
    "\(packets.map { Array($0.bytes) })")
  rig.pump.close()
}

func testNotListeningAnyMoreStopsTheTapAndBlocksThatCameLateAreNotSent() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.feed(1024)
  check(rig.session.packets.count == 1)
  guard let first = rig.taps.latest else { return check(false, "no tap") }
  rig.pump.hear(false)
  rig.pump.idle()
  check(first.stops == 1, "the tap was stopped \(first.stops) times, and the Mac does not play again")
  check(rig.encoders.encoders[0].invalidated == 1)
  // A block that the tap had in its queue when it was stopped is not sound that anyone wants.
  first.readLate(rig.ramp(1024))
  rig.pump.idle()
  check(rig.session.packets.count == 1, "a late block was sent")
  // A 0 where nothing is read changes nothing.
  rig.pump.hear(false)
  rig.pump.idle()
  check(first.stops == 1 && rig.taps.taps.count == 1)
  // Listening again makes another tap. The late blocks of the first are still not its sound.
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.taps.taps.count == 2, "\(rig.taps.taps.count) taps")
  first.readLate(rig.ramp(1024))
  rig.pump.idle()
  check(rig.session.packets.count == 1, "a block of the tap that was stopped was sent")
  rig.feed(1024)
  check(rig.session.packets.count == 2, "\(rig.session.packets.count) packets")
  check(rig.session.begun == [form48, form48], "\(rig.session.begun)")
  rig.pump.close()
}

func testAQuietScreenKeepsTheMacQuietForTheWholeSessionAndSendsOnlyWhatIsWanted() {
  let rig = SoundRig(quiet: true)
  rig.pump.keepQuiet()
  check(rig.taps.taps.count == 1 && rig.taps.latest?.starts == [true], "the Mac is not quiet")
  // Nobody listens yet: what is read is thrown away, and the library is not told of any run.
  rig.feed(2048)
  check(rig.session.packets.isEmpty && rig.session.begun.isEmpty, "sound was sent that nobody wants")
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.taps.taps.count == 1, "another tap for a screen that is already quiet")
  check(rig.session.begun == [form48], "\(rig.session.begun)")
  rig.feed(1024)
  check(rig.session.packets.count == 1, "\(rig.session.packets.count) packets")
  // The person turns the sound off. The Mac stays quiet, since nobody is there to hear it.
  rig.pump.hear(false)
  rig.pump.idle()
  check(rig.taps.latest?.stops == 0, "the Mac played again while the screen is private")
  rig.feed(2048)
  check(rig.session.packets.count == 1, "sound was sent that nobody wants")
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.session.begun == [form48, form48], "\(rig.session.begun)")
  rig.feed(1024)
  check(rig.session.packets.count == 2, "\(rig.session.packets.count) packets")
  // The session is over and the Mac is locked again: it plays, and nothing begins any more.
  rig.pump.close()
  check(rig.taps.latest?.stops == 1)
  rig.pump.hear(true)
  rig.pump.keepQuiet()
  rig.pump.idle()
  check(rig.taps.taps.count == 1 && rig.session.begun.count == 2, "something began after the end")
}

func testListeningAgainWithNoStopBetweenBeginsAnotherRunFromNothing() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.feed(1024 + 512)
  check(rig.session.packets.count == 1)
  // The person asks anew while no sound is being sent (they try again after a silence), and the
  // library answers the 1 with a begin.
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.session.begun == [form48, form48], "\(rig.session.begun)")
  check(rig.taps.taps.count == 1 && rig.taps.latest?.starts == [true], "another tap was made")
  check(rig.taps.latest?.stops == 0)
  check(rig.encoders.encoders.count == 2 && rig.encoders.encoders[0].invalidated == 1)
  // What was left over from the first run is not the start of the second.
  rig.feed(512)
  check(rig.session.packets.count == 1, "a packet was made of what the run before left")
  rig.feed(512)
  check(rig.session.packets.count == 2, "\(rig.session.packets.count) packets")
  check(rig.encoders.encoders[1].received.first?.first == Float(1 + 2 * (1024 + 512)))
  rig.pump.close()
}

// MARK: - A Mac that has not let the app record its sound

func testTwoSecondsOfSilenceWhileAProgramPlaysAreTakenForAMacThatHasNotLetTheAppRecord() {
  let rig = SoundRig()
  rig.taps.playing = true
  rig.pump.hear(true)
  rig.pump.idle()
  // 93 blocks of 1024 frames are 1.98 seconds at 48 kHz: not yet.
  rig.silence(blocks: 93)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 0)
  rig.silence(blocks: 1)
  check(rig.session.silences == [.noPermission], "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 1, "the Mac was left quiet")
  // The run is over, and so are the blocks that were in the queue.
  let sent = rig.session.packets.count
  rig.taps.latest?.readLate(rig.ramp(2048))
  rig.pump.idle()
  check(rig.session.packets.count == sent, "sound was sent after the reason was told")
  // The person tries again, and the Mac is asked again.
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.taps.taps.count == 2 && rig.session.begun.count == 2, "\(rig.taps.taps.count) taps")
  rig.pump.close()
}

func testSilenceWhileNothingPlaysIsNotReportedAndAProgramThatBeginsLaterNeedsACountOfItsOwn() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.pump.idle()
  // Nothing plays, so the silence is the sound of the Mac. The pump looks for a program when a count
  // of the silence begins, and when it ends, which is where the next count begins: at blocks 1, 94
  // and 188 of the 200, and not at every block.
  rig.silence(blocks: 200)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 0)
  check(rig.taps.latest?.looks == 3, "\(String(describing: rig.taps.latest?.looks)) looks")
  // A program starts to play, and the tap reads nothing from it. The count that is running ends at
  // block 282 with the program there, but it was not there when the count began.
  rig.taps.latest?.setPlaying(true)
  rig.silence(blocks: 94)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  // The count that begins with the program ends at block 376, and not before.
  rig.silence(blocks: 81)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  rig.silence(blocks: 1)
  check(rig.session.silences == [.noPermission], "\(rig.session.silences)")
  rig.pump.close()
}

func testAProgramThatBeginsJustBeforeTheEndOfTheCountIsNotTakenForAMacThatHasNotLetTheAppRecord() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.pump.idle()
  // A Mac that has let the app record. Nothing plays for 1.98 seconds, and then a program starts,
  // a moment before the count of two seconds is over and before the tap has read any of its sound.
  rig.silence(blocks: 93)
  rig.taps.latest?.setPlaying(true)
  rig.silence(blocks: 1)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 0, "the Mac was left quiet")
  // Its sound comes, and the run goes on with it, silence and sound alike.
  rig.feed(1024)
  rig.silence(blocks: 300)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 0)
  check(rig.session.packets.count == 94 + 1 + 300, "\(rig.session.packets.count) packets")
  rig.pump.close()
}

func testSoundThatWasHeardOnceRestsTheQuestionOfThePermission() {
  let rig = SoundRig()
  rig.taps.playing = true
  rig.pump.hear(true)
  rig.feed(1024)
  // A program that plays silence on purpose is not a Mac that gives none.
  rig.silence(blocks: 300)
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 0)
  check(rig.session.packets.count == 301, "\(rig.session.packets.count) packets")
  rig.pump.close()
}

// MARK: - A run that cannot go on

func testATapOrAnEncoderThatCannotBeMadeIsToldAsFailedAndTheMacPlaysAgain() {
  do {
    // The tap cannot be made.
    let rig = SoundRig()
    rig.taps.failure = SoundFailure(step: "make the tap", code: -50)
    rig.pump.hear(true)
    rig.pump.idle()
    check(rig.session.silences == [.failed], "\(rig.session.silences)")
    check(rig.session.begun.isEmpty && rig.encoders.encoders.isEmpty)
    check(rig.taps.taps.count == 1)
    // The person tries again, and a tap is made anew.
    rig.taps.failure = nil
    rig.pump.hear(true)
    rig.pump.idle()
    check(rig.taps.taps.count == 2 && rig.session.begun.count == 1, "\(rig.taps.taps.count) taps")
    check(rig.session.silences == [.failed], "\(rig.session.silences)")
    rig.pump.close()
  }
  do {
    // A rate that the protocol does not take, or that is not a number, is not read.
    for rate in [4_000.0, 200_000.0, Double.nan, Double.infinity, -48_000.0, 0.0] {
      let rig = SoundRig()
      rig.taps.rate = rate
      rig.pump.hear(true)
      rig.pump.idle()
      check(rig.session.silences == [.failed], "rate \(rate): \(rig.session.silences)")
      check(rig.taps.latest?.stops == 1, "rate \(rate): the tap was left reading")
      check(rig.session.begun.isEmpty, "rate \(rate): \(rig.session.begun)")
      rig.pump.close()
    }
  }
  do {
    // The encoder cannot be made: the Mac plays again.
    let rig = SoundRig()
    rig.encoders.failure = SoundFailure(step: "make the encoder", code: -1)
    rig.pump.hear(true)
    rig.pump.idle()
    check(rig.session.silences == [.failed], "\(rig.session.silences)")
    check(rig.session.begun.isEmpty)
    check(rig.taps.latest?.stops == 1, "the Mac was left quiet")
    rig.pump.close()
  }
  do {
    // A screen that is kept quiet stays quiet, and says why there is no sound.
    let rig = SoundRig(quiet: true)
    rig.encoders.failure = SoundFailure(step: "make the encoder", code: -1)
    rig.pump.keepQuiet()
    rig.pump.hear(true)
    rig.pump.idle()
    check(rig.session.silences == [.failed], "\(rig.session.silences)")
    check(rig.taps.latest?.stops == 0, "the Mac played again while the screen is private")
    rig.pump.close()
    check(rig.taps.latest?.stops == 1)
  }
  do {
    // The library does not take the form.
    let rig = SoundRig()
    rig.session.answerBegin(.refused)
    rig.pump.hear(true)
    rig.pump.idle()
    check(rig.session.silences == [.failed], "\(rig.session.silences)")
    check(rig.session.begun == [form48])
    check(rig.encoders.encoders.first?.invalidated == 1)
    check(rig.taps.latest?.stops == 1)
    rig.taps.latest?.readLate(rig.ramp(1024))
    rig.pump.idle()
    check(rig.session.packets.isEmpty, "sound was sent after the form was refused")
    rig.pump.close()
  }
  do {
    // The session is over: nobody is told.
    let rig = SoundRig()
    rig.session.answerBegin(.over)
    rig.pump.hear(true)
    rig.pump.idle()
    check(rig.session.silences.isEmpty, "\(rig.session.silences)")
    check(rig.taps.latest?.stops == 1)
    check(rig.encoders.encoders.first?.invalidated == 1)
    rig.pump.close()
  }
}

func testAnEncoderThatFailsEndsTheRunAsFailedAndOneWithNoPacketYetSkipsOne() {
  do {
    let rig = SoundRig()
    rig.encoders.failEncodeAt = 3
    rig.pump.hear(true)
    rig.feed(4 * 1024)
    check(rig.session.packets.count == 2, "\(rig.session.packets.count) packets")
    check(rig.session.silences == [.failed], "\(rig.session.silences)")
    check(rig.taps.latest?.stops == 1, "the Mac was left quiet")
    rig.pump.close()
  }
  do {
    // An encoder that has no packet for some samples yet (the first call of AAC has none): the
    // packets that follow keep their places in time.
    let rig = SoundRig()
    rig.encoders.emptyAt = [1, 3]
    rig.pump.hear(true)
    rig.feed(4 * 1024)
    let packets = rig.session.packets
    check(packets.map { Array($0.bytes) } == [[2, 170], [4, 170]], "\(packets.map { Array($0.bytes) })")
    if packets.count == 2 {
      let apart = Int64(packets[1].captureTime) - Int64(packets[0].captureTime)
      check(apart == 42_666 || apart == 42_667, "packets \(apart) microseconds apart")
    }
    check(rig.session.silences.isEmpty)
    rig.pump.close()
  }
}

func testPacketsThatAreDroppedOrTooLargeOrRefusedGoOnAndOverEndsTheRun() {
  let rig = SoundRig()
  rig.session.answerSends([.sent, .dropped, .tooLarge, .refused, .over])
  rig.pump.hear(true)
  // Six packets in one block: the fifth is the last that is handed over.
  rig.feed(6 * 1024)
  check(rig.session.packets.count == 5, "\(rig.session.packets.count) packets")
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 1, "the Mac was left quiet when the session was over")
  rig.taps.latest?.readLate(rig.ramp(2048))
  rig.pump.idle()
  check(rig.session.packets.count == 5, "sound was sent after the session was over")
  rig.pump.close()
}

func testAClockThatIsGoneEndsTheRunWithoutSayingWhy() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.pump.idle()
  rig.session.answerClock(nil)
  rig.feed(1024)
  check(rig.session.packets.isEmpty, "a packet with no time")
  check(rig.session.silences.isEmpty, "\(rig.session.silences)")
  check(rig.taps.latest?.stops == 1)
  rig.pump.close()
}

func testCloseIsFinalAndLetsGoOfTheTapAndTheEncoderOnce() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.feed(1024)
  rig.pump.close()
  rig.pump.close()
  check(rig.taps.latest?.stops == 1, "stopped \(String(describing: rig.taps.latest?.stops)) times")
  check(rig.encoders.encoders.first?.invalidated == 1)
  let sent = rig.session.packets.count
  rig.pump.hear(true)
  rig.taps.latest?.readLate(rig.ramp(1024))
  rig.pump.idle()
  check(rig.taps.taps.count == 1 && rig.session.begun.count == 1, "something began after the end")
  check(rig.session.packets.count == sent, "sound was sent after the end")
  // A pump that was never used is closed like any other.
  let unused = SoundRig()
  unused.pump.close()
  check(unused.taps.taps.isEmpty && unused.session.begun.isEmpty)
}

// MARK: - The rate

func testTheRateOfTheSessionIsThatOfTheMacWhenItIs44100Or48000AndAnyOtherIs48000() {
  check(soundRate(forTap: 44_100) == 44_100)
  check(soundRate(forTap: 48_000) == 48_000)
  for rate in [8_000.0, 22_050, 32_000, 88_200, 96_000] {
    check(soundRate(forTap: rate) == 48_000, "\(rate)")
  }
  let cd = SoundRig()
  cd.taps.rate = 44_100
  cd.pump.hear(true)
  cd.pump.idle()
  check(cd.session.begun == [SoundForm(rate: 44_100, channels: 2, samples: 1024)], "\(cd.session.begun)")
  // A rate that is kept does not change the samples.
  cd.feed(1024)
  check(cd.encoders.encoders[0].received.first == counting(2048), "a rate that is kept changed the samples")
  cd.pump.close()

  let high = SoundRig()
  high.taps.rate = 96_000
  high.pump.hear(true)
  high.pump.idle()
  check(high.session.begun == [form48], "\(high.session.begun)")
  // Two frames of the Mac are one of the session, and one of them is taken.
  high.feed(2048)
  check(high.session.packets.count == 1, "\(high.session.packets.count) packets")
  let first = high.encoders.encoders[0].received.first ?? []
  check(first.count == 2048 && Array(first.prefix(6)) == [1, 2, 5, 6, 9, 10], "\(Array(first.prefix(6)))")
  high.pump.close()
}

func testASecondRunKeepsTheFormOfTheSessionWhenTheMacChangesItsRate() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.session.begun == [form48])
  rig.pump.hear(false)
  // The speakers of the Mac run at 44.1 kHz now: the form of the session is the one it began with.
  rig.taps.rate = 44_100
  rig.pump.hear(true)
  rig.pump.idle()
  check(rig.session.begun == [form48, form48], "the form of the session changed: \(rig.session.begun)")
  // A level stays a level at any rate: 1306 frames at 48 kHz come from two blocks of 600 frames.
  let level = [Float](repeating: 0.5, count: 2 * 600)
  rig.taps.latest?.read(level)
  rig.taps.latest?.read(level)
  rig.pump.idle()
  check(rig.session.packets.count == 1, "\(rig.session.packets.count) packets")
  let inputs = rig.encoders.encoders.last?.received ?? []
  check(inputs.count == 1 && inputs[0].count == 2048, "\(inputs.map { $0.count })")
  check(inputs.first?.allSatisfy { $0 == 0.5 } == true, "the level changed")
  rig.pump.close()
}

func testTheResamplerKeepsALineStraightAcrossBlocksOfAnySize() {
  guard var resampler = LinearResampler(from: 44_100, to: 48_000) else {
    return check(false, "no resampler")
  }
  // A line: the left channel of frame i holds i / 1000, and the right one the opposite.
  let total = 4410
  var input: [Float] = []
  for frame in 0..<total {
    input.append(Float(frame) * 0.001)
    input.append(-Float(frame) * 0.001)
  }
  var output: [Float] = []
  var at = 0
  for size in [100, 37, 1000, 1, 262, 3010] {
    output += resampler.convert(Array(input[(at * 2)..<((at + size) * 2)]))
    at += size
  }
  check(at == total)
  // 4410 frames at 44.1 kHz are 4800 at 48 kHz, but for the frame that waits for the next block.
  let frames = output.count / 2
  check(frames >= 4798 && frames <= 4800, "\(frames) frames")
  let step = 44_100.0 / 48_000.0
  var worst: Float = 0
  for frame in 0..<frames {
    let expected = Float(Double(frame) * step * 0.001)
    worst = max(worst, abs(output[frame * 2] - expected), abs(output[frame * 2 + 1] + expected))
  }
  check(worst < 1e-4, "off by \(worst)")

  // Going down to half, every other frame is taken.
  guard var down = LinearResampler(from: 96_000, to: 48_000) else {
    return check(false, "no resampler")
  }
  let halved = down.convert([1, -1, 2, -2, 3, -3, 4, -4, 5, -5])
  check(halved == [1, -1, 3, -3], "\(halved)")
}

func testTheResamplerNeedsRatesAboveZeroAndKeepsTheSameRateWhole() {
  check(LinearResampler(from: 0, to: 48_000) == nil)
  check(LinearResampler(from: 48_000, to: 0) == nil)
  check(LinearResampler(from: -44_100, to: 48_000) == nil)
  check(LinearResampler(from: .nan, to: 48_000) == nil)
  check(LinearResampler(from: 48_000, to: .infinity) == nil)
  guard var same = LinearResampler(from: 48_000, to: 48_000) else {
    return check(false, "no resampler")
  }
  // The same rate gives the frames back as they came, but for the last, which waits for the next.
  let output = same.convert([1, -1, 2, -2, 3, -3]) + same.convert([4, -4, 5, -5])
  check(output == [1, -1, 2, -2, 3, -3, 4, -4], "\(output)")
  check(same.convert([]) == [], "a block with nothing in it gave samples")
  // The sample that does not make a frame is left out.
  guard var odd = LinearResampler(from: 48_000, to: 48_000) else {
    return check(false, "no resampler")
  }
  let first = odd.convert([1, 2, 3])
  let second = odd.convert([5, 6])
  check(first == [] && second == [1, 2], "\(first) \(second)")
}

// MARK: - The calls to the library

func testTheLibrarySessionMakesTheCallsOfTheInterfaceWithWhatItWasGiven() {
  resetSourceStubs()
  let session = LibrarySoundSession(link: 7)
  check(session.clock() == 777, "\(String(describing: session.clock()))")
  check(session.begin(SoundForm(rate: 44_100, channels: 2, samples: 1024)) == .taken)
  session.silent(.noPermission)
  session.silent(.failed)
  check(session.send(Data([1, 2, 3]), captureTime: 123) == .sent)
  check(session.send(Data(), captureTime: 456) == .sent)
  stubLock.lock()
  let calls = sourceCalls
  stubLock.unlock()
  let packetSize = UInt32(MemoryLayout<BatonSoundPacket>.size)
  let expected: [SourceCall] = [
    .clock(7),
    .soundBegin(
      7,
      SoundRecord(
        size: UInt32(MemoryLayout<BatonSound>.size), codec: UInt32(BATON_SOUND_CODEC_AAC),
        rate: 44_100, channels: 2, samples: 1024)),
    .soundSilent(7, UInt32(BATON_SILENT_NO_PERMISSION)),
    .soundSilent(7, UInt32(BATON_SILENT_FAILED)),
    .sound(7, SoundPacketRecord(size: packetSize, captureTime: 123, bytes: [1, 2, 3])),
    .sound(7, SoundPacketRecord(size: packetSize, captureTime: 456, bytes: [])),
  ]
  check(calls == expected, "\(calls)")
}

func testTheLibrarySessionTellsWhatBecameOfEveryCall() {
  let session = LibrarySoundSession(link: 3)
  let form = SoundForm(rate: 48_000, channels: 2, samples: 1024)
  func answer(_ change: (inout SourceAnswers) -> Void) {
    stubLock.lock()
    change(&sourceAnswers)
    stubLock.unlock()
  }

  resetSourceStubs()
  answer { $0.clockCode = Int32(BATON_E_UNKNOWN_LINK) }
  check(session.clock() == nil, "a time from a link that is closed")

  resetSourceStubs()
  check(session.begin(form) == .taken)
  answer { $0.soundBeginCode = Int32(BATON_E_UNKNOWN_LINK) }
  check(session.begin(form) == .over)
  answer { $0.soundBeginCode = Int32(BATON_E_ARGUMENT) }
  check(session.begin(form) == .refused)

  resetSourceStubs()
  let packet = Data([9, 8, 7])
  check(session.send(packet, captureTime: 1) == .sent)
  let outcomes: [(UInt32, SoundSent)] = [
    (UInt32(BATON_SOUND_SENT), .sent), (UInt32(BATON_SOUND_DROPPED), .dropped),
    (UInt32(BATON_SOUND_TOO_LARGE), .tooLarge), (UInt32(BATON_SOUND_OVER), .over),
    // An outcome that this version does not know is of no use, and the next packet goes on.
    (99, .dropped),
  ]
  for (outcome, expected) in outcomes {
    answer { $0.soundOutcome = outcome }
    let sent = session.send(packet, captureTime: 1)
    check(sent == expected, "outcome \(outcome): \(sent)")
  }
  answer { $0.soundCode = Int32(BATON_E_UNKNOWN_LINK) }
  check(session.send(packet, captureTime: 1) == .over)
  answer { $0.soundCode = Int32(BATON_E_ARGUMENT) }
  check(session.send(packet, captureTime: 1) == .refused)
  answer { $0.soundCode = Int32(BATON_E_PLATFORM) }
  check(session.send(packet, captureTime: 1) == .refused)
  resetSourceStubs()
}

// MARK: - Many threads

func testTheCallsOfTheSoundMayComeFromManyThreadsAtOnce() {
  let rig = SoundRig()
  rig.pump.hear(true)
  rig.pump.idle()
  let block = [Float](repeating: 0.25, count: 2 * 700)
  let group = DispatchGroup()
  // One thread turns the sound on and off while three others read.
  group.enter()
  DispatchQueue.global().async {
    for index in 0..<100 { rig.pump.hear(index % 2 == 1) }
    group.leave()
  }
  for _ in 0..<3 {
    group.enter()
    DispatchQueue.global().async {
      for _ in 0..<150 { rig.taps.latest?.read(block) }
      group.leave()
    }
  }
  group.wait()
  rig.pump.close()
  let sent = rig.session.packets.count
  // Nothing is sent after the pump was closed, whichever tap the blocks come from.
  for tap in rig.taps.taps { tap.readLate(block) }
  rig.pump.idle()
  check(rig.session.packets.count == sent, "sound was sent after the end")
  check(rig.taps.taps.allSatisfy { $0.stops == 1 }, "\(rig.taps.taps.map { $0.stops })")
  check(rig.encoders.encoders.allSatisfy { $0.invalidated == 1 }, "\(rig.encoders.encoders.map { $0.invalidated })")
}

// MARK: - The encoder of AAC

#if canImport(AudioToolbox)

func testTheAacEncoderMakesPacketsOfATone() {
  let encoder: AacEncoder
  do {
    encoder = try AacEncoder(form: form48)
  } catch {
    return check(false, "no encoder: \(error)")
  }
  var sizes: [Int] = []
  var phase = 0.0
  let step = 2.0 * Double.pi * 440.0 / 48_000.0
  // A second of a tone is 46.9 packets, and the encoder has none to give for the first samples.
  for _ in 0..<47 {
    var samples = [Float](repeating: 0, count: 2048)
    for frame in 0..<1024 {
      let value = Float(0.5 * sin(phase))
      phase += step
      samples[frame * 2] = value
      samples[frame * 2 + 1] = value
    }
    do {
      if let packet = try encoder.encode(samples) { sizes.append(packet.count) }
    } catch {
      check(false, "a packet could not be made: \(error)")
    }
  }
  check(sizes.count >= 44 && sizes.count <= 47, "\(sizes.count) packets")
  // A packet is an access unit that is never more than the protocol takes. A pure tone costs the
  // encoder much less than the 341 bytes to a packet that 128 kbit/s allows on average (a Mac made
  // 72 on average), so the mean is only kept under that.
  check(sizes.allSatisfy { $0 > 0 && $0 <= 8192 }, "\(sizes)")
  let mean = Double(sizes.reduce(0, +)) / Double(max(sizes.count, 1))
  check(mean > 10 && mean < 400, "a mean of \(mean) bytes")
  encoder.invalidate()
}

func testTheAacEncoderRefusesWhatItCannotMakeAndIsOverWhenLetGoOf() {
  // The protocol and the encoder have 1024 samples to a packet, in one channel or two.
  for form in [
    SoundForm(rate: 48_000, channels: 2, samples: 512),
    SoundForm(rate: 48_000, channels: 3, samples: 1024),
    SoundForm(rate: 48_000, channels: 0, samples: 1024),
  ] {
    do {
      _ = try AacEncoder(form: form)
      check(false, "an encoder was made for \(form)")
    } catch let failure as SoundFailure {
      check(!failure.step.isEmpty, "\(failure)")
    } catch {
      check(false, "an error that is not a failure of the sound: \(error)")
    }
  }
  check((try? AacEncoder(form: SoundForm(rate: 44_100, channels: 1, samples: 1024))) != nil, "no mono encoder")
  guard let encoder = try? AacEncoder(form: SoundForm(rate: 44_100, channels: 2, samples: 1024)) else {
    return check(false, "no encoder")
  }
  do {
    _ = try encoder.encode([0, 0, 0, 0])
    check(false, "samples that are not a packet were taken")
  } catch {}
  encoder.invalidate()
  encoder.invalidate()
  do {
    _ = try encoder.encode([Float](repeating: 0, count: 2048))
    check(false, "an encoder that was let go of made a packet")
  } catch {}
}

/// What the Mac sends and what the phone plays are two files that never meet but on the wire: this
/// is where the packets of the one are given to the other, as they go from the Mac to the phone.
func testTheDecoderMakesTheToneBackThatTheEncoderOfTheMacMadeOfIt() {
  guard let encoder = try? AacEncoder(form: form48), let decoder = try? AacDecoder(form: form48)
  else { return check(false, "no encoder or no decoder") }
  defer {
    encoder.invalidate()
    decoder.invalidate()
  }
  var tone: [Float] = []
  var packets = 0
  var phase = 0.0
  // 1500 Hz is 32 waves in a packet of 1024 frames at 48 kHz.
  let step = 2.0 * Double.pi * 1500.0 / 48_000.0
  for _ in 0..<16 {
    var samples = [Float](repeating: 0, count: 2048)
    for frame in 0..<1024 {
      let value = Float(0.5 * sin(phase))
      phase += step
      samples[frame * 2] = value
      samples[frame * 2 + 1] = value
    }
    do {
      guard let packet = try encoder.encode(samples) else { continue }
      packets += 1
      guard let decoded = try decoder.decode(packet) else {
        check(false, "packet \(packets) gave no samples")
        continue
      }
      check(decoded.count == 2048, "packet \(packets) gave \(decoded.count) samples")
      // The first four hold the beginning of the tone that the encoder and the decoder lead in with.
      if packets > 4 { tone += decoded }
    } catch {
      check(false, "packet \(packets) was not made or not decoded: \(error)")
    }
  }
  check(packets >= 12, "\(packets) packets")
  let wanted = loudness(of: tone, channels: 2, frequency: 1500, rate: 48_000)
  let other = loudness(of: tone, channels: 2, frequency: 6000, rate: 48_000)
  check(wanted > 0.42 && wanted < 0.58, "the tone is \(wanted) loud, and was made 0.5")
  check(other < 0.02, "a tone of 6000 Hz that was not there is \(other) loud")
}

#endif
