// SPDX-License-Identifier: Apache-2.0

// The Swift side of the C interface through which a device shares a window of its own:
// crates/baton-stream-abi/include/baton_stream_abi.h says what the two sides promise each other,
// and docs/architecture.md says why it is this small.
//
// This file holds what does not depend on the frameworks of an Apple device: the functions that
// the Rust side calls, the reading of what it passes, the picture that is chosen for a window, and
// the rules for when a picture is made and what becomes of it. The picker, the capture and the
// encoder are given to it as protocols. So that it can be built and tested on any computer that
// has a Swift compiler (apps/baton/rust_builder/swift_tests), nothing here may import more than
// Foundation.
//
// A call from Rust must never end in a trap, because a trap ends the whole app: nothing here
// force-unwraps, indexes without looking, or asserts on what Rust passed.

import Foundation

// MARK: - What the platform is asked to do

/// What the viewer of the other device can take, as it said when it accepted the window.
struct ViewerTakes: Equatable {
  var maxWidth: UInt32
  var maxHeight: UInt32
  var maxFps: UInt32
  /// The codecs its decoder takes, as BATON_CODEC_ bits.
  var codecs: UInt32
}

/// The picture that the platform makes of a window: H.264, this large, and at most this many
/// frames a second.
struct PictureFormat: Equatable {
  var width: UInt32
  var height: UInt32
  var fps: UInt32
}

/// The window, or the whole display, that the person picked.
struct PickedWindowInfo: Equatable {
  /// The size of the window or the display in pixels.
  var width: UInt32
  var height: UInt32
  /// A title names a document, a person or a page: it is shown to the person at the other device
  /// and goes nowhere else. A display has no application, and its title is its name, if there is
  /// one.
  var title: String
  var app: String
  /// Whether it is a whole display, with every window on it, and not one window.
  var display = false

  /// What it is called in a sentence for the person.
  var noun: String { display ? "screen" : "window" }
}

/// Why a window could not be picked, prepared or started, as a sentence that the person is shown.
struct SharerFailure: Error, Equatable {
  var sentence: String
}

/// The part of the platform that shares windows: one for the life of the process.
protocol SharerPlatform: AnyObject {
  /// Whether the person can let another device control a window that they share.
  var canControl: Bool { get }

  /// Shows the picker of the system. It does not wait for the person: the answer is given to
  /// `answerPick`, once for each request, from any thread but not from inside this call. Throws
  /// `SharerFailure` if the picker cannot be shown, and nothing is answered then.
  func pick(request: UInt64) throws

  /// Nobody waits for the answer to `request` any more: take the picker down if it is up. An
  /// answer that comes anyway is refused, and the window it names stays with the platform.
  func cancelPick(request: UInt64)
}

/// A window that the person picked, and everything that the platform keeps for it. The Rust side
/// calls these one at a time, but not always on the same thread.
protocol SharedWindow: AnyObject {
  /// The other device said yes, and `takes` is what it can take. Chooses the picture to begin
  /// with, within that, and gets ready to make it. Throws if no picture within what it takes can
  /// be made. Called at most once.
  func prepare(takes: ViewerTakes) throws -> PictureFormat

  /// The session runs, and `link` is its number for the calls about it. Starts to capture the
  /// window and to encode it. It does not wait for the first picture. Throws `SharerFailure` with
  /// a sentence if the capture cannot begin: the session ends, and nothing follows but `released`.
  func start(link: UInt64) throws

  /// The session is over, whoever ended it. Stops capturing and encoding and lets go of what the
  /// capture holds. The link is closed before this is called, and a call into the library that is
  /// on its way is answered with the news that it is over, so this may wait for the threads of the
  /// capture to leave. Called only after a `start` that worked, once.
  func ended()

  /// The window is not needed any more, whether or not `ended` came first, and whether or not it
  /// was ever started. Lets go of all of it, the result of the picker included. Called once.
  func released()
}

// MARK: - The answer to a pick

/// What the person did at the picker.
enum PickAnswer {
  /// The person chose this window or display. The Rust side owns it from then on if it takes the
  /// answer.
  case chose(SharedWindow, PickedWindowInfo)
  /// The picker failed, with a sentence for the person.
  case failed(String)
  /// The person closed the picker without choosing.
  case gaveUp
}

/// What the context pointer of a window points to.
private final class WindowBox {
  let window: SharedWindow

  init(_ window: SharedWindow) {
    self.window = window
  }
}

/// What the context pointer of the registered table points to.
private final class SharerBox {
  let platform: SharerPlatform

  init(_ platform: SharerPlatform) {
    self.platform = platform
  }
}

private let ok = Int32(BATON_OK)
private let badArgument = Int32(BATON_E_ARGUMENT)
private let platformFailure = Int32(BATON_E_PLATFORM)

/// Gives the Rust side what the person did at the picker. Call it once for each request that
/// `SharerPlatform.pick` was given, from any thread but not from inside `pick`. Returns what
/// `baton_stream_pick_done` returned: when it is not `BATON_OK`, the window stays with the
/// platform, which is the caller here, and is let go of when the caller lets go of it.
@discardableResult
func answerPick(request: UInt64, _ answer: PickAnswer) -> Int32 {
  switch answer {
  case .chose(let window, let info):
    var title = info.title
    var app = info.app
    // The Rust side takes the window with the answer and gives it back with `release`.
    let box = Unmanaged.passRetained(WindowBox(window))
    let code = title.withUTF8 { titleBytes in
      app.withUTF8 { appBytes in
        var picked = BatonPicked(
          size: UInt32(MemoryLayout<BatonPicked>.size), width: info.width, height: info.height,
          kind: info.display ? BATON_PICKED_DISPLAY : BATON_PICKED_WINDOW, context: box.toOpaque(),
          title: BatonBytes(ptr: titleBytes.baseAddress, len: titleBytes.count),
          app: BatonBytes(ptr: appBytes.baseAddress, len: appBytes.count))
        return baton_stream_pick_done(request, &picked, nil)
      }
    }
    if code != ok {
      box.release()
    }
    return code
  case .failed(var sentence):
    return sentence.withUTF8 { bytes in
      var failure = BatonBytes(ptr: bytes.baseAddress, len: bytes.count)
      return baton_stream_pick_done(request, nil, &failure)
    }
  case .gaveUp:
    return baton_stream_pick_done(request, nil, nil)
  }
}

// MARK: - The functions that the Rust side calls

private func sharerOf(_ context: UnsafeMutableRawPointer?) -> SharerPlatform? {
  guard let context else { return nil }
  return Unmanaged<SharerBox>.fromOpaque(context).takeUnretainedValue().platform
}

private func windowOf(_ pointer: UnsafeMutableRawPointer?) -> SharedWindow? {
  guard let pointer else { return nil }
  return Unmanaged<WindowBox>.fromOpaque(pointer).takeUnretainedValue().window
}

private func sharerControl(_ context: UnsafeMutableRawPointer?) -> UInt32 {
  guard let platform = sharerOf(context) else { return 0 }
  return platform.canControl ? 1 : 0
}

private func sharerPick(
  _ context: UnsafeMutableRawPointer?, _ request: UInt64,
  _ message: UnsafeMutablePointer<BatonMessage>?
) -> Int32 {
  guard let platform = sharerOf(context), let message,
    message.pointee.size >= UInt32(MemoryLayout<BatonMessage>.size)
  else { return badArgument }
  do {
    try platform.pick(request: request)
    return ok
  } catch let failure as SharerFailure {
    write(failure.sentence, to: message)
    return platformFailure
  } catch {
    write("The window picker could not be shown.", to: message)
    return platformFailure
  }
}

private func sharerCancelPick(_ context: UnsafeMutableRawPointer?, _ request: UInt64) {
  sharerOf(context)?.cancelPick(request: request)
}

private func sharerPrepare(
  _ picked: UnsafeMutableRawPointer?, _ takes: UnsafePointer<BatonTakes>?,
  _ format: UnsafeMutablePointer<BatonFormat>?
) -> Int32 {
  guard let window = windowOf(picked), let takes, let format,
    takes.pointee.size >= UInt32(MemoryLayout<BatonTakes>.size),
    format.pointee.size >= UInt32(MemoryLayout<BatonFormat>.size)
  else { return badArgument }
  let taken = ViewerTakes(
    maxWidth: takes.pointee.max_width, maxHeight: takes.pointee.max_height,
    maxFps: takes.pointee.max_fps, codecs: takes.pointee.codecs)
  do {
    let chosen = try window.prepare(takes: taken)
    format.pointee.codec = BATON_CODEC_H264
    format.pointee.width = chosen.width
    format.pointee.height = chosen.height
    format.pointee.fps = chosen.fps
    return ok
  } catch {
    return platformFailure
  }
}

private func sharerStart(
  _ picked: UnsafeMutableRawPointer?, _ link: UInt64, _ message: UnsafeMutablePointer<BatonMessage>?
) -> Int32 {
  guard let window = windowOf(picked), let message,
    message.pointee.size >= UInt32(MemoryLayout<BatonMessage>.size)
  else { return badArgument }
  do {
    try window.start(link: link)
    return ok
  } catch let failure as SharerFailure {
    write(failure.sentence, to: message)
    return platformFailure
  } catch {
    write("This device could not start sharing the window.", to: message)
    return platformFailure
  }
}

private func sharerEnded(_ picked: UnsafeMutableRawPointer?) {
  windowOf(picked)?.ended()
}

private func sharerRelease(_ picked: UnsafeMutableRawPointer?) {
  guard let picked else { return }
  let box = Unmanaged<WindowBox>.fromOpaque(picked)
  box.takeUnretainedValue().window.released()
  box.release()
}

/// Gives the Rust side the platform's sharer: a device that has registered one lets the person
/// share a window, and a device that has not shares none. Call it once, at launch, before the app
/// starts. Returns what `baton_stream_register_sharer` returned.
@discardableResult
func registerSharer(_ platform: SharerPlatform) -> Int32 {
  // The Rust side keeps the context for as long as the process lives, so it is let go of only if
  // the registration failed.
  let context = Unmanaged.passRetained(SharerBox(platform))
  var table = BatonSharerVTable(
    version: BATON_STREAM_ABI_VERSION,
    size: UInt32(MemoryLayout<BatonSharerVTable>.size),
    context: context.toOpaque(),
    control: sharerControl,
    pick: sharerPick,
    cancel_pick: sharerCancelPick,
    prepare: sharerPrepare,
    start: sharerStart,
    ended: sharerEnded,
    release: sharerRelease)
  let code = baton_stream_register_sharer(&table)
  if code != ok {
    context.release()
  }
  return code
}

// MARK: - The picture that is made of a window

/// The most pixels along the longer edge of a picture. A window is looked at on a phone or a
/// tablet, from the distance of a hand, so a larger picture would only be bandwidth.
let longestPictureEdge: UInt32 = 1920

/// The most frames a second. A window that is looked at and, later, steered does not need more,
/// and an encoder that is asked for more spends the battery of a laptop on it.
let mostPictureFps: UInt32 = 30

/// The picture to make of a window of this size, within what the viewer takes: the window as it
/// is, or scaled down to fit, with its shape kept, and never scaled up. Both edges are even and at
/// least 2, since H.264 stores colour at half the size. nil if the viewer takes no H.264 or no
/// picture at all.
func chooseFormat(windowWidth: UInt32, windowHeight: UInt32, takes: ViewerTakes) -> PictureFormat? {
  guard takes.codecs & BATON_CODEC_H264 != 0, takes.maxFps >= 1, takes.maxWidth >= 2,
    takes.maxHeight >= 2, windowWidth >= 1, windowHeight >= 1
  else { return nil }
  let widest = UInt64(min(takes.maxWidth, longestPictureEdge))
  let highest = UInt64(min(takes.maxHeight, longestPictureEdge))
  var width = UInt64(windowWidth)
  var height = UInt64(windowHeight)
  if width > widest || height > highest {
    // The edge that is more over its limit decides how much the window is scaled.
    if widest * height <= highest * width {
      height = height * widest / width
      width = widest
    } else {
      width = width * highest / height
      height = highest
    }
  }
  return PictureFormat(
    width: evenEdge(width), height: evenEdge(height), fps: min(takes.maxFps, mostPictureFps))
}

/// An edge rounded down to an even number of pixels, but not below 2: a window that is far wider
/// than it is high is scaled to an edge of 0, and gets 2.
private func evenEdge(_ edge: UInt64) -> UInt32 {
  let even = UInt32(truncatingIfNeeded: edge) & ~1
  return max(2, even)
}

/// The bits a second to give the encoder for pictures of this format: enough for the text and the
/// edges of a window to stay sharp, and not so much that a link which is slower than the local
/// network falls behind.
func targetBitrate(for format: PictureFormat) -> Int {
  let pixelsPerSecond = Double(format.width) * Double(format.height) * Double(format.fps)
  return Int(min(max(pixelsPerSecond / 10, 1_000_000), 16_000_000))
}

// MARK: - The session of a window that is shared

/// What `baton_stream_source_begin` says about the next picture.
enum BeginAnswer: Equatable {
  /// No picture is wanted now: ask again later.
  case notNow
  /// Make a key frame.
  case key
  /// Make a recovery frame. A platform that marks no picture as a long-term reference is never
  /// asked for one, and makes a key frame.
  case recovery
  /// Make a frame that follows the one before.
  case delta
  /// The session is over, or the call was refused: stop.
  case over
}

/// What became of a picture that was handed over.
enum SendAnswer: Equatable {
  /// It went out.
  case sent
  /// It is of no use any more: drop it and begin again.
  case stale
  /// It is larger than the protocol allows. Nothing went out.
  case tooLarge
  /// The session is over. Nothing went out.
  case over
  /// The library did not take it, because it is not a picture that can go on the wire.
  case refused
}

/// A picture as the encoder made it.
struct EncodedPicture: Equatable {
  /// The picture in AVCC form: the units behind lengths of `lengthSize` bytes.
  var sample: Data
  var lengthSize: UInt32
  /// The sequence and picture parameter sets of the encoder's format, each without a start code.
  /// A key frame has both.
  var sets: [Data]
  /// When the window was captured, on the clock of the session.
  var captureTime: UInt64
}

/// The calls the platform makes about a window that it shares, as `baton_stream_source_*` has them.
/// Each one may be made from any thread.
protocol SourceSession: AnyObject {
  /// The microseconds since the session began, or nil once it is over.
  func clock() -> UInt64?
  func begin() -> BeginAnswer
  /// The picture that `begin` announced could not be made.
  func cancel()
  /// Whether the next picture has to be something other than a delta, so that one is wanted even
  /// if the window has not changed. nil once the session is over.
  func recoveryPending() -> Bool?
  func send(_ picture: EncodedPicture) -> SendAnswer
  /// Ends the session. A sentence says that something went wrong, and the person is shown it;
  /// without one the person closed the window themselves.
  func end(sentence: String?)
}

/// The calls about a window that is shared, made to the Rust library.
final class LibrarySourceSession: SourceSession {
  private let link: UInt64

  init(link: UInt64) {
    self.link = link
  }

  func clock() -> UInt64? {
    var micros: UInt64 = 0
    return baton_stream_source_clock(link, &micros) == ok ? micros : nil
  }

  func begin() -> BeginAnswer {
    var kind: UInt32 = BATON_BEGIN_NOT_NOW
    guard baton_stream_source_begin(link, &kind) == ok else { return .over }
    switch kind {
    case BATON_BEGIN_NOT_NOW: return .notNow
    case BATON_BEGIN_KEY: return .key
    case BATON_BEGIN_RECOVERY: return .recovery
    case BATON_BEGIN_DELTA: return .delta
    default:
      // A kind that a newer library has: the picture it announced is given up on, and none is
      // made.
      _ = baton_stream_source_cancel(link)
      return .notNow
    }
  }

  func cancel() {
    _ = baton_stream_source_cancel(link)
  }

  func recoveryPending() -> Bool? {
    var pending: UInt32 = 0
    return baton_stream_source_recovery_pending(link, &pending) == ok ? pending != 0 : nil
  }

  func send(_ picture: EncodedPicture) -> SendAnswer {
    let link = self.link
    return picture.sample.withUnsafeBytes { sample in
      withBatonBytes(of: picture.sets[...]) { sets in
        // Every picture is marked as one that a later frame may refer to: an encoder that makes
        // a plain chain of frames, each from the one before, has no other kind, and a frame that
        // is wrongly marked as not referred to could be given up on alone, which would break the
        // frames that follow it.
        var c = BatonPicture(
          size: UInt32(MemoryLayout<BatonPicture>.size), flags: BATON_FRAME_REFERENCED,
          capture_time: picture.captureTime, length_size: picture.lengthSize, reserved: 0,
          sample: BatonBytes(
            ptr: sample.bindMemory(to: UInt8.self).baseAddress, len: sample.count),
          sets: sets.baseAddress, set_count: sets.count)
        var outcome: UInt32 = 0
        var number: UInt64 = 0
        let code = baton_stream_source_sent(link, &c, &outcome, &number)
        if code == Int32(BATON_E_UNKNOWN_LINK) { return .over }
        if code != ok {
          // The person is told in words that a picture could not be sent, and the log says why,
          // for whoever reads it afterwards: a window is first tried where nobody watches. The
          // log holds codes and sizes and never what is in a window.
          NSLog("Baton: the library did not take a picture of the shared window (%d).", code)
          return .refused
        }
        switch outcome {
        case BATON_SENT: return .sent
        case BATON_SENT_STALE: return .stale
        case BATON_SENT_TOO_LARGE:
          NSLog("Baton: a picture of %ld bytes is larger than the protocol allows.", sample.count)
          return .tooLarge
        case BATON_SENT_OVER: return .over
        default: return .stale
        }
      }
    }
  }

  func end(sentence: String?) {
    guard var text = sentence, !text.isEmpty else {
      _ = baton_stream_source_end(link, nil)
      return
    }
    text.withUTF8 { bytes in
      var c = BatonBytes(ptr: bytes.baseAddress, len: bytes.count)
      _ = baton_stream_source_end(link, &c)
    }
  }
}

/// Calls `body` with a `BatonBytes` for each of `sets`, valid for the call and not after it.
private func withBatonBytes<R>(
  of sets: ArraySlice<Data>, _ body: (UnsafeBufferPointer<BatonBytes>) -> R
) -> R {
  func walk(_ rest: ArraySlice<Data>, _ made: [BatonBytes]) -> R {
    guard let first = rest.first else {
      return made.withUnsafeBufferPointer(body)
    }
    return first.withUnsafeBytes { raw in
      var next = made
      next.append(
        BatonBytes(ptr: raw.bindMemory(to: UInt8.self).baseAddress, len: raw.count))
      return walk(rest.dropFirst(), next)
    }
  }
  return walk(sets, [])
}

// MARK: - When a picture is made

/// What makes the pictures: it turns an image of the window into one picture. It is used from one
/// queue at a time.
protocol PictureEncoder: AnyObject {
  /// What the capture hands over for each picture of the window.
  associatedtype Image

  /// Encodes `image`, as a key frame if `key` is true. Returns when the picture is ready. Throws
  /// if the encoder could not make it.
  func encode(_ image: Image, key: Bool, captureTime: UInt64) throws -> EncodedPicture

  /// Makes the encoder anew after a failure, so that the next picture is a key frame.
  func reset() throws

  /// Lets go of the encoder.
  func invalidate()
}

/// Asks the library when to make a picture of a window and what kind, makes it with the encoder
/// and hands it over. It is the only thing that decides when a picture is made, and it makes one
/// picture at a time: every `begin` is answered with exactly one `send` or `cancel`, before the
/// next `begin`.
///
/// The capture of a window hands over an image only when the window changes, and the library says
/// whether to make a picture only when it is asked, so an image that was not sent because none was
/// wanted at the time is kept, and asked about again on the frame clock. The same image is made
/// into a key frame again when the viewer needs one and the window has not changed.
///
/// Nothing here is thread-safe: the capture and the frame clock call it from one serial queue.
final class FramePump<Encoder: PictureEncoder> {
  private let session: SourceSession
  private let encoder: Encoder
  private let mostFailures: Int

  private var latest: Encoder.Image?
  private var latestTime: UInt64 = 0
  /// Whether `latest` is an image that no picture was made of.
  private var changed = false
  private var failures = 0
  private(set) var over = false

  /// `mostFailures` is how many pictures in a row may fail before the session is ended with a
  /// sentence.
  init(session: SourceSession, encoder: Encoder, mostFailures: Int = 3) {
    self.session = session
    self.encoder = encoder
    self.mostFailures = mostFailures
  }

  /// The capture has a new image of the window.
  func captured(_ image: Encoder.Image) {
    if over { return }
    guard let micros = session.clock() else {
      stop()
      return
    }
    latest = image
    latestTime = micros
    changed = true
    attempt()
  }

  /// The frame clock ticks: the newest image is made into a picture if none was, or if the viewer
  /// needs something other than a delta.
  func tick() {
    // After the end there is no image: `stop` lets go of it, and `captured` keeps none.
    if latest == nil { return }
    if changed {
      attempt()
      return
    }
    switch session.recoveryPending() {
    case nil: stop()
    case true?: attempt()
    case false?: break
    }
  }

  /// The session is over, or ended by the platform: nothing is made any more, and the image that
  /// was kept is let go of.
  func stop() {
    over = true
    latest = nil
  }

  private func attempt() {
    guard let image = latest else { return }
    switch session.begin() {
    case .over: stop()
    case .notNow: break
    case .key, .recovery: make(image, key: true)
    case .delta: make(image, key: false)
    }
  }

  private func make(_ image: Encoder.Image, key: Bool) {
    let picture: EncodedPicture
    do {
      picture = try encoder.encode(image, key: key, captureTime: latestTime)
    } catch {
      NSLog(
        "Baton: a picture of the shared window could not be made (%@).",
        String(describing: error))
      // The picture that `begin` announced is given up on.
      session.cancel()
      if failedAgain("This device could not encode the window.") {
        // The encoder is made anew, and the next picture is a key frame.
        do {
          try encoder.reset()
        } catch {
          NSLog("Baton: the encoder could not be made anew (%@).", String(describing: error))
          fail("This device could not encode the window.")
        }
      }
      return
    }
    switch session.send(picture) {
    case .sent:
      failures = 0
      changed = false
    case .stale:
      // Of no use any more; the next tick asks again.
      break
    case .tooLarge:
      _ = failedAgain("The window is too detailed to send.")
    case .over:
      stop()
    case .refused:
      _ = failedAgain("This device could not send the picture of the window.")
    }
  }

  /// A picture failed. The session goes on unless it has failed too often in a row, and then it
  /// ends with the sentence; false says that it did.
  private func failedAgain(_ sentence: String) -> Bool {
    failures += 1
    if failures >= mostFailures {
      fail(sentence)
      return false
    }
    return true
  }

  private func fail(_ sentence: String) {
    stop()
    session.end(sentence: sentence)
  }
}

// MARK: - What the person is told when the capture stops

/// What ScreenCaptureKit numbers its errors with (SCStreamError.Code), for the few that the person
/// is told differently. An error with another number is told in general words.
enum CaptureErrorCode {
  /// The person did not allow Baton to record the screen.
  static let userDeclined = -3801
  /// The window or the display to capture is gone.
  static let noCaptureSource = -3815
  /// The person pressed Stop in the menu bar of the Mac, where the system shows that a window is
  /// shared.
  static let userStopped = -3817
  /// The system ended the capture, for example because the display went to sleep or the person
  /// logged out.
  static let systemStopped = -3821
}

/// The sentence that tells the person why the capture of a window stopped, or nil if they stopped it
/// themselves and need not be told. `domain` and `code` are those of the error that the system gave.
func captureStoppedSentence(domain: String, code: Int) -> String? {
  // Numbers of another domain mean something else.
  guard domain.hasSuffix("SCStreamErrorDomain") else {
    return "The system stopped capturing the window."
  }
  switch code {
  case CaptureErrorCode.userStopped:
    return nil
  case CaptureErrorCode.userDeclined:
    return "Baton is not allowed to record the screen. Allow it in System Settings, under Privacy "
      + "& Security, Screen & System Audio Recording, and share the window again."
  case CaptureErrorCode.noCaptureSource:
    return "The window is not there any more."
  case CaptureErrorCode.systemStopped:
    return "The system stopped sharing the window."
  default:
    return "The system stopped capturing the window."
  }
}
