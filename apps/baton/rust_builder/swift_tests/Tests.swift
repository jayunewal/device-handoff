// SPDX-License-Identifier: Apache-2.0

// Tests of BatonViewer.swift. They play the Rust side: the functions of the registered table are
// called with the C structs that the Rust side builds. A small runner stands in for XCTest, so
// that the tests need nothing but a Swift compiler.

import Foundation

// MARK: - The runner

var checks = 0
var failures = 0

func check(
  _ condition: @autoclosure () -> Bool, _ message: @autoclosure () -> String = "",
  file: StaticString = #file, line: UInt = #line
) {
  checks += 1
  if !condition() {
    failures += 1
    print("FAILED \(file):\(line) \(message())")
  }
}

/// Waits until `condition` holds, for at most five seconds.
func eventually(_ condition: () -> Bool) -> Bool {
  let end = Date().addingTimeInterval(5)
  while Date() < end {
    if condition() { return true }
    Thread.sleep(forTimeInterval: 0.005)
  }
  return condition()
}

/// Memory that stays valid until the arena goes away: what the Rust side points to in a call.
final class Arena {
  private var blocks: [UnsafeMutableRawBufferPointer] = []

  func bytes(_ values: [UInt8]) -> UnsafeRawBufferPointer {
    let block = UnsafeMutableRawBufferPointer.allocate(byteCount: max(values.count, 1), alignment: 8)
    blocks.append(block)
    block.copyBytes(from: values)
    return UnsafeRawBufferPointer(rebasing: block[0..<values.count])
  }

  func batonBytes(_ values: [UInt8]) -> BatonBytes {
    let view = bytes(values)
    return BatonBytes(ptr: view.baseAddress?.assumingMemoryBound(to: UInt8.self), len: view.count)
  }

  /// A struct that is only `size` bytes long, as one from an older Rust side would be, with its
  /// size field set. A read of more than that is a read past the end of the allocation, which the
  /// address sanitizer reports.
  func shortStruct<T>(_ type: T.Type, size: Int) -> UnsafeMutablePointer<T> {
    let block = UnsafeMutableRawBufferPointer.allocate(
      byteCount: size, alignment: MemoryLayout<T>.alignment)
    blocks.append(block)
    block.initializeMemory(as: UInt8.self, repeating: 0)
    block.storeBytes(of: UInt32(size), toByteOffset: 0, as: UInt32.self)
    return block.baseAddress!.assumingMemoryBound(to: T.self)
  }

  /// A frame of the interface, the way the Rust side builds it.
  func frame(
    flags: UInt32 = 0, number: UInt64 = 0, sample: [UInt8] = [0, 0, 0, 1, 0x65],
    sets: [[UInt8]] = [], sequenceSets: Int = 0
  ) -> UnsafeMutablePointer<BatonFrame> {
    let table = UnsafeMutablePointer<BatonBytes>.allocate(capacity: max(sets.count, 1))
    for (at, set) in sets.enumerated() {
      (table + at).initialize(to: batonBytes(set))
    }
    let frame = UnsafeMutablePointer<BatonFrame>.allocate(capacity: 1)
    frame.initialize(
      to: BatonFrame(
        size: UInt32(MemoryLayout<BatonFrame>.size), flags: flags, number: number, depends_on: 0,
        capture_time: 0, chain: 0, epoch: 0, sample: batonBytes(sample),
        sets: sets.isEmpty ? nil : UnsafePointer(table), set_count: sets.count,
        sequence_sets: sequenceSets))
    frames.append((frame, table))
    return frame
  }

  private var frames: [(UnsafeMutablePointer<BatonFrame>, UnsafeMutablePointer<BatonBytes>)] = []

  deinit {
    for block in blocks { block.deallocate() }
    for (frame, table) in frames {
      frame.deallocate()
      table.deallocate()
    }
  }
}

/// A frame for the code that is not about the C structs.
func input(
  _ arena: Arena, number: UInt64, key: Bool = false, sets: [[UInt8]] = [], sequenceSets: Int = 0,
  sample: [UInt8] = [0, 0, 0, 1, 0x41]
) -> FrameInput {
  FrameInput(
    flags: key ? UInt32(BATON_FRAME_KEY) : 0, number: number, dependsOn: 0, captureTime: 0,
    chain: 0, epoch: 0, sample: arena.bytes(sample), sets: sets.map { arena.bytes($0) },
    sequenceSets: sequenceSets)
}

// MARK: - A platform and a screen to play

/// What a fake screen saw of a frame.
struct SeenFrame: Equatable {
  var number: UInt64
  var isKey: Bool
  var sample: [UInt8]
  var sets: [[UInt8]]
  var sequenceSets: Int
}

final class FakeScreen: ViewerScreen {
  var frames: [SeenFrame] = []
  var accept = true
  var shown: [UInt64] = []
  var sources: [SourceInput] = []
  var report = DecoderReport(decodeMicros: 1234, thermal: UInt32(BATON_THERMAL_SERIOUS))
  var endedCalls = 0
  var releasedCalls = 0
  var onDeinit: (() -> Void)?

  func decode(_ frame: FrameInput) -> Bool {
    frames.append(
      SeenFrame(
        number: frame.number, isKey: frame.isKey, sample: Array(frame.sample),
        sets: frame.sets.map { Array($0) }, sequenceSets: frame.sequenceSets))
    return accept
  }

  func show(_ number: UInt64) { shown.append(number) }
  func source(_ message: SourceInput) { sources.append(message) }
  func state() -> DecoderReport { report }
  func ended() { endedCalls += 1 }
  func released() { releasedCalls += 1 }
  deinit { onDeinit?() }
}

final class FakePlatform: ViewerPlatform {
  var limits = ViewerLimits(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: UInt32(BATON_CODEC_H264))
  var windows: [(WindowInfo, UInt64)] = []
  var failure: ScreenFailure?
  var made: [FakeScreen] = []
  var nextId: Int64 = 7

  func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen {
    windows.append((window, link))
    if let failure { throw failure }
    let screen = FakeScreen()
    made.append(screen)
    return MadeScreen(screen: screen, id: nextId)
  }
}

/// A way to look at an object without holding it.
func weakly<T: AnyObject>(_ object: T) -> () -> T? {
  return { [weak object] in object }
}

/// Registers `platform` against the stub and gives the table that the Rust side would hold.
func registered(_ platform: ViewerPlatform) -> BatonViewerVTable? {
  stubLock.lock()
  registeredTable = nil
  registrationCode = 0
  stubLock.unlock()
  let code = registerViewer(platform)
  check(code == 0, "the registration was refused with \(code)")
  stubLock.lock()
  defer { stubLock.unlock() }
  return registeredTable
}

/// A window as the Rust side builds it.
func window(_ arena: Arena, width: UInt32 = 1280, height: UInt32 = 800, control: Bool = false)
  -> BatonWindow
{
  BatonWindow(
    size: UInt32(MemoryLayout<BatonWindow>.size), width: width, height: height,
    control: control ? 1 : 0, title: arena.batonBytes(Array("A secret".utf8)),
    app: arena.batonBytes(Array("Notes".utf8)))
}

/// Calls `open` the way the Rust side does, with room for a sentence of `room` bytes. Gives the
/// code, the screen's context and id, and the sentence.
func open(
  _ table: BatonViewerVTable, _ window: BatonWindow, link: UInt64 = 41, room: Int = 256
) -> (code: Int32, context: UnsafeMutableRawPointer?, id: Int64, sentence: String) {
  var window = window
  var screen = BatonScreen(
    size: UInt32(MemoryLayout<BatonScreen>.size), reserved: 0, context: nil, id: 0)
  let text = UnsafeMutablePointer<UInt8>.allocate(capacity: max(room, 1))
  defer { text.deallocate() }
  text.initialize(repeating: 0, count: max(room, 1))
  var message = BatonMessage(
    size: UInt32(MemoryLayout<BatonMessage>.size), cap: UInt32(room), len: 0, reserved: 0,
    text: text)
  let code = table.open?(table.context, &window, link, &screen, &message) ?? 99
  let sentence = String(decoding: UnsafeBufferPointer(start: text, count: Int(message.len)), as: UTF8.self)
  return (code, screen.context, screen.id, sentence)
}

// MARK: - The table

func testRegistrationGivesTheRustSideAWholeTable() {
  let platform = FakePlatform()
  guard let table = registered(platform) else {
    check(false, "no table was registered")
    return
  }
  check(table.version == UInt32(BATON_STREAM_ABI_VERSION), "version \(table.version)")
  check(table.size == UInt32(MemoryLayout<BatonViewerVTable>.size), "size \(table.size)")
  check(table.context != nil)
  check(table.limits != nil && table.open != nil && table.decode != nil && table.show != nil)
  check(table.source != nil && table.state != nil && table.ended != nil && table.release != nil)
  // A platform that plays no sound leaves out the last three entries, which are for the sound, and
  // the devices it is paired with are not told that it can.
  check(table.sound == nil && table.play == nil && table.stopped == nil)
  check(MemoryLayout<BatonViewerVTable>.size == 104, "size \(MemoryLayout<BatonViewerVTable>.size)")
  check(MemoryLayout<BatonViewerVTable>.offset(of: \.sound) == 80)
  check(MemoryLayout<BatonViewerVTable>.offset(of: \.play) == 88)
  check(MemoryLayout<BatonViewerVTable>.offset(of: \.stopped) == 96)
}

func testARegistrationThatFailsReturnsItsCodeAndLetsGoOfThePlatform() {
  weak var weakPlatform: FakePlatform?
  do {
    let platform = FakePlatform()
    weakPlatform = platform
    stubLock.lock()
    registrationCode = Int32(BATON_E_REGISTERED)
    stubLock.unlock()
    let code = registerViewer(platform)
    check(code == Int32(BATON_E_REGISTERED), "code \(code)")
  }
  stubLock.lock()
  registrationCode = 0
  stubLock.unlock()
  check(weakPlatform == nil, "the platform is still held after a failed registration")
}

func testARegistrationThatWorksKeepsThePlatformForTheLifeOfTheProcess() {
  weak var weakPlatform: FakePlatform?
  do {
    let platform = FakePlatform()
    weakPlatform = platform
    _ = registered(platform)
  }
  check(weakPlatform != nil, "the platform was let go of while the Rust side may call it")
}

func testLimitsAreWhatThePlatformSaysAndTheRoomIsChecked() {
  let platform = FakePlatform()
  platform.limits = ViewerLimits(maxWidth: 1920, maxHeight: 1080, maxFps: 30, codecs: 1)
  guard let table = registered(platform), let limits = table.limits else { return check(false) }
  var wanted = BatonLimits(
    size: UInt32(MemoryLayout<BatonLimits>.size), max_width: 0, max_height: 0, max_fps: 0, codecs: 0,
    reserved: 0)
  check(limits(table.context, &wanted) == 0)
  check(
    wanted.max_width == 1920 && wanted.max_height == 1080 && wanted.max_fps == 30
      && wanted.codecs == 1, "\(wanted)")
  check(wanted.reserved == 0, "reserved is written by nobody")

  var small = wanted
  small.size = 8
  check(limits(table.context, &small) == Int32(BATON_E_ARGUMENT), "a struct that is too short")
  check(limits(table.context, nil) == Int32(BATON_E_ARGUMENT), "no struct")
  check(limits(nil, &wanted) == Int32(BATON_E_ARGUMENT), "no context")
}

func testAnOpenMakesAScreenAndGivesItsPointerAndNumber() {
  let platform = FakePlatform()
  platform.nextId = 12
  guard let table = registered(platform) else { return check(false) }
  let arena = Arena()
  let result = open(table, window(arena, width: 1600, height: 900, control: true), link: 77)
  check(result.code == 0, "code \(result.code)")
  check(result.context != nil, "a screen without a pointer")
  check(result.id == 12, "id \(result.id)")
  check(platform.windows.count == 1)
  check(platform.windows.first?.0 == WindowInfo(width: 1600, height: 900, control: true))
  check(platform.windows.first?.1 == 77, "the link")
  check(platform.made.count == 1)
  table.release?(result.context)
}

func testOpensThatOverlapEachOtherAndTheCallsOfAScreenAreSafe() {
  // The Rust side opens the windows that two devices offer at the same moment on two threads, with
  // no lock around the calls, and calls the screens that are open meanwhile.
  final class SafePlatform: ViewerPlatform {
    let limits = ViewerLimits(maxWidth: 1, maxHeight: 1, maxFps: 1, codecs: 1)
    private let lock = NSLock()
    private var count: Int64 = 0

    func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen {
      lock.lock()
      count += 1
      let id = count
      lock.unlock()
      return MadeScreen(screen: FakeScreen(), id: id)
    }
  }
  guard let table = registered(SafePlatform()) else { return check(false) }
  let arena = Arena()
  let rooms = 8
  let each = 25
  let windows = (0..<rooms).map { _ in window(arena) }
  /// What the threads hand over, behind one lock.
  final class Collected: @unchecked Sendable {
    private let lock = NSLock()
    private var codes: [Int32] = []
    private var contexts: [UnsafeMutableRawPointer] = []
    private var ids = Set<Int64>()

    func opened(code: Int32, id: Int64) {
      lock.lock()
      codes.append(code)
      ids.insert(id)
      lock.unlock()
    }

    func kept(_ more: [UnsafeMutableRawPointer]) {
      lock.lock()
      contexts += more
      lock.unlock()
    }

    func all() -> (codes: [Int32], contexts: [UnsafeMutableRawPointer], ids: Set<Int64>) {
      lock.lock()
      defer { lock.unlock() }
      return (codes, contexts, ids)
    }
  }
  let done = DispatchGroup()
  let collected = Collected()
  for (number, window) in windows.enumerated() {
    done.enter()
    Thread {
      var mine: [UnsafeMutableRawPointer] = []
      for _ in 0..<each {
        let result = open(table, window, link: UInt64(number + 1))
        collected.opened(code: result.code, id: result.id)
        if let context = result.context { mine.append(context) }
      }
      // The calls of a screen that is open, while the others are being opened.
      for context in mine {
        table.show?(context, 0)
        table.ended?(context)
      }
      collected.kept(mine)
      done.leave()
    }.start()
  }
  done.wait()
  let (codes, contexts, ids) = collected.all()
  check(codes.count == rooms * each && codes.allSatisfy { $0 == 0 }, "\(codes)")
  check(Set(contexts).count == rooms * each, "\(Set(contexts).count) different screens")
  check(ids.count == rooms * each, "\(ids.count) different numbers")
  for context in contexts { table.release?(context) }
}

func testAnOpenThatFailsWritesTheSentenceAndMakesNoPointer() {
  let platform = FakePlatform()
  platform.failure = ScreenFailure(sentence: "The last window is still closing.")
  guard let table = registered(platform) else { return check(false) }
  let result = open(table, window(Arena()))
  check(result.code == Int32(BATON_E_PLATFORM), "code \(result.code)")
  check(result.context == nil, "a pointer for a screen that was not made")
  check(result.sentence == "The last window is still closing.", result.sentence)
}

func testAnOpenThatFailsInAnotherWayHasASentenceOfItsOwn() {
  struct Odd: Error {}
  final class OddPlatform: ViewerPlatform {
    let limits = ViewerLimits(maxWidth: 1, maxHeight: 1, maxFps: 1, codecs: 1)
    func makeScreen(for window: WindowInfo, link: UInt64) throws -> MadeScreen { throw Odd() }
  }
  guard let table = registered(OddPlatform()) else { return check(false) }
  let result = open(table, window(Arena()))
  check(result.code == Int32(BATON_E_PLATFORM), "code \(result.code)")
  check(result.context == nil)
  check(result.sentence == "The window could not be shown.", result.sentence)
}

func testASentenceThatDoesNotFitIsCutBetweenTwoCharacters() {
  let platform = FakePlatform()
  platform.failure = ScreenFailure(sentence: "Ça ne marche pas: étonnant")
  guard let table = registered(platform) else { return check(false) }
  for room in 0...30 {
    let result = open(table, window(Arena()), room: room)
    check(result.code == Int32(BATON_E_PLATFORM))
    check(result.sentence.utf8.count <= room, "room \(room): \(result.sentence)")
    check("Ça ne marche pas: étonnant".hasPrefix(result.sentence), "room \(room): \(result.sentence)")
    check(!result.sentence.contains("\u{FFFD}"), "room \(room): a character was cut in two")
    // As much as fits: the character that follows the cut would not have.
    let rest = "Ça ne marche pas: étonnant".unicodeScalars.dropFirst(
      result.sentence.unicodeScalars.count)
    if let next = rest.first {
      check(
        result.sentence.utf8.count + String(next).utf8.count > room,
        "room \(room): \(result.sentence) leaves room for \(next)")
    }
  }
  let whole = open(table, window(Arena()), room: 256)
  check(whole.sentence == "Ça ne marche pas: étonnant", whole.sentence)
}

func testAnOpenWithStructsThatAreTooShortOrMissingIsRefused() {
  let platform = FakePlatform()
  guard let table = registered(platform), let open = table.open else { return check(false) }
  let arena = Arena()
  var theWindow = window(arena)
  var screen = BatonScreen(
    size: UInt32(MemoryLayout<BatonScreen>.size), reserved: 0, context: nil, id: 0)
  let text = UnsafeMutablePointer<UInt8>.allocate(capacity: 16)
  defer { text.deallocate() }
  var message = BatonMessage(
    size: UInt32(MemoryLayout<BatonMessage>.size), cap: 16, len: 0, reserved: 0, text: text)
  let refused = Int32(BATON_E_ARGUMENT)

  check(open(table.context, &theWindow, 1, &screen, &message) == 0)
  table.release?(screen.context)
  screen.context = nil

  var shortWindow = theWindow
  shortWindow.size = 12
  check(open(table.context, &shortWindow, 1, &screen, &message) == refused)
  var shortScreen = screen
  shortScreen.size = 8
  check(open(table.context, &theWindow, 1, &shortScreen, &message) == refused)
  var shortMessage = message
  shortMessage.size = 8
  check(open(table.context, &theWindow, 1, &screen, &shortMessage) == refused)
  check(open(table.context, nil, 1, &screen, &message) == refused)
  check(open(table.context, &theWindow, 1, nil, &message) == refused)
  check(open(table.context, &theWindow, 1, &screen, nil) == refused)
  check(open(nil, &theWindow, 1, &screen, &message) == refused)
  check(platform.windows.count == 1, "a refused open reached the platform")
}

func testAMessageWithNoRoomWritesNothing() {
  let platform = FakePlatform()
  platform.failure = ScreenFailure(sentence: "No.")
  guard let table = registered(platform), let open = table.open else { return check(false) }
  var theWindow = window(Arena())
  var screen = BatonScreen(
    size: UInt32(MemoryLayout<BatonScreen>.size), reserved: 0, context: nil, id: 0)
  var message = BatonMessage(
    size: UInt32(MemoryLayout<BatonMessage>.size), cap: 16, len: 99, reserved: 0, text: nil)
  check(open(table.context, &theWindow, 1, &screen, &message) == Int32(BATON_E_PLATFORM))
  check(message.len == 0, "len \(message.len)")
}

// MARK: - What a screen is called with

/// Opens a screen on a fresh platform.
func openedScreen() -> (BatonViewerVTable, FakeScreen, UnsafeMutableRawPointer)? {
  let platform = FakePlatform()
  guard let table = registered(platform) else { return nil }
  let result = open(table, window(Arena()))
  guard result.code == 0, let context = result.context, let screen = platform.made.first else {
    return nil
  }
  return (table, screen, context)
}

func testAFrameReachesTheScreenAsTheRustSideBuiltIt() {
  guard let (table, screen, context) = openedScreen(), let decode = table.decode else {
    return check(false)
  }
  let arena = Arena()
  let key = arena.frame(
    flags: UInt32(BATON_FRAME_KEY), number: 9, sample: [0, 0, 0, 2, 0x65, 0x88],
    sets: [[0x67, 1, 2], [0x67, 3], [0x68, 4]], sequenceSets: 2)
  check(decode(context, key) == 0)
  let delta = arena.frame(number: 10, sample: [0, 0, 0, 1, 0x41])
  check(decode(context, delta) == 0)
  check(
    screen.frames == [
      SeenFrame(
        number: 9, isKey: true, sample: [0, 0, 0, 2, 0x65, 0x88],
        sets: [[0x67, 1, 2], [0x67, 3], [0x68, 4]], sequenceSets: 2),
      SeenFrame(number: 10, isKey: false, sample: [0, 0, 0, 1, 0x41], sets: [], sequenceSets: 0),
    ], "\(screen.frames)")

  screen.accept = false
  check(decode(context, delta) == Int32(BATON_E_PLATFORM), "a frame that the screen cannot decode")
  table.release?(context)
}

func testAFrameThatIsNotWellFormedNeverReachesTheScreen() {
  guard let (table, screen, context) = openedScreen(), let decode = table.decode else {
    return check(false)
  }
  let arena = Arena()
  let refused = Int32(BATON_E_ARGUMENT)

  let noSample = arena.frame(sample: [1, 2, 3])
  noSample.pointee.sample.ptr = nil
  check(decode(context, noSample) == refused, "a sample with a length and no pointer")

  let negativeSample = arena.frame(sample: [1, 2, 3])
  negativeSample.pointee.sample.len = -1
  check(decode(context, negativeSample) == refused, "a sample with a length below zero")

  let negativeSet = arena.frame(sets: [[0x67], [0x68]], sequenceSets: 1)
  UnsafeMutablePointer(mutating: negativeSet.pointee.sets)?.pointee.len = -4
  check(decode(context, negativeSet) == refused, "a set with a length below zero")

  let emptySample = arena.frame(sample: [])
  check(decode(context, emptySample) == 0, "an empty sample is a sample with no bytes")

  let noSets = arena.frame(sets: [[0x67], [0x68]], sequenceSets: 1)
  noSets.pointee.sets = nil
  check(decode(context, noSets) == refused, "sets with a count and no table")

  let aSetWithNoPointer = arena.frame(sets: [[0x67], [0x68]], sequenceSets: 1)
  UnsafeMutablePointer(mutating: aSetWithNoPointer.pointee.sets)?.pointee.ptr = nil
  check(decode(context, aSetWithNoPointer) == refused, "a set with a length and no pointer")

  let tooManySequenceSets = arena.frame(sets: [[0x67], [0x68]], sequenceSets: 3)
  check(decode(context, tooManySequenceSets) == refused, "more sequence sets than sets")

  let negative = arena.frame(sets: [[0x67]], sequenceSets: 1)
  negative.pointee.sequence_sets = -1
  check(decode(context, negative) == refused, "a count of sequence sets below zero")

  let fewer = arena.frame(sets: [[0x67]], sequenceSets: 0)
  fewer.pointee.set_count = -1
  check(decode(context, fewer) == refused, "a count of sets below zero")

  let most = arena.frame(
    sets: Array(repeating: [0x67], count: FrameInput.mostSets), sequenceSets: 1)
  check(decode(context, most) == 0, "as many sets as a frame may have")
  let tooMany = arena.frame(
    sets: Array(repeating: [0x67], count: FrameInput.mostSets + 1), sequenceSets: 1)
  check(decode(context, tooMany) == refused, "more sets than a frame may have")

  let short = arena.shortStruct(BatonFrame.self, size: 40)
  short.pointee.number = 9
  check(decode(context, short) == refused, "a struct that is shorter than this version knows")

  check(decode(context, nil) == refused, "no struct")
  check(decode(nil, arena.frame()) == refused, "no screen")

  check(screen.frames.count == 2, "the screen saw \(screen.frames.count) frames")
  table.release?(context)
}

func testWhatTheSourceSaysReachesTheScreen() {
  guard let (table, screen, context) = openedScreen(), let source = table.source else {
    return check(false)
  }
  func message(_ kind: UInt32, reason: UInt32 = 0, count: UInt32 = 0) -> BatonSourceMessage {
    BatonSourceMessage(
      size: UInt32(MemoryLayout<BatonSourceMessage>.size), kind: kind, epoch: 3,
      codec: UInt32(BATON_CODEC_H264), width: 1280, height: 720, fps: 30, reason: reason,
      count: count, reserved: 0)
  }
  var config = message(UInt32(BATON_SOURCE_CONFIG))
  let pause = message(UInt32(BATON_SOURCE_PAUSE), reason: UInt32(BATON_PAUSE_LOCKED))
  let resume = message(UInt32(BATON_SOURCE_RESUME))
  let notice = message(
    UInt32(BATON_SOURCE_NOTICE), reason: UInt32(BATON_DROPPED_TOO_FAST), count: 5)
  let unknown = message(99)
  for var each in [config, pause, resume, notice, unknown] {
    source(context, &each)
  }
  // Only 12 bytes are there, a kind and no more, which is a kind that is known.
  let arena = Arena()
  let short = arena.shortStruct(BatonSourceMessage.self, size: 12)
  short.pointee.kind = UInt32(BATON_SOURCE_RESUME)
  source(context, short)
  source(context, nil)
  source(nil, &config)
  check(
    screen.sources == [
      .config(epoch: 3, width: 1280, height: 720, fps: 30),
      .pause(reason: UInt32(BATON_PAUSE_LOCKED)),
      .resume,
      .notice(count: 5, reason: UInt32(BATON_DROPPED_TOO_FAST)),
    ], "\(screen.sources)")
  table.release?(context)
}

func testShowStateAndEndedReachTheScreen() {
  guard let (table, screen, context) = openedScreen(), let show = table.show,
    let state = table.state, let ended = table.ended
  else { return check(false) }
  show(context, 5)
  show(context, 6)
  show(nil, 7)
  check(screen.shown == [5, 6], "\(screen.shown)")

  var report = BatonDecoderState(
    size: UInt32(MemoryLayout<BatonDecoderState>.size), decode_us: 0, thermal: 0, reserved: 0)
  state(context, &report)
  check(report.decode_us == 1234 && report.thermal == UInt32(BATON_THERMAL_SERIOUS), "\(report)")
  var short = BatonDecoderState(size: 8, decode_us: 5, thermal: 5, reserved: 0)
  state(context, &short)
  check(short.decode_us == 5 && short.thermal == 5, "a struct that is too short is left alone")
  state(context, nil)
  state(nil, &report)

  ended(context)
  ended(nil)
  check(screen.endedCalls == 1)
  table.release?(context)
}

func testReleaseTellsTheScreenOnceAndLetsGoOfIt() {
  let platform = FakePlatform()
  guard let table = registered(platform), let release = table.release else { return check(false) }
  var gone = false
  let result = open(table, window(Arena()))
  guard let context = result.context else { return check(false) }
  var screen: FakeScreen! = platform.made.removeFirst()
  screen.onDeinit = { gone = true }
  let probe = weakly(screen)
  check(screen.releasedCalls == 0)
  release(context)
  check(screen.releasedCalls == 1, "released \(screen.releasedCalls) times")
  check(probe() != nil, "the test itself holds the screen")
  screen = nil
  check(probe() == nil && gone, "the screen is still held after the release")
  release(nil)
}

// MARK: - Which picture is shown

func testAPictureIsShownWhenBothTheDecoderAndTheRustSideHaveSaidSo() {
  let table = PictureTable<String>()
  check(table.show(1) == nil, "nothing decoded yet")
  check(table.decoded(1, "one") == "one", "decoded after it was asked for")
  check(table.decoded(2, "two") == nil, "decoded before it was asked for")
  check(table.show(2) == "two")
  check(table.show(2) == nil, "a picture is shown once")
}

func testAnOlderPictureIsNeverShownAfterANewerOne() {
  let table = PictureTable<String>()
  _ = table.decoded(1, "one")
  _ = table.decoded(2, "two")
  _ = table.decoded(3, "three")
  check(table.show(3) == "three")
  check(table.show(2) == nil, "asked for after a newer one was shown")
  check(table.decoded(2, "two again") == nil, "decoded after a newer one was shown")
  check(table.decoded(3, "three again") == nil, "decoded again after it was shown")
  check(table.held == 0, "held \(table.held): nothing older than the picture shown is kept")
  check(table.show(1) == nil)
  check(table.decoded(4, "four") == nil)
  check(table.show(4) == "four")
}

func testAFrameThatNeverCameOutDoesNotHoldBackTheNext() {
  let table = PictureTable<String>()
  check(table.show(5) == nil, "asked for and never decoded")
  check(table.show(6) == nil)
  check(table.decoded(5, "five") == nil, "decoded after something newer was asked for")
  check(table.decoded(6, "six") == "six")
}

func testPicturesThatNobodyAsksForAreKeptUpToTheCapacity() {
  let table = PictureTable<String>(capacity: 3)
  for number in UInt64(1)...5 {
    _ = table.decoded(number, "p\(number)")
  }
  check(table.held == 3, "held \(table.held)")
  check(table.show(1) == nil && table.show(2) == nil, "the oldest are dropped")
  check(table.show(3) == "p3", "the newest three are kept")
}

func testWhatIsOlderThanThePictureShownIsLetGoOf() {
  let table = PictureTable<String>()
  for number in UInt64(1)...4 {
    _ = table.decoded(number, "p\(number)")
  }
  check(table.held == 4)
  check(table.show(3) == "p3")
  check(table.held == 1, "held \(table.held): only the picture after the one shown is kept")
}

func testClearForgetsEverythingIncludingWhatWasShown() {
  let table = PictureTable<String>()
  _ = table.decoded(1, "one")
  check(table.show(1) == "one")
  _ = table.decoded(2, "two")
  check(table.held == 1)
  table.clear()
  check(table.held == 0, "held \(table.held) after a clear")
  check(table.decoded(1, "one again") == nil)
  check(table.show(1) == "one again", "a new session counts from nothing")
}

func testTheTimeAFrameTakesIsSmoothedAndNeverOutOfRange() {
  var time = SmoothedMicros()
  check(time.rounded == 0)
  time.add(8000)
  check(time.rounded == 8000)
  time.add(16000)
  check(time.rounded == 9000, "\(time.rounded)")
  time.add(-5)
  time.add(.nan)
  time.add(.infinity)
  check(time.rounded == 9000, "numbers that are not times are left out")
  time.add(1e300)
  check(time.rounded <= UInt32.max, "no trap")
}

// MARK: - A screen

final class FakeDecoder: PictureDecoder {
  typealias Picture = String

  var prepared: [ParameterSets] = []
  var prepareWorks = true
  /// Called while the decoder prepares, as the decoder's own threads may call back meanwhile.
  var onPrepare: (() -> Void)?
  var takes = true
  var samples: [[UInt8]] = []
  private let lock = NSLock()
  private var waiting: [UInt64: (Result<String, DecodeFailure>) -> Void] = [:]
  var stopped = 0

  func prepare(_ sets: ParameterSets) -> Bool {
    prepared.append(sets)
    onPrepare?()
    return prepareWorks
  }

  func decode(
    _ sample: UnsafeRawBufferPointer, number: UInt64,
    then finish: @escaping (Result<String, DecodeFailure>) -> Void
  ) -> Bool {
    samples.append(Array(sample))
    if !takes { return false }
    lock.lock()
    waiting[number] = finish
    lock.unlock()
    return true
  }

  func stop() { stopped += 1 }

  /// The decoder is done with a frame.
  func finish(_ number: UInt64, _ result: Result<String, DecodeFailure>? = nil) {
    lock.lock()
    let call = waiting.removeValue(forKey: number)
    lock.unlock()
    call?(result ?? .success("picture \(number)"))
  }
}

/// A screen over a fake decoder, and what it did.
final class Rig {
  let decoder = FakeDecoder()
  var thermal = UInt32(BATON_THERMAL_FAIR)
  // What the screen calls on threads of its own is kept under the lock.
  private let lock = NSLock()
  private var presented: [String?] = []
  private var failureCount = 0
  private var releaseCount = 0
  lazy var screen = WindowScreen(
    decoder: decoder,
    present: { [self] picture in
      lock.lock()
      presented.append(picture)
      lock.unlock()
    },
    failed: { [self] in
      lock.lock()
      failureCount += 1
      lock.unlock()
    },
    thermal: { [self] in thermal },
    onRelease: { [self] in
      lock.lock()
      releaseCount += 1
      lock.unlock()
    })

  let arena = Arena()
  let sps: [UInt8] = [0x67, 0x4d, 0x00, 0x1e]
  let pps: [UInt8] = [0x68, 0xee, 0x3c]

  func key(_ number: UInt64) -> FrameInput {
    input(arena, number: number, key: true, sets: [sps, pps], sequenceSets: 1)
  }

  func delta(_ number: UInt64) -> FrameInput {
    input(arena, number: number)
  }

  var shown: [String?] {
    lock.lock()
    defer { lock.unlock() }
    return presented
  }

  var failures: Int {
    lock.lock()
    defer { lock.unlock() }
    return failureCount
  }

  var released: Int {
    lock.lock()
    defer { lock.unlock() }
    return releaseCount
  }
}

func testAKeyFramePreparesTheDecoderAndLaterFramesDoNot() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  check(rig.screen.decode(rig.delta(1)))
  check(rig.screen.decode(rig.key(2)), "a key frame with the same sets")
  check(rig.decoder.prepared.count == 1, "prepared \(rig.decoder.prepared.count) times")
  check(rig.decoder.prepared.first?.sequence == [Data(rig.sps)])
  check(rig.decoder.prepared.first?.picture == [Data(rig.pps)])
  check(rig.decoder.samples.count == 3)
}

func testNewParameterSetsPrepareTheDecoderAgain() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  let other = input(rig.arena, number: 1, key: true, sets: [[0x67, 9, 9], rig.pps], sequenceSets: 1)
  check(rig.screen.decode(other))
  check(rig.decoder.prepared.count == 2, "prepared \(rig.decoder.prepared.count) times")
  let late = input(rig.arena, number: 2, sets: [[0x67, 8, 8], rig.pps], sequenceSets: 1)
  check(rig.screen.decode(late), "a frame that is not a key frame may bring new sets")
  check(rig.decoder.prepared.count == 3)
}

func testAFrameIsNotDecodedBeforeAKeyFrameHasCome() {
  let rig = Rig()
  check(!rig.screen.decode(rig.delta(0)), "no key frame yet")
  check(rig.decoder.samples.isEmpty, "the decoder was given a frame it cannot use")
  check(rig.screen.decode(rig.key(1)))
  check(rig.screen.decode(rig.delta(2)))
}

func testAKeyFrameThatCameWithNoParameterSetsIsRefused() {
  let rig = Rig()
  check(!rig.screen.decode(input(rig.arena, number: 0, key: true)), "no sets, none before")
  check(rig.decoder.samples.isEmpty && rig.decoder.prepared.isEmpty)
  check(rig.screen.decode(rig.key(1)))
  check(rig.screen.decode(input(rig.arena, number: 2, key: true)), "a key frame may rely on the sets before")
}

func testParameterSetsThatThePlatformCannotUseRefuseTheFrame() {
  let rig = Rig()
  rig.decoder.prepareWorks = false
  check(!rig.screen.decode(rig.key(0)))
  check(rig.decoder.samples.isEmpty)
  rig.decoder.prepareWorks = true
  check(!rig.screen.decode(rig.delta(1)), "a frame after a refused key frame")
  check(rig.screen.decode(rig.key(2)), "the next key frame prepares the decoder again")
  check(rig.decoder.prepared.count == 2)
}

func testAfterParametersThatWereRefusedNothingIsDecodedUntilAKeyFrameComes() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  check(rig.decoder.prepared.count == 1)
  rig.decoder.prepareWorks = false
  let other = input(rig.arena, number: 1, key: true, sets: [[0x67, 7], rig.pps], sequenceSets: 1)
  check(!rig.screen.decode(other))
  rig.decoder.prepareWorks = true
  let before = rig.decoder.samples.count
  let late = input(rig.arena, number: 2, sets: [[0x67, 8], rig.pps], sequenceSets: 1)
  check(!rig.screen.decode(late), "a frame that is not a key frame, with sets that would work")
  check(!rig.screen.decode(rig.delta(3)), "a frame with no sets")
  check(rig.decoder.samples.count == before, "the decoder was given a frame after the refusal")

  check(rig.screen.decode(rig.key(4)), "a key frame with the sets from before the refusal")
  check(
    rig.decoder.prepared.count == 3,
    "prepared \(rig.decoder.prepared.count) times: the decoder is told again after a refusal")
}

func testAFrameThatTheDecoderDoesNotTakeMakesTheScreenWaitForAKeyFrame() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  rig.decoder.takes = false
  check(!rig.screen.decode(rig.delta(1)))
  rig.decoder.takes = true
  let before = rig.decoder.samples.count
  check(!rig.screen.decode(rig.delta(2)), "nothing is decoded between the failure and a key frame")
  check(rig.decoder.samples.count == before)
  check(rig.screen.decode(rig.key(3)))
  check(rig.screen.decode(rig.delta(4)))
}

func testAPictureIsShownInEitherOrderOfTheDecoderAndTheRustSide() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  rig.screen.show(0)
  check(rig.shown.isEmpty, "shown before it was decoded")
  rig.decoder.finish(0)
  check(rig.shown == ["picture 0"], "\(rig.shown)")

  check(rig.screen.decode(rig.delta(1)))
  rig.decoder.finish(1)
  check(rig.shown.count == 1, "decoded but not asked for")
  rig.screen.show(1)
  check(rig.shown == ["picture 0", "picture 1"], "\(rig.shown)")
}

func testALateFailureIsReportedOnceAndAKeyFrameStartsAgain() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  check(rig.screen.decode(rig.delta(1)))
  check(rig.screen.decode(rig.delta(2)))
  rig.decoder.finish(1, .failure(.status(-12909)))
  rig.decoder.finish(2, .failure(.dropped))
  check(eventually { rig.failures >= 1 }, "the failure was not reported")
  Thread.sleep(forTimeInterval: 0.05)
  check(rig.failures == 1, "reported \(rig.failures) times")
  check(!rig.screen.decode(rig.delta(3)), "frames wait for a key frame after a failure")

  check(rig.screen.decode(rig.key(4)))
  check(rig.screen.decode(rig.delta(5)))
  rig.decoder.finish(5, .failure(.dropped))
  check(eventually { rig.failures == 2 }, "a second run of failures is reported again")
}

func testAFailureIsNotReportedFromTheThreadThatIsInsideTheCall() {
  // A decoder may finish a frame before `decode` returns, on the same thread. The Rust side
  // refuses a call into it from there, so the report has to come from another thread.
  final class QuickDecoder: PictureDecoder {
    typealias Picture = String
    func prepare(_ sets: ParameterSets) -> Bool { true }
    func decode(
      _ sample: UnsafeRawBufferPointer, number: UInt64,
      then finish: @escaping (Result<String, DecodeFailure>) -> Void
    ) -> Bool {
      finish(.failure(.dropped))
      return true
    }
    func stop() {}
  }
  let caller = Thread.current
  var reportedOn: [Thread] = []
  let lock = NSLock()
  let arena = Arena()
  let screen = WindowScreen(
    decoder: QuickDecoder(), present: { _ in },
    failed: {
      lock.lock()
      reportedOn.append(Thread.current)
      lock.unlock()
    }, thermal: { 0 }, onRelease: {})
  check(
    screen.decode(input(arena, number: 0, key: true, sets: [[0x67], [0x68]], sequenceSets: 1)))
  check(
    eventually {
      lock.lock()
      defer { lock.unlock() }
      return !reportedOn.isEmpty
    }, "no report")
  lock.lock()
  check(reportedOn.count == 1 && reportedOn.first !== caller, "reported on the calling thread")
  lock.unlock()
}

func testNothingIsShownOrReportedAfterTheSessionEnded() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  check(rig.screen.decode(rig.delta(1)))
  rig.screen.ended()
  check(rig.decoder.stopped == 1, "the decoder was not stopped")
  check(rig.shown == [nil], "the picture on the screen is taken away: \(rig.shown)")

  rig.decoder.finish(0)
  rig.screen.show(0)
  rig.decoder.finish(1, .failure(.dropped))
  Thread.sleep(forTimeInterval: 0.05)
  check(rig.shown == [nil], "\(rig.shown)")
  check(rig.failures == 0, "a failure after the end")
  check(!rig.screen.decode(rig.key(2)), "a frame after the end")

  rig.screen.ended()
  check(rig.decoder.stopped == 1, "ended twice stops the decoder twice")
}

func testAPictureOnItsWayToTheScreenIsThereBeforeTheEndTakesItAway() {
  // The decoder answers on a thread of its own, and the end of the session comes from another. A
  // picture that was chosen must not reach the screen after the end took the picture away, because
  // by then the texture may belong to another window.
  let entered = DispatchSemaphore(value: 0)
  let leave = DispatchSemaphore(value: 0)
  let lock = NSLock()
  var events: [String] = []
  func note(_ event: String) {
    lock.lock()
    events.append(event)
    lock.unlock()
  }
  func seen() -> [String] {
    lock.lock()
    defer { lock.unlock() }
    return events
  }
  let decoder = FakeDecoder()
  let screen = WindowScreen(
    decoder: decoder,
    present: { picture in
      guard picture != nil else { return note("nothing") }
      note("picture begins")
      entered.signal()
      _ = leave.wait(timeout: .now() + 5)
      note("picture ends")
    }, failed: {}, thermal: { 0 }, onRelease: {})
  let arena = Arena()
  check(screen.decode(input(arena, number: 0, key: true, sets: [[0x67], [0x68]], sequenceSets: 1)))
  screen.show(0)
  Thread { decoder.finish(0) }.start()
  check(entered.wait(timeout: .now() + 5) == .success, "the picture never reached the screen")

  Thread { screen.ended() }.start()
  Thread.sleep(forTimeInterval: 0.1)
  check(seen() == ["picture begins"], "the end came while the picture was on its way: \(seen())")
  leave.signal()
  check(
    eventually { seen() == ["picture begins", "picture ends", "nothing"] },
    "\(seen())")
}

func testPicturesReachTheScreenOneAtATimeInTheOrderTheyWereChosen() {
  // The Rust side asks for a picture on its thread and the decoder gives one on its own. Two
  // pictures that are chosen at about the same time must not be shown by two threads at once,
  // since the older could then be the last to reach the texture.
  let lock = NSLock()
  var inside = 0
  var mostInside = 0
  var order: [String] = []
  let began = DispatchSemaphore(value: 0)
  let decoder = FakeDecoder()
  let screen = WindowScreen(
    decoder: decoder,
    present: { picture in
      lock.lock()
      inside += 1
      mostInside = max(mostInside, inside)
      order.append(picture ?? "nothing")
      lock.unlock()
      began.signal()
      Thread.sleep(forTimeInterval: 0.1)
      lock.lock()
      inside -= 1
      lock.unlock()
    }, failed: {}, thermal: { 0 }, onRelease: {})
  let arena = Arena()
  check(screen.decode(input(arena, number: 0, key: true, sets: [[0x67], [0x68]], sequenceSets: 1)))
  check(screen.decode(input(arena, number: 1)))
  check(screen.decode(input(arena, number: 2)))
  decoder.finish(2)
  screen.show(1)
  Thread { decoder.finish(1) }.start()
  check(began.wait(timeout: .now() + 5) == .success, "the first picture never reached the screen")
  Thread { screen.show(2) }.start()
  check(
    eventually {
      lock.lock()
      defer { lock.unlock() }
      return order.count == 2 && inside == 0
    }, "the second picture never reached the screen")
  lock.lock()
  check(mostInside == 1, "\(mostInside) pictures were being shown at once")
  check(order == ["picture 1", "picture 2"], "\(order)")
  lock.unlock()
}

func testAPictureThatComesAfterTheEndIsNotKept() {
  // A decoder may answer after the session ended. The picture is a buffer of the size of a window
  // and nobody will ask for it, so the screen must not hold on to it.
  final class Marker {}
  final class OneDecoder: PictureDecoder {
    typealias Picture = Marker
    private let lock = NSLock()
    private var answer: ((Result<Marker, DecodeFailure>) -> Void)?

    func prepare(_ sets: ParameterSets) -> Bool { true }

    func decode(
      _ sample: UnsafeRawBufferPointer, number: UInt64,
      then finish: @escaping (Result<Marker, DecodeFailure>) -> Void
    ) -> Bool {
      lock.lock()
      answer = finish
      lock.unlock()
      return true
    }

    func stop() {}

    func answerWith(_ marker: Marker) {
      lock.lock()
      let call = answer
      lock.unlock()
      call?(.success(marker))
    }
  }
  let decoder = OneDecoder()
  let screen = WindowScreen(
    decoder: decoder, present: { _ in }, failed: {}, thermal: { 0 }, onRelease: {})
  let arena = Arena()
  check(screen.decode(input(arena, number: 0, key: true, sets: [[0x67], [0x68]], sequenceSets: 1)))
  screen.ended()
  weak var late: Marker?
  do {
    let marker = Marker()
    late = marker
    decoder.answerWith(marker)
  }
  check(late == nil, "a picture that came after the end is still held")
}

func testReleasingAScreenEndsItAndLetsGoOfWhatItHeldOnce() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  rig.screen.released()
  check(rig.released == 1)
  check(rig.decoder.stopped == 1, "a screen that was not ended is ended by its release")
  check(rig.shown == [nil])
}

func testTheStateTellsHowLongFramesTakeAndHowHotTheDeviceIs() {
  let rig = Rig()
  check(rig.screen.state() == DecoderReport(decodeMicros: 0, thermal: UInt32(BATON_THERMAL_FAIR)))
  check(rig.screen.decode(rig.key(0)))
  Thread.sleep(forTimeInterval: 0.01)
  rig.decoder.finish(0)
  let report = rig.screen.state()
  check(report.decodeMicros >= 10_000, "\(report.decodeMicros) microseconds")
  check(report.decodeMicros < 5_000_000, "\(report.decodeMicros) microseconds")
  rig.thermal = UInt32(BATON_THERMAL_CRITICAL)
  check(rig.screen.state().thermal == UInt32(BATON_THERMAL_CRITICAL))
}

func testAFrameIsGivenTheSampleAndNothingOfTheSets() {
  let rig = Rig()
  let frame = input(
    rig.arena, number: 0, key: true, sets: [rig.sps, rig.pps], sequenceSets: 1,
    sample: [0, 0, 0, 3, 0x65, 1, 2])
  check(rig.screen.decode(frame))
  check(rig.decoder.samples == [[0, 0, 0, 3, 0x65, 1, 2]], "\(rig.decoder.samples)")
}

func testParameterSetsAreCopiesSplitAtTheSequenceSets() {
  let arena = Arena()
  let frame = input(
    arena, number: 0, key: true, sets: [[0x67, 1], [0x67, 2], [0x68, 3]], sequenceSets: 2)
  let sets = frame.parameterSets
  check(sets?.sequence == [Data([0x67, 1]), Data([0x67, 2])], "\(String(describing: sets))")
  check(sets?.picture == [Data([0x68, 3])])
  check(sets?.all.count == 3)
  check(input(arena, number: 1).parameterSets == nil, "a frame with no sets has none")
}

func testAFailureThatComesWhileAFrameIsPreparedForIsNotUndone() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  // The next frame is not a key frame, and comes with parameter sets of another size. While the
  // decoder is made ready for them, the first frame fails on a thread of its own.
  rig.decoder.onPrepare = { rig.decoder.finish(0, .failure(.dropped)) }
  let sets = [[0x67, 0x4d, 0x00, 0x28], rig.pps]
  let dependent = input(rig.arena, number: 1, sets: sets, sequenceSets: 1)
  check(!rig.screen.decode(dependent), "a frame after a failure is not decoded until a key frame")
  check(rig.decoder.samples.count == 1, "\(rig.decoder.samples.count) frames reached the decoder")
  check(eventually { rig.failures == 1 }, "the failure was reported once: \(rig.failures)")
  rig.decoder.onPrepare = nil
  check(!rig.screen.decode(rig.delta(2)), "and neither is the one after it")
  check(rig.screen.decode(rig.key(3)), "a key frame starts again")
  check(rig.decoder.samples.count == 2)
  check(
    rig.decoder.prepared.count == 3,
    "prepared \(rig.decoder.prepared.count) times: the key frame brings its own sets back")
}

func testAScreenThatEndsWhileAFrameIsPreparedForDecodesNothing() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  rig.decoder.onPrepare = { rig.screen.ended() }
  let sets = [[0x67, 0x4d, 0x00, 0x28], rig.pps]
  let late = input(rig.arena, number: 1, key: true, sets: sets, sequenceSets: 1)
  check(!rig.screen.decode(late), "a frame is not decoded by a screen that ended")
  check(rig.decoder.samples.count == 1, "\(rig.decoder.samples.count) frames reached the decoder")
}

func testAPictureThatWasHeldIsNotShownAfterTheEnd() {
  let rig = Rig()
  check(rig.screen.decode(rig.key(0)))
  rig.decoder.finish(0)
  check(rig.shown.isEmpty, "nobody has asked for the picture yet: \(rig.shown)")
  rig.screen.ended()
  rig.screen.show(0)
  check(rig.shown == [nil], "only the picture that is taken away: \(rig.shown)")
}

func testOnlyTheKeyFlagMakesAKeyFrame() {
  let arena = Arena()
  func frame(_ flags: UInt32) -> FrameInput {
    FrameInput(
      flags: flags, number: 0, dependsOn: 0, captureTime: 0, chain: 0, epoch: 0,
      sample: arena.bytes([0, 0, 0, 1, 0x65]), sets: [], sequenceSets: 0)
  }
  let key = UInt32(BATON_FRAME_KEY)
  let referenced = UInt32(BATON_FRAME_REFERENCED)
  let recovery = UInt32(BATON_FRAME_RECOVERY)
  check(!frame(0).isKey)
  check(!frame(referenced).isKey, "a frame that others depend on is not a key frame")
  check(!frame(recovery).isKey, "a frame that heals a loss is not a key frame")
  check(!frame(referenced | recovery).isKey)
  check(frame(key).isKey)
  check(frame(key | referenced).isKey)
  check(frame(key | recovery | referenced).isKey)
}

func testTheStateCanBeReadWhilePicturesComeOut() {
  // The decoder answers on a thread of its own while the Rust side asks how it is doing. The
  // times are read under the lock that the answers are written under, which the thread sanitizer
  // shows if they are not.
  let rig = Rig()
  let frames: UInt64 = 200
  check(rig.screen.decode(rig.key(0)))
  for number in 1..<frames { check(rig.screen.decode(rig.delta(number))) }
  let group = DispatchGroup()
  group.enter()
  Thread.detachNewThread {
    for number in 0..<frames {
      rig.decoder.finish(number)
      usleep(50)
    }
    group.leave()
  }
  var reads = 0
  while group.wait(timeout: .now()) != .success {
    _ = rig.screen.state()
    reads += 1
  }
  check(reads > 0, "the state was read while the pictures came out")
  check(rig.screen.state().decodeMicros > 0, "the pictures took time to come out")
}
