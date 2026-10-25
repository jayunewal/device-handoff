// SPDX-License-Identifier: Apache-2.0

// The Swift side of the C interface through which a device shows a window that another device
// shares: crates/baton-stream-abi/include/baton_stream_abi.h says what the two sides promise each
// other, and docs/architecture.md says why it is this small.
//
// This file holds what does not depend on the frameworks of an Apple device: the functions that
// the Rust side calls, the reading of what it passes, and the rules for which picture is shown
// and when a decoder has failed. The decoder, the screen and the texture are given to it as
// protocols. So that it can be built and tested on any computer that has a Swift compiler
// (apps/baton/rust_builder/swift_tests), nothing here may import more than Foundation.
//
// A call from Rust must never end in a trap, because a trap ends the whole app: nothing here
// force-unwraps, indexes without looking, or asserts on what Rust passed.

import Foundation

// MARK: - What the platform is asked to do

/// What the device can show. The Rust side checks the numbers against what the protocol allows.
struct ViewerLimits: Equatable {
  var maxWidth: UInt32
  var maxHeight: UInt32
  var maxFps: UInt32
  /// The codecs the decoder takes, as BATON_CODEC_ bits.
  var codecs: UInt32
}

/// The window that another device offered and the person said yes to, or the screen that this
/// device asked another one to start sharing. The screen is made before it is offered, so its size
/// is not known: `width` and `height` are 0, and `control` is what the person wished for. The
/// title and the name of the application are not carried: they name a document, a person or a
/// page, and nothing in the platform needs them.
struct WindowInfo: Equatable {
  var width: UInt32
  var height: UInt32
  var control: Bool
}

/// Why a screen could not be made, as a sentence that the person is shown.
struct ScreenFailure: Error, Equatable {
  var sentence: String
}

/// The part of the platform that shows windows: one for the life of the process.
protocol ViewerPlatform: AnyObject {
  /// What the device can show. The same each time.
  var limits: ViewerLimits { get }

  /// Makes the decoder and the screen for `window`, and gives the number that the user interface
  /// draws the screen with. `link` is the number to give `baton_stream_link_decode_failed` for
  /// this screen. The call does not wait: a platform that needs time fails, with a sentence. Two
  /// windows that are said yes to at the same moment are made on two threads, and the calls of the
  /// screens that are open go on meanwhile, so whatever this shares with them is kept under a
  /// lock.
  func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen
}

struct MadeScreen {
  var screen: ViewerScreen
  var id: Int64
}

/// One window that is shown. The Rust side calls these one at a time, but not always on the same
/// thread, and `ended` and `released` may come while a picture is still being decoded.
protocol ViewerScreen: AnyObject {
  /// Decodes the frame. false means that this frame cannot be decoded, and the Rust side asks the
  /// source for a key frame.
  func decode(_ frame: FrameInput) -> Bool
  /// Shows the picture of frame `number`, the newest that `decode` took.
  func show(_ number: UInt64)
  /// The source said something about the pictures.
  func source(_ message: SourceInput)
  /// How the decoder is doing.
  func state() -> DecoderReport
  /// The session is over: let go of what the screen shows.
  func ended()
  /// The screen is not needed any more, whether or not `ended` came first. Called once.
  func released()
}

// MARK: - What the Rust side passes

/// A frame to decode. The memory behind `sample` and `sets` is valid until the call returns and
/// not after: whatever has to outlive the call is copied.
struct FrameInput {
  var flags: UInt32
  var number: UInt64
  var dependsOn: UInt64
  var captureTime: UInt64
  var chain: UInt32
  var epoch: UInt32
  /// The picture in AVCC form: the units behind lengths of four bytes, without parameter sets.
  var sample: UnsafeRawBufferPointer
  /// The parameter sets that the frame came with: sequence parameter sets first, then picture
  /// parameter sets, each without a start code. Empty for most frames that are not key frames.
  var sets: [UnsafeRawBufferPointer]
  /// How many of the first `sets` are sequence parameter sets.
  var sequenceSets: Int

  var isKey: Bool { flags & BATON_FRAME_KEY != 0 }

  /// Copies of the parameter sets, or nil if the frame came with none.
  var parameterSets: ParameterSets? {
    if sets.isEmpty { return nil }
    let copies = sets.map { Data($0) }
    return ParameterSets(
      sequence: Array(copies.prefix(sequenceSets)), picture: Array(copies.dropFirst(sequenceSets)))
  }

  /// The most parameter sets that a frame may come with. An encoder writes a few.
  static let mostSets = 64

  /// Reads a frame of the interface. nil if the struct is shorter than this version of the
  /// interface knows it, or a pointer or a count is wrong.
  init?(reading frame: UnsafePointer<BatonFrame>) {
    // The size is read alone first: a struct that is shorter than this version of the interface
    // knows it is not copied, since the copy would read past its end.
    guard frame.pointee.size >= UInt32(MemoryLayout<BatonFrame>.size) else { return nil }
    let c = frame.pointee
    guard let sample = UnsafeRawBufferPointer(reading: c.sample) else { return nil }
    // 0 <= sequence_sets <= set_count also keeps set_count from being below zero.
    guard c.set_count <= FrameInput.mostSets, c.sequence_sets >= 0,
      c.sequence_sets <= c.set_count
    else { return nil }
    var sets: [UnsafeRawBufferPointer] = []
    if c.set_count > 0 {
      guard let table = c.sets else { return nil }
      for entry in UnsafeBufferPointer(start: table, count: c.set_count) {
        guard let set = UnsafeRawBufferPointer(reading: entry) else { return nil }
        sets.append(set)
      }
    }
    self.init(
      flags: c.flags, number: c.number, dependsOn: c.depends_on, captureTime: c.capture_time,
      chain: c.chain, epoch: c.epoch, sample: sample, sets: sets, sequenceSets: c.sequence_sets)
  }

  init(
    flags: UInt32, number: UInt64, dependsOn: UInt64, captureTime: UInt64, chain: UInt32,
    epoch: UInt32, sample: UnsafeRawBufferPointer, sets: [UnsafeRawBufferPointer],
    sequenceSets: Int
  ) {
    self.flags = flags
    self.number = number
    self.dependsOn = dependsOn
    self.captureTime = captureTime
    self.chain = chain
    self.epoch = epoch
    self.sample = sample
    self.sets = sets
    self.sequenceSets = sequenceSets
  }
}

extension UnsafeRawBufferPointer {
  /// The bytes of a `BatonBytes`. nil if it has a length and no pointer.
  init?(reading bytes: BatonBytes) {
    if bytes.len == 0 {
      self.init(start: nil, count: 0)
      return
    }
    guard bytes.len > 0, let start = bytes.ptr else { return nil }
    self.init(start: start, count: bytes.len)
  }
}

/// The parameter sets that frames come with, as copies.
struct ParameterSets: Equatable {
  var sequence: [Data]
  var picture: [Data]

  var all: [Data] { sequence + picture }
}

/// What the source says about the pictures.
enum SourceInput: Equatable {
  /// The pictures that follow have this epoch, size and top rate.
  case config(epoch: UInt32, width: UInt32, height: UInt32, fps: UInt32)
  /// No pictures for a while, for a reason (BATON_PAUSE_).
  case pause(reason: UInt32)
  /// Pictures go on.
  case resume
  /// The source dropped `count` events of this device, for a reason (BATON_DROPPED_).
  case notice(count: UInt32, reason: UInt32)

  /// Reads a message of the interface. nil if the struct is shorter than this version of the
  /// interface knows it, or the kind is one it does not know.
  init?(reading message: UnsafePointer<BatonSourceMessage>) {
    // The size first and alone, as for a frame.
    guard message.pointee.size >= UInt32(MemoryLayout<BatonSourceMessage>.size) else { return nil }
    let c = message.pointee
    switch c.kind {
    case BATON_SOURCE_CONFIG:
      self = .config(epoch: c.epoch, width: c.width, height: c.height, fps: c.fps)
    case BATON_SOURCE_PAUSE:
      self = .pause(reason: c.reason)
    case BATON_SOURCE_RESUME:
      self = .resume
    case BATON_SOURCE_NOTICE:
      self = .notice(count: c.count, reason: c.reason)
    default:
      return nil
    }
  }
}

/// How the decoder is doing.
struct DecoderReport: Equatable {
  /// The time a frame takes, smoothed, in microseconds.
  var decodeMicros: UInt32
  /// BATON_THERMAL_.
  var thermal: UInt32
}

// MARK: - The functions that the Rust side calls

/// What the context pointer of the registered table points to.
private final class ViewerBox {
  let platform: ViewerPlatform

  init(_ platform: ViewerPlatform) {
    self.platform = platform
  }
}

/// What the context pointer of a screen points to.
private final class ScreenBox {
  let screen: ViewerScreen

  init(_ screen: ViewerScreen) {
    self.screen = screen
  }
}

private let ok = Int32(BATON_OK)
private let badArgument = Int32(BATON_E_ARGUMENT)
private let platformFailure = Int32(BATON_E_PLATFORM)

private func viewerLimits(
  _ context: UnsafeMutableRawPointer?, _ limits: UnsafeMutablePointer<BatonLimits>?
) -> Int32 {
  guard let context, let limits, limits.pointee.size >= UInt32(MemoryLayout<BatonLimits>.size)
  else { return badArgument }
  let wanted = Unmanaged<ViewerBox>.fromOpaque(context).takeUnretainedValue().platform.limits
  limits.pointee.max_width = wanted.maxWidth
  limits.pointee.max_height = wanted.maxHeight
  limits.pointee.max_fps = wanted.maxFps
  limits.pointee.codecs = wanted.codecs
  return ok
}

private func viewerOpen(
  _ context: UnsafeMutableRawPointer?, _ window: UnsafePointer<BatonWindow>?, _ link: UInt64,
  _ screen: UnsafeMutablePointer<BatonScreen>?, _ message: UnsafeMutablePointer<BatonMessage>?
) -> Int32 {
  guard let context, let window, let screen, let message,
    window.pointee.size >= UInt32(MemoryLayout<BatonWindow>.size),
    screen.pointee.size >= UInt32(MemoryLayout<BatonScreen>.size),
    message.pointee.size >= UInt32(MemoryLayout<BatonMessage>.size)
  else { return badArgument }
  let platform = Unmanaged<ViewerBox>.fromOpaque(context).takeUnretainedValue().platform
  let info = WindowInfo(
    width: window.pointee.width, height: window.pointee.height,
    control: window.pointee.control != 0)
  do {
    let made = try platform.makeScreen(for: info, link: link)
    screen.pointee.context = Unmanaged.passRetained(ScreenBox(made.screen)).toOpaque()
    screen.pointee.id = made.id
    return ok
  } catch let failure as ScreenFailure {
    write(failure.sentence, to: message)
    return platformFailure
  } catch {
    write("The window could not be shown.", to: message)
    return platformFailure
  }
}

/// Writes the sentence into the room the Rust side gave for it: as much of it as fits, cut
/// between two characters.
func write(_ sentence: String, to message: UnsafeMutablePointer<BatonMessage>) {
  guard let text = message.pointee.text else {
    message.pointee.len = 0
    return
  }
  let room = Int(message.pointee.cap)
  var written = 0
  for scalar in sentence.unicodeScalars {
    let bytes = Array(String(scalar).utf8)
    if written + bytes.count > room { break }
    for byte in bytes {
      text[written] = byte
      written += 1
    }
  }
  message.pointee.len = UInt32(written)
}

/// The screen that a context pointer of the Rust side stands for.
private func screenOf(_ pointer: UnsafeMutableRawPointer?) -> ViewerScreen? {
  guard let pointer else { return nil }
  return Unmanaged<ScreenBox>.fromOpaque(pointer).takeUnretainedValue().screen
}

private func viewerDecode(_ screen: UnsafeMutableRawPointer?, _ frame: UnsafePointer<BatonFrame>?)
  -> Int32
{
  guard let screen = screenOf(screen), let frame, let input = FrameInput(reading: frame)
  else { return badArgument }
  return screen.decode(input) ? ok : platformFailure
}

private func viewerShow(_ screen: UnsafeMutableRawPointer?, _ number: UInt64) {
  screenOf(screen)?.show(number)
}

private func viewerSource(
  _ screen: UnsafeMutableRawPointer?, _ message: UnsafePointer<BatonSourceMessage>?
) {
  guard let screen = screenOf(screen), let message, let input = SourceInput(reading: message)
  else { return }
  screen.source(input)
}

private func viewerState(
  _ screen: UnsafeMutableRawPointer?, _ state: UnsafeMutablePointer<BatonDecoderState>?
) {
  guard let screen = screenOf(screen), let state,
    state.pointee.size >= UInt32(MemoryLayout<BatonDecoderState>.size)
  else { return }
  let report = screen.state()
  state.pointee.decode_us = report.decodeMicros
  state.pointee.thermal = report.thermal
}

private func viewerEnded(_ screen: UnsafeMutableRawPointer?) {
  screenOf(screen)?.ended()
}

private func viewerRelease(_ screen: UnsafeMutableRawPointer?) {
  guard let screen else { return }
  let box = Unmanaged<ScreenBox>.fromOpaque(screen)
  box.takeUnretainedValue().screen.released()
  box.release()
}

/// Gives the Rust side the platform's viewer: a device that has registered one takes the windows
/// that other devices offer, and a device that has not takes none. Call it once, at launch,
/// before the app starts. Returns what `baton_stream_register_viewer` returned.
@discardableResult
func registerViewer(_ platform: ViewerPlatform) -> Int32 {
  // The Rust side keeps the context for as long as the process lives, so it is let go of only if
  // the registration failed.
  let context = Unmanaged.passRetained(ViewerBox(platform))
  var table = BatonViewerVTable(
    version: BATON_STREAM_ABI_VERSION,
    size: UInt32(MemoryLayout<BatonViewerVTable>.size),
    context: context.toOpaque(),
    limits: viewerLimits,
    open: viewerOpen,
    decode: viewerDecode,
    show: viewerShow,
    source: viewerSource,
    state: viewerState,
    ended: viewerEnded,
    release: viewerRelease)
  let code = baton_stream_register_viewer(&table)
  if code != ok {
    context.release()
  }
  return code
}

// MARK: - Which picture is shown

/// The pictures that came out of the decoder and are not shown yet. The Rust side says which
/// frame to show, the decoder gives its pictures back in its own time, and either can come
/// first: a picture is shown once both have happened for it, and one older than the picture
/// shown last is never shown.
final class PictureTable<Picture> {
  private let lock = NSLock()
  private var pictures: [UInt64: Picture] = [:]
  private var wanted: UInt64?
  private var shown: UInt64?
  private let capacity: Int

  /// `capacity` is the most pictures kept that nobody has asked to show. The decoder is not
  /// told to hold more than a few, so more than that is a source of frames that nobody shows.
  init(capacity: Int = 16) {
    self.capacity = capacity
  }

  /// A picture came out of the decoder. The picture to show now, if this is the one.
  func decoded(_ number: UInt64, _ picture: Picture) -> Picture? {
    lock.lock()
    defer { lock.unlock() }
    if let shown, number <= shown { return nil }
    if wanted == number { return take(number, picture) }
    pictures[number] = picture
    while pictures.count > capacity, let oldest = pictures.keys.min() {
      pictures[oldest] = nil
    }
    return nil
  }

  /// The Rust side wants the picture of this frame on the screen. The picture to show now, if it
  /// is here already. A picture older than the one shown is gone, so it is not found.
  func show(_ number: UInt64) -> Picture? {
    lock.lock()
    defer { lock.unlock() }
    wanted = number
    guard let picture = pictures[number] else { return nil }
    return take(number, picture)
  }

  /// How many pictures are kept that nobody has asked to show.
  var held: Int {
    lock.lock()
    defer { lock.unlock() }
    return pictures.count
  }

  /// Forgets everything, as the end of a session does.
  func clear() {
    lock.lock()
    defer { lock.unlock() }
    pictures = [:]
    wanted = nil
    shown = nil
  }

  /// Marks `number` as shown, and drops the pictures that are older. The lock is held.
  private func take(_ number: UInt64, _ picture: Picture) -> Picture {
    shown = number
    pictures = pictures.filter { $0.key > number }
    return picture
  }
}

// MARK: - A screen

/// Why a picture did not come out of the decoder.
enum DecodeFailure: Error, Equatable {
  /// The decoder reported this status for the frame.
  case status(Int32)
  /// The decoder dropped the frame without saying why.
  case dropped
}

/// A decoder for the pictures of one window. On a device it is VideoToolbox.
protocol PictureDecoder: AnyObject {
  associatedtype Picture

  /// Makes ready for frames that are coded with these parameter sets. false if that cannot be
  /// done, for example because the sets describe a picture that the device cannot decode.
  func prepare(_ sets: ParameterSets) -> Bool

  /// Starts to decode `sample` and returns at once, with whether it was taken. If it was, `then`
  /// is called once, on any thread, with the picture or the reason there is none. The bytes of
  /// `sample` are copied before the call returns.
  func decode(
    _ sample: UnsafeRawBufferPointer, number: UInt64,
    then finish: @escaping (Result<Picture, DecodeFailure>) -> Void
  ) -> Bool

  /// Lets go of the decoder. Frames that are still being decoded may or may not come out.
  func stop()
}

/// The time a frame takes to decode, smoothed: each new time moves the value an eighth of the way.
struct SmoothedMicros {
  private var value: Double?

  mutating func add(_ micros: Double) {
    guard micros.isFinite, micros >= 0 else { return }
    value = value.map { $0 + (micros - $0) / 8 } ?? micros
  }

  var rounded: UInt32 {
    guard let value, value.isFinite else { return 0 }
    return UInt32(min(max(value, 0), Double(UInt32.max)))
  }
}

/// A window that is shown: it takes the frames, decodes them, and shows the pictures that the
/// Rust side asks for. What shows a picture, and what tells how hot the device is, are given to it.
final class WindowScreen<Decoder: PictureDecoder>: ViewerScreen {
  private let decoder: Decoder
  private let pictures = PictureTable<Decoder.Picture>()
  private let present: (Decoder.Picture?) -> Void
  private let failed: () -> Void
  private let thermal: () -> UInt32
  private let onRelease: () -> Void

  private let lock = NSLock()
  private var sets: ParameterSets?
  private var waitingForKey = true
  private var over = false
  private var timing = SmoothedMicros()

  /// `present` shows a picture, or nothing; it may be called on any thread, but only one call at
  /// a time, in the order in which the pictures were chosen, and with nothing after the end of the
  /// screen except the nothing that takes the picture away. It is called with a lock held, so it
  /// must not call back into the screen. `failed` tells the Rust side that the decoder failed on a
  /// frame after it had taken it, and is called on a thread of its own, never from within a call
  /// of the Rust side. `onRelease` is called once, when the Rust side no longer needs the screen.
  init(
    decoder: Decoder, present: @escaping (Decoder.Picture?) -> Void,
    failed: @escaping () -> Void, thermal: @escaping () -> UInt32,
    onRelease: @escaping () -> Void
  ) {
    self.decoder = decoder
    self.present = present
    self.failed = failed
    self.thermal = thermal
    self.onRelease = onRelease
  }

  func decode(_ frame: FrameInput) -> Bool {
    lock.lock()
    let finished = over
    // After a failure nothing is decoded until a key frame comes: the frames in between would
    // come out wrong, or not at all.
    let skip = waitingForKey && !frame.isKey
    let known = sets
    lock.unlock()
    if finished || skip { return false }

    var current = known
    if let fresh = frame.parameterSets, fresh != known {
      guard decoder.prepare(fresh) else {
        lock.lock()
        sets = nil
        waitingForKey = true
        lock.unlock()
        return false
      }
      current = fresh
    }
    if current == nil { return false }

    lock.lock()
    // What the decoder is prepared for, whether or not this frame is decoded.
    sets = current
    // A decoder may have failed on an earlier frame while this one was prepared for, or the
    // screen may have ended: after a failure only a key frame goes on, and the end takes all.
    if over || (waitingForKey && !frame.isKey) {
      lock.unlock()
      return false
    }
    waitingForKey = false
    lock.unlock()

    let started = DispatchTime.now().uptimeNanoseconds
    let number = frame.number
    let taken = decoder.decode(frame.sample, number: number) { [weak self] result in
      self?.decoded(number, started: started, result)
    }
    if !taken {
      lock.lock()
      waitingForKey = true
      lock.unlock()
    }
    return taken
  }

  private func decoded(_ number: UInt64, started: UInt64, _ result: Result<Decoder.Picture, DecodeFailure>) {
    switch result {
    case .success(let picture):
      // The picture is shown with the lock held. The end of the screen takes the lock to say that
      // it is over, so a picture is either on the screen before the end takes it away, or is not
      // shown at all: it cannot come after the texture was given to another window.
      lock.lock()
      defer { lock.unlock() }
      if over { return }
      let elapsed = DispatchTime.now().uptimeNanoseconds &- started
      timing.add(Double(elapsed) / 1000)
      if let shown = pictures.decoded(number, picture) { present(shown) }
    case .failure:
      lock.lock()
      let report = !over && !waitingForKey
      waitingForKey = true
      lock.unlock()
      // Once for a run of failures: the Rust side asks for a key frame and sends none other until
      // it comes. The call is made from a queue and not from here, since the decoder may call
      // this on the thread that is inside `decode`, which the Rust side would refuse.
      if report { DispatchQueue.global().async { [failed] in failed() } }
    }
  }

  func show(_ number: UInt64) {
    // With the lock held, for the reason given in `decoded`, and so that two pictures that are
    // chosen at the same time reach the screen in the order they were chosen.
    lock.lock()
    defer { lock.unlock() }
    if let picture = pictures.show(number) { present(picture) }
  }

  /// The source's messages change nothing here yet: the decoder works from the parameter sets,
  /// which come with the key frames.
  func source(_ message: SourceInput) {}

  func state() -> DecoderReport {
    lock.lock()
    let micros = timing.rounded
    lock.unlock()
    return DecoderReport(decodeMicros: micros, thermal: thermal())
  }

  func ended() {
    lock.lock()
    let already = over
    over = true
    lock.unlock()
    if already { return }
    decoder.stop()
    pictures.clear()
    present(nil)
  }

  func released() {
    ended()
    onRelease()
  }
}

// MARK: - Where windows are drawn

/// What `ScreenPlaces` has made.
struct PlacesStatus: Equatable {
  var made: Int
  var wanted: Int
  /// How many times `fill` was called.
  var tries: Int
}

/// The places that windows are drawn on, a fixed number of them, each used by one window at a
/// time. On a device a place is a texture of Flutter. Flutter takes a texture only on the platform
/// thread, and the Rust side asks for a screen from a thread of its own and does not wait, so the
/// places are made ahead of the first window, on the platform thread, and a window takes one for as
/// long as it is shown.
///
/// A place cannot always be made when the app starts: Flutter on a Mac takes a texture only once
/// its engine runs, and that is after the plugins are registered. `fill` makes the places that are
/// missing and says whether they all are there, so that it can be called again (`keepTrying`).
final class ScreenPlaces<Place: AnyObject> {
  /// Held while places are made, so that `make` is never called twice at the same time.
  private let filling = NSLock()
  /// Held for a moment, never while `make` runs, so that a window does not wait for Flutter.
  private let lock = NSLock()
  private let wanted: Int
  private let make: () -> Place?
  private var places: [Place] = []
  private var taken = Set<ObjectIdentifier>()
  private var tries = 0

  /// `make` makes a place, or answers nil if the system would not give one. It is called by
  /// `fill` only, and so on the thread that calls that.
  init(count: Int, make: @escaping () -> Place?) {
    wanted = max(count, 0)
    self.make = make
  }

  /// Makes the places that are missing, and stops at the first that cannot be made. true if all
  /// of them are there. Call it on the thread that `make` needs. A place can be taken as soon as
  /// it is made.
  @discardableResult
  func fill() -> Bool {
    filling.lock()
    defer { filling.unlock() }
    lock.lock()
    tries += 1
    var missing = wanted - places.count
    lock.unlock()
    while missing > 0 {
      guard let place = make() else { break }
      lock.lock()
      places.append(place)
      lock.unlock()
      missing -= 1
    }
    lock.lock()
    defer { lock.unlock() }
    return places.count >= wanted
  }

  /// A place that no window uses, or nil if there is none: every one is in use, or none was made.
  func take() -> Place? {
    lock.lock()
    defer { lock.unlock() }
    guard let free = places.first(where: { !taken.contains(ObjectIdentifier($0)) }) else {
      return nil
    }
    taken.insert(ObjectIdentifier(free))
    return free
  }

  /// Gives the place back. A place that is not in use, or is not one of these, changes nothing.
  func give(_ place: Place) {
    lock.lock()
    taken.remove(ObjectIdentifier(place))
    lock.unlock()
  }

  var status: PlacesStatus {
    lock.lock()
    defer { lock.unlock() }
    return PlacesStatus(made: places.count, wanted: wanted, tries: tries)
  }
}

/// Does `attempt` and, as long as it says that it is not done, does it again after each of
/// `delays`, in turn. `giveUp` is called when the last attempt was not done either. `later` runs a
/// piece of work after a time, on the thread that `attempt` needs.
func keepTrying(
  after delays: [TimeInterval],
  later: @escaping (TimeInterval, @escaping () -> Void) -> Void,
  attempt: @escaping () -> Bool,
  giveUp: @escaping () -> Void
) {
  func step(_ next: Int) {
    if attempt() { return }
    guard next < delays.count else {
      giveUp()
      return
    }
    later(delays[next]) { step(next + 1) }
  }
  step(0)
}
