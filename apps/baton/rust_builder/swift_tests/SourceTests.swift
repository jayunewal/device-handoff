// SPDX-License-Identifier: Apache-2.0

// Tests of BatonSource.swift. They play the Rust side for the table that the Swift code registers:
// the functions of the table are called with the C structs that the Rust side builds. And they
// play the library for the calls about a window that is shared: the stubs in Stubs.swift keep what
// the Swift code gave them, and answer as a test says.

import Foundation

// MARK: - A platform and a window to play

final class FakeSharedWindow: SharedWindow {
  var takes: [ViewerTakes] = []
  var format: PictureFormat? = PictureFormat(width: 1280, height: 800, fps: 30)
  var prepareError: Error?
  var started: [UInt64] = []
  var startFailure: Error?
  var endedCalls = 0
  var releasedCalls = 0
  var onDeinit: (() -> Void)?
  /// What the window says of its surroundings, and what is put to it.
  var around: WindowSurroundings?
  var posts: [InputPost] = []
  var postWorks = true

  func surroundings() -> WindowSurroundings? { around }

  func post(_ post: InputPost) -> Bool {
    posts.append(post)
    return postWorks
  }

  func prepare(takes: ViewerTakes) throws -> PictureFormat {
    self.takes.append(takes)
    if let prepareError { throw prepareError }
    guard let format else { throw SharerFailure(sentence: "No picture.") }
    return format
  }

  func start(link: UInt64) throws {
    started.append(link)
    if let startFailure { throw startFailure }
  }

  func ended() { endedCalls += 1 }
  func released() { releasedCalls += 1 }
  deinit { onDeinit?() }
}

final class FakeSharerPlatform: SharerPlatform {
  var canControl = false
  var picks: [UInt64] = []
  var cancelled: [UInt64] = []
  var pickFailure: Error?

  func pick(request: UInt64) throws {
    picks.append(request)
    if let pickFailure { throw pickFailure }
  }

  func cancelPick(request: UInt64) { cancelled.append(request) }
}

/// Registers `platform` against the stub and gives the table that the Rust side would hold.
func registeredSharing(_ platform: SharerPlatform) -> BatonSharerVTable? {
  stubLock.lock()
  registeredSharer = nil
  sharerRegistrationCode = 0
  stubLock.unlock()
  let code = registerSharer(platform)
  check(code == 0, "the registration was refused with \(code)")
  stubLock.lock()
  defer { stubLock.unlock() }
  return registeredSharer
}

/// A message with room for `room` bytes, the way the Rust side builds it, and what is written to it.
func withMessage(room: Int = 256, size: Int? = nil, _ body: (UnsafeMutablePointer<BatonMessage>) -> Int32)
  -> (code: Int32, sentence: String)
{
  let text = UnsafeMutablePointer<UInt8>.allocate(capacity: max(room, 1))
  defer { text.deallocate() }
  text.initialize(repeating: 0, count: max(room, 1))
  var message = BatonMessage(
    size: UInt32(size ?? MemoryLayout<BatonMessage>.size), cap: UInt32(room), len: 0, reserved: 0,
    text: text)
  let code = body(&message)
  let sentence = String(
    decoding: UnsafeBufferPointer(start: text, count: Int(message.len)), as: UTF8.self)
  return (code, sentence)
}

/// The window that the person picked, the way the Rust side holds it after an answer.
func chosen(_ window: FakeSharedWindow) -> UnsafeMutableRawPointer? {
  resetSourceStubs()
  let info = PickedWindowInfo(width: 1440, height: 900, title: "Report.pdf", app: "Preview")
  check(answerPick(request: 9, .chose(window, info)) == 0)
  stubLock.lock()
  defer { stubLock.unlock() }
  return pickDones.last?.chosen?.context
}

// MARK: - The table

func testTheSharerRegistrationGivesTheRustSideAWholeTable() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform) else {
    check(false, "no table was registered")
    return
  }
  check(table.version == BATON_STREAM_ABI_VERSION, "version \(table.version)")
  check(table.size == UInt32(MemoryLayout<BatonSharerVTable>.size), "size \(table.size)")
  check(table.context != nil)
  check(table.control != nil && table.pick != nil && table.cancel_pick != nil)
  check(table.prepare != nil && table.start != nil && table.ended != nil && table.release != nil)
}

func testASharerRegistrationThatFailsReturnsItsCodeAndLetsGoOfThePlatform() {
  weak var weakPlatform: FakeSharerPlatform?
  do {
    let platform = FakeSharerPlatform()
    weakPlatform = platform
    stubLock.lock()
    sharerRegistrationCode = Int32(BATON_E_REGISTERED)
    stubLock.unlock()
    let code = registerSharer(platform)
    check(code == Int32(BATON_E_REGISTERED), "code \(code)")
  }
  stubLock.lock()
  sharerRegistrationCode = 0
  stubLock.unlock()
  check(weakPlatform == nil, "the platform is still held after a failed registration")
}

func testASharerRegistrationThatWorksKeepsThePlatformForTheLifeOfTheProcess() {
  weak var weakPlatform: FakeSharerPlatform?
  do {
    let platform = FakeSharerPlatform()
    weakPlatform = platform
    _ = registeredSharing(platform)
  }
  check(weakPlatform != nil, "the platform was let go of while the Rust side may call it")
}

func testWhetherAWindowCanBeControlledIsWhatThePlatformSaysEachTime() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let control = table.control else {
    return check(false)
  }
  check(control(table.context) == 0, "a platform that cannot post input")
  platform.canControl = true
  check(control(table.context) == 1, "asked again")
  check(control(nil) == 0, "no context")
}

func testAPickIsShownAndATakenDownPickIsTold() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let pick = table.pick,
    let cancel = table.cancel_pick
  else { return check(false) }
  let result = withMessage { pick(table.context, 31, $0) }
  check(result.code == 0 && result.sentence == "", "\(result)")
  check(platform.picks == [31], "\(platform.picks)")
  cancel(table.context, 31)
  check(platform.cancelled == [31], "\(platform.cancelled)")
  cancel(nil, 32)
  check(platform.cancelled == [31], "no context")
}

func testAPickThatCannotBeShownWritesTheSentenceAndAnythingElseHasOneOfItsOwn() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let pick = table.pick else {
    return check(false)
  }
  platform.pickFailure = SharerFailure(sentence: "Screen sharing is not allowed here.")
  var result = withMessage { pick(table.context, 1, $0) }
  check(result.code == Int32(BATON_E_PLATFORM), "code \(result.code)")
  check(result.sentence == "Screen sharing is not allowed here.", result.sentence)

  struct Other: Error {}
  platform.pickFailure = Other()
  result = withMessage { pick(table.context, 2, $0) }
  check(result.code == Int32(BATON_E_PLATFORM))
  check(result.sentence == "The picker could not be shown.", result.sentence)

  // A message that is too short, or none, or no context, is refused without asking the platform.
  platform.pickFailure = nil
  let before = platform.picks.count
  check(withMessage(size: 8) { pick(table.context, 3, $0) }.code == Int32(BATON_E_ARGUMENT))
  check(pick(table.context, 4, nil) == Int32(BATON_E_ARGUMENT))
  check(withMessage { pick(nil, 5, $0) }.code == Int32(BATON_E_ARGUMENT))
  check(platform.picks.count == before, "the platform was asked")
}

// MARK: - The answer to a pick

func testTheAnswerToAPickGivesTheWindowAndWhatIsKnownOfIt() {
  resetSourceStubs()
  let window = FakeSharedWindow()
  let info = PickedWindowInfo(width: 1440, height: 900, title: "Report.pdf", app: "Preview")
  let code = answerPick(request: 12, .chose(window, info))
  check(code == 0, "code \(code)")
  stubLock.lock()
  let done = pickDones
  stubLock.unlock()
  check(done.count == 1, "\(done.count) answers")
  guard let answer = done.first, let chosen = answer.chosen else { return check(false) }
  check(answer.request == 12 && answer.failure == nil)
  check(chosen.size == UInt32(MemoryLayout<BatonPicked>.size), "size \(chosen.size)")
  check(chosen.width == 1440 && chosen.height == 900, "\(chosen)")
  check(chosen.title == "Report.pdf" && chosen.app == "Preview", "\(chosen)")
  check(chosen.kind == UInt32(BATON_PICKED_WINDOW), "kind \(chosen.kind)")
  check(chosen.context != nil, "no pointer for the window")
  // The Rust side owns the window now, and gives it back with release.
  var table = BatonSharerVTable()
  _ = registeredSharing(FakeSharerPlatform()).map { table = $0 }
  table.release?(chosen.context)
  check(window.releasedCalls == 1)
}

func testADisplayIsGivenAsADisplayAndAWindowIsNot() {
  resetSourceStubs()
  let screen = FakeSharedWindow()
  let info = PickedWindowInfo(width: 3024, height: 1964, title: "", app: "", display: true)
  check(answerPick(request: 2, .chose(screen, info)) == 0)
  stubLock.lock()
  let display = pickDones.last?.chosen
  stubLock.unlock()
  check(display?.kind == UInt32(BATON_PICKED_DISPLAY), "kind \(String(describing: display?.kind))")
  check(display?.width == 3024 && display?.height == 1964, "\(String(describing: display))")
  check(display?.title == "" && display?.app == "", "\(String(describing: display))")
  check(display?.context != nil)

  // Not saying what it is makes it a window.
  let window = FakeSharedWindow()
  let plain = PickedWindowInfo(width: 10, height: 10, title: "Notes", app: "Text")
  check(!plain.display)
  check(answerPick(request: 3, .chose(window, plain)) == 0)
  stubLock.lock()
  let chosen = pickDones.last?.chosen
  stubLock.unlock()
  check(chosen?.kind == UInt32(BATON_PICKED_WINDOW), "kind \(String(describing: chosen?.kind))")
}

func testAWindowAndADisplayAreCalledWhatTheyAreInASentence() {
  check(PickedWindowInfo(width: 1, height: 1, title: "", app: "").noun == "window")
  check(PickedWindowInfo(width: 1, height: 1, title: "", app: "", display: true).noun == "screen")
  // The numbers that the platform writes are the ones the Rust side reads.
  check(BATON_PICKED_WINDOW == 0 && BATON_PICKED_DISPLAY == 1)
}

func testAWindowWithNoTitleAndNoNameIsGivenWithEmptyText() {
  resetSourceStubs()
  let window = FakeSharedWindow()
  check(
    answerPick(request: 1, .chose(window, PickedWindowInfo(width: 1, height: 1, title: "", app: "")))
      == 0)
  stubLock.lock()
  let chosen = pickDones.last?.chosen
  stubLock.unlock()
  check(chosen?.title == "" && chosen?.app == "", "\(String(describing: chosen))")
  check(chosen?.context != nil)
}

func testATitleThatIsNotAsciiGoesAsItIs() {
  resetSourceStubs()
  let window = FakeSharedWindow()
  let info = PickedWindowInfo(width: 5, height: 6, title: "Käse – 日本語 🧀", app: "Écrire")
  check(answerPick(request: 1, .chose(window, info)) == 0)
  stubLock.lock()
  let chosen = pickDones.last?.chosen
  stubLock.unlock()
  check(chosen?.title == "Käse – 日本語 🧀" && chosen?.app == "Écrire", "\(String(describing: chosen))")
}

func testAnAnswerThatTheRustSideRefusesLeavesTheWindowWithThePlatform() {
  resetSourceStubs()
  weak var weakWindow: FakeSharedWindow?
  do {
    let window = FakeSharedWindow()
    weakWindow = window
    stubLock.lock()
    pickDoneCode = Int32(BATON_E_UNKNOWN_LINK)
    stubLock.unlock()
    let info = PickedWindowInfo(width: 10, height: 10, title: "", app: "")
    check(answerPick(request: 3, .chose(window, info)) == Int32(BATON_E_UNKNOWN_LINK))
    check(window.releasedCalls == 0, "the window was released by the platform's own code")
  }
  stubLock.lock()
  pickDoneCode = 0
  stubLock.unlock()
  check(weakWindow == nil, "a window that the Rust side did not take is still held")
}

func testAPickerThatFailedAndAPersonWhoGaveUpAreTold() {
  resetSourceStubs()
  check(answerPick(request: 4, .failed("Nothing can be shared right now.")) == 0)
  check(answerPick(request: 5, .gaveUp) == 0)
  check(answerPick(request: 6, .failed("")) == 0)
  stubLock.lock()
  let done = pickDones
  stubLock.unlock()
  check(done.count == 3)
  check(done[0].request == 4 && done[0].chosen == nil && done[0].failure == "Nothing can be shared right now.")
  check(done[1].request == 5 && done[1].chosen == nil && done[1].failure == nil)
  check(done[2].request == 6 && done[2].chosen == nil && done[2].failure == "")
}

// MARK: - What the Rust side calls about a window

func testAWindowIsPreparedWithinWhatTheOtherDeviceTakes() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let prepare = table.prepare else {
    return check(false)
  }
  let window = FakeSharedWindow()
  window.format = PictureFormat(width: 1024, height: 640, fps: 24)
  let context = chosen(window)
  // The other device may take a codec that this version has never heard of, and says so: the bits
  // are passed on as they come.
  var takes = BatonTakes(
    size: UInt32(MemoryLayout<BatonTakes>.size), max_width: 3840, max_height: 2160, max_fps: 60,
    codecs: BATON_CODEC_H264 | 2, reserved: 0)
  var format = BatonFormat(
    size: UInt32(MemoryLayout<BatonFormat>.size), codec: 0, width: 0, height: 0, fps: 0, reserved: 0)
  check(prepare(context, &takes, &format) == 0)
  check(
    window.takes == [ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: 3)],
    "\(window.takes)")
  check(format.codec == BATON_CODEC_H264, "codec \(format.codec)")
  check(format.width == 1024 && format.height == 640 && format.fps == 24, "\(format)")
  check(format.size == UInt32(MemoryLayout<BatonFormat>.size) && format.reserved == 0)
  table.release?(context)
}

func testAPictureThatCannotBeMadeIsAFailureAndNothingIsWritten() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let prepare = table.prepare else {
    return check(false)
  }
  let window = FakeSharedWindow()
  window.format = nil
  let context = chosen(window)
  var takes = BatonTakes(
    size: UInt32(MemoryLayout<BatonTakes>.size), max_width: 64, max_height: 64, max_fps: 5,
    codecs: 0, reserved: 0)
  var format = BatonFormat(
    size: UInt32(MemoryLayout<BatonFormat>.size), codec: 0, width: 0, height: 0, fps: 0, reserved: 0)
  check(prepare(context, &takes, &format) == Int32(BATON_E_PLATFORM))
  check(format.codec == 0 && format.width == 0 && format.height == 0 && format.fps == 0, "\(format)")
  struct Other: Error {}
  window.prepareError = Other()
  check(prepare(context, &takes, &format) == Int32(BATON_E_PLATFORM), "an error of another kind")
  table.release?(context)
}

func testAPrepareWithStructsThatAreTooShortOrMissingIsRefused() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let prepare = table.prepare else {
    return check(false)
  }
  let window = FakeSharedWindow()
  let context = chosen(window)
  let bad = Int32(BATON_E_ARGUMENT)
  let arena = Arena()
  var takes = BatonTakes(
    size: UInt32(MemoryLayout<BatonTakes>.size), max_width: 64, max_height: 64, max_fps: 5,
    codecs: 1, reserved: 0)
  var format = BatonFormat(
    size: UInt32(MemoryLayout<BatonFormat>.size), codec: 0, width: 0, height: 0, fps: 0, reserved: 0)
  var shortTakes = takes
  shortTakes.size = 8
  var shortFormat = format
  shortFormat.size = 8
  check(prepare(context, &shortTakes, &format) == bad, "takes that are too short")
  check(prepare(context, &takes, &shortFormat) == bad, "a format that is too short")
  check(prepare(context, nil, &format) == bad, "no takes")
  check(prepare(context, &takes, nil) == bad, "no format")
  check(prepare(nil, &takes, &format) == bad, "no window")
  // A struct that is only as long as it says is read no further: the address sanitizer shows it.
  let exactTakes = arena.shortStruct(BatonTakes.self, size: MemoryLayout<BatonTakes>.size)
  let exactFormat = arena.shortStruct(BatonFormat.self, size: MemoryLayout<BatonFormat>.size)
  exactTakes.pointee = takes
  exactFormat.pointee = format
  check(prepare(context, exactTakes, exactFormat) == 0, "structs of exactly the size they state")
  check(window.takes.count == 1, "the platform was asked \(window.takes.count) times")
  table.release?(context)
}

func testAStartGivesTheWindowItsLinkAndAFailureWritesItsSentence() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let start = table.start else {
    return check(false)
  }
  let window = FakeSharedWindow()
  let context = chosen(window)
  var result = withMessage { start(context, 41, $0) }
  check(result.code == 0 && result.sentence == "", "\(result)")
  check(window.started == [41], "\(window.started)")

  window.startFailure = SharerFailure(sentence: "Screen recording is off for Baton.")
  result = withMessage { start(context, 42, $0) }
  check(result.code == Int32(BATON_E_PLATFORM), "code \(result.code)")
  check(result.sentence == "Screen recording is off for Baton.", result.sentence)

  struct Other: Error {}
  window.startFailure = Other()
  result = withMessage { start(context, 43, $0) }
  check(result.code == Int32(BATON_E_PLATFORM))
  check(result.sentence == "This device could not start sharing the window.", result.sentence)

  window.startFailure = nil
  let before = window.started.count
  check(withMessage(size: 8) { start(context, 44, $0) }.code == Int32(BATON_E_ARGUMENT))
  check(start(context, 45, nil) == Int32(BATON_E_ARGUMENT))
  check(withMessage { start(nil, 46, $0) }.code == Int32(BATON_E_ARGUMENT))
  check(window.started.count == before, "the platform was asked")
  table.release?(context)
}

func testTheSentenceOfAFailedStartIsCutBetweenTwoCharactersWhereItDoesNotFit() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let start = table.start else {
    return check(false)
  }
  let window = FakeSharedWindow()
  window.startFailure = SharerFailure(sentence: "Fenster geschlossen: äöü")
  let context = chosen(window)
  // "ä" is two bytes: room for 21 bytes cuts before it.
  let result = withMessage(room: 21) { start(context, 1, $0) }
  check(result.sentence == "Fenster geschlossen: ", "\(result.sentence)")
  table.release?(context)
}

func testTheEndAndTheReleaseOfAWindowReachIt() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform), let ended = table.ended,
    let release = table.release
  else { return check(false) }
  weak var weakWindow: FakeSharedWindow?
  var context: UnsafeMutableRawPointer?
  do {
    let window = FakeSharedWindow()
    weakWindow = window
    context = chosen(window)
    ended(context)
    check(window.endedCalls == 1 && window.releasedCalls == 0)
    ended(nil)
    release(nil)
    release(context)
    check(window.releasedCalls == 1, "released \(window.releasedCalls) times")
    check(window.endedCalls == 1)
  }
  check(weakWindow == nil, "the window is still held after the release")
}

func testAWindowThatWasNeverStartedIsReleasedWithoutAnEnd() {
  let platform = FakeSharerPlatform()
  guard let table = registeredSharing(platform) else { return check(false) }
  let window = FakeSharedWindow()
  let context = chosen(window)
  table.release?(context)
  check(window.endedCalls == 0 && window.releasedCalls == 1 && window.started.isEmpty)
}

// MARK: - The picture that is made of a window

func testThePictureIsTheWindowItselfWhenItFits() {
  let takes = ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: 1)
  check(
    chooseFormat(windowWidth: 1280, windowHeight: 800, takes: takes)
      == PictureFormat(width: 1280, height: 800, fps: 30))
  check(
    chooseFormat(windowWidth: 640, windowHeight: 480, takes: takes)
      == PictureFormat(width: 640, height: 480, fps: 30), "never scaled up")
  check(
    chooseFormat(windowWidth: 1920, windowHeight: 1080, takes: takes)
      == PictureFormat(width: 1920, height: 1080, fps: 30), "exactly at the longest edge")
}

func testALargeWindowIsScaledDownWithItsShapeKept() {
  let takes = ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: 1)
  check(
    chooseFormat(windowWidth: 2880, windowHeight: 1800, takes: takes)
      == PictureFormat(width: 1920, height: 1200, fps: 30))
  check(
    chooseFormat(windowWidth: 3840, windowHeight: 2160, takes: takes)
      == PictureFormat(width: 1920, height: 1080, fps: 30))
  // A tall window is limited by its height, which can be larger than the width may be.
  check(
    chooseFormat(windowWidth: 1000, windowHeight: 4000, takes: takes)
      == PictureFormat(width: 480, height: 1920, fps: 30))
  check(
    chooseFormat(windowWidth: 16384, windowHeight: 16384, takes: takes)
      == PictureFormat(width: 1920, height: 1920, fps: 30))
}

func testThePictureStaysWithinWhatTheOtherDeviceTakes() {
  let small = ViewerTakes(maxWidth: 800, maxHeight: 600, maxFps: 15, codecs: 1)
  check(
    chooseFormat(windowWidth: 1600, windowHeight: 1000, takes: small)
      == PictureFormat(width: 800, height: 500, fps: 15))
  check(
    chooseFormat(windowWidth: 1000, windowHeight: 1000, takes: small)
      == PictureFormat(width: 600, height: 600, fps: 15))
  check(
    chooseFormat(windowWidth: 400, windowHeight: 300, takes: small)
      == PictureFormat(width: 400, height: 300, fps: 15))
}

func testTheEdgesOfThePictureAreEvenAndNeverBelowTwo() {
  let takes = ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: 1)
  check(
    chooseFormat(windowWidth: 641, windowHeight: 479, takes: takes)
      == PictureFormat(width: 640, height: 478, fps: 30))
  let odd = ViewerTakes(maxWidth: 1001, maxHeight: 1001, maxFps: 60, codecs: 1)
  check(
    chooseFormat(windowWidth: 2000, windowHeight: 2000, takes: odd)
      == PictureFormat(width: 1000, height: 1000, fps: 30))
  check(
    chooseFormat(windowWidth: 1, windowHeight: 1, takes: takes)
      == PictureFormat(width: 2, height: 2, fps: 30))
  check(
    chooseFormat(windowWidth: 10000, windowHeight: 3, takes: takes)
      == PictureFormat(width: 1920, height: 2, fps: 30), "a window that is only a line")
  check(
    chooseFormat(windowWidth: 3, windowHeight: 10000, takes: takes)
      == PictureFormat(width: 2, height: 1920, fps: 30), "a window that is only a tall line")
}

func testOurOwnLimitsAreWithinWhatTheProtocolAllows() {
  // The format is chosen within what the viewer takes and our own limits only; the protocol
  // allows 3840 pixels along an edge and 120 frames a second, and the library refuses a picture
  // shape that is past them.
  check(longestPictureEdge <= 3840, "\(longestPictureEdge)")
  check(mostPictureFps <= 120, "\(mostPictureFps)")
  let wide = ViewerTakes(maxWidth: UInt32.max, maxHeight: UInt32.max, maxFps: UInt32.max, codecs: 1)
  check(
    chooseFormat(windowWidth: UInt32.max, windowHeight: UInt32.max, takes: wide)
      == PictureFormat(width: longestPictureEdge, height: longestPictureEdge, fps: mostPictureFps),
    "a viewer that takes everything")
}

func testNoPictureIsMadeForAViewerThatTakesNone() {
  let h264 = UInt32(BATON_CODEC_H264)
  func choose(_ takes: ViewerTakes) -> PictureFormat? {
    chooseFormat(windowWidth: 800, windowHeight: 600, takes: takes)
  }
  check(choose(ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: 0)) == nil, "no codec")
  check(choose(ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: 2)) == nil, "not H.264")
  check(choose(ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 0, codecs: h264)) == nil, "no rate")
  check(choose(ViewerTakes(maxWidth: 1, maxHeight: 2160, maxFps: 60, codecs: h264)) == nil, "no width")
  check(choose(ViewerTakes(maxWidth: 3840, maxHeight: 1, maxFps: 60, codecs: h264)) == nil, "no height")
  check(
    chooseFormat(
      windowWidth: 0, windowHeight: 600,
      takes: ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: h264)) == nil,
    "a window with no width")
  check(
    chooseFormat(
      windowWidth: 800, windowHeight: 0,
      takes: ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 60, codecs: h264)) == nil,
    "a window with no height")
  // The rate is the viewer's own if that is lower than ours, and one at least.
  check(
    chooseFormat(
      windowWidth: 800, windowHeight: 600,
      takes: ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 1, codecs: h264))?.fps == 1)
  check(
    chooseFormat(
      windowWidth: 800, windowHeight: 600,
      takes: ViewerTakes(maxWidth: 3840, maxHeight: 2160, maxFps: 120, codecs: h264))?.fps == 30)
}

func testTheBitrateGrowsWithThePictureWithinBounds() {
  func rate(_ width: UInt32, _ height: UInt32, _ fps: UInt32) -> Int {
    targetBitrate(for: PictureFormat(width: width, height: height, fps: fps))
  }
  check(rate(2, 2, 1) == 1_000_000, "a floor of one megabit")
  check(rate(1920, 1200, 30) == 6_912_000, "\(rate(1920, 1200, 30))")
  check(rate(1280, 800, 30) == 3_072_000, "\(rate(1280, 800, 30))")
  check(rate(3840, 3840, 120) == 16_000_000, "a ceiling of sixteen megabits")
  check(rate(1280, 800, 30) < rate(1920, 1200, 30))
  // The same picture at a lower rate needs fewer bits a second, and at a higher rate more.
  check(rate(1280, 800, 15) == 1_536_000, "\(rate(1280, 800, 15))")
  check(rate(1280, 800, 60) == 6_144_000, "\(rate(1280, 800, 60))")
  // Nor does the shape alone decide it: a tall picture with as many pixels takes as many bits.
  check(rate(800, 1280, 30) == rate(1280, 800, 30), "\(rate(800, 1280, 30))")
}

// MARK: - The calls about a window that is shared

/// Calls the library as the Swift code does, with the stubs answering.
func withSession(_ body: (LibrarySourceSession) -> Void) {
  resetSourceStubs()
  body(LibrarySourceSession(link: 55))
}

func testTheClockAndTheQuestionsAreAskedAboutTheLink() {
  withSession { session in
    stubLock.lock()
    sourceAnswers.micros = 123_456
    sourceAnswers.pending = 1
    stubLock.unlock()
    check(session.clock() == 123_456)
    check(session.recoveryPending() == true)
    stubLock.lock()
    sourceAnswers.pending = 0
    stubLock.unlock()
    check(session.recoveryPending() == false)
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(calls == [.clock(55), .recoveryPending(55), .recoveryPending(55)], "\(calls)")
  }
}

func testAClockAndAQuestionThatAreRefusedMeanThatTheSessionIsOver() {
  withSession { session in
    stubLock.lock()
    sourceAnswers.clockCode = Int32(BATON_E_UNKNOWN_LINK)
    sourceAnswers.pendingCode = Int32(BATON_E_UNKNOWN_LINK)
    stubLock.unlock()
    check(session.clock() == nil)
    check(session.recoveryPending() == nil)
  }
}

func testWhatBeginSaysIsTheKindOfPictureToMake() {
  withSession { session in
    let kinds: [(UInt32, BeginAnswer)] = [
      (BATON_BEGIN_NOT_NOW, .notNow), (BATON_BEGIN_KEY, .key), (BATON_BEGIN_RECOVERY, .recovery),
      (BATON_BEGIN_DELTA, .delta),
    ]
    for (kind, expected) in kinds {
      stubLock.lock()
      sourceAnswers.beginKind = kind
      stubLock.unlock()
      check(session.begin() == expected, "kind \(kind)")
    }
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(!calls.contains(.cancel(55)), "a picture that was announced was given up on")
  }
}

func testABeginThatIsRefusedOrNotUnderstoodEndsOrGivesUpThePicture() {
  withSession { session in
    stubLock.lock()
    sourceAnswers.beginCode = Int32(BATON_E_UNKNOWN_LINK)
    stubLock.unlock()
    check(session.begin() == .over)

    // A kind that a newer library has: nothing is made, and what it announced is given up on.
    stubLock.lock()
    sourceAnswers.beginCode = 0
    sourceAnswers.beginKind = 77
    sourceCalls = []
    stubLock.unlock()
    check(session.begin() == .notNow)
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(calls == [.begin(55), .cancel(55)], "\(calls)")
  }
}

func testABeginThatSaysYesWithoutSayingWhatKindMakesNoPicture() {
  withSession { session in
    stubLock.lock()
    sourceAnswers.beginKind = BATON_BEGIN_KEY
    sourceAnswers.beginWritesKind = false
    stubLock.unlock()
    check(session.begin() == .notNow, "a picture was made from nothing")
  }
}

func testAPictureIsHandedOverWithItsSetsInOrderAndMarkedAsReferenced() {
  withSession { session in
    let picture = EncodedPicture(
      sample: Data([0, 0, 0, 2, 0x65, 0x88]), lengthSize: 4,
      sets: [Data([0x67, 1, 2]), Data([0x68, 3])], captureTime: 4_321)
    check(session.send(picture) == .sent)
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(
      calls == [
        .sent(
          55,
          SentRecord(
            size: UInt32(MemoryLayout<BatonPicture>.size), flags: BATON_FRAME_REFERENCED,
            captureTime: 4_321, lengthSize: 4, sample: [0, 0, 0, 2, 0x65, 0x88],
            sets: [[0x67, 1, 2], [0x68, 3]]))
      ], "\(calls)")
  }
}

func testAPictureWithNoSetsAndOneWithNoBytesAreHandedOverToo() {
  withSession { session in
    check(
      session.send(
        EncodedPicture(sample: Data([0, 0, 0, 1, 0x41]), lengthSize: 4, sets: [], captureTime: 1))
        == .sent)
    check(session.send(EncodedPicture(sample: Data(), lengthSize: 2, sets: [Data()], captureTime: 2)) == .sent)
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(calls.count == 2, "\(calls.count) calls")
    if case .sent(_, let first) = calls[0] {
      check(first.sets.isEmpty && first.sample == [0, 0, 0, 1, 0x41], "\(first)")
    } else {
      check(false)
    }
    if case .sent(_, let second) = calls[1] {
      check(second.sample.isEmpty && second.sets == [[]] && second.lengthSize == 2, "\(second)")
    } else {
      check(false)
    }
  }
}

func testWhatBecameOfAPictureIsToldInTheWordsOfTheLibrary() {
  withSession { session in
    let picture = EncodedPicture(
      sample: Data([0, 0, 0, 1, 0x41]), lengthSize: 4, sets: [], captureTime: 1)
    let outcomes: [(UInt32, SendAnswer)] = [
      (BATON_SENT, .sent), (BATON_SENT_STALE, .stale), (BATON_SENT_TOO_LARGE, .tooLarge),
      (BATON_SENT_OVER, .over), (99, .stale),
    ]
    for (outcome, expected) in outcomes {
      stubLock.lock()
      sourceAnswers.outcome = outcome
      stubLock.unlock()
      check(session.send(picture) == expected, "outcome \(outcome)")
    }
    stubLock.lock()
    sourceAnswers.sentCode = Int32(BATON_E_UNKNOWN_LINK)
    stubLock.unlock()
    check(session.send(picture) == .over, "a link that is closed")
    for code in [BATON_E_ARGUMENT, BATON_E_REENTRANT, BATON_E_PANIC] {
      stubLock.lock()
      sourceAnswers.sentCode = Int32(code)
      stubLock.unlock()
      check(session.send(picture) == .refused, "code \(code)")
    }
  }
}

func testTheEndOfASessionGoesWithItsSentenceOrWithNone() {
  withSession { session in
    session.end(sentence: nil)
    session.end(sentence: "")
    session.end(sentence: "Screen recording was switched off.")
    session.end(sentence: "Käse")
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(
      calls == [
        .end(55, nil), .end(55, nil), .end(55, "Screen recording was switched off."),
        .end(55, "Käse"),
      ], "\(calls)")
  }
}

func testACancelReachesTheLibrary() {
  withSession { session in
    session.cancel()
    stubLock.lock()
    let calls = sourceCalls
    stubLock.unlock()
    check(calls == [.cancel(55)], "\(calls)")
  }
}

// MARK: - When a picture is made

/// An image of the window, which the encoder turns into a picture.
final class Img {
  let number: Int
  static var alive = 0

  init(_ number: Int) {
    self.number = number
    Img.alive += 1
  }

  deinit { Img.alive -= 1 }
}

struct EncoderFailure: Error {}

final class FakeEncoder: PictureEncoder {
  typealias Image = Img

  var encoded: [(image: Int, key: Bool, time: UInt64)] = []
  /// For each call, whether it fails; calls beyond the list succeed.
  var failures: [Bool] = []
  var resets = 0
  var resetFails = false
  var invalidated = 0

  func encode(_ image: Img, key: Bool, captureTime: UInt64) throws -> EncodedPicture {
    encoded.append((image.number, key, captureTime))
    if !failures.isEmpty, failures.removeFirst() { throw EncoderFailure() }
    return EncodedPicture(
      sample: Data([UInt8(truncatingIfNeeded: image.number), key ? 1 : 0]), lengthSize: 4,
      sets: key ? [Data([0x67]), Data([0x68])] : [], captureTime: captureTime)
  }

  func reset() throws {
    resets += 1
    if resetFails { throw EncoderFailure() }
  }

  func invalidate() { invalidated += 1 }
}

/// The library, played: it answers `begin` and `send` from lists, and checks that every `begin`
/// is answered with one `send` or `cancel` before the next.
final class FakeSession: SourceSession {
  var calls: [String] = []
  var micros: UInt64 = 1_000
  var alive = true
  /// What `begin` says, in order; beyond the list it says not now.
  var begins: [BeginAnswer] = []
  /// What `send` says, in order; beyond the list it says sent.
  var sends: [SendAnswer] = []
  var pending: Bool? = false
  var sent: [EncodedPicture] = []
  var ended: [String?] = []
  var violations: [String] = []
  /// The capture times of the changes that were told, in order.
  var changes: [UInt64] = []
  var pauses: [PauseReason] = []
  var resumes = 0
  private(set) var announced = false
  /// How many pictures `begin` announced.
  private(set) var announcements = 0

  func clock() -> UInt64? {
    calls.append("clock")
    return alive ? micros : nil
  }

  func begin() -> BeginAnswer {
    calls.append("begin")
    if announced { violations.append("begin while a picture is announced") }
    let answer = begins.isEmpty ? BeginAnswer.notNow : begins.removeFirst()
    announced = answer == .key || answer == .recovery || answer == .delta
    if announced { announcements += 1 }
    return answer
  }

  func cancel() {
    calls.append("cancel")
    if !announced { violations.append("cancel with nothing announced") }
    announced = false
  }

  func recoveryPending() -> Bool? {
    calls.append("pending")
    return pending
  }

  func send(_ picture: EncodedPicture) -> SendAnswer {
    calls.append("send")
    if !announced { violations.append("send with nothing announced") }
    announced = false
    sent.append(picture)
    return sends.isEmpty ? .sent : sends.removeFirst()
  }

  func changed(_ captureTime: UInt64) {
    calls.append("changed")
    changes.append(captureTime)
  }

  func pause(_ reason: PauseReason) {
    calls.append("pause")
    pauses.append(reason)
  }

  func resume() {
    calls.append("resume")
    resumes += 1
  }

  func end(sentence: String?) {
    calls.append("end")
    ended.append(sentence)
  }
}

func testAnImageIsMadeIntoThePictureThatTheLibraryAsksForAtTheTimeItWasCaptured() {
  let session = FakeSession()
  session.begins = [.key, .delta, .delta]
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  for (number, time) in [(1, UInt64(1_000)), (2, 2_500), (3, 4_000)] {
    session.micros = time
    pump.captured(Img(number))
  }
  check(encoder.encoded.map { $0.image } == [1, 2, 3], "\(encoder.encoded)")
  check(encoder.encoded.map { $0.key } == [true, false, false])
  check(encoder.encoded.map { $0.time } == [1_000, 2_500, 4_000])
  check(session.sent.map { $0.captureTime } == [1_000, 2_500, 4_000])
  check(session.sent[0].sets.count == 2 && session.sent[1].sets.isEmpty)
  check(session.violations.isEmpty, "\(session.violations)")
  check(session.ended.isEmpty && !pump.over)
}

func testARecoveryFrameIsMadeAsAKeyFrameSinceNothingIsMarkedAsALongTermReference() {
  let session = FakeSession()
  session.begins = [.recovery]
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  pump.captured(Img(1))
  check(encoder.encoded.map { $0.key } == [true])
  check(session.sent.count == 1 && session.violations.isEmpty)
}

func testAnImageForWhichNoPictureIsWantedIsKeptAndMadeWhenTheFrameClockTicks() {
  let session = FakeSession()
  session.begins = [.notNow, .notNow, .key]
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  session.micros = 5_000
  pump.captured(Img(1))
  check(encoder.encoded.isEmpty && session.sent.isEmpty, "a picture was made when none was wanted")
  session.micros = 9_000
  pump.tick()
  check(encoder.encoded.isEmpty)
  pump.tick()
  check(encoder.encoded.map { $0.image } == [1], "\(encoder.encoded)")
  // It is still the picture of the window as it was captured, not as the clock shows now.
  check(session.sent.map { $0.captureTime } == [5_000], "\(session.sent.map { $0.captureTime })")
  check(session.violations.isEmpty)
}

func testOnlyTheNewestImageIsMadeIntoAPicture() {
  let session = FakeSession()
  session.begins = [.notNow, .notNow, .key]
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  session.micros = 1_000
  pump.captured(Img(1))
  session.micros = 2_000
  pump.captured(Img(2))
  session.micros = 3_000
  pump.captured(Img(3))
  check(encoder.encoded.map { $0.image } == [3], "\(encoder.encoded)")
  check(session.sent.map { $0.captureTime } == [3_000])
}

func testAnImageThatWasSentIsNotMadeAgainUnlessTheViewerNeedsSomethingOtherThanADelta() {
  let session = FakeSession()
  session.begins = [.key]
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  session.micros = 7_000
  pump.captured(Img(1))
  check(session.sent.count == 1)

  // Nothing changed and nothing is wanted: the frame clock only asks.
  session.calls = []
  session.micros = 20_000
  pump.tick()
  pump.tick()
  check(session.calls == ["pending", "pending"], "\(session.calls)")
  check(session.sent.count == 1)

  // The viewer lost a picture: the same image is made into a key frame, and it is still the
  // image that was captured at 7 ms.
  session.pending = true
  session.begins = [.key]
  pump.tick()
  check(encoder.encoded.map { $0.image } == [1, 1], "\(encoder.encoded)")
  check(encoder.encoded.map { $0.key } == [true, true])
  check(session.sent.map { $0.captureTime } == [7_000, 7_000], "\(session.sent.map { $0.captureTime })")
  check(session.violations.isEmpty)
}

func testTheFrameClockDoesNothingBeforeTheFirstImageComes() {
  let session = FakeSession()
  let pump = FramePump(session: session, encoder: FakeEncoder())
  pump.tick()
  pump.tick()
  check(session.calls.isEmpty, "\(session.calls)")
  check(!pump.over)
}

func testAPictureThatIsStaleIsAskedForAgainOnTheNextTick() {
  let session = FakeSession()
  session.begins = [.key, .key]
  session.sends = [.stale, .sent]
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  pump.captured(Img(1))
  check(session.sent.count == 1 && !pump.over)
  pump.tick()
  check(session.sent.count == 2, "\(session.sent.count) pictures")
  check(encoder.encoded.map { $0.image } == [1, 1])
  // Now it is sent: the next tick asks only whether something is wanted.
  session.calls = []
  pump.tick()
  check(session.calls == ["pending"], "\(session.calls)")
  check(session.ended.isEmpty && session.violations.isEmpty)
}

func testAPictureThatTheEncoderCannotMakeIsGivenUpOnAndTheEncoderIsMadeAnew() {
  let session = FakeSession()
  session.begins = [.key, .key]
  let encoder = FakeEncoder()
  encoder.failures = [true, false]
  let pump = FramePump(session: session, encoder: encoder)
  pump.captured(Img(1))
  check(session.calls.contains("cancel") && session.sent.isEmpty, "\(session.calls)")
  check(encoder.resets == 1)
  pump.tick()
  check(session.sent.count == 1, "the next try made a picture")
  check(session.violations.isEmpty, "\(session.violations)")
  check(session.ended.isEmpty)
}

func testASessionEndsWithASentenceWhenPicturesFailTooOftenInARow() {
  let session = FakeSession()
  session.begins = [.key, .key, .key, .key]
  let encoder = FakeEncoder()
  encoder.failures = [true, true, true]
  // Three in a row is what the pump puts up with unless it is told otherwise.
  let pump = FramePump(session: session, encoder: encoder)
  pump.captured(Img(1))
  pump.tick()
  check(!pump.over && session.ended.isEmpty, "ended after two failures")
  pump.tick()
  check(pump.over, "not over after three failures")
  check(session.ended == ["This device could not encode the window."], "\(session.ended)")
  check(encoder.resets == 2, "the encoder was made anew \(encoder.resets) times")
  // Nothing is asked after the end.
  session.calls = []
  pump.captured(Img(2))
  pump.tick()
  check(session.calls.isEmpty, "\(session.calls)")
  check(session.violations.isEmpty)
}

func testAStalePictureIsNotAFailure() {
  let session = FakeSession()
  session.begins = Array(repeating: .key, count: 5)
  session.sends = [.stale, .stale, .stale, .refused, .sent]
  let pump = FramePump(session: session, encoder: FakeEncoder(), mostFailures: 3)
  pump.captured(Img(1))
  pump.tick()
  pump.tick()
  check(session.sent.count == 3 && !pump.over, "three stale pictures")
  // The first picture that fails is the first failure, however many were stale before it.
  pump.tick()
  check(!pump.over && session.ended.isEmpty, "\(session.ended)")
  pump.tick()
  check(session.sent.count == 5 && !pump.over && session.violations.isEmpty)
}

func testAPictureThatWorksStartsTheCountOfFailuresAgain() {
  let session = FakeSession()
  session.begins = Array(repeating: .key, count: 6)
  let encoder = FakeEncoder()
  encoder.failures = [true, true, false, true, true]
  let pump = FramePump(session: session, encoder: encoder, mostFailures: 3)
  pump.captured(Img(1))
  pump.tick()
  pump.tick()
  check(session.sent.count == 1, "the third try worked")
  // The window changes again, and two more fail: still not three in a row.
  pump.captured(Img(2))
  pump.tick()
  check(!pump.over && session.ended.isEmpty, "\(session.ended)")
}

func testAnEncoderThatCannotBeMadeAnewEndsTheSession() {
  let session = FakeSession()
  session.begins = [.key]
  let encoder = FakeEncoder()
  encoder.failures = [true]
  encoder.resetFails = true
  let pump = FramePump(session: session, encoder: encoder)
  pump.captured(Img(1))
  check(pump.over)
  check(session.ended == ["This device could not encode the window."], "\(session.ended)")
  check(session.violations.isEmpty, "\(session.violations)")
}

func testPicturesThatAreTooLargeOrNotTakenEndTheSessionWhenTheyRepeat() {
  for (answer, sentence) in [
    (SendAnswer.tooLarge, "The window is too detailed to send."),
    (SendAnswer.refused, "This device could not send the picture of the window."),
  ] {
    let session = FakeSession()
    session.begins = Array(repeating: .key, count: 3)
    session.sends = [answer, answer, answer]
    let pump = FramePump(session: session, encoder: FakeEncoder(), mostFailures: 3)
    pump.captured(Img(1))
    pump.tick()
    check(!pump.over, "\(answer): over after two")
    pump.tick()
    check(pump.over && session.ended == [sentence], "\(answer): \(session.ended)")
    check(session.violations.isEmpty)
  }
}

func testATickAsksOnceAboutAnImageThatWasNotMadeAndNotAlsoAboutALostPicture() {
  let session = FakeSession()
  session.pending = true
  let encoder = FakeEncoder()
  let pump = FramePump(session: session, encoder: encoder)
  session.begins = [.notNow]
  pump.captured(Img(1))
  // The viewer lost a picture too, but the image has not been made yet: one question to the
  // library, and a picture if it wants one, not a second question about the lost picture.
  session.calls = []
  session.begins = [.notNow, .notNow]
  pump.tick()
  check(session.calls == ["begin"], "\(session.calls)")
  session.calls = []
  session.begins = [.delta, .key]
  pump.tick()
  check(session.calls == ["begin", "send"], "\(session.calls)")
  check(encoder.encoded.map { $0.key } == [false], "\(encoder.encoded)")
  check(session.violations.isEmpty)
}

func testASessionThatIsOverStopsTheMakingOfPictures() {
  // The library says so to begin.
  do {
    let session = FakeSession()
    session.begins = [.over]
    let encoder = FakeEncoder()
    let pump = FramePump(session: session, encoder: encoder)
    pump.captured(Img(1))
    check(pump.over && encoder.encoded.isEmpty)
    session.calls = []
    pump.captured(Img(2))
    pump.tick()
    check(session.calls.isEmpty, "\(session.calls)")
    check(session.ended.isEmpty, "a session that is over is not ended again")
  }
  // The library says so to send.
  do {
    let session = FakeSession()
    session.begins = [.key]
    session.sends = [.over]
    let pump = FramePump(session: session, encoder: FakeEncoder())
    pump.captured(Img(1))
    check(pump.over && session.ended.isEmpty)
  }
  // The clock says so.
  do {
    let session = FakeSession()
    session.alive = false
    let pump = FramePump(session: session, encoder: FakeEncoder())
    pump.captured(Img(1))
    check(pump.over && session.calls == ["clock"], "\(session.calls)")
  }
  // The question of the frame clock says so.
  do {
    let session = FakeSession()
    session.begins = [.key]
    let pump = FramePump(session: session, encoder: FakeEncoder())
    pump.captured(Img(1))
    session.pending = nil
    pump.tick()
    check(pump.over && session.ended.isEmpty)
  }
}

func testTheImageThatWasKeptIsLetGoOfWhenThePumpStops() {
  let before = Img.alive
  let session = FakeSession()
  session.begins = [.key]
  let pump = FramePump(session: session, encoder: FakeEncoder())
  pump.captured(Img(1))
  check(Img.alive == before + 1, "alive \(Img.alive - before)")
  pump.stop()
  check(Img.alive == before, "the image is still held: \(Img.alive - before)")
  check(pump.over)
  pump.captured(Img(2))
  check(Img.alive == before, "an image that comes after the stop is kept")
}

func testEveryBeginIsAnsweredByOneSendOrOneCancelWhateverHappens() {
  let session = FakeSession()
  session.begins = [.key, .delta, .notNow, .key, .delta, .delta, .key, .delta, .key, .delta]
  session.sends = [.sent, .stale, .sent, .tooLarge, .sent, .refused, .sent]
  let encoder = FakeEncoder()
  encoder.failures = [false, false, true, false, false, false, true, false, false]
  let pump = FramePump(session: session, encoder: encoder, mostFailures: 100)
  for number in 1...12 {
    session.pending = number % 2 == 0
    pump.captured(Img(number))
    check(!session.announced, "a picture is announced after image \(number)")
    pump.tick()
    check(!session.announced, "a picture is announced after tick \(number)")
  }
  check(session.violations.isEmpty, "\(session.violations)")
  let answers = session.calls.filter { $0 == "send" || $0 == "cancel" }.count
  check(session.announcements >= 8 && session.announcements == answers, "\(session.calls)")
}

// MARK: - What the person is told when the capture stops

func testTheSentenceForAStoppedCaptureDependsOnWhyItStopped() {
  let domain = "com.apple.ScreenCaptureKit.SCStreamErrorDomain"
  // The numbers of SCStreamError.Code in Apple's SCError.h, written out here so that a number in the
  // code which is not the system's is noticed.
  check(CaptureErrorCode.userDeclined == -3801 && CaptureErrorCode.noCaptureSource == -3815)
  check(CaptureErrorCode.userStopped == -3817 && CaptureErrorCode.systemStopped == -3821)
  // The person pressed Stop where the system shows that a window is shared: they know.
  check(captureStoppedSentence(domain: domain, code: CaptureErrorCode.userStopped) == nil)

  let declined = captureStoppedSentence(domain: domain, code: CaptureErrorCode.userDeclined)
  check(
    declined?.contains("Screen & System Audio Recording") == true,
    "\(String(describing: declined))")
  check(
    captureStoppedSentence(domain: domain, code: CaptureErrorCode.noCaptureSource)
      == "The window is not there any more.")
  check(
    captureStoppedSentence(domain: domain, code: CaptureErrorCode.systemStopped)
      == "The system stopped sharing the window.")
  // Any other number is told in general words, and none is left without a sentence.
  for code in [-3802, -3811, -3816, 0, 1, Int.min, Int.max] {
    check(
      captureStoppedSentence(domain: domain, code: code) == "The system stopped capturing the window.",
      "\(code)")
  }
}

func testACodeOfAnotherDomainIsNeverTakenForOneThatThePersonCaused() {
  // The number that means "the person pressed Stop" means something else in another domain, and an
  // error that is not understood is told: a silent end would leave the person to wonder.
  for domain in ["NSPOSIXErrorDomain", "", "com.apple.coremedia", "SCStreamErrorDomainX"] {
    for code in [CaptureErrorCode.userStopped, CaptureErrorCode.userDeclined, 0, 1] {
      check(
        captureStoppedSentence(domain: domain, code: code)
          == "The system stopped capturing the window.",
        "\(domain) \(code)")
    }
  }
}

func testEverySentenceForAStoppedCaptureFitsInTheLibrarysMessage() {
  let domain = "com.apple.ScreenCaptureKit.SCStreamErrorDomain"
  let codes = [
    CaptureErrorCode.userDeclined, CaptureErrorCode.noCaptureSource,
    CaptureErrorCode.systemStopped, 0,
  ]
  for code in codes {
    guard let sentence = captureStoppedSentence(domain: domain, code: code) else {
      check(false, "\(code) has no sentence")
      continue
    }
    check(
      sentence.utf8.count <= Int(BATON_MESSAGE_MAX), "\(code): \(sentence.utf8.count) bytes")
    check(!sentence.isEmpty && sentence.last == ".", "\(code): \(sentence)")
  }
}
